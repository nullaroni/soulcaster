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
use soulcaster_core::command::{Command, CommandEnd, CommandOutput, CommandRunner};
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
    Clock, EngineError, ReadBack, compare_defaults, dock_path, dock_paths, git_key, library_hidden,
    link_on_the_way, lock_run, parse_array, reason_label, rendered_git,
};

/// A check that takes this long isn't going to answer, and the person is looking at a window.
const LOOK_LIMIT: Duration = Duration::from_secs(30);

const APPLE_STUBS: [&str; 2] = ["/usr/bin/git", "/usr/bin/xcodebuild"];

enum Answer {
    Yes(String),
    No(i32),
    Unknown,
}

/// What one store's contents say, in the Config's terms.
enum Decoded {
    Says(Value),
    /// Holds nothing about this value, such as a companion key that's the same for every choice.
    Silent,
    /// Holds something the Catalog has no name for, so it isn't what the Config says.
    Foreign(Value),
    /// Holds something the Config has no way to state.
    Cannot,
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
            Ok(CommandOutput {
                end: CommandEnd::Exited(code),
                ..
            }) => Answer::No(code),
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
                        && error.message.ends_with("(os error 2)") =>
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
                    Answer::No(1) => Stored::Unset,
                    _ => Stored::Unknown,
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
                    // Exit 1 is a key that isn't set. Anything else says nothing about the key,
                    // and git prints the same empty line for `key` (true) and `key =` (false).
                    Answer::No(1) => Stored::Unset,
                    _ => Stored::Unknown,
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
        let stores: Vec<Store> = setting
            .stores
            .iter()
            .filter(|store| !macos_arranges(store))
            .map(|store| act::with_home(self.home, store))
            .collect();
        let mut readings = Vec::new();
        for store in &stores {
            match (self.read(store), companion_value(store)) {
                (Stored::Unknown, _) => return Seen::CantTell,
                // A companion left alone says nothing. One changed by hand, such as a hot corner
                // that needs a modifier key, means this isn't the choice it looks like.
                (Stored::Unset, Some(_)) => {}
                (Stored::Text(text), Some(usual)) => {
                    if text.trim() != usual {
                        return Seen::CantTell;
                    }
                }
                (reading, None) => readings.push((store, reading)),
            }
        }
        if readings.is_empty() {
            return Seen::CantTell;
        }
        let last_value =
            last.and_then(|value| serde_json::from_value::<SettingValue>(value.clone()).ok());
        // The Run's own comparison decides what counts as the same, so a value it wouldn't touch
        // is never reported as changed, whatever its spelling on this Mac.
        for candidate in config.into_iter().chain(last_value.iter()) {
            if readings
                .iter()
                .all(|(store, reading)| holds(self.home, setting, store, reading, candidate))
            {
                return match serde_json::to_value(candidate) {
                    Ok(value) => Seen::Present(value),
                    Err(_) => Seen::CantTell,
                };
            }
        }
        // Every store that has something to say must say the same thing.
        let mut said: Option<Option<Value>> = None;
        let mut foreign = None;
        for (store, reading) in &readings {
            let this = match reading {
                Stored::Unset => setting.unset_means.clone(),
                Stored::Text(text) => match decode(self.home, setting, store, text) {
                    Decoded::Says(value) => Some(value),
                    Decoded::Silent => continue,
                    Decoded::Foreign(value) => {
                        foreign.get_or_insert(value);
                        continue;
                    }
                    Decoded::Cannot => return Seen::CantTell,
                },
                Stored::Unknown => return Seen::CantTell,
            };
            match &said {
                None => said = Some(this),
                Some(earlier) if *earlier == this => {}
                Some(_) => return Seen::CantTell,
            }
        }
        let decoded = match (foreign, said) {
            (Some(value), _) => value,
            (None, None) => return Seen::CantTell,
            (None, Some(None)) => return Seen::Absent,
            (None, Some(Some(value))) => value,
        };
        // Stores that read as a candidate the Run's comparison turned down: nothing is certain.
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
        let casks = read(&["list", "--cask", "--full-name"])?;
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

/// A menu bar item's position, which macOS sets itself as soon as the item is placed or dragged.
fn macos_arranges(store: &Store) -> bool {
    matches!(store, Store::Defaults { key, .. } if key.starts_with("NSStatusItem Preferred Position"))
}

/// For a key written alongside the real one, the same for every choice: what it always holds.
fn companion_value(store: &Store) -> Option<String> {
    let Store::Defaults {
        encode: Some(encode),
        ..
    } = store
    else {
        return None;
    };
    let mut stored = encode.values();
    let first = stored.next()?;
    if encode.len() < 2 || !stored.all(|value| value == first) {
        return None;
    }
    Some(match first {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    })
}

/// A path under the home folder, as the `~` a Config can hold.
fn with_tilde(home: &Path, path: String) -> String {
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path,
    }
}

