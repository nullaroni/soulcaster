//! Drift: one row per entry, decided from three things: the Config, the Machine, and what was
//! last applied here. Whoever changed is the side that differs from last-applied. This decides
//! what a row is. It changes nothing, and a missing line in the Config is never a removal by itself.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{Catalog, Lookup};
use crate::config::{Config, SettingValue};
use crate::lifecycle::SkipReason;
use crate::lint::{DEFAULT_CLONE_LOCATION, lint};
use crate::plan::{Plan, Why};
use crate::recipe::{Recipe, Step};

/// What a look at the Machine found for one entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Seen {
    Absent,
    Present(Value),
    /// The check couldn't answer. Such an entry is reported apart, and never acted on.
    CantTell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RowKind {
    /// `row.machine-moved`
    ChangedHere,
    /// `row.removed-here`
    RemovedHere,
    /// `row.config-moved`
    ChangedInConfig,
    /// `row.new-in-config`
    NewInConfig,
    /// `row.dropped`. It's still on this Mac, and nothing is removed unless the person says so.
    Dropped,
    /// `row.extra`
    Extra,
    /// `row.conflict`
    Conflict,
}

impl RowKind {
    /// Apply all covers only these. Everything else is one entry at a time.
    pub fn in_apply_all(self) -> bool {
        matches!(self, Self::ChangedInConfig | Self::NewInConfig)
    }

    /// A removal is only ever offered for something this Machine last applied.
    pub fn may_offer_removal(self) -> bool {
        self == Self::Dropped
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Same,
    Row(RowKind),
    CantTell,
}

pub fn classify(config: Option<&Value>, machine: &Seen, last: Option<&Value>) -> Verdict {
    let machine = match machine {
        Seen::CantTell => return Verdict::CantTell,
        Seen::Absent => None,
        Seen::Present(value) => Some(value),
    };
    if config == machine {
        return Verdict::Same;
    }
    Verdict::Row(match (config, machine, last) {
        (Some(config), Some(_), Some(last)) if last == config => RowKind::ChangedHere,
        (Some(_), Some(machine), Some(last)) if last == machine => RowKind::ChangedInConfig,
        // Both present and no agreed past: nothing says which one moved, so the person chooses.
        (Some(_), Some(_), _) => RowKind::Conflict,
        (Some(config), None, Some(last)) if last == config => RowKind::RemovedHere,
        (Some(_), None, Some(_)) => RowKind::Conflict,
        (Some(_), None, None) => RowKind::NewInConfig,
        (None, Some(_), Some(_)) => RowKind::Dropped,
        // With no record of applying it, this can be Adopted or kept, and never uninstalled.
        (None, Some(_), None) => RowKind::Extra,
        (None, None, _) => return Verdict::Same,
    })
}

/// One thing a person reviewed and chose to save. `seen` is the Config's value they looked at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reviewed {
    /// The entry in the Machine Record.
    pub id: String,
    /// The entry as the file spells it, which an old ID or a covered package can make different.
    pub key: String,
    pub seen: Option<Value>,
    /// What the Config should say, or `None` to take the line out.
    pub write: Option<Value>,
}

/// An entry's value in a Config. Items and packages are `true` when listed.
pub fn value_in(config: &Config, key: &str) -> Option<Value> {
    if let Some(name) = key.strip_prefix("formula:") {
        return config
            .packages
            .formulae
            .iter()
            .any(|f| f == name)
            .then_some(Value::Bool(true));
    }
    if let Some(name) = key.strip_prefix("cask:") {
        return config
            .packages
            .casks
            .iter()
            .any(|c| c == name)
            .then_some(Value::Bool(true));
    }
    if let Some(value) = config.settings.get(key) {
        return serde_json::to_value(value).ok();
    }
    config
        .items
        .iter()
        .any(|item| item == key)
        .then_some(Value::Bool(true))
}

/// A setting's key always has a dot in the Catalog, which an Item's id never does.
fn is_setting(key: &str) -> bool {
    key.contains('.') && !key.starts_with("formula:") && !key.starts_with("cask:")
}

fn set_listed(list: &mut Vec<String>, name: &str, listed: bool) {
    list.retain(|entry| entry != name);
    if listed {
        list.push(name.to_string());
        list.sort();
    }
}

/// The reviewed entries that can't be written onto this Config: it changed there since the
/// person looked, or the value is one a setting can't hold.
pub fn moved_since(latest: &Config, reviewed: &[Reviewed]) -> Vec<String> {
    let unusable = |entry: &Reviewed| {
        is_setting(&entry.key)
            && entry
                .write
                .as_ref()
                .is_some_and(|value| serde_json::from_value::<SettingValue>(value.clone()).is_err())
    };
    reviewed
        .iter()
        .filter(|entry| {
            let now = value_in(latest, &entry.key);
            unusable(entry) || (now != entry.seen && now != entry.write)
        })
        .map(|entry| entry.id.clone())
        .collect()
}

