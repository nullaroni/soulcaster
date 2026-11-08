//! A look at the Machine for Drift, and what a person decides about what it finds.
//!
//! The look changes nothing and sends nothing. It has no password and no way to get one, it never
//! writes the Machine Record, and every command it issues only reads. A command that can't answer
//! is "could not check", which is never a row and never a reason to act.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde_json::Number;
use serde_json::Value;
use soulcaster_core::catalog::{Catalog, Lifecycle};
use soulcaster_core::command::{Command, CommandEnd, CommandRunner};
use soulcaster_core::config::{Config, SettingValue};
use soulcaster_core::drift::{
    Facts, Keg, Report, Reviewed, Seen, Uninstall, current_id, kegs_of, plain_package, removal_of,
    report,
};
use soulcaster_core::lifecycle::{Assurance, SkipReason};
use soulcaster_core::plan::Plan;
use soulcaster_core::recipe::{
    BuiltinProbe, BuiltinStore, InstallRecipe, Probe, Recipe, SettingRecipe, Store, StoredType,
    ValueType,
};

use crate::act;
use crate::record::{LastApplied, MachineRecord};
use crate::run::{
    Clock, EngineError, ReadBack, compare_defaults, dock_path, dock_paths, git_key,
    link_on_the_way, lock_run, parse_array, reason_label, rendered_git,
};

/// A check that takes this long isn't going to answer, and the person is looking at a window.
const LOOK_LIMIT: Duration = Duration::from_secs(30);

const APPLE_STUBS: [&str; 2] = ["/usr/bin/git", "/usr/bin/xcodebuild"];

enum Answer {
    Yes(String),
    No,
    Unknown,
}

/// What one store holds, kept as read so each candidate value can be compared the way a Run does.
enum Stored {
    Text(String),
    Unset,
    Unknown,
}

struct Look<'a> {
    runner: &'a dyn CommandRunner,
    home: &'a Path,
    probes: BTreeMap<String, Option<bool>>,
}

