//! The Drift screen's rows. Real ones come from the engine under `--real`, and these samples stand
//! in for them in the simulator, for tests and for scenes, so both show exactly the same choices.

use serde_json::{Value, json};
use soulcaster_core::drift::{Choice, Row, RowKind, confirm_adopt_removal, shown};

#[derive(Clone)]
pub struct DriftEntry {
    pub row: Row,
    /// `None` is Later: nothing changes for this row.
    pub choice: Option<Choice>,
}

impl DriftEntry {
    pub fn new(row: Row) -> Self {
        Self { row, choice: None }
    }
}

/// A short word for a choice's button.
pub fn button_word(choice: Choice) -> &'static str {
    match choice {
        Choice::Adopt => "Save",
        Choice::Apply => "Apply",
        Choice::NotHere => "Leave",
        Choice::Keep => "Keep",
        Choice::Remove => "Remove\u{2026}",
        Choice::AddBack => "Add back",
        Choice::KeepHere => "Keep here",
    }
}

/// What a value looks like in a column. An app has no value of its own, only whether it's there.
pub fn cell(row: &Row, value: Option<&Value>, in_config: bool) -> String {
    match (value, row.is_setting, in_config) {
        (None, _, true) => "Not listed".to_string(),
        (None, true, false) => "Not set".to_string(),
        (None, false, false) => "Not installed".to_string(),
        (Some(_), false, true) => "Listed".to_string(),
        (Some(_), false, false) => "Installed".to_string(),
        (Some(Value::Bool(true)), true, _) => "On".to_string(),
        (Some(Value::Bool(false)), true, _) => "Off".to_string(),
        (Some(value), true, _) => shown(value),
    }
}

/// The full sentence for a choice, which the design's `{value}` can't always fill for an app.
pub fn choice_help(row: &Row, choice: Choice) -> String {
    let name = &row.name;
    match (choice, row.is_setting) {
        (Choice::Adopt, _) if row.on_mac.is_none() => {
            format!(
                "Take {name} out of your Machine Type. {}",
                confirm_adopt_removal(name)
            )
        }
        (Choice::Adopt, true) => choice.words(&row.on_mac.as_ref().map_or(String::new(), shown)),
        (Choice::Adopt, false) => choice.words(name),
        (Choice::Apply, true) if row.in_config.is_some() => {
            choice.words(&row.in_config.as_ref().map_or(String::new(), shown))
        }
        (Choice::Apply, _) => format!("Install {name} on this Mac"),
        _ => choice.words(""),
    }
}

pub const SAMPLE_UNAVAILABLE: &[(&str, &str)] = &[
    ("Xcode", "No Apple Account on this Mac"),
    (
        "Show Spotlight in the menu bar",
        "Your organization manages this setting",
    ),
    ("Launchpad in Dock contents", "Launchpad isn't on macOS 27"),
];

pub const SAMPLE_COULD_NOT_CHECK: &[&str] =
    &["Show the path bar", "Rearrange Spaces by recent use"];

pub const UNTRACKED: &[&str] = &[
    "Affinity Photo",
    "Bear",
    "Calibre",
    "DaVinci Resolve",
    "Keynote",
    "Logic Pro",
    "Numbers",
    "Pages",
    "Pixelmator Pro",
    "Spotify",
    "Things",
    "VLC",
];

fn entry(
    id: &str,
    name: &str,
    kind: RowKind,
    in_config: Option<Value>,
    on_mac: Option<Value>,
) -> DriftEntry {
    let is_setting = id.contains('.') && !id.contains(':');
    DriftEntry::new(Row {
        id: id.to_string(),
        file_key: id.to_string(),
        name: name.to_string(),
        kind,
        in_config,
        on_mac,
        is_setting,
        removable: kind == RowKind::Dropped && !is_setting,
        adoptable: true,
        waits_for_update: false,
        only_needed: false,
    })
}

pub fn sample() -> Vec<DriftEntry> {
    use RowKind::*;
    vec![
        entry(
            "dock.autohide",
            "Hide the Dock",
            ChangedHere,
            Some(json!(true)),
            Some(json!(false)),
        ),
        entry(
            "keyboard.key-repeat",
            "Key repeat",
            ChangedHere,
            Some(json!(2)),
            Some(json!(6)),
        ),
        entry(
            "dock.position",
            "Dock position",
            ChangedInConfig,
            Some(json!("left")),
            Some(json!("bottom")),
        ),
        entry(
            "git.default-branch",
            "Default branch",
            Conflict,
            Some(json!("main")),
            Some(json!("trunk")),
        ),
        entry("ghostty", "Ghostty", RemovedHere, Some(json!(true)), None),
        entry("zed", "Zed", NewInConfig, Some(json!(true)), None),
        entry(
            "docker-desktop",
            "Docker Desktop",
            Dropped,
            None,
            Some(json!(true)),
        ),
        entry("formula:ripgrep", "ripgrep", Extra, None, Some(json!(true))),
    ]
}
