//! The Catalog as the screens index it: Catalog rows hold positions, not IDs, so toggling is cheap.

use std::sync::OnceLock;

use macflayer_core::catalog::{Catalog, Entry};
use macflayer_core::recipe::{Recipe, SettingRecipe};
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Choice {
    pub id: String,
    pub title: String,
    pub members: Vec<String>,
}

#[derive(Deserialize)]
struct Overlay {
    choices: Vec<Choice>,
}

pub struct CatalogView {
    pub catalog: &'static Catalog,
    pub items: Vec<&'static Entry>,
    pub settings: Vec<&'static SettingRecipe>,
    pub choices: Vec<Choice>,
}

pub fn view() -> &'static CatalogView {
    static VIEW: OnceLock<CatalogView> = OnceLock::new();
    VIEW.get_or_init(|| {
        let catalog = Catalog::shipped();
        let overlay: Overlay =
            serde_json::from_str(include_str!("../data/overlay.json")).expect("overlay.json");
        CatalogView {
            catalog,
            items: catalog.listed().collect(),
            settings: catalog
                .settings()
                .filter_map(|entry| match &entry.recipe {
                    Recipe::Setting(setting) => Some(setting),
                    _ => None,
                })
                .collect(),
            choices: overlay.choices,
        }
    })
}

impl CatalogView {
    pub fn name_of(&self, id: &str) -> String {
        self.catalog.entry(id).map_or_else(
            || id.rsplit(':').next().unwrap_or(id).to_string(),
            |entry| entry.name().to_string(),
        )
    }

    pub fn setting(&self, id: &str) -> Option<&'static SettingRecipe> {
        self.settings
            .iter()
            .copied()
            .find(|setting| setting.id == id)
    }

    pub fn choice_of(&self, id: &str) -> Option<&Choice> {
        self.choices
            .iter()
            .find(|choice| choice.members.iter().any(|member| member == id))
    }
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