/// Writes the reviewed entries, and nothing else, onto the latest Config. An entry that changed
/// there since the person looked is not written: it comes back to be chosen again. One already
/// holding what's being written needs nothing, which is how a save the app died after settles.
/// A write also drops a line naming something always installed, which is never in a Config.
pub fn adopt_onto(
    latest: &Config,
    reviewed: &[Reviewed],
    always_installed: &BTreeSet<&str>,
) -> Result<Config, Vec<String>> {
    let moved = moved_since(latest, reviewed);
    if !moved.is_empty() {
        return Err(moved);
    }
    let mut config = latest.clone();
    for entry in reviewed {
        let listed = entry.write.is_some();
        if let Some(name) = entry.key.strip_prefix("formula:") {
            set_listed(&mut config.packages.formulae, name, listed);
        } else if let Some(name) = entry.key.strip_prefix("cask:") {
            set_listed(&mut config.packages.casks, name, listed);
        } else if is_setting(&entry.key) {
            let value = entry
                .write
                .clone()
                .map(serde_json::from_value::<SettingValue>);
            match value {
                Some(Ok(value)) => {
                    config.settings.insert(entry.key.clone(), value);
                }
                Some(Err(_)) => {}
                None => {
                    config.settings.remove(&entry.key);
                }
            }
        } else {
            set_listed(&mut config.items, &entry.key, listed);
        }
    }
    config
        .items
        .retain(|item| !always_installed.contains(item.as_str()));
    config
        .packages
        .formulae
        .retain(|name| !always_installed.contains(name.as_str()));
    Ok(config)
}

/// How something a Run put here comes off again. Only a single Homebrew step can be undone
/// exactly, and one that needs the password isn't offered: removal never asks for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uninstall {
    Formula(String),
    Cask(String),
}

/// A name Homebrew could have given: one part, or `owner/tap/name`, each starting with a letter
/// or a digit. A name from the Machine Record or from Homebrew's output is checked against this
/// before it's used, so it can't be an option or lead anywhere else.
pub fn plain_package(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    name.len() <= 128
        && matches!(parts.len(), 1 | 3)
        && !name.contains("..")
        && !name.ends_with(".rb")
        && !name.starts_with("homebrew/")
        && !parts.get(1).is_some_and(|tap| tap.starts_with("homebrew-"))
        && parts.iter().all(|part| {
            part.bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_' | b'.' | b'+' | b'@')
                })
        })
}

/// What Homebrew keeps an install under: its kind and its short name. A tap's name and the plain
/// one are the same keg to `brew uninstall`.
pub type Keg = (bool, String);

fn keg(cask: bool, name: &str) -> Keg {
    (cask, name.rsplit('/').next().unwrap_or(name).to_string())
}

pub fn kegs_of(id: &str, recipe: Option<&Recipe>) -> Vec<Keg> {
    let mut out = Vec::new();
    if let Some(name) = id.strip_prefix("formula:") {
        out.push(keg(false, name));
    } else if let Some(name) = id.strip_prefix("cask:") {
        out.push(keg(true, name));
    }
    if let Some(Recipe::Install(install)) = recipe {
        for step in &install.steps {
            match step {
                Step::BrewFormula { name, .. } => out.push(keg(false, name)),
                Step::BrewCask { name, .. } => out.push(keg(true, name)),
                _ => {}
            }
        }
    }
    out
}

/// In the file but not being applied, and everything those need. None of it is unwanted.
pub fn set_aside(plan: &Plan, catalog: &Catalog) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut todo: Vec<String> = plan
        .skipped
        .iter()
        .map(|entry| current_id(catalog, &entry.raw))
        .chain(plan.conflicts.iter().flat_map(|conflict| {
            [
                current_id(catalog, &conflict.item),
                current_id(catalog, &conflict.other),
            ]
        }))
        .collect();
    while let Some(id) = todo.pop() {
        if let Some(entry) = catalog.entry(&id) {
            todo.extend(
                entry
                    .recipe
                    .requires()
                    .iter()
                    .filter(|needed| !out.contains(*needed))
                    .cloned(),
            );
        }
        out.insert(id);
    }
    out
}

/// How to take this off this Mac, if it may be taken off at all. The row and the uninstall both
/// ask this, so what's offered and what's allowed can't disagree.
pub fn removal_of(plan: &Plan, catalog: &Catalog, id: &str) -> Option<Uninstall> {
    if plan.has_unknown_ids() {
        return None;
    }
    let id = current_id(catalog, id);
    let uninstall = uninstall_of(catalog, &id)?;
    if catalog.base_items().any(|entry| entry.id() == id) {
        return None;
    }
    let aside = set_aside(plan, catalog);
    if aside.contains(&id) || plan.entries.iter().any(|entry| entry.id == id) {
        return None;
    }
    let target = match &uninstall {
        Uninstall::Formula(name) => keg(false, name),
        Uninstall::Cask(name) => keg(true, name),
    };
    let still_wanted = plan
        .entries
        .iter()
        .flat_map(|entry| kegs_of(&entry.id, Some(&entry.recipe)))
        .chain(
            aside
                .iter()
                .flat_map(|id| kegs_of(id, catalog.entry(id).map(|entry| &entry.recipe))),
        )
        .any(|wanted| wanted == target);
    (!still_wanted).then_some(uninstall)
}

