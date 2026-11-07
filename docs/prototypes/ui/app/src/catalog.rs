//! The Catalog as the UI reads it. To use a newer Catalog, replace `data/catalog.json`, or point
//! `SOULCASTER_CATALOG` at a file: nothing else knows where it came from. Fields the UI doesn't
//! read are ignored, and the ones it does read default, so a changed file degrades instead of
//! failing to start.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
struct CatalogFile {
    sections: Vec<Section>,
    items: Vec<Item>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub summary: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub section: String,
    pub description: String,
    pub group: Option<String>,
    pub tap: Option<String>,
    pub license: License,
    pub install: Install,
    pub needs_from_person: Vec<Need>,
    pub opens_after: Vec<Note>,
    pub conflicts_with: Vec<Conflict>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct License {
    pub model: String,
    pub spdx: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Install {
    pub kind: String,
    pub token: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Need {
    pub kind: String,
    pub detail: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Note {
    pub detail: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Conflict {
    pub token: String,
    pub reason: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Setting {
    pub id: String,
    pub title: String,
    pub value_type: String,
    pub values: Vec<String>,
    pub unset_means: Option<serde_json::Value>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub evidence: Evidence,
    pub notes: Vec<String>,
    pub requires: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Evidence {
    pub group: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SettingsFile {
    settings: Vec<Setting>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Choice {
    pub id: String,
    pub title: String,
    pub members: Vec<String>,
    pub advice: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct PersonStep {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub action: String,
    pub kind: String,
}

/// What the UI needs that the Catalog doesn't hold yet. Sample data: see `data/overlay.json`.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Overlay {
    pricing: BTreeMap<String, String>,
    choices: Vec<Choice>,
    requires: BTreeMap<String, Vec<String>>,
    person_steps: BTreeMap<String, PersonStep>,
}

/// How an Item costs money, in the words on screen.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Price {
    /// Open source or free: nothing to show.
    Free,
    FreePlan,
    PaidAtWork,
    Subscription,
    Paid,
    /// The Catalog hasn't classified it yet.
    NotChecked,
}

pub struct BaseItem {
    pub id: &'static str,
    pub name: &'static str,
    pub why: &'static str,
}

pub const BASE: &[BaseItem] = &[
    BaseItem { id: "command-line-tools", name: "Command Line Tools", why: "Apple's compilers and Git. Homebrew needs them." },
    BaseItem { id: "homebrew", name: "Homebrew", why: "Installs almost everything below." },
    BaseItem { id: "gh", name: "GitHub CLI", why: "Signs you in to GitHub and reads your Config." },
];

pub struct Catalog {
    pub sections: Vec<Section>,
    pub items: Vec<Item>,
    pub settings: Vec<Setting>,
    pub choices: Vec<Choice>,
    pricing: BTreeMap<String, String>,
    requires: BTreeMap<String, Vec<String>>,
    person_steps: BTreeMap<String, PersonStep>,
    by_id: BTreeMap<String, usize>,
    by_token: BTreeMap<String, usize>,
}

impl Catalog {
    fn load() -> Catalog {
        let raw = std::env::var("SOULCASTER_CATALOG").ok().and_then(|path| std::fs::read_to_string(path).ok());
        let file: CatalogFile = serde_json::from_str(raw.as_deref().unwrap_or(include_str!("../data/catalog.json")))
            .expect("the Catalog file isn't valid JSON for this schema");
        let settings: SettingsFile = serde_json::from_str(include_str!("../data/settings.json")).expect("settings.json");
        let overlay: Overlay = serde_json::from_str(include_str!("../data/overlay.json")).expect("overlay.json");
        let by_id = file.items.iter().enumerate().map(|(i, item)| (item.id.clone(), i)).collect();
        let by_token = file.items.iter().enumerate().filter_map(|(i, item)| item.install.token.clone().map(|t| (t, i))).collect();
        Catalog {
            sections: file.sections,
            items: file.items,
            settings: settings.settings,
            choices: overlay.choices,
            pricing: overlay.pricing,
            requires: overlay.requires,
            person_steps: overlay.person_steps,
            by_id,
            by_token,
        }
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.by_id.get(id).map(|&i| &self.items[i])
    }

    pub fn setting(&self, id: &str) -> Option<&Setting> {
        self.settings.iter().find(|s| s.id == id)
    }

    /// An Item's name on screen; a pulled-in requirement may not be in the Catalog file yet.
    pub fn name_of(&self, id: &str) -> String {
        if let Some(base) = BASE.iter().find(|b| b.id == id) {
            return base.name.to_string();
        }
        if let Some(setting) = self.setting(id) {
            return setting.title.clone();
        }
        self.item(id).map_or_else(|| id.to_string(), |item| item.name.clone())
    }

    pub fn price(&self, item: &Item) -> Price {
        match self.pricing.get(&item.id).map(String::as_str) {
            Some("paid") => Price::Paid,
            Some("free-plan") => Price::FreePlan,
            Some("paid-at-work") => Price::PaidAtWork,
            Some("subscription") => Price::Subscription,
            _ => match item.license.model.as_str() {
                "open-source" | "free" => Price::Free,
                "freemium" => Price::FreePlan,
                "paid" => Price::Paid,
                _ => Price::NotChecked,
            },
        }
    }

    pub fn requires(&self, id: &str) -> &[String] {
        self.requires.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn person_step(&self, id: &str) -> Option<&PersonStep> {
        self.person_steps.get(id)
    }

    /// Selected Items a hard conflict rules out, resolved through install tokens: the Catalog
    /// lists conflicts by Homebrew name, and only some of those are Items.
    pub fn conflicts_of<'a>(&'a self, item: &'a Item) -> Vec<(&'a Item, &'a str)> {
        item.conflicts_with
            .iter()
            .filter_map(|c| self.by_token.get(&c.token).map(|&i| (&self.items[i], c.reason.as_str())))
            .collect()
    }

    pub fn choice_of(&self, id: &str) -> Option<&Choice> {
        self.choices.iter().find(|c| c.members.iter().any(|m| m == id))
    }
}

pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(Catalog::load)
}

/// The words a settings surface shows under, from its ID's first part.
pub fn surface_name(id: &str) -> &'static str {
    match id.split('.').next().unwrap_or("") {
        "dock" => "Dock",
        "finder" => "Finder",
        "git" => "Git",
        "hot-corners" => "Hot corners",
        "keyboard" => "Keyboard",
        "menu-bar" => "Menu bar",
        "mission-control" | "spaces" => "Mission Control",
        "trackpad" => "Trackpad",
        _ => "Other",
    }
}