impl Look<'_> {
    fn ask(&self, command: Command) -> Answer {
        match self
            .runner
            .run(&command.stall_limit(LOOK_LIMIT), &mut |_, _| {})
        {
            Ok(output) if output.is_success() => Answer::Yes(output.stdout_text()),
            Ok(output) if matches!(output.end, CommandEnd::Exited(_)) => Answer::No,
            _ => Answer::Unknown,
        }
    }

    /// `Some(true)` passed, `Some(false)` failed, `None` can't tell.
    fn probe(&mut self, probe: &Probe) -> Option<bool> {
        let key = serde_json::to_string(probe).unwrap_or_default();
        if let Some(known) = self.probes.get(&key) {
            return *known;
        }
        // Apple's stubs open an install dialog when the developer tools aren't there, and a look
        // that happens whenever the window is focused must never put up a dialog.
        let needs_tools = act::probe_commands(self.home, probe)
            .iter()
            .any(|command| APPLE_STUBS.contains(&command.program.to_string_lossy().as_ref()));
        let verdict = if needs_tools && !self.developer_tools_here() {
            None
        } else {
            self.probe_now(probe)
        };
        self.probes.insert(key, verdict);
        verdict
    }

    fn probe_now(&self, probe: &Probe) -> Option<bool> {
        if matches!(
            probe,
            Probe::Unknowable { .. }
                | Probe::Builtin {
                    name: BuiltinProbe::GithubSignedIn
                }
        ) {
            return None;
        }
        let commands = act::probe_commands(self.home, probe);
        if commands.is_empty() {
            return None;
        }
        for command in commands {
            let spawned = self
                .runner
                .run(&command.clone().stall_limit(LOOK_LIMIT), &mut |_, _| {});
            match spawned {
                Ok(output) if output.is_success() => {}
                Ok(output) if matches!(output.end, CommandEnd::Exited(_)) => return Some(false),
                Ok(_) => return None,
                // A tool that isn't there can't print its version. Anything else stopping it
                // from starting says nothing about whether it's there.
                Err(error)
                    if matches!(probe, Probe::ToolVersion { .. })
                        && error.message.contains("os error 2") =>
                {
                    return Some(false);
                }
                Err(_) => return None,
            }
        }
        Some(true)
    }

    fn installed(&mut self, install: &InstallRecipe) -> Seen {
        let mut unknown = false;
        for probe in &install.checks {
            match self.probe(probe) {
                Some(false) => return Seen::Absent,
                None => unknown = true,
                Some(true) => {}
            }
        }
        if unknown {
            Seen::CantTell
        } else {
            Seen::Present(Value::Bool(true))
        }
    }

    fn developer_tools_here(&mut self) -> bool {
        // `/usr/bin/git` is Apple's stub until these exist, and running it opens an install dialog.
        self.probe(&Probe::Builtin {
            name: BuiltinProbe::CommandLineTools,
        }) == Some(true)
    }

    fn read(&mut self, store: &Store) -> Stored {
        match store {
            Store::Defaults { .. } => {
                let Some(command) = act::defaults_read(self.home, store) else {
                    return Stored::Unknown;
                };
                match self.ask(command) {
                    Answer::Yes(text) => Stored::Text(text),
                    Answer::No => Stored::Unset,
                    Answer::Unknown => Stored::Unknown,
                }
            }
            Store::GitConfig { key } => {
                if link_on_the_way(self.home, Path::new(".gitconfig"))
                    || !self.developer_tools_here()
                {
                    return Stored::Unknown;
                }
                let mut command = act::git_command(self.home);
                command.args = vec![
                    "config".into(),
                    "--file".into(),
                    act::git_file(self.home).to_string_lossy().into_owned(),
                    "--get".into(),
                    git_key(key).into(),
                ];
                match self.ask(command) {
                    Answer::Yes(text) if !text.trim().is_empty() => {
                        Stored::Text(text.trim().to_string())
                    }
                    Answer::Yes(_) | Answer::No => Stored::Unset,
                    Answer::Unknown => Stored::Unknown,
                }
            }
            Store::Builtin {
                name: BuiltinStore::LibraryVisible,
            } => match self.ask(act::library_read(self.home)) {
                Answer::Yes(text) => Stored::Text(text),
                _ => Stored::Unknown,
            },
            Store::Builtin {
                name: BuiltinStore::DockTiles,
            } => match self.ask(act::dock_list(self.home)) {
                Answer::Yes(text) => Stored::Text(text),
                _ => Stored::Unknown,
            },
            Store::Builtin { .. } => Stored::Unknown,
        }
    }

    fn setting(
        &mut self,
        setting: &SettingRecipe,
        config: Option<&SettingValue>,
        last: Option<&Value>,
    ) -> Seen {
        let mut readings = Vec::new();
        for store in &setting.stores {
            match self.read(store) {
                Stored::Unknown => return Seen::CantTell,
                reading => readings.push((store, reading)),
            }
        }
        let Some((first, first_reading)) = readings.first() else {
            return Seen::CantTell;
        };
        let last_value =
            last.and_then(|value| serde_json::from_value::<SettingValue>(value.clone()).ok());
        // The Run's own comparison decides what counts as the same, so a value it wouldn't touch
        // is never reported as changed, whatever its spelling on this Mac.
        for candidate in config.into_iter().chain(last_value.iter()) {
            if readings
                .iter()
                .all(|(store, reading)| holds(setting, store, reading, candidate))
            {
                return match serde_json::to_value(candidate) {
                    Ok(value) => Seen::Present(value),
                    Err(_) => Seen::CantTell,
                };
            }
        }
        let decoded = match first_reading {
            Stored::Unset => {
                return match &setting.unset_means {
                    Some(value) => Seen::Present(value.clone()),
                    None => Seen::Absent,
                };
            }
            Stored::Text(text) => decode(setting, first, text),
            Stored::Unknown => None,
        };
        let Some(decoded) = decoded else {
            return Seen::CantTell;
        };
        // The first store agrees with a value the others don't: one thing can't be said of it.
        let looks_settled = config
            .and_then(|value| serde_json::to_value(value).ok())
            .iter()
            .chain(last)
            .any(|candidate| *candidate == decoded);
        if looks_settled {
            return Seen::CantTell;
        }
        Seen::Present(decoded)
    }

    fn min_macos_unmet(
        &mut self,
        install: &InstallRecipe,
        macos: &mut Option<Option<String>>,
    ) -> bool {
        let Some(minimum) = &install.min_macos else {
            return false;
        };
        let version = macos.get_or_insert_with(|| match self.ask(act::sw_vers(self.home)) {
            Answer::Yes(text) => Some(text.trim().to_string()),
            _ => None,
        });
        version
            .as_deref()
            .is_some_and(|machine| act::version_below(machine, minimum))
    }

    fn managed(&self, setting: &SettingRecipe) -> bool {
        setting.stores.iter().any(|store| {
            let Store::Defaults { domain, key, .. } = store else {
                return false;
            };
            act::managed_preference_reads(self.home, domain.as_str(), key)
                .into_iter()
                .any(|command| matches!(self.ask(command), Answer::Yes(text) if !text.trim().is_empty()))
        })
    }

    fn cant_apply(&mut self, recipe: &Recipe, macos: &mut Option<Option<String>>) -> bool {
        match recipe {
            Recipe::Install(install) => self.min_macos_unmet(install, macos),
            Recipe::Setting(setting) => {
                setting.is_broken()
                    || self.managed(setting)
                    || (setting
                        .stores
                        .iter()
                        .any(|store| matches!(store, Store::GitConfig { .. }))
                        && link_on_the_way(self.home, Path::new(".gitconfig")))
            }
            Recipe::MachineValues(_) => false,
        }
    }

    /// What Homebrew installed by name. `None` means the question couldn't be asked. Homebrew
    /// starts by looking for git, and with no developer tools it installs one, so it isn't even
    /// started without them.
    fn homebrew_lists(&mut self) -> Option<Vec<String>> {
        if !self.developer_tools_here() {
            return None;
        }
        let read = |args: &[&str]| match self.ask(act::brew_read(self.home, args)) {
            Answer::Yes(text) => Some(text),
            _ => None,
        };
        let formulae = read(&["leaves", "--installed-on-request"])?;
        let casks = read(&["list", "--cask", "-1"])?;
        let keys = |text: String, prefix: &str| {
            text.lines()
                .map(str::trim)
                .filter(|name| plain_package(name))
                .map(|name| format!("{prefix}:{name}"))
                .collect::<Vec<_>>()
        };
        let mut out = keys(formulae, "formula");
        out.extend(keys(casks, "cask"));
        Some(out)
    }
}

