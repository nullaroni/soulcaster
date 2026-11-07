//! From a Config and the Catalog to what a Run will do, reading nothing from the Machine or the
//! network. Each entry that can't apply is skipped and reported, never a reason to refuse the file.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::catalog::{Catalog, Entry, Lifecycle, ListingNeedKind, Lookup};
use crate::config::{Config, SettingValue};
use crate::event::PlanItem;
use crate::lifecycle::{ItemId, SkipReason, Stage};
use crate::lint::{DEFAULT_CLONE_LOCATION, Finding, FindingKind, lint};
use crate::recipe::{InstallKind, InstallRecipe, Probe, Recipe, Step, Store, tap_of};

/// Where an entry falls within its stage, from the lifecycle's Run order. Dotfiles links run after
/// `ThirdParty` and before `GitValues`, and the Dotfiles script after everything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunGroup {
    Base,
    Official,
    Download,
    Setting,
    ThirdParty,
    GitValues,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Why {
    Base,
    Selected,
    PulledIn { by: ItemId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NeedKind {
    AdminPassword,
    TapApproval,
    License,
    AppleAccount,
    /// A step only a person can do, shown in the Needs-You Queue.
    Person,
    /// Filled from the Machine first, and queued if nothing there has it. A Run never prompts.
    MachineValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PersonNeed {
    pub kind: NeedKind,
    pub text: String,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanEntry {
    pub id: ItemId,
    pub name: String,
    pub stage: Stage,
    pub group: RunGroup,
    pub why: Why,
    pub recipe: Recipe,
    /// The Config's value, for a setting.
    pub value: Option<SettingValue>,
    pub needs: Vec<PersonNeed>,
}

/// An entry in the file that this Run won't attempt. It stays in the file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkippedEntry {
    /// The ID as the file spells it.
    pub raw: String,
    pub reason: SkipReason,
    pub note: String,
}

/// Neither side installs until a person picks one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlanConflict {
    pub item: ItemId,
    pub other: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DotfilesPlan {
    pub repo: String,
    pub links: BTreeMap<String, String>,
    pub refused_links: Vec<String>,
    pub install_script: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub config_name: String,
    /// In Run order: the Admin Stage, then the User Stage, Base Items first in each.
    pub entries: Vec<PlanEntry>,
    pub skipped: Vec<SkippedEntry>,
    pub conflicts: Vec<PlanConflict>,
    /// One line per thing worth telling a person before the Run.
    pub notes: Vec<String>,
    pub findings: Vec<Finding>,
    /// Each third-party tap and which entries load it. Each needs its own approval.
    pub taps: BTreeMap<String, Vec<ItemId>>,
    pub dotfiles: Option<DotfilesPlan>,
}

impl Plan {
    pub fn new(config: &Config, catalog: &Catalog) -> Plan {
        Self::with_clone_location(config, catalog, DEFAULT_CLONE_LOCATION)
    }

    /// For a Machine whose local record overrides where its Dotfiles Repo lives.
    pub fn with_clone_location(config: &Config, catalog: &Catalog, clone_location: &str) -> Plan {
        Builder::new(config, catalog, clone_location).build()
    }

    /// Whether anything in this plan needs the password, so the Run knows to ask.
    pub fn needs_admin(&self) -> bool {
        self.entries.iter().any(|entry| entry.stage == Stage::Admin)
    }

    /// While a Config holds an ID this app doesn't know, there's no removal offer and no package
    /// Adopt: the file might name that package through the ID it can't read.
    pub fn has_unknown_ids(&self) -> bool {
        self.skipped
            .iter()
            .any(|entry| entry.reason == SkipReason::UnknownId)
    }

    /// Lifecycle rule 1: check every Item before assigning stages. Before that check, a plan
    /// treats nothing as done. Afterwards the engine calls this with what it learned, and the
    /// order is worked out again.
    pub fn assign_stages(&mut self, decide: impl Fn(&PlanEntry) -> Stage) {
        for entry in &mut self.entries {
            entry.stage = decide(entry);
        }
        sort_for_run(&mut self.entries);
    }

    /// What `run.started` carries, so a front end can draw every Item at once.
    pub fn summary(&self) -> Vec<PlanItem> {
        self.entries
            .iter()
            .map(|entry| PlanItem {
                id: entry.id.clone(),
                name: entry.name.clone(),
                stage: entry.stage,
                is_setting: matches!(entry.recipe, Recipe::Setting(_)),
                pulled_in_by: match &entry.why {
                    Why::PulledIn { by } => Some(by.clone()),
                    _ => None,
                },
            })
            .collect()
    }
}

/// Run order: stage first, then group, but always after whatever an entry requires.
fn sort_for_run(entries: &mut Vec<PlanEntry>) {
    let ids: BTreeSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut remaining: Vec<PlanEntry> = std::mem::take(entries);
    remaining.sort_by_key(|entry| (entry.stage, entry.group));
    let mut placed: BTreeSet<String> = BTreeSet::new();
    while !remaining.is_empty() {
        // A cycle can't reach here on a linted Catalog. Taking the first entry anyway stops a loop.
        let next = remaining
            .iter()
            .position(|entry| {
                entry
                    .recipe
                    .requires()
                    .iter()
                    .all(|r| placed.contains(r) || !ids.contains(r))
            })
            .unwrap_or(0);
        let entry = remaining.remove(next);
        placed.insert(entry.id.clone());
        entries.push(entry);
    }
}

fn group_of(recipe: &Recipe) -> RunGroup {
    if recipe.is_base() {
        return RunGroup::Base;
    }
    match recipe {
        Recipe::MachineValues(_) => RunGroup::GitValues,
        Recipe::Setting(setting)
            if setting
                .stores
                .iter()
                .any(|s| matches!(s, Store::GitConfig { .. })) =>
        {
            RunGroup::GitValues
        }
        Recipe::Setting(_) => RunGroup::Setting,
        Recipe::Install(_) if !recipe.taps().is_empty() => RunGroup::ThirdParty,
        Recipe::Install(_)
            if recipe
                .steps()
                .iter()
                .any(|s| matches!(s, Step::Download { .. })) =>
        {
            RunGroup::Download
        }
        Recipe::Install(_) => RunGroup::Official,
    }
}

/// What an Item needs from a person, worked out from what its Recipe really does. A listing's words
/// are used where they exist, because they say why.
fn needs_of(entry: Option<&Entry>, recipe: &Recipe) -> Vec<PersonNeed> {
    let listing_detail = |kind: ListingNeedKind| {
        entry
            .and_then(|e| e.listing.as_ref())
            .and_then(|listing| {
                listing
                    .needs_from_person
                    .iter()
                    .find(|need| need.kind == kind)
            })
            .map(|need| need.detail.clone())
    };
    let mut needs = Vec::new();
    for tap in recipe.taps() {
        let text = listing_detail(ListingNeedKind::TapApproval).unwrap_or_else(|| {
            format!("This loads the third-party tap {tap}, which needs its own approval.")
        });
        needs.push(PersonNeed {
            kind: NeedKind::TapApproval,
            text,
            link: None,
        });
    }
    if let Some(detail) = listing_detail(ListingNeedKind::AppleAccount) {
        needs.push(PersonNeed {
            kind: NeedKind::AppleAccount,
            text: detail,
            link: None,
        });
    }
    for step in recipe.steps() {
        match step {
            Step::NeedsYou { text, link, .. } => {
                needs.push(PersonNeed {
                    kind: NeedKind::Person,
                    text: text.clone(),
                    link: link.clone(),
                });
            }
            Step::AcceptLicense { text, link, .. } => {
                needs.push(PersonNeed {
                    kind: NeedKind::License,
                    text: text.clone(),
                    link: link.clone(),
                });
            }
            _ => {}
        }
    }
    if recipe.has_admin_step() {
        let text = listing_detail(ListingNeedKind::AdminPassword)
            .unwrap_or_else(|| "This needs your password during the Admin Stage.".to_string());
        needs.push(PersonNeed {
            kind: NeedKind::AdminPassword,
            text,
            link: None,
        });
    }
    if let Recipe::MachineValues(values) = recipe {
        for value in &values.values {
            needs.push(PersonNeed {
                kind: NeedKind::MachineValue,
                text: value.label.clone(),
                link: None,
            });
        }
    }
    needs
}

/// The Recipe the lifecycle generates for a name in a Config's `packages`, the same as a Catalog
/// Item's. It installs by full name, which trusts only that one formula or cask.
fn package_recipe(key: &str, cask: bool, name: &str) -> Recipe {
    let (kind, step, check) = if cask {
        (
            InstallKind::App,
            Step::BrewCask {
                name: name.to_string(),
                admin: false,
                unless: None,
            },
            Probe::BrewCask {
                name: name.to_string(),
            },
        )
    } else {
        (
            InstallKind::Cli,
            Step::BrewFormula {
                name: name.to_string(),
                unless: None,
            },
            Probe::BrewFormula {
                name: name.to_string(),
            },
        )
    };
    Recipe::Install(InstallRecipe {
        id: key.to_string(),
        kind,
        title: name.to_string(),
        base: false,
        requires: vec!["homebrew".to_string()],
        min_macos: None,
        conflicts: vec![],
        checks: vec![check],
        facts: vec![],
        steps: vec![step],
        notes: vec![],
        bootstrap: None,
    })
}

struct Builder<'a> {
    config: &'a Config,
    catalog: &'a Catalog,
    entries: Vec<PlanEntry>,
    seen: BTreeSet<String>,
    skipped: Vec<SkippedEntry>,
    conflicts: Vec<PlanConflict>,
    notes: Vec<String>,
    findings: Vec<Finding>,
    flagged: BTreeMap<String, FindingKind>,
}

impl<'a> Builder<'a> {
    fn new(config: &'a Config, catalog: &'a Catalog, clone_location: &str) -> Self {
        let findings = lint(config, catalog, clone_location);
        let flagged = findings
            .iter()
            .map(|finding| (finding.at.clone(), finding.kind))
            .collect();
        Self {
            config,
            catalog,
            entries: Vec::new(),
            seen: BTreeSet::new(),
            skipped: Vec::new(),
            conflicts: Vec::new(),
            notes: Vec::new(),
            findings,
            flagged,
        }
    }

    fn skip(&mut self, raw: &str, reason: SkipReason, note: String) {
        self.notes.push(note.clone());
        self.skipped.push(SkippedEntry {
            raw: raw.to_string(),
            reason,
            note,
        });
    }

    /// Why this entry won't apply because of a lint finding, if one flagged it.
    fn skip_if_flagged(&mut self, raw: &str) -> bool {
        let Some(kind) = self.flagged.get(raw).copied() else {
            return false;
        };
        let reason = if kind == FindingKind::WritesThroughLink {
            SkipReason::WritesThroughLink
        } else {
            SkipReason::Lint
        };
        let message = self
            .findings
            .iter()
            .find(|f| f.at == raw)
            .map(|f| f.message.clone())
            .unwrap_or_default();
        self.skip(
            raw,
            reason,
            format!("{message}: skipped and kept in the file"),
        );
        true
    }

    /// The live entry for an ID from the file, or `None` after reporting why it won't apply.
    fn resolve(&mut self, raw: &'a str) -> Option<&'a Entry> {
        let catalog = self.catalog;
        match catalog.lookup(raw) {
            Lookup::Cycle => {
                self.skip(
                    raw,
                    SkipReason::RenameCycle,
                    format!("rename cycle at {raw}: skipped and kept in the file"),
                );
                None
            }
            Lookup::Unknown { suggestion } => {
                let hint = suggestion.map_or(String::new(), |id| format!(" Did you mean {id}?"));
                self.skip(
                    raw,
                    SkipReason::UnknownId,
                    format!("unknown ID {raw}: skipped and kept in the file.{hint}"),
                );
                None
            }
            Lookup::Found {
                entry,
                renamed_from,
            } => {
                if renamed_from.is_some() {
                    self.notes.push(format!(
                        "renamed {raw} -> {}: resolved, and the file isn't rewritten",
                        entry.id()
                    ));
                }
                match &entry.lifecycle {
                    Lifecycle::Retired(notice) => {
                        let replacement = notice
                            .replaced_by
                            .as_ref()
                            .map_or(String::new(), |id| format!(" Replacement offered: {id}"));
                        self.skip(
                            raw,
                            SkipReason::Retired,
                            format!("retired {raw}: skipped and kept, nothing uninstalled. {}{replacement}", notice.message),
                        );
                        None
                    }
                    Lifecycle::Deprecated(notice) => {
                        let replacement = notice
                            .replaced_by
                            .as_ref()
                            .map_or(String::new(), |id| format!(", replacement offered: {id}"));
                        self.notes
                            .push(format!("deprecated {raw}: still installed{replacement}"));
                        Some(entry)
                    }
                    Lifecycle::Active => Some(entry),
                }
            }
        }
    }

    fn selected_items(&mut self) -> Vec<&'a Entry> {
        let mut selected: Vec<&'a Entry> = Vec::new();
        let config = self.config;
        for raw in &config.items {
            if self.skip_if_flagged(raw) {
                continue;
            }
            let Some(entry) = self.resolve(raw) else {
                continue;
            };
            if entry.recipe.is_base() {
                self.notes.push(format!(
                    "{raw} is always installed: ignored, and dropped at the next reviewed write"
                ));
            } else if matches!(entry.recipe, Recipe::Setting(_)) {
                self.skip(
                    raw,
                    SkipReason::WrongKind,
                    format!("{raw} is a setting, not an Item: skipped and kept in the file"),
                );
            } else if !selected.iter().any(|known| known.id() == entry.id()) {
                selected.push(entry);
            }
        }
        selected
    }

    /// Packages the Catalog doesn't cover, as `(key, is_cask, name)`. A covered one joins `selected`.
    fn uncovered_packages(&mut self, selected: &mut Vec<&'a Entry>) -> Vec<(String, bool, String)> {
        let mut packages = Vec::new();
        let config = self.config;
        let lists = [
            (false, "formula", &config.packages.formulae),
            (true, "cask", &config.packages.casks),
        ];
        for (cask, prefix, names) in lists {
            for name in names {
                let key = format!("{prefix}:{name}");
                let owner = self.catalog.owner_of(&key);
                if owner.is_some_and(|entry| entry.recipe.is_base()) {
                    self.notes.push(format!("{name} is always installed: ignored, and dropped at the next reviewed write"));
                    continue;
                }
                let short = name.rsplit('/').next().unwrap_or(name);
                if tap_of(name).is_some()
                    && self.catalog.base_items().any(|base| base.id() == short)
                {
                    self.notes.push(format!("warning: {name} has the same name as always-installed {short}, and is a different program"));
                }
                if let Some(owner) = owner {
                    self.notes.push(format!("{name} is covered by Item {}: installed once. An explicit Tidy can move it.", owner.id()));
                    if !selected.iter().any(|known| known.id() == owner.id()) {
                        selected.push(owner);
                    }
                } else {
                    packages.push((key, cask, name.clone()));
                }
            }
        }
        packages
    }

    /// Removes both sides of every conflict from what will install, so a person picks before a Run.
    fn drop_conflicts(
        &mut self,
        selected: &mut Vec<&'a Entry>,
        packages: &mut Vec<(String, bool, String)>,
    ) {
        let mut removed_items: BTreeSet<String> = BTreeSet::new();
        for entry in selected.iter() {
            for conflict in &entry.conflicts_with {
                if let Some(at) = packages.iter().position(|(key, ..)| *key == conflict.key) {
                    packages.remove(at);
                    removed_items.insert(entry.id().to_string());
                    self.record_conflict(entry.id(), &conflict.key, conflict.reason.clone());
                    continue;
                }
                let other = selected
                    .iter()
                    .find(|o| o.id() != entry.id() && o.covers_packages.contains(&conflict.key));
                if let Some(other) = other {
                    let first_mention =
                        !removed_items.contains(other.id()) || !removed_items.contains(entry.id());
                    removed_items.insert(entry.id().to_string());
                    removed_items.insert(other.id().to_string());
                    if first_mention {
                        self.record_conflict(entry.id(), other.id(), conflict.reason.clone());
                    }
                }
            }
        }
        selected.retain(|entry| !removed_items.contains(entry.id()));
    }

    fn record_conflict(&mut self, item: &str, other: &str, reason: Option<String>) {
        self.notes.push(format!(
            "conflict: {item} and {other} can't both be installed. Neither is, until you pick one."
        ));
        self.conflicts.push(PlanConflict {
            item: item.to_string(),
            other: other.to_string(),
            reason,
        });
    }

    fn add(&mut self, entry: &'a Entry, why: Why, value: Option<SettingValue>) {
        if !self.seen.insert(entry.id().to_string()) {
            return;
        }
        let catalog = self.catalog;
        for requirement in entry.recipe.requires() {
            if let Some(needed) = catalog.entry(requirement) {
                self.add(
                    needed,
                    Why::PulledIn {
                        by: entry.id().to_string(),
                    },
                    None,
                );
            }
        }
        self.entries.push(PlanEntry {
            id: entry.id().to_string(),
            name: entry.name().to_string(),
            stage: entry.recipe.stage(|_| false),
            group: group_of(&entry.recipe),
            why,
            recipe: entry.recipe.clone(),
            value,
            needs: needs_of(Some(entry), &entry.recipe),
        });
    }

    fn add_settings(&mut self) {
        // Old spellings resolve first, so two spellings of one setting are seen as one.
        let mut spellings: BTreeMap<String, Vec<(&'a String, &'a SettingValue)>> = BTreeMap::new();
        let config = self.config;
        for (raw, value) in &config.settings {
            let id = match self.catalog.lookup(raw) {
                Lookup::Found { entry, .. } => entry.id().to_string(),
                _ => raw.clone(),
            };
            spellings.entry(id).or_default().push((raw, value));
        }
        for (id, group) in spellings {
            if group.len() > 1 {
                let names = group
                    .iter()
                    .map(|(raw, _)| raw.as_str())
                    .collect::<Vec<_>>()
                    .join(" and ");
                if group.iter().any(|(_, value)| *value != group[0].1) {
                    for (raw, _) in &group {
                        self.skip(raw, SkipReason::RenameCollision, format!("{names} are one setting with different values: both skipped and kept in the file"));
                    }
                    continue;
                }
                self.notes.push(format!(
                    "{names} are one setting with the same value: applied once"
                ));
            }
            // The current spelling is the one to act on and report against.
            let (raw, value) = group
                .iter()
                .copied()
                .find(|(raw, _)| **raw == id)
                .unwrap_or(group[group.len() - 1]);
            self.add_setting(raw, value);
        }
    }

    fn add_setting(&mut self, raw: &'a str, value: &'a SettingValue) {
        if self.skip_if_flagged(raw) {
            return;
        }
        let Some(entry) = self.resolve(raw) else {
            return;
        };
        let Recipe::Setting(setting) = &entry.recipe else {
            self.skip(
                raw,
                SkipReason::WrongKind,
                format!("{raw} isn't a setting: skipped and kept in the file"),
            );
            return;
        };
        if let Err(problem) = setting.check_value(value) {
            self.skip(
                raw,
                SkipReason::BadValue,
                format!("bad value for {raw}: {problem}: skipped and kept in the file"),
            );
            return;
        }
        if setting.is_broken() {
            let why = setting.notes.first().map_or("", String::as_str);
            self.skip(
                raw,
                SkipReason::BrokenHere,
                format!("{raw} doesn't work on this macOS: unavailable here. {why}"),
            );
            return;
        }
        self.add(entry, Why::Selected, Some(value.clone()));
    }

    fn dotfiles(&self) -> Option<DotfilesPlan> {
        let dotfiles = self.config.dotfiles.as_ref()?;
        if self.flagged.contains_key("dotfiles.repo") {
            return None;
        }
        let refused: BTreeSet<&String> = dotfiles
            .links
            .keys()
            .filter(|target| self.flagged.contains_key(*target))
            .collect();
        Some(DotfilesPlan {
            repo: dotfiles.repo.clone(),
            links: dotfiles
                .links
                .iter()
                .filter(|(target, _)| !refused.contains(target))
                .map(|(t, s)| (t.clone(), s.clone()))
                .collect(),
            refused_links: refused.into_iter().cloned().collect(),
            install_script: dotfiles.install_script.clone(),
        })
    }

    fn build(mut self) -> Plan {
        let catalog = self.catalog;
        for base in catalog.base_items() {
            self.add(base, Why::Base, None);
        }
        let mut selected = self.selected_items();
        let mut packages = self.uncovered_packages(&mut selected);
        self.drop_conflicts(&mut selected, &mut packages);
        for entry in selected {
            self.add(entry, Why::Selected, None);
        }
        for (key, cask, name) in packages {
            let recipe = package_recipe(&key, cask, &name);
            self.seen.insert(key.clone());
            self.entries.push(PlanEntry {
                id: key,
                name,
                stage: Stage::User,
                group: group_of(&recipe),
                why: Why::Selected,
                needs: needs_of(None, &recipe),
                recipe,
                value: None,
            });
        }
        self.add_settings();

        let mut taps: BTreeMap<String, Vec<ItemId>> = BTreeMap::new();
        for entry in &self.entries {
            for tap in entry.recipe.taps() {
                taps.entry(tap).or_default().push(entry.id.clone());
            }
        }
        let dotfiles = self.dotfiles();
        sort_for_run(&mut self.entries);
        Plan {
            config_name: self.config.name.clone(),
            entries: self.entries,
            skipped: self.skipped,
            conflicts: self.conflicts,
            notes: self.notes,
            findings: self.findings,
            taps,
            dotfiles,
        }
    }
}
