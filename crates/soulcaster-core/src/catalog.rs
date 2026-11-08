//! The Catalog: everything the app knows how to install or configure. It ships inside the signed
//! app as `catalog/catalog.json`. An ID stays forever and old IDs resolve at read time.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::recipe::{InstallKind, InstallRecipe, Probe, Recipe, Step, tap_of};

const SHIPPED: &str = include_str!("../../../catalog/catalog.json");
const BASE_ITEMS: [&str; 3] = ["command-line-tools", "homebrew", "gh"];

/// Fields the Config format owns, kept beside a Recipe rather than in it.
const LIFECYCLE_FIELDS: [&str; 5] = [
    "renamedTo",
    "deprecated",
    "retired",
    "coversPackages",
    "conflictsWith",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
}

/// What the Catalog screen shows for an Item. Only Items in the launch list have one: a Base Item,
/// a setting or an Item a Recipe pulls in doesn't.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub id: String,
    pub name: String,
    pub section: String,
    pub description: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub tap: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    pub license: License,
    pub install: Install,
    #[serde(default)]
    pub needs_from_person: Vec<ListingNeed>,
    #[serde(default)]
    pub opens_after: Vec<ListingNote>,
    #[serde(default)]
    pub conflicts_with: Vec<ListingConflict>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct License {
    pub model: String,
    #[serde(default)]
    pub spdx: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Install {
    pub kind: InstallSource,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallSource {
    Formula,
    Cask,
    AppStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ListingNeedKind {
    AdminPassword,
    TapApproval,
    License,
    AppleAccount,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ListingNeed {
    pub kind: ListingNeedKind,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ListingNote {
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ListingConflict {
    pub token: String,
    pub kind: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub message: String,
    #[serde(default)]
    pub replaced_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Active,
    /// Still installed, with a message and a replacement offered.
    Deprecated(Notice),
    /// Skipped and kept in the file. Its detection data stays, so a leftover isn't mistaken for
    /// something to Adopt.
    Retired(Notice),
}

/// A package that can't sit beside an Item, as an entry key such as `formula:tailscale`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageConflict {
    pub key: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub recipe: Recipe,
    pub listing: Option<Listing>,
    pub lifecycle: Lifecycle,
    /// Packages that mean this Item, as entry keys. One in a Config's `packages` installs once, through the Item.
    pub covers_packages: Vec<String>,
    pub conflicts_with: Vec<PackageConflict>,
}

impl Entry {
    pub fn id(&self) -> &str {
        self.recipe.id()
    }

    /// The Item's name on screen.
    pub fn name(&self) -> &str {
        self.listing
            .as_ref()
            .map_or_else(|| self.recipe.title(), |listing| &listing.name)
    }
}

/// The answer for an ID from a Config.
#[derive(Debug)]
pub enum Lookup<'a> {
    Found {
        entry: &'a Entry,
        /// The ID it was found through, when that was an old one. The file isn't rewritten.
        renamed_from: Option<&'a str>,
    },
    Unknown {
        suggestion: Option<String>,
    },
    /// The Catalog's lint refuses a rename cycle, and the app also stops on one rather than hanging.
    Cycle,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the Catalog can't be read: {}", self.0)
    }
}

impl std::error::Error for CatalogError {}

#[derive(Deserialize)]
struct CatalogFile {
    #[serde(default)]
    version: String,
    #[serde(default)]
    sections: Vec<Section>,
    #[serde(default)]
    items: Vec<Listing>,
    #[serde(default)]
    recipes: Vec<Value>,
}

#[derive(Debug)]
pub struct Catalog {
    version: String,
    sections: Vec<Section>,
    entries: BTreeMap<String, Entry>,
    renames: BTreeMap<String, String>,
    /// IDs in file order, because that order is the Catalog screen's.
    listed: Vec<String>,
    /// Recipe IDs in file order, which keeps Base Items in their lifecycle order.
    recipe_order: Vec<String>,
}

impl Catalog {
    /// The Catalog inside this release.
    pub fn shipped() -> &'static Catalog {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| Catalog::parse(SHIPPED).expect("the shipped Catalog parses"))
    }

    pub fn parse(json: &str) -> Result<Catalog, CatalogError> {
        let file: CatalogFile =
            serde_json::from_str(json).map_err(|e| CatalogError(e.to_string()))?;
        let mut explicit: BTreeMap<String, Entry> = BTreeMap::new();
        let mut renames = BTreeMap::new();
        let mut recipe_order = Vec::new();
        for raw in file.recipes {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let context = |e: &dyn std::fmt::Display| CatalogError(format!("{id}: {e}"));
            if let Some(target) = raw.get("renamedTo").and_then(Value::as_str) {
                renames.insert(id.clone(), target.to_string());
                continue;
            }
            let entry = entry_from(raw).map_err(|e| context(&e))?;
            recipe_order.push(id.clone());
            explicit.insert(id, entry);
        }

        let mut entries = BTreeMap::new();
        let mut listed = Vec::new();
        for listing in file.items {
            let id = listing.id.clone();
            listed.push(id.clone());
            let mut entry = match explicit.remove(&id) {
                Some(entry) => entry,
                None => {
                    recipe_order.push(id.clone());
                    derived_entry(&listing).map_err(|e| CatalogError(format!("{id}: {e}")))?
                }
            };
            for conflict in &listing.conflicts_with {
                let key = format!("{}:{}", conflict.kind, conflict.token);
                match entry
                    .conflicts_with
                    .iter_mut()
                    .find(|known| known.key == key)
                {
                    Some(known) => known.reason = conflict.reason.clone().or(known.reason.take()),
                    None => entry.conflicts_with.push(PackageConflict {
                        key,
                        reason: conflict.reason.clone(),
                    }),
                }
            }
            entry.listing = Some(listing);
            entries.insert(id, entry);
        }
        entries.append(&mut explicit);

        Ok(Catalog {
            version: file.version,
            sections: file.sections,
            entries,
            renames,
            listed,
            recipe_order,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The Items the Catalog screen lists, in file order. Base Items are never here.
    pub fn listed(&self) -> impl Iterator<Item = &Entry> {
        self.listed.iter().filter_map(|id| self.entries.get(id))
    }

    /// Every entry, in file order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.recipe_order
            .iter()
            .filter_map(|id| self.entries.get(id))
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Always installed, whatever a Config says, in the order a Run takes them.
    pub fn base_items(&self) -> impl Iterator<Item = &Entry> {
        BASE_ITEMS.iter().filter_map(|id| self.entries.get(*id))
    }

    /// Settings the Catalog exposes, in file order.
    pub fn settings(&self) -> impl Iterator<Item = &Entry> {
        self.entries()
            .filter(|entry| matches!(entry.recipe, Recipe::Setting(_)))
    }

    /// An old ID that now means `id`, if it has one.
    pub fn renames_to(&self, id: &str) -> Option<&str> {
        self.renames
            .iter()
            .find(|(_, new)| new.as_str() == id)
            .map(|(old, _)| old.as_str())
    }

    /// Follows old IDs, forever, for an ID read from a Config.
    pub fn lookup<'a>(&'a self, raw: &'a str) -> Lookup<'a> {
        let mut seen: BTreeSet<&str> = BTreeSet::from([raw]);
        let mut id = raw;
        while let Some(next) = self.renames.get(id) {
            if !seen.insert(next) {
                return Lookup::Cycle;
            }
            id = next;
        }
        match self.entries.get(id) {
            Some(entry) => Lookup::Found {
                entry,
                renamed_from: (id != raw).then_some(raw),
            },
            None => Lookup::Unknown {
                suggestion: self.suggest(raw),
            },
        }
    }

    /// The Item a package key means, if any. A retired Item no longer claims its packages.
    pub fn owner_of(&self, key: &str) -> Option<&Entry> {
        self.entries.values().find(|entry| {
            !matches!(entry.lifecycle, Lifecycle::Retired(_))
                && entry.covers_packages.iter().any(|covered| covered == key)
        })
    }

    /// A close ID, so `ghosty` shows "did you mean ghostty" before a Run.
    fn suggest(&self, raw: &str) -> Option<String> {
        self.entries
            .keys()
            .map(|id| (similarity(raw, id), id))
            .filter(|(score, _)| *score >= 0.8)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id.clone())
    }

    /// Rules over the whole Catalog: everything that has to hold for Runs to be sound. Empty means
    /// clean. CI runs this on the shipped Catalog.
    pub fn lint(&self) -> Vec<String> {
        let mut problems = Vec::new();
        self.lint_base(&mut problems);
        self.lint_renames(&mut problems);
        for entry in self.entries.values() {
            self.lint_entry(entry, &mut problems);
        }
        self.lint_requirement_cycles(&mut problems);
        let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for entry in self
            .entries
            .values()
            .filter(|e| !matches!(e.lifecycle, Lifecycle::Retired(_)))
        {
            for key in &entry.covers_packages {
                owners.entry(key).or_default().push(entry.id());
            }
        }
        for (key, ids) in owners.iter().filter(|(_, ids)| ids.len() > 1) {
            problems.push(format!("{key} is covered by more than one Item: {ids:?}"));
        }
        problems
    }

    fn lint_base(&self, problems: &mut Vec<String>) {
        let base: BTreeSet<&str> = self
            .entries
            .values()
            .filter(|e| e.recipe.is_base())
            .map(Entry::id)
            .collect();
        // Fragments of a Catalog, such as a test's, may have no Base Items at all.
        if !base.is_empty() && base != BTreeSet::from(BASE_ITEMS) {
            problems.push(format!(
                "base Items must be exactly Command Line Tools, Homebrew and gh, got {base:?}"
            ));
        }
    }

    fn lint_renames(&self, problems: &mut Vec<String>) {
        for (old, target) in &self.renames {
            if matches!(self.lookup(old), Lookup::Cycle) {
                problems.push(format!("{old}: rename cycle"));
            } else if !self.entries.contains_key(target) && !self.renames.contains_key(target) {
                problems.push(format!("{old}: renamedTo names an unknown ID"));
            } else if old.contains('.') != target.contains('.') {
                problems.push(format!(
                    "{old}: a rename can't change an Item into a setting"
                ));
            }
        }
    }

    fn known(&self, id: &str) -> bool {
        self.entries.contains_key(id) || self.renames.contains_key(id)
    }

    fn lint_entry(&self, entry: &Entry, problems: &mut Vec<String>) {
        let id = entry.id();
        if let Lifecycle::Deprecated(notice) | Lifecycle::Retired(notice) = &entry.lifecycle
            && notice
                .replaced_by
                .as_deref()
                .is_some_and(|replacement| !self.known(replacement))
        {
            problems.push(format!("{id}: replacedBy names an unknown ID"));
        }
        for requirement in entry.recipe.requires() {
            if !self.entries.contains_key(requirement) {
                problems.push(format!("{id}: unknown requirement {requirement}"));
            }
        }
        for key in &entry.covers_packages {
            if !(key.starts_with("formula:") || key.starts_with("cask:")) {
                problems.push(format!(
                    "{id}: coversPackages entry {key} needs formula: or cask:"
                ));
            }
        }
        for conflict in &entry.conflicts_with {
            if !(conflict.key.starts_with("formula:") || conflict.key.starts_with("cask:")) {
                problems.push(format!(
                    "{id}: conflictsWith entry {} needs formula: or cask:",
                    conflict.key
                ));
            }
            if entry.covers_packages.contains(&conflict.key) {
                problems.push(format!(
                    "{id}: {} can't be both covered and a conflict",
                    conflict.key
                ));
            }
        }
        match &entry.recipe {
            Recipe::Install(recipe) => self.lint_install(entry, recipe, problems),
            Recipe::Setting(setting) => {
                if setting.stores.is_empty() {
                    problems.push(format!("{id}: a setting needs a read path"));
                }
                if setting.value_type == crate::recipe::ValueType::Integer
                    && (setting.minimum.is_none() || setting.maximum.is_none())
                {
                    problems.push(format!("{id}: an integer setting needs bounds"));
                }
                if let Some(pattern) = &setting.pattern
                    && regex::Regex::new(pattern).is_err()
                {
                    problems.push(format!("{id}: a pattern can't use lookaround, and it must compile under plain regex"));
                }
                if setting.is_broken() && !setting.evidence.confirmed_on.is_empty() {
                    problems.push(format!("{id}: broken and confirmed can't both be true"));
                }
                self.lint_git_requirement(id, &setting.stores, &setting.requires, problems);
            }
            Recipe::MachineValues(values) => {
                let stores: Vec<_> = values.values.iter().map(|v| v.store.clone()).collect();
                self.lint_git_requirement(id, &stores, &values.requires, problems);
            }
        }
    }

    fn lint_git_requirement(
        &self,
        id: &str,
        stores: &[crate::recipe::Store],
        requires: &[String],
        problems: &mut Vec<String>,
    ) {
        let uses_git = stores
            .iter()
            .any(|s| matches!(s, crate::recipe::Store::GitConfig { .. }));
        if uses_git && !requires.iter().any(|r| r == "command-line-tools") {
            problems.push(format!("{id}: /usr/bin/git is a stub until Command Line Tools exist, so it must require command-line-tools"));
        }
    }

    fn lint_install(&self, entry: &Entry, recipe: &InstallRecipe, problems: &mut Vec<String>) {
        let id = entry.id();
        if recipe.checks.is_empty() {
            let what = if matches!(entry.lifecycle, Lifecycle::Retired(_)) {
                "a retired Item keeps its detection data"
            } else {
                "an Item needs a read path"
            };
            problems.push(format!("{id}: {what}"));
        }
        let third_party = !entry.recipe.taps().is_empty();
        for step in &recipe.steps {
            if let Some(name) = step.brew_name() {
                if !recipe.requires.iter().any(|r| r == "homebrew") {
                    problems.push(format!("{id}: a brew step must require homebrew"));
                }
                let kind = if matches!(step, Step::BrewCask { .. }) {
                    "cask"
                } else {
                    "formula"
                };
                let key = format!("{kind}:{name}");
                if !self.is_fragment() && !entry.covers_packages.contains(&key) {
                    problems.push(format!(
                        "{id}: coversPackages must list {key}, which its steps install"
                    ));
                }
            }
        }
        if third_party && entry.recipe.has_admin_step() {
            problems.push(format!(
                "{id}: an Item that loads tap code can't also use the helper"
            ));
        }
        if recipe.bootstrap.is_some() && id != "gh" {
            problems.push(format!("{id}: only gh may bootstrap outside Homebrew"));
        }
        if entry.recipe.has_admin_step() {
            for requirement in &recipe.requires {
                let admin = self
                    .entries
                    .get(requirement)
                    .is_none_or(|r| r.recipe.has_admin_step());
                if !admin {
                    problems.push(format!("{id}: an admin Item may require only admin Items, and {requirement} isn't one"));
                }
            }
        }
    }

    /// The lifecycle's examples predate `coversPackages`, which the Config format adds.
    fn is_fragment(&self) -> bool {
        self.entries
            .values()
            .all(|entry| entry.covers_packages.is_empty())
    }

    fn lint_requirement_cycles(&self, problems: &mut Vec<String>) {
        fn visit<'a>(
            catalog: &'a Catalog,
            id: &'a str,
            path: &mut Vec<&'a str>,
            done: &mut BTreeSet<&'a str>,
            problems: &mut Vec<String>,
        ) {
            if path.contains(&id) {
                problems.push(format!("{id}: requirement cycle through {path:?}"));
                return;
            }
            if !done.insert(id) {
                return;
            }
            let Some(entry) = catalog.entries.get(id) else {
                return;
            };
            path.push(id);
            for requirement in entry.recipe.requires() {
                visit(catalog, requirement, path, done, problems);
            }
            path.pop();
        }
        let mut done = BTreeSet::new();
        for id in self.entries.keys() {
            visit(self, id, &mut Vec::new(), &mut done, problems);
        }
    }
}

fn entry_from(mut raw: Value) -> Result<Entry, String> {
    let object = raw.as_object_mut().ok_or("a recipe must be an object")?;
    let mut owned: BTreeMap<&str, Value> = BTreeMap::new();
    for field in LIFECYCLE_FIELDS {
        if let Some(value) = object.remove(field) {
            owned.insert(field, value);
        }
    }
    let lifecycle = if let Some(retired) = owned.remove("retired") {
        Lifecycle::Retired(serde_json::from_value(retired).map_err(|e| e.to_string())?)
    } else if let Some(deprecated) = owned.remove("deprecated") {
        Lifecycle::Deprecated(serde_json::from_value(deprecated).map_err(|e| e.to_string())?)
    } else {
        Lifecycle::Active
    };
    let strings = |value: Option<Value>| -> Result<Vec<String>, String> {
        value.map_or(Ok(vec![]), |v| {
            serde_json::from_value(v).map_err(|e| e.to_string())
        })
    };
    let covers_packages = strings(owned.remove("coversPackages"))?;
    let conflicts_with = strings(owned.remove("conflictsWith"))?
        .into_iter()
        .map(|key| PackageConflict { key, reason: None })
        .collect();
    let recipe = serde_json::from_value(raw).map_err(|e| e.to_string())?;
    Ok(Entry {
        recipe,
        listing: None,
        lifecycle,
        covers_packages,
        conflicts_with,
    })
}

/// A plain Item's Recipe, worked out from its install data so 200 Items don't repeat one shape.
/// The name is fully qualified for a tap, which trusts only that one formula or cask.
fn derived_entry(listing: &Listing) -> Result<Entry, String> {
    let token = listing
        .install
        .token
        .as_deref()
        .ok_or("an Item with no token needs an explicit recipe")?;
    let name = match &listing.tap {
        Some(tap) => format!("{tap}/{token}"),
        None => token.to_string(),
    };
    let needs_admin = listing
        .needs_from_person
        .iter()
        .find(|need| need.kind == ListingNeedKind::AdminPassword);
    let mut notes: Vec<String> = listing
        .opens_after
        .iter()
        .map(|note| note.detail.clone())
        .collect();
    let (kind, step, check, key) = match listing.install.kind {
        InstallSource::Formula => {
            // A formula is never privileged: its admin need is a caveat for afterwards, so it's a note.
            notes.extend(needs_admin.map(|need| need.detail.clone()));
            (
                InstallKind::Cli,
                Step::BrewFormula {
                    name: name.clone(),
                    unless: None,
                },
                Probe::BrewFormula { name: name.clone() },
                format!("formula:{name}"),
            )
        }
        InstallSource::Cask => (
            InstallKind::App,
            Step::BrewCask {
                name: name.clone(),
                admin: needs_admin.is_some() && tap_of(&name).is_none(),
                unless: None,
            },
            Probe::BrewCask { name: name.clone() },
            format!("cask:{name}"),
        ),
        InstallSource::AppStore => {
            return Err("an App Store Item needs an explicit recipe".to_string());
        }
    };
    Ok(Entry {
        recipe: Recipe::Install(InstallRecipe {
            id: listing.id.clone(),
            kind,
            title: listing.name.clone(),
            base: false,
            requires: vec!["homebrew".to_string()],
            min_macos: None,
            conflicts: vec![],
            checks: vec![check],
            facts: vec![],
            steps: vec![step],
            notes,
            bootstrap: None,
        }),
        listing: None,
        lifecycle: Lifecycle::Active,
        covers_packages: vec![key],
        conflicts_with: vec![],
    })
}

/// 1 minus the edit distance over the longer length.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = previous + usize::from(ca != cb);
            previous = row[j + 1];
            row[j + 1] = cost.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    let longest = a.len().max(b.len()).max(1);
    1.0 - row[b.len()] as f64 / longest as f64
}