fn holds(
    setting: &SettingRecipe,
    store: &Store,
    reading: &Stored,
    candidate: &SettingValue,
) -> bool {
    match (reading, store) {
        (Stored::Unset, _) => setting
            .unset_means
            .as_ref()
            .is_some_and(|unset| serde_json::to_value(candidate).ok().as_ref() == Some(unset)),
        (Stored::Text(text), Store::Defaults { .. }) => {
            matches!(compare_defaults(store, candidate, text), ReadBack::Match)
        }
        (Stored::Text(text), Store::GitConfig { .. }) => *text == rendered_git(candidate),
        (
            Stored::Text(text),
            Store::Builtin {
                name: BuiltinStore::LibraryVisible,
            },
        ) => matches!(candidate, SettingValue::Bool(true)) != text.contains("hidden"),
        (
            Stored::Text(text),
            Store::Builtin {
                name: BuiltinStore::DockTiles,
            },
        ) => {
            let SettingValue::List(wanted) = candidate else {
                return false;
            };
            let got = dock_paths(text);
            got.len() == wanted.len()
                && got
                    .iter()
                    .zip(wanted)
                    .all(|(got, want)| dock_path(got) == dock_path(want))
        }
        _ => false,
    }
}

/// The Config's token for what's stored. Several tokens stored the same way can't be told apart.
fn token_for(
    encode: &BTreeMap<String, Value>,
    matches: impl Fn(&Value) -> bool,
) -> Option<Option<String>> {
    let tokens: Vec<&String> = encode
        .iter()
        .filter(|(_, stored)| matches(stored))
        .map(|(token, _)| token)
        .collect();
    match tokens.as_slice() {
        [] => Some(None),
        [one] => Some(Some((*one).clone())),
        _ => None,
    }
}