fn git_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn holds(
    home: &Path,
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
        (Stored::Text(text), Store::GitConfig { .. }) => match candidate {
            SettingValue::Bool(flag) => git_bool(text) == Some(*flag),
            // A shell expands `~` before git stores it, so both spellings are this file.
            _ => {
                let rendered = rendered_git(candidate);
                *text == rendered || *text == *act::expand_home(home, &rendered).to_string_lossy()
            }
        },
        (
            Stored::Text(text),
            Store::Builtin {
                name: BuiltinStore::LibraryVisible,
            },
        ) => matches!(candidate, SettingValue::Bool(true)) != library_hidden(text),
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
                && got.iter().zip(wanted).all(|(got, want)| {
                    dock_path(got) == dock_path(&act::expand_home(home, want).to_string_lossy())
                })
        }
        _ => false,
    }
}

/// The Config's token for what's stored. `Err` when several tokens are stored the same way, so
/// the stored value can't pick one.
fn token_for(
    encode: &BTreeMap<String, Value>,
    matches: impl Fn(&Value) -> bool,
) -> Result<Option<String>, ()> {
    let tokens: Vec<&String> = encode
        .iter()
        .filter(|(_, stored)| matches(stored))
        .map(|(token, _)| token)
        .collect();
    match tokens.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some((*one).clone())),
        _ => Err(()),
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

fn says(value: Option<Value>) -> Decoded {
    value.map_or(Decoded::Cannot, Decoded::Says)
}

