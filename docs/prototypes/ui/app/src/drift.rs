//! Sample Drift for the Drift screen: every kind the Config format and lifecycle designs name.
//! Hardcoded, because no engine has checked anything. #13 still owns how sync behaves.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriftKind {
    /// The Config and the last-applied record agree, and this Mac moved.
    ChangedHere,
    /// The record and this Mac agree, and the Config moved, usually from another Mac.
    ChangedInConfig,
    /// All three differ, so no side is obviously right.
    ChangedBoth,
    /// Removed here, still in the Config.
    RemovedHere,
    /// Applied here earlier, and since dropped from the Config.
    DroppedFromConfig,
    /// A Homebrew package installed here that the Config doesn't name.
    NewHere,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    Later,
    /// Save what's on this Mac to the Config.
    Adopt,
    /// Change this Mac to match the Config.
    Apply,
    /// Keep it installed and stop tracking it.
    KeepUntracked,
    /// Uninstall it from this Mac. Only reached through a confirmation.
    Remove,
}

#[derive(Clone)]
pub struct DriftEntry {
    pub key: String,
    pub name: String,
    pub kind: DriftKind,
    pub in_config: String,
    pub on_mac: String,
    pub last_applied: Option<String>,
    pub decision: Decision,
    pub apply_needs_password: bool,
}

pub struct Unavailable {
    pub name: &'static str,
    pub reason: &'static str,
}

pub const UNAVAILABLE: &[Unavailable] = &[
    Unavailable { name: "Xcode", reason: "No Apple Account on this Mac. You marked this from the list after your first setup." },
    Unavailable { name: "Show Spotlight in the menu bar", reason: "Your organization manages this setting on this Mac." },
    Unavailable { name: "Launchpad in Dock contents", reason: "Launchpad isn't on macOS 27. The rest of your Dock list still applies, and the tile stays in your Config for other Macs." },
];

pub const COULD_NOT_CHECK: &[&str] = &["Show the path bar", "Rearrange Spaces by recent use"];

pub const UNTRACKED: &[&str] = &[
    "Affinity Photo", "Bear", "Calibre", "DaVinci Resolve", "Keynote", "Logic Pro", "Numbers", "Pages", "Pixelmator Pro", "Spotify", "Things", "VLC",
];

fn entry(key: &str, name: &str, kind: DriftKind, in_config: &str, on_mac: &str, last: Option<&str>) -> DriftEntry {
    DriftEntry {
        key: key.to_string(),
        name: name.to_string(),
        kind,
        in_config: in_config.to_string(),
        on_mac: on_mac.to_string(),
        last_applied: last.map(str::to_string),
        decision: Decision::Later,
        apply_needs_password: false,
    }
}

pub fn sample() -> Vec<DriftEntry> {
    vec![
        entry("dock.autohide", "Automatically hide the Dock", DriftKind::ChangedHere, "On", "Off", Some("On")),
        entry("keyboard.key-repeat", "Key repeat rate", DriftKind::ChangedHere, "2", "6", Some("2")),
        entry("dock.position", "Dock position", DriftKind::ChangedInConfig, "Left", "Bottom", Some("Bottom")),
        entry("git.default-branch", "Default Git branch", DriftKind::ChangedBoth, "main", "trunk", Some("master")),
        entry("ghostty", "Ghostty", DriftKind::RemovedHere, "Listed", "Not installed", Some("Installed")),
        DriftEntry {
            apply_needs_password: true,
            ..entry("tailscale", "Tailscale", DriftKind::RemovedHere, "Listed", "Not installed", Some("Installed"))
        },
        entry("docker-desktop", "Docker Desktop", DriftKind::DroppedFromConfig, "Not listed", "Installed", Some("Installed")),
        entry("formula:ripgrep", "ripgrep", DriftKind::NewHere, "Not listed", "Installed with Homebrew", None),
    ]
}