pub fn uninstall_of(catalog: &Catalog, id: &str) -> Option<Uninstall> {
    if let Some(name) = id.strip_prefix("formula:") {
        return plain_package(name).then(|| Uninstall::Formula(name.to_string()));
    }
    if let Some(name) = id.strip_prefix("cask:") {
        return plain_package(name).then(|| Uninstall::Cask(name.to_string()));
    }
    let Recipe::Install(install) = &catalog.entry(id)?.recipe else {
        return None;
    };
    if install.base {
        return None;
    }
    match install.steps.as_slice() {
        [Step::BrewFormula { name, .. }] => Some(Uninstall::Formula(name.clone())),
        [
            Step::BrewCask {
                name, admin: false, ..
            },
        ] => Some(Uninstall::Cask(name.clone())),
        _ => None,
    }
}

/// The ID an entry goes by now: an old name and a covered package both mean the Catalog's Item.
pub fn current_id(catalog: &Catalog, raw: &str) -> String {
    if let Some(entry) = catalog.owner_of(raw) {
        return entry.id().to_string();
    }
    match catalog.lookup(raw) {
        Lookup::Found { entry, .. } => entry.id().to_string(),
        _ => raw.to_string(),
    }
}

/// Every way the file spells this entry. One entry can be listed as an Item and as the package
/// that Item covers.
pub fn spellings_in(config: &Config, catalog: &Catalog, id: &str) -> Vec<String> {
    config
        .items
        .iter()
        .chain(config.settings.keys())
        .cloned()
        .chain(
            config
                .packages
                .formulae
                .iter()
                .map(|f| format!("formula:{f}")),
        )
        .chain(config.packages.casks.iter().map(|c| format!("cask:{c}")))
        .filter(|raw| raw == id || current_id(catalog, raw) == id)
        .collect()
}

/// How the file spells this entry, if the file has it at all.
fn file_key(facts: &Facts<'_>, id: &str) -> Option<String> {
    spellings_in(facts.config, facts.catalog, id)
        .into_iter()
        .next()
}

/// The reviewed entries as this Config spells them now. A line that was only respelled is still
/// the same entry, and taking an entry out takes out every line naming it.
pub fn respelled(latest: &Config, catalog: &Catalog, reviewed: &[Reviewed]) -> Vec<Reviewed> {
    let mut out = Vec::new();
    for entry in reviewed {
        let spellings = spellings_in(latest, catalog, &entry.id);
        if spellings.is_empty() || (entry.write.is_some() && spellings.contains(&entry.key)) {
            out.push(entry.clone());
        } else if entry.write.is_some() {
            out.push(Reviewed {
                key: spellings[0].clone(),
                ..entry.clone()
            });
        } else {
            out.extend(spellings.into_iter().map(|key| Reviewed {
                key,
                ..entry.clone()
            }));
        }
    }
    out
}

/// A person's answer to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Adopt,
    Apply,
    NotHere,
    Keep,
    Remove,
    AddBack,
    KeepHere,
}

impl Choice {
    /// The sync design's sentence.
    pub fn words(self, value: &str) -> String {
        match self {
            Self::Adopt => format!("Save {value} to your Machine Type"),
            Self::Apply => format!("Change this Mac to {value}"),
            Self::NotHere => "Leave this Mac as it is and don't ask again".to_string(),
            Self::Keep => "Keep it here and stop tracking it".to_string(),
            Self::Remove => "Remove it from this Mac".to_string(),
            Self::AddBack => "Put it back in your Machine Type".to_string(),
            Self::KeepHere => "Keep it on this Mac only".to_string(),
        }
    }
}

pub fn confirm_remove(names: &str) -> String {
    format!("Remove {names} from this Mac? This uninstalls them. Your other Macs aren't affected.")
}

pub fn confirm_adopt_removal(name: &str) -> String {
    format!("Your other Macs will be asked what to do about {name}. Nothing is removed from them.")
}

pub fn changed_elsewhere(name: &str) -> String {
    format!(
        "{name} also changed in your Machine Type, so this wasn't saved. Choose which one to keep."
    )
}

pub fn changed_again(name: &str) -> String {
    format!("{name} changed again on this Mac, so this wasn't saved. Review it again.")
}

pub fn not_adoptable(name: &str, why: &str) -> String {
    format!("{name} can't be saved to a Machine Type because {why}. It stays on this Mac.")
}

/// A value the way a person reads it.
pub fn shown(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(list) => list.iter().map(shown).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub file_key: String,
    pub name: String,
    pub kind: RowKind,
    pub in_config: Option<Value>,
    pub on_mac: Option<Value>,
    pub is_setting: bool,
    pub removable: bool,
    pub adoptable: bool,
    /// The file holds an ID this version can't read, and that ID might be this entry.
    pub waits_for_update: bool,
    /// Here only because something in the Config needs it, so the file has no line for it.
    pub only_needed: bool,
}

impl Row {
    pub fn sentence(&self) -> String {
        let name = &self.name;
        if self.waits_for_update {
            return held_sentence(name);
        }
        match self.kind {
            RowKind::NewInConfig if self.only_needed => {
                format!("Something in your Machine Type needs {name}, and it isn't on this Mac.")
            }
            RowKind::RemovedHere if self.only_needed => {
                format!(
                    "{name} was removed from this Mac, but something in your Machine Type needs it."
                )
            }
            RowKind::ChangedHere => format!("{name} changed on this Mac."),
            RowKind::RemovedHere => {
                format!("{name} was removed from this Mac, but your Machine Type still lists it.")
            }
            RowKind::ChangedInConfig => format!("{name} changed in your Machine Type."),
            RowKind::NewInConfig => format!("{name} is new in your Machine Type."),
            RowKind::Dropped => format!(
                "{name} is no longer in this Machine Type. It's still on this Mac, and nothing will be \
                 removed unless you say so."
            ),
            RowKind::Extra => format!("{name} is on this Mac but not in your Machine Type."),
            RowKind::Conflict => {
                format!(
                    "{name} changed in two places, and they don't match. You choose which one to keep."
                )
            }
        }
    }