/// What a store holds, in the Config's terms.
fn decode(home: &Path, setting: &SettingRecipe, store: &Store, text: &str) -> Decoded {
    match store {
        Store::Defaults {
            stored_type,
            encode,
            ..
        } => {
            let text = text.trim();
            match stored_type {
                StoredType::Bool => {
                    let flag = match text {
                        "1" | "true" => true,
                        "0" | "false" => false,
                        _ => return Decoded::Cannot,
                    };
                    if let Some(map) = encode {
                        match token_for(map, |stored| stored.as_bool() == Some(flag)) {
                            Err(()) => return Decoded::Silent,
                            Ok(Some(token)) => return says(typed(setting, &token)),
                            Ok(None) => {}
                        }
                    }
                    Decoded::Says(Value::Bool(flag))
                }
                StoredType::Int | StoredType::Float => {
                    if let Some(map) = encode {
                        let stored_as = |stored: &Value| match stored {
                            Value::Number(number) => number.to_string() == text,
                            _ => false,
                        };
                        match token_for(map, stored_as) {
                            Err(()) => return Decoded::Silent,
                            Ok(Some(token)) => return says(typed(setting, &token)),
                            Ok(None) => {
                                return match text.parse::<i64>() {
                                    Ok(number) => Decoded::Foreign(Value::Number(number.into())),
                                    Err(_) => Decoded::Cannot,
                                };
                            }
                        }
                    }
                    if let Ok(number) = text.parse::<i64>() {
                        return Decoded::Says(Value::Number(number.into()));
                    }
                    says(
                        text.parse::<f64>()
                            .ok()
                            .and_then(Number::from_f64)
                            .map(Value::Number),
                    )
                }
                StoredType::String => {
                    if let Some(map) = encode {
                        match token_for(map, |stored| stored.as_str() == Some(text)) {
                            Err(()) => return Decoded::Silent,
                            Ok(Some(token)) => return says(typed(setting, &token)),
                            Ok(None) => return Decoded::Foreign(Value::String(text.to_string())),
                        }
                    }
                    Decoded::Says(Value::String(text.to_string()))
                }
                StoredType::StringArray => {
                    let mut list = Vec::new();
                    for item in parse_array(text) {
                        let token = match encode {
                            Some(map) => {
                                match token_for(map, |stored| {
                                    stored.as_str() == Some(item.as_str())
                                }) {
                                    Ok(token) => token,
                                    Err(()) => return Decoded::Silent,
                                }
                            }
                            None => None,
                        };
                        list.push(Value::String(token.unwrap_or(item)));
                    }
                    Decoded::Says(Value::Array(list))
                }
                StoredType::Dict => {
                    let Some(map) = encode else {
                        return Decoded::Cannot;
                    };
                    // The same reading the Run's comparison makes of this dictionary.
                    let on = map.iter().filter(|(_, stored)| {
                        let key = stored.as_str().unwrap_or("");
                        text.contains(&format!("{key} = 1"))
                            || text.contains(&format!("{key} = true"))
                    });
                    Decoded::Says(Value::Array(
                        on.map(|(token, _)| Value::String(token.clone())).collect(),
                    ))
                }
            }
        }
        Store::GitConfig { .. } => match setting.value_type {
            ValueType::Boolean => says(git_bool(text.trim()).map(Value::Bool)),
            _ => says(typed(setting, &with_tilde(home, text.trim().to_string()))),
        },
        Store::Builtin {
            name: BuiltinStore::LibraryVisible,
        } => Decoded::Says(Value::Bool(!library_hidden(text))),
        Store::Builtin {
            name: BuiltinStore::DockTiles,
        } => {
            // A path under the home folder goes back to the `~` a Config can hold.
            Decoded::Says(Value::Array(
                dock_paths(text)
                    .into_iter()
                    .map(|path| Value::String(with_tilde(home, path)))
                    .collect(),
            ))
        }
        Store::Builtin { .. } => Decoded::Silent,
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
    let mut found = report(&Facts {
        plan,
        catalog,
        config,
        seen: &seen,
        last: &last,
        unavailable: &unavailable,
        kept: &record.kept_here,
    });
    // A removal starts Homebrew, which isn't started without the developer tools.
    if !look.developer_tools_here() {
        for row in &mut found.rows {
            row.removable = false;
        }
    }
    Ok(found)
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
        forget_applied(record, id);
        record.kept_here.insert(id.to_string());
    })
}

/// The record may hold this entry under a name from before a Catalog release.
fn same_entry(a: &str, b: &str) -> bool {
    let catalog = Catalog::shipped();
    current_id(catalog, a) == current_id(catalog, b)
}

fn forget_applied(record: &mut MachineRecord, id: &str) {
    record.last_applied.retain(|key, _| !same_entry(key, id));
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
            record.kept_here.retain(|key| !same_entry(key, &entry.id));
            // Back in the Config by the person's own choice, so "leave it alone" is over.
            let marked = reason_label(SkipReason::MarkedByPerson);
            record
                .unavailable
                .retain(|key, reason| reason != marked || !same_entry(key, &entry.id));
            match &entry.write {
                None => forget_applied(record, &entry.id),
                Some(value) => {
                    let earlier = record
                        .last_applied
                        .iter()
                        .find(|(key, _)| same_entry(key, &entry.id))
                        .map(|(_, applied)| applied.clone());
                    let earlier = earlier.as_ref();
                    let applied = LastApplied {
                        value: value.clone(),
                        applied_at: clock.now_ms(),
                        run_id: "saved-to-config".to_string(),
                        catalog_version: catalog_version.to_string(),
                        macos_version: earlier.and_then(|e| e.macos_version.clone()),
                        assurance: earlier.map_or(Assurance::Stored, |e| e.assurance),
                        prior_value: earlier.and_then(|e| e.prior_value.clone()),
                    };
                    forget_applied(record, &entry.id);
                    record.last_applied.insert(entry.id.clone(), applied);
                }
            }
        }
        record
            .waiting_saves
            .retain(|waiting| !saved.iter().any(|entry| entry.id == waiting.id));
    })
}

/// The Config this Mac follows, as it last read it.
pub fn set_followed(home: &Path, followed: Value) -> Result<(), EngineError> {
    with_record(home, |record| record.follows = Some(followed))
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
