//! Sample Drift for the Drift screen: every kind the Config format and lifecycle designs name.
//! Hardcoded, because no engine compares a Config with a Machine yet.

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
    pub decision: Decision,
    pub apply_needs_password: bool,
}

pub struct Unavailable {
    pub name: &'static str,
    pub reason: &'static str,
}

pub const UNAVAILABLE: &[Unavailable] = &[
    Unavailable {
        name: "Xcode",
        reason: "No Apple Account on this Mac",
    },
    Unavailable {
        name: "Show Spotlight in the menu bar",
        reason: "Your organization manages this setting",
    },
    Unavailable {
        name: "Launchpad in Dock contents",
        reason: "Launchpad isn't on macOS 27",
    },
];

pub const COULD_NOT_CHECK: &[&str] = &["Show the path bar", "Rearrange Spaces by recent use"];

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

fn entry(key: &str, name: &str, kind: DriftKind, in_config: &str, on_mac: &str) -> DriftEntry {
    DriftEntry {
        key: key.to_string(),
        name: name.to_string(),
        kind,
        in_config: in_config.to_string(),
        on_mac: on_mac.to_string(),
        decision: Decision::Later,
        apply_needs_password: false,
    }
}

pub fn sample() -> Vec<DriftEntry> {
    vec![
        entry(
            "dock.autohide",
            "Hide the Dock",
            DriftKind::ChangedHere,
            "On",
            "Off",
        ),
        entry(
            "keyboard.key-repeat",
            "Key repeat",
            DriftKind::ChangedHere,
            "2",
            "6",
        ),
        entry(
            "dock.position",
            "Dock position",
            DriftKind::ChangedInConfig,
            "Left",
            "Bottom",
        ),
        entry(
            "git.default-branch",
            "Default branch",
            DriftKind::ChangedBoth,
            "main",
            "trunk",
        ),
        entry(
            "ghostty",
            "Ghostty",
            DriftKind::RemovedHere,
            "Installed",
            "Not installed",
        ),
        DriftEntry {
            apply_needs_password: true,
            ..entry(
                "tailscale",
                "Tailscale",
                DriftKind::RemovedHere,
                "Installed",
                "Not installed",
            )
        },
        entry(
            "docker-desktop",
            "Docker Desktop",
            DriftKind::DroppedFromConfig,
            "Not listed",
            "Installed",
        ),
        entry(
            "formula:ripgrep",
            "ripgrep",
            DriftKind::NewHere,
            "Not listed",
            "Installed",
        ),
    ]
}