fn typed(setting: &SettingRecipe, text: &str) -> Option<Value> {
    match setting.value_type {
        ValueType::Boolean => match text {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        ValueType::Integer => text.parse::<i64>().ok().map(|n| Value::Number(n.into())),
        ValueType::String => Some(Value::String(text.to_string())),
        ValueType::StringList => None,
    }
}

/// What a store holds, in the Config's terms. `None` when that can't be said honestly.
fn decode(setting: &SettingRecipe, store: &Store, text: &str) -> Option<Value> {
    match store {
        Store::Defaults {
            stored_type,
            encode,
            ..
        } => {
            let text = text.trim();
            match stored_type {
                StoredType::Bool => match text {
                    "1" | "true" => Some(Value::Bool(true)),
                    "0" | "false" => Some(Value::Bool(false)),
                    _ => None,
                },
                StoredType::Int | StoredType::Float => {
                    if let Some(map) = encode {
                        let token = token_for(map, |stored| match stored {
                            Value::Number(number) => number.to_string() == text,
                            _ => false,
                        })?;
                        if let Some(token) = token {
                            return typed(setting, &token);
                        }
                    }
                    if let Ok(number) = text.parse::<i64>() {
                        return Some(Value::Number(number.into()));
                    }
                    let number: f64 = text.parse().ok()?;
                    Number::from_f64(number).map(Value::Number)
                }
                StoredType::String => {
                    if let Some(map) = encode {
                        let token = token_for(map, |stored| stored.as_str() == Some(text))?;
                        if let Some(token) = token {
                            return typed(setting, &token);
                        }
                    }
                    Some(Value::String(text.to_string()))
                }
                StoredType::StringArray => {
                    let mut list = Vec::new();
                    for item in parse_array(text) {
                        let token = match encode {
                            Some(map) => {
                                token_for(map, |stored| stored.as_str() == Some(item.as_str()))?
                            }
                            None => None,
                        };
                        list.push(Value::String(token.unwrap_or(item)));
                    }
                    Some(Value::Array(list))
                }
                StoredType::Dict => None,
            }
        }
        Store::GitConfig { .. } => typed(setting, text.trim()),
        Store::Builtin {
            name: BuiltinStore::LibraryVisible,
        } => Some(Value::Bool(!text.contains("hidden"))),
        Store::Builtin {
            name: BuiltinStore::DockTiles,
        } => Some(Value::Array(
            dock_paths(text).into_iter().map(Value::String).collect(),
        )),
        Store::Builtin { .. } => None,
    }
}

fn recipe_for(catalog: &Catalog, plan: &Plan, id: &str) -> Option<Recipe> {
    if let Some(entry) = plan.entries.iter().find(|entry| entry.id == id) {
        return Some(entry.recipe.clone());
    }
    if let Some(entry) = catalog.entry(id) {
        return Some(entry.recipe.clone());
    }
    let (cask, name) = if let Some(name) = id.strip_prefix("formula:") {
        (false, name)
    } else {
        (true, id.strip_prefix("cask:")?)
    };
    if !plain_package(name) {
        return None;
    }
    let probe = if cask {
        Probe::BrewCask {
            name: name.to_string(),
        }
    } else {
        Probe::BrewFormula {
            name: name.to_string(),
        }
    };
    Some(Recipe::package_check(id, probe))
}

fn seen_for(
    look: &mut Look<'_>,
    recipe: &Recipe,
    config: Option<&SettingValue>,
    last: Option<&Value>,
) -> Option<Seen> {
    match recipe {
        Recipe::Install(install) => Some(look.installed(install)),
        Recipe::Setting(setting) => Some(look.setting(setting, config, last)),
        Recipe::MachineValues(_) => None,
    }
}

/// Looks at this Machine and says how it differs from `plan`. Changes nothing and sends nothing.
pub fn check_drift(
    runner: &dyn CommandRunner,
    home: &Path,
    catalog: &Catalog,
    plan: &Plan,
    config: &Config,
) -> Result<Report, EngineError> {
    let record = MachineRecord::load(home)?;
    let mut look = Look {
        runner,
        home,
        probes: BTreeMap::new(),
    };
    let last: BTreeMap<String, Value> = record
        .last_applied
        .iter()
        .map(|(id, applied)| (id.clone(), applied.value.clone()))
        .collect();
    let recorded: BTreeMap<String, &Value> = last
        .iter()
        .map(|(id, value)| (current_id(catalog, id), value))
        .collect();
    let always: BTreeSet<&str> = catalog.base_items().map(|entry| entry.id()).collect();
    let mut seen = BTreeMap::new();
    let mut unavailable: BTreeSet<String> = record.unavailable.keys().cloned().collect();
    let mut macos = None;
    for entry in &plan.entries {
        if always.contains(entry.id.as_str()) {
            continue;
        }
        if look.cant_apply(&entry.recipe, &mut macos) {
            unavailable.insert(entry.id.clone());
            continue;
        }
        let last = recorded.get(&entry.id).copied();
        if let Some(found) = seen_for(&mut look, &entry.recipe, entry.value.as_ref(), last) {
            seen.insert(entry.id.clone(), found);
        }
    }
    for (id, value) in &recorded {
        if seen.contains_key(id) || always.contains(id.as_str()) {
            continue;
        }
        let Some(recipe) = recipe_for(catalog, plan, id) else {
            continue;
        };
        if let Some(found) = seen_for(&mut look, &recipe, None, Some(value)) {
            seen.insert(id.clone(), found);
        }
    }
    // On this Mac and in the Catalog, or named to Homebrew. Anything else is Untracked, not Drift.
    let candidates: Vec<Recipe> = catalog
        .entries()
        .filter(|entry| !matches!(entry.lifecycle, Lifecycle::Retired(_)))
        .filter(|entry| !always.contains(entry.id()) && !seen.contains_key(entry.id()))
        .filter(|entry| matches!(entry.recipe, Recipe::Install(_)))
        .map(|entry| entry.recipe.clone())
        .collect();
    for recipe in candidates {
        let Recipe::Install(install) = &recipe else {
            continue;
        };
        if look.installed(install) == Seen::Present(Value::Bool(true)) {
            seen.insert(recipe.id().to_string(), Seen::Present(Value::Bool(true)));
        }
    }
    // Homebrew lists a tap's cask by its short name, so what's known is matched as a keg. The
    // entry that owns the keg has already been looked at, and speaks for it.
    let known: BTreeSet<Keg> = catalog
        .entries()
        .flat_map(|entry| kegs_of(entry.id(), Some(&entry.recipe)))
        .chain(
            plan.entries
                .iter()
                .flat_map(|entry| kegs_of(&entry.id, Some(&entry.recipe))),
        )
        .chain(recorded.keys().flat_map(|id| kegs_of(id, None)))
        .collect();
    for key in look.homebrew_lists().unwrap_or_default() {
        if kegs_of(&key, None).iter().any(|keg| known.contains(keg)) {
            continue;
        }
        seen.insert(key, Seen::Present(Value::Bool(true)));
    }
    Ok(report(&Facts {
        plan,
        catalog,
        config,
        seen: &seen,
        last: &last,
        unavailable: &unavailable,
        kept: &record.kept_here,
    }))
}

/// The record changes below all take the Run lock, so none can land in the middle of a Run.
fn with_record<T>(
    home: &Path,
    change: impl FnOnce(&mut MachineRecord) -> T,
) -> Result<T, EngineError> {
    let _lock = lock_run(home)?;
    let mut record = MachineRecord::load(home)?;
    let out = change(&mut record);
    record.save(home)?;
    Ok(out)
}

/// "Leave this Mac as it is and don't ask again." A Run then leaves it alone too.
pub fn leave_here(home: &Path, id: &str) -> Result<(), EngineError> {
    with_record(home, |record| {
        record.unavailable.insert(
            id.to_string(),
            reason_label(SkipReason::MarkedByPerson).to_string(),
        );
    })
}

/// "Keep it here and stop tracking it", and "Keep it on this Mac only".
pub fn keep_here(home: &Path, id: &str) -> Result<(), EngineError> {
    with_record(home, |record| {
        record.last_applied.remove(id);
        record.kept_here.insert(id.to_string());
    })
}

/// A save GitHub now has. Only this moves the last-applied record.
pub fn record_saved(
    home: &Path,
    saved: &[Reviewed],
    catalog_version: &str,
    clock: &dyn Clock,
) -> Result<(), EngineError> {
    with_record(home, |record| {
        for entry in saved {
            record.kept_here.remove(&entry.id);
            // Back in the Config by the person's own choice, so "leave it alone" is over.
            if record.unavailable.get(&entry.id).map(String::as_str)
                == Some(reason_label(SkipReason::MarkedByPerson))
            {
                record.unavailable.remove(&entry.id);
            }
            match &entry.write {
                None => {
                    record.last_applied.remove(&entry.id);
                }
                Some(value) => {
                    let earlier = record.last_applied.get(&entry.id);
                    let applied = LastApplied {
                        value: value.clone(),
                        applied_at: clock.now_ms(),
                        run_id: "saved-to-config".to_string(),
                        catalog_version: catalog_version.to_string(),
                        macos_version: earlier.and_then(|e| e.macos_version.clone()),
                        assurance: earlier.map_or(Assurance::Stored, |e| e.assurance),
                        prior_value: earlier.and_then(|e| e.prior_value.clone()),
                    };
                    record.last_applied.insert(entry.id.clone(), applied);
                }
            }
        }
        record
            .waiting_saves
            .retain(|waiting| !saved.iter().any(|entry| entry.id == waiting.id));
    })
}

/// Saves that are still only on this Mac, kept until they're sent or discarded.
pub fn waiting_saves(home: &Path) -> Result<Vec<Reviewed>, EngineError> {
    Ok(MachineRecord::load(home)?.waiting_saves)
}

pub fn set_waiting_saves(home: &Path, waiting: &[Reviewed]) -> Result<(), EngineError> {
    with_record(home, |record| record.waiting_saves = waiting.to_vec())
}

/// The person answered the separate confirmation. Only the screen that showed it makes one.
#[derive(Debug)]
pub struct RemovalConfirmed(());

impl RemovalConfirmed {
    pub fn by_the_person() -> Self {
        Self(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removal {
    Removed,
    /// Nothing ran. It isn't something this can take off this Mac.
    Refused,
    /// Homebrew said no, or it's still there afterwards. The entry stays tracked.
    Failed,
}

/// Uninstalls entries the Config dropped. It checks each one again here, whatever the screen
/// showed: never something the Config still wants, never without a last-applied record, never
/// something always installed, and only on this Mac. It never asks for the password.
pub fn remove_from_mac(
    runner: &dyn CommandRunner,
    home: &Path,
    catalog: &Catalog,
    plan: &Plan,
    ids: &[String],
    _confirmed: RemovalConfirmed,
) -> Result<BTreeMap<String, Removal>, EngineError> {
    let _lock = lock_run(home)?;
    let mut record = MachineRecord::load(home)?;
    let mut look = Look {
        runner,
        home,
        probes: BTreeMap::new(),
    };
    let mut out = BTreeMap::new();
    for id in ids {
        let current = current_id(catalog, id);
        let recorded = record
            .last_applied
            .keys()
            .find(|key| current_id(catalog, key) == current)
            .cloned();
        let uninstall = removal_of(plan, catalog, &current);
        let recipe = recipe_for(catalog, plan, &current);
        let (Some(recorded), Some(uninstall), Some(Recipe::Install(install))) =
            (recorded, uninstall, recipe)
        else {
            out.insert(id.clone(), Removal::Refused);
            continue;
        };
        // Homebrew installs git for itself when the developer tools are gone.
        if !look.developer_tools_here() {
            out.insert(id.clone(), Removal::Refused);
            continue;
        }
        if look.installed(&install) != Seen::Present(Value::Bool(true)) {
            out.insert(id.clone(), Removal::Refused);
            continue;
        }
        let (name, cask) = match &uninstall {
            Uninstall::Formula(name) => (name.as_str(), false),
            Uninstall::Cask(name) => (name.as_str(), true),
        };
        let ran = runner.run(&act::brew_uninstall(home, name, cask), &mut |_, _| {});
        look.probes.clear();
        let gone = look.installed(&install) == Seen::Absent;
        if ran.is_ok_and(|output| output.is_success()) && gone {
            record.last_applied.remove(&recorded);
            record.outcomes.remove(&recorded);
            record.save(home)?;
            out.insert(id.clone(), Removal::Removed);
        } else {
            out.insert(id.clone(), Removal::Failed);
        }
    }
    Ok(out)
}