    pub fn choices(&self) -> Vec<Choice> {
        if self.waits_for_update {
            return Vec::new();
        }
        let mut out = match self.kind {
            RowKind::ChangedHere | RowKind::RemovedHere | RowKind::Conflict => {
                vec![Choice::Adopt, Choice::Apply, Choice::NotHere]
            }
            // Only the Config moved, so saving would put this Mac's older value back over it.
            RowKind::ChangedInConfig | RowKind::NewInConfig => {
                vec![Choice::Apply, Choice::NotHere]
            }
            RowKind::Dropped => vec![Choice::Keep, Choice::AddBack],
            RowKind::Extra => vec![Choice::Adopt, Choice::KeepHere],
        };
        if !self.adoptable {
            out.retain(|choice| !matches!(choice, Choice::Adopt | Choice::AddBack));
        }
        if self.removable {
            out.push(Choice::Remove);
        }
        out
    }

    /// What saving this row writes: this Mac's value, or no line if it's gone from this Mac.
    pub fn reviewed(&self) -> Reviewed {
        Reviewed {
            id: self.id.clone(),
            key: self.file_key.clone(),
            seen: self.in_config.clone(),
            write: self.on_mac.clone(),
        }
    }
}

pub struct Facts<'a> {
    pub plan: &'a Plan,
    pub catalog: &'a Catalog,
    pub config: &'a Config,
    pub seen: &'a BTreeMap<String, Seen>,
    pub last: &'a BTreeMap<String, Value>,
    /// Entries this Machine can't apply, or the person said to leave.
    pub unavailable: &'a BTreeSet<String>,
    /// Entries the person kept here and stopped tracking.
    pub kept: &'a BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub rows: Vec<Row>,
    pub unavailable: Vec<String>,
    pub could_not_check: Vec<String>,
    /// IDs in the file this version doesn't recognize. The line stays, and while one is there
    /// nothing offers a removal or saves a package.
    pub held: Vec<String>,
}

pub fn held_sentence(name: &str) -> String {
    format!(
        "{name}: update Soulcaster to review this. Your Machine Type lists something this version \
         doesn't recognize."
    )
}

/// Whether adding this entry's line leaves everything else in the Config being applied. A line
/// that conflicts with one already there would stop both, on every Machine.
fn saving_keeps_applying(
    facts: &Facts<'_>,
    key: &str,
    machine: &Seen,
    always: &BTreeSet<&str>,
) -> bool {
    let Seen::Present(value) = machine else {
        return true;
    };
    let line = Reviewed {
        id: key.to_string(),
        key: key.to_string(),
        seen: None,
        write: Some(value.clone()),
    };
    adopt_onto(facts.config, &[line], always).is_ok_and(|after| {
        Plan::new(&after, facts.catalog).conflicts.len() <= facts.plan.conflicts.len()
    })
}

fn may_leave_this_mac(facts: &Facts<'_>, key: &str, value: SettingValue) -> bool {
    let mut only_this = facts.config.clone();
    only_this.items.clear();
    only_this.packages = Default::default();
    only_this.dotfiles = None;
    only_this.settings = BTreeMap::from([(key.to_string(), value)]);
    Config::parse(only_this.to_canonical_json().as_bytes()).is_ok()
        && !lint(&only_this, facts.catalog, DEFAULT_CLONE_LOCATION)
            .iter()
            .any(|finding| finding.must_stay_on_this_mac() && finding.at == key)
}

pub fn report(facts: &Facts<'_>) -> Report {
    let catalog = facts.catalog;
    let always: BTreeSet<&str> = catalog.base_items().map(|entry| entry.id()).collect();
    let held: Vec<String> = facts
        .plan
        .skipped
        .iter()
        .filter(|entry| entry.reason == SkipReason::UnknownId)
        .map(|entry| entry.raw.clone())
        .collect();
    let aside = set_aside(facts.plan, catalog);
    let wanted: BTreeMap<&str, Value> = facts
        .plan
        .entries
        .iter()
        .filter(|entry| entry.why != Why::Base && !matches!(entry.recipe, Recipe::MachineValues(_)))
        .map(|entry| {
            let value = entry
                .value
                .as_ref()
                .and_then(|value| serde_json::to_value(value).ok())
                .unwrap_or(Value::Bool(true));
            (entry.id.as_str(), value)
        })
        .collect();
    let last: BTreeMap<String, &Value> = facts
        .last
        .iter()
        .map(|(raw, value)| (current_id(catalog, raw), value))
        .collect();
    let unavailable: BTreeSet<String> = facts
        .unavailable
        .iter()
        .map(|raw| current_id(catalog, raw))
        .collect();
    let kept: BTreeSet<String> = facts
        .kept
        .iter()
        .map(|raw| current_id(catalog, raw))
        .collect();
    let seen: BTreeMap<String, &Seen> = facts
        .seen
        .iter()
        .map(|(raw, value)| (current_id(catalog, raw), value))
        .collect();
    let keys: BTreeSet<&str> = wanted
        .keys()
        .copied()
        .chain(last.keys().map(String::as_str))
        .chain(seen.keys().map(String::as_str))
        .collect();
    let mut report = Report {
        held,
        ..Report::default()
    };
    for key in keys {
        if always.contains(key) || aside.contains(key) {
            continue;
        }
        let in_plan = facts.plan.entries.iter().find(|entry| entry.id == key);
        if in_plan.is_some_and(|entry| matches!(entry.recipe, Recipe::MachineValues(_)))
            || matches!(
                catalog.entry(key).map(|entry| &entry.recipe),
                Some(Recipe::MachineValues(_))
            )
        {
            continue;
        }
        let config = wanted.get(key);
        if config.is_some() && unavailable.contains(key) {
            report.unavailable.push(key.to_string());
            continue;
        }
        if config.is_none() && kept.contains(key) {
            continue;
        }
        let last_value = last.get(key).copied();
        let machine = seen.get(key).map_or(Seen::CantTell, |seen| (*seen).clone());
        let kind = match classify(config, &machine, last_value) {
            Verdict::Same => continue,
            Verdict::CantTell => {
                report.could_not_check.push(key.to_string());
                continue;
            }
            Verdict::Row(kind) => kind,
        };
        let is_setting = matches!(
            catalog.entry(key).map(|entry| &entry.recipe),
            Some(Recipe::Setting(_))
        );
        let name = in_plan
            .map(|entry| entry.name.clone())
            .or_else(|| catalog.entry(key).map(|entry| entry.name().to_string()))
            .unwrap_or_else(|| {
                key.trim_start_matches("formula:")
                    .trim_start_matches("cask:")
                    .to_string()
            });
        let removable = kind.may_offer_removal()
            && last_value.is_some()
            && !is_setting
            && removal_of(facts.plan, catalog, key).is_some();
        // A value the Config would refuse, or one that must stay on this Mac, is never offered.
        let value_fits = match (catalog.entry(key).map(|entry| &entry.recipe), &machine) {
            (Some(Recipe::Setting(setting)), Seen::Present(value)) => {
                serde_json::from_value::<SettingValue>(value.clone()).is_ok_and(|value| {
                    setting.check_value(&value).is_ok() && may_leave_this_mac(facts, key, value)
                })
            }
            _ => true,
        };
        let spelled = file_key(facts, key);
        let only_needed = config.is_some() && spelled.is_none();
        let keeps_applying = config.is_some()
            || !matches!(machine, Seen::Present(_))
            || saving_keeps_applying(facts, key, &machine, &always);
        // That unreadable ID may be this entry's new name, for a setting that left the file too.
        let held_up = !report.held.is_empty() && (!is_setting || kind == RowKind::Dropped);
        report.rows.push(Row {
            id: key.to_string(),
            file_key: spelled.unwrap_or_else(|| key.to_string()),
            name,
            kind,
            in_config: config.cloned(),
            on_mac: match machine {
                Seen::Present(value) => Some(value),
                _ => None,
            },
            is_setting,
            removable,
            adoptable: !held_up && value_fits && !only_needed && keeps_applying,
            waits_for_update: held_up && matches!(kind, RowKind::Dropped | RowKind::Extra),
            only_needed,
        });
    }
    report
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Config;

    fn verdict(config: Option<Value>, machine: Seen, last: Option<Value>) -> Verdict {
        classify(config.as_ref(), &machine, last.as_ref())
    }

    #[test]
    fn every_combination_is_the_row_the_design_names() {
        let (a, b, c) = (json!(1), json!(2), json!(3));
        let here = |value: &Value| Seen::Present(value.clone());
        for (config, machine, last, want) in [
            (Some(&a), here(&a), Some(&a), Verdict::Same),
            (Some(&a), here(&a), None, Verdict::Same),
            (Some(&a), here(&a), Some(&b), Verdict::Same),
            (None, Seen::Absent, None, Verdict::Same),
            (None, Seen::Absent, Some(&a), Verdict::Same),
            (
                Some(&a),
                here(&b),
                Some(&a),
                Verdict::Row(RowKind::ChangedHere),
            ),
            (
                Some(&b),
                here(&a),
                Some(&a),
                Verdict::Row(RowKind::ChangedInConfig),
            ),
            (
                Some(&a),
                here(&b),
                Some(&c),
                Verdict::Row(RowKind::Conflict),
            ),
            (Some(&a), here(&b), None, Verdict::Row(RowKind::Conflict)),
            (
                Some(&a),
                Seen::Absent,
                Some(&a),
                Verdict::Row(RowKind::RemovedHere),
            ),
            (
                Some(&a),
                Seen::Absent,
                Some(&b),
                Verdict::Row(RowKind::Conflict),
            ),
            (
                Some(&a),
                Seen::Absent,
                None,
                Verdict::Row(RowKind::NewInConfig),
            ),
            (None, here(&a), Some(&a), Verdict::Row(RowKind::Dropped)),
            (None, here(&a), Some(&b), Verdict::Row(RowKind::Dropped)),
            (None, here(&a), None, Verdict::Row(RowKind::Extra)),
            (Some(&a), Seen::CantTell, Some(&a), Verdict::CantTell),
            (None, Seen::CantTell, None, Verdict::CantTell),
        ] {
            assert_eq!(
                verdict(config.cloned(), machine.clone(), last.cloned()),
                want,
                "config {config:?}, machine {machine:?}, last {last:?}"
            );
        }
    }

    #[test]
    fn apply_all_and_removal_are_as_narrow_as_the_design_says() {
        use RowKind as R;
        let all = [
            R::ChangedHere,
            R::RemovedHere,
            R::ChangedInConfig,
            R::NewInConfig,
            R::Dropped,
            R::Extra,
            R::Conflict,
        ];
        let apply: Vec<_> = all.iter().filter(|k| k.in_apply_all()).collect();
        assert_eq!(apply, [&R::ChangedInConfig, &R::NewInConfig]);
        let remove: Vec<_> = all.iter().filter(|k| k.may_offer_removal()).collect();
        assert_eq!(
            remove,
            [&R::Dropped],
            "an unrecorded Extra is never offered an uninstall"
        );
    }

    fn config(text: &str) -> Config {
        Config::parse(text.as_bytes()).unwrap()
    }

    #[test]
    fn adopting_writes_the_reviewed_entries_and_keeps_what_else_landed() {
        let latest = config(
            r#"{"schemaVersion":1,"name":"x","items":["jq","zed"],
                "packages":{"formulae":["tree"]},
                "settings":{"dock.tilesize":48,"dock.autohide":true}}"#,
        );
        let out = adopt_onto(
            &latest,
            &[
                Reviewed {
                    id: "dock.tilesize".into(),
                    key: "dock.tilesize".into(),
                    seen: Some(json!(48)),
                    write: Some(json!(36)),
                },
                Reviewed {
                    id: "ripgrep".into(),
                    key: "ripgrep".into(),
                    seen: None,
                    write: Some(json!(true)),
                },
                Reviewed {
                    id: "jq".into(),
                    key: "jq".into(),
                    seen: Some(json!(true)),
                    write: None,
                },
                Reviewed {
                    id: "formula:bat".into(),
                    key: "formula:bat".into(),
                    seen: None,
                    write: Some(json!(true)),
                },
                Reviewed {
                    id: "formula:tree".into(),
                    key: "formula:tree".into(),
                    seen: Some(json!(true)),
                    write: None,
                },
            ],
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(out.items, ["ripgrep", "zed"]);
        assert_eq!(out.packages.formulae, ["bat"]);
        assert_eq!(value_in(&out, "dock.tilesize"), Some(json!(36)));
        assert_eq!(
            value_in(&out, "dock.autohide"),
            Some(json!(true)),
            "an entry nobody reviewed moved"
        );
    }

    #[test]
    fn an_entry_that_moved_since_it_was_reviewed_comes_back_and_nothing_is_written() {
        let latest = config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.tilesize":36}}"#);
        let moved = adopt_onto(
            &latest,
            &[
                Reviewed {
                    id: "dock.tilesize".into(),
                    key: "dock.tilesize".into(),
                    seen: Some(json!(48)),
                    write: Some(json!(64)),
                },
                Reviewed {
                    id: "dock.autohide".into(),
                    key: "dock.autohide".into(),
                    seen: None,
                    write: Some(json!(true)),
                },
            ],
            &BTreeSet::new(),
        )
        .unwrap_err();
        assert_eq!(moved, ["dock.tilesize"]);
    }

    #[test]
    fn a_value_a_setting_cannot_hold_comes_back_instead_of_deleting_the_line() {
        let latest = config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.tilesize":48}}"#);
        let moved = adopt_onto(
            &latest,
            &[Reviewed {
                id: "dock.tilesize".into(),
                key: "dock.tilesize".into(),
                seen: Some(json!(48)),
                write: Some(json!({"not": "a setting"})),
            }],
            &BTreeSet::new(),
        )
        .unwrap_err();
        assert_eq!(moved, ["dock.tilesize"]);
    }

    #[test]
    fn a_save_github_already_has_is_not_refused_as_changed() {
        let latest = config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.tilesize":36}}"#);
        let out = adopt_onto(
            &latest,
            &[Reviewed {
                id: "dock.tilesize".into(),
                key: "dock.tilesize".into(),
                seen: Some(json!(48)),
                write: Some(json!(36)),
            }],
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(out, latest);
    }

    #[test]
    fn a_write_drops_a_line_naming_something_always_installed() {
        let latest = config(r#"{"schemaVersion":1,"name":"x","items":["gh","jq"]}"#);
        let out = adopt_onto(&latest, &[], &BTreeSet::from(["gh"])).unwrap();
        assert_eq!(out.items, ["jq"]);
    }

    #[test]
    fn every_catalog_setting_has_a_dot_and_nothing_else_does() {
        let catalog = Catalog::shipped();
        for entry in catalog.entries() {
            let dotted = entry.id().contains('.');
            let setting = matches!(entry.recipe, Recipe::Setting(_));
            assert_eq!(dotted, setting, "{}", entry.id());
            assert!(!entry.id().contains(':'), "{}", entry.id());
        }
    }

    struct World {
        config: Config,
        seen: BTreeMap<String, Seen>,
        last: BTreeMap<String, Value>,
        unavailable: BTreeSet<String>,
        kept: BTreeSet<String>,
    }

    impl World {
        fn new(text: &str) -> Self {
            Self {
                config: config(text),
                seen: BTreeMap::new(),
                last: BTreeMap::new(),
                unavailable: BTreeSet::new(),
                kept: BTreeSet::new(),
            }
        }

        fn here(mut self, key: &str, value: Value) -> Self {
            self.seen.insert(key.into(), Seen::Present(value));
            self
        }

        fn gone(mut self, key: &str) -> Self {
            self.seen.insert(key.into(), Seen::Absent);
            self
        }

        fn was(mut self, key: &str, value: Value) -> Self {
            self.last.insert(key.into(), value);
            self
        }

        fn report(&self) -> Report {
            let catalog = Catalog::shipped();
            let plan = Plan::new(&self.config, catalog);
            report(&Facts {
                plan: &plan,
                catalog,
                config: &self.config,
                seen: &self.seen,
                last: &self.last,
                unavailable: &self.unavailable,
                kept: &self.kept,
            })
        }

        fn row(&self, id: &str) -> Option<Row> {
            self.report().rows.into_iter().find(|row| row.id == id)
        }
    }

    const EMPTY: &str = r#"{"schemaVersion":1,"name":"x"}"#;

    #[test]
    fn something_pulled_in_by_the_config_is_not_dropped() {
        let world = World::new(r#"{"schemaVersion":1,"name":"x","items":["android-emulator"]}"#)
            .here("android-emulator", json!(true))
            .was("android-emulator", json!(true))
            .here("android-sdk", json!(true))
            .was("android-sdk", json!(true));
        assert_eq!(
            world.report().rows,
            [],
            "android-sdk is what the emulator needs"
        );
    }

    #[test]
    fn what_is_always_installed_is_never_a_row() {
        let world = World::new(EMPTY)
            .here("gh", json!(true))
            .was("gh", json!(true))
            .gone("homebrew")
            .was("homebrew", json!(true));
        assert_eq!(world.report(), Report::default());
    }

    #[test]
    fn a_line_this_version_will_not_apply_is_not_a_removal() {
        let world = World::new(r#"{"schemaVersion":1,"name":"x","items":["no-such-thing"]}"#)
            .here("no-such-thing", json!(true))
            .was("no-such-thing", json!(true));
        let report = world.report();
        assert_eq!(report.rows, []);
        assert_eq!(report.held, ["no-such-thing"]);
    }

    #[test]
    fn while_the_file_has_an_unknown_id_nothing_offers_a_removal_or_saves_a_package() {
        let world = World::new(r#"{"schemaVersion":1,"name":"x","items":["no-such-thing"],"settings":{"keyboard.key-repeat":48}}"#)
            .here("formula:nobody-covers-this", json!(true))
            .was("formula:nobody-covers-this", json!(true))
            .here("keyboard.key-repeat", json!(36))
            .was("keyboard.key-repeat", json!(48));
        let package = world.row("formula:nobody-covers-this").unwrap();
        assert_eq!(package.kind, RowKind::Dropped);
        assert_eq!(package.choices(), []);
        assert_eq!(
            world.row("keyboard.key-repeat").unwrap().choices(),
            [Choice::Adopt, Choice::Apply, Choice::NotHere],
            "a setting can still be saved"
        );
    }

    #[test]
    fn a_dropped_package_offers_removal_and_an_unrecorded_one_never_does() {
        let dropped = World::new(EMPTY)
            .here("formula:nobody-covers-this", json!(true))
            .was("formula:nobody-covers-this", json!(true))
            .row("formula:nobody-covers-this")
            .unwrap();
        assert_eq!(
            dropped.choices(),
            [Choice::Keep, Choice::AddBack, Choice::Remove]
        );
        let extra = World::new(EMPTY)
            .here("formula:nobody-covers-this", json!(true))
            .row("formula:nobody-covers-this")
            .unwrap();
        assert_eq!(extra.kind, RowKind::Extra);
        assert_eq!(extra.choices(), [Choice::Adopt, Choice::KeepHere]);
    }

    #[test]
    fn a_setting_has_no_uninstall() {
        let row = World::new(EMPTY)
            .here("keyboard.key-repeat", json!(36))
            .was("keyboard.key-repeat", json!(36))
            .row("keyboard.key-repeat")
            .unwrap();
        assert_eq!(row.kind, RowKind::Dropped);
        assert_eq!(row.choices(), [Choice::Keep, Choice::AddBack]);
    }

    #[test]
    fn only_a_single_homebrew_step_can_be_undone() {
        let catalog = Catalog::shipped();
        assert_eq!(
            uninstall_of(catalog, "formula:nobody-covers-this"),
            Some(Uninstall::Formula("nobody-covers-this".into()))
        );
        assert_eq!(uninstall_of(catalog, "homebrew"), None);
        assert_eq!(uninstall_of(catalog, "keyboard.key-repeat"), None);
        assert_eq!(uninstall_of(catalog, "android-emulator"), None);
        assert_eq!(uninstall_of(catalog, "no-such-thing"), None);
        for entry in catalog.entries() {
            if let Recipe::Install(install) = &entry.recipe
                && install
                    .steps
                    .iter()
                    .any(|step| matches!(step, Step::BrewCask { admin: true, .. }))
            {
                assert_eq!(
                    uninstall_of(catalog, entry.id()),
                    None,
                    "removal never asks for the password"
                );
            }
        }
    }

    #[test]
    fn kept_and_unavailable_entries_stay_out_of_the_rows() {
        let mut world =
            World::new(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":36}}"#)
                .here("keyboard.key-repeat", json!(64))
                .was("keyboard.key-repeat", json!(48))
                .here("formula:nobody-covers-this", json!(true));
        world.unavailable.insert("keyboard.key-repeat".into());
        world.kept.insert("formula:nobody-covers-this".into());
        let report = world.report();
        assert_eq!(report.rows, []);
        assert_eq!(report.unavailable, ["keyboard.key-repeat"]);
    }

    #[test]
    fn something_left_alone_here_is_asked_about_again_once_it_leaves_the_config() {
        let mut world = World::new(EMPTY)
            .here("keyboard.key-repeat", json!(64))
            .was("keyboard.key-repeat", json!(48));
        world.unavailable.insert("keyboard.key-repeat".into());
        assert_eq!(
            world.row("keyboard.key-repeat").unwrap().kind,
            RowKind::Dropped
        );
    }

    #[test]
    fn a_machine_that_cannot_answer_is_reported_apart_and_never_a_row() {
        let world =
            World::new(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":36}}"#);
        let report = world.report();
        assert_eq!(report.rows, []);
        assert_eq!(report.could_not_check, ["keyboard.key-repeat"]);
    }

    #[test]
    fn an_old_name_in_the_record_is_the_same_entry() {
        let catalog = Catalog::shipped();
        let Some((old, new)) = catalog.entries().find_map(|entry| {
            let raw = entry.id();
            catalog
                .renames_to(raw)
                .map(|old| (old.to_string(), raw.to_string()))
        }) else {
            return;
        };
        let text = format!(r#"{{"schemaVersion":1,"name":"x","items":["{new}"]}}"#);
        let world = World::new(&text)
            .here(&new, json!(true))
            .was(&old, json!(true));
        assert_eq!(world.report().rows, []);
    }

    #[test]
    fn a_save_says_how_the_file_spells_the_entry() {
        let catalog = Catalog::shipped();
        let Some((key, id)) = catalog.entries().find_map(|entry| {
            let key = entry.covers_packages.first()?;
            Some((key.clone(), entry.id().to_string()))
        }) else {
            return;
        };
        let name = key.split_once(':').unwrap().1;
        let list = if key.starts_with("cask:") {
            "casks"
        } else {
            "formulae"
        };
        let text =
            format!(r#"{{"schemaVersion":1,"name":"x","packages":{{"{list}":["{name}"]}}}}"#);
        let row = World::new(&text)
            .gone(&id)
            .was(&id, json!(true))
            .row(&id)
            .unwrap();
        assert_eq!(row.kind, RowKind::RemovedHere);
        let reviewed = row.reviewed();
        assert_eq!(reviewed.key, key);
        assert_eq!(reviewed.write, None);
        let out = adopt_onto(&config(&text), &[reviewed], &BTreeSet::new()).unwrap();
        assert!(out.packages.formulae.is_empty() && out.packages.casks.is_empty());
    }

    #[test]
    fn no_sentence_uses_a_git_word() {
        const GIT_WORDS: [&str; 21] = [
            "merge",
            "conflict",
            "commit",
            "push",
            "pull",
            "rebase",
            "branch",
            "head",
            "remote",
            "origin",
            "fetch",
            "upstream",
            "stash",
            "sha",
            "diverge",
            "force",
            "clone",
            "checkout",
            "ours",
            "theirs",
            "fast-forward",
        ];
        let row = |kind| Row {
            id: "x".into(),
            file_key: "x".into(),
            name: "Name".into(),
            kind,
            in_config: None,
            on_mac: None,
            is_setting: false,
            removable: true,
            adoptable: true,
            waits_for_update: false,
            only_needed: false,
        };
        let mut all = vec![
            confirm_remove("Name"),
            confirm_adopt_removal("Name"),
            changed_elsewhere("Name"),
            changed_again("Name"),
            not_adoptable("Name", "why"),
            held_sentence("Name"),
        ];
        for kind in [
            RowKind::ChangedHere,
            RowKind::RemovedHere,
            RowKind::ChangedInConfig,
            RowKind::NewInConfig,
            RowKind::Dropped,
            RowKind::Extra,
            RowKind::Conflict,
        ] {
            let row = row(kind);
            all.push(row.sentence());
            all.extend(row.choices().into_iter().map(|c| c.words("value")));
        }
        for sentence in all {
            let lower = sentence.to_lowercase();
            for word in lower.split(|c: char| !c.is_ascii_alphabetic() && c != '-') {
                assert!(!GIT_WORDS.contains(&word), "{word} in {sentence}");
            }
        }
    }

    #[test]
    fn the_row_sentences_are_the_designs_words() {
        let row = World::new(EMPTY)
            .here("formula:nobody-covers-this", json!(true))
            .was("formula:nobody-covers-this", json!(true))
            .row("formula:nobody-covers-this")
            .unwrap();
        assert_eq!(
            row.sentence(),
            "nobody-covers-this is no longer in this Machine Type. It's still on this Mac, and nothing will be removed unless you say so."
        );
    }
}
