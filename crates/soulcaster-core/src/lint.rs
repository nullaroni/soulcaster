//! Rules that may change between releases, so they aren't in the frozen schema. On read, a finding
//! skips that entry and reports it, and the rest of the Config applies. Secret detection is best
//! effort: the real guard is that a Config has no free-form field.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::catalog::{Catalog, Lookup};
use crate::config::{Config, SettingValue};
use crate::recipe::{Recipe, Store};

pub const DEFAULT_CLONE_LOCATION: &str = "~/Developer/dotfiles";

/// Where this app keeps the Machine Record, approvals and backups. A link here could grant itself
/// an approval, so it is refused like the clone location.
pub const OWN_FOLDER: &str = "~/Library/Application Support/Soulcaster";

const PROTECTED: &[&str] = &[
    "~/Applications",
    "~/Desktop",
    "~/Documents",
    "~/Downloads",
    "~/Library",
    "~/Movies",
    "~/Music",
    "~/Pictures",
    "~/Public",
    "~/.ssh",
    "~/.gnupg",
    "~/.Trash",
    "~/.config",
    "~/.local",
    "~/.local/bin",
    "~/.local/share",
    "~/Library/Application Support",
    "~/Library/Containers",
    "~/Library/Group Containers",
    "~/Library/Keychains",
    "~/Library/LaunchAgents",
    "~/Library/Mobile Documents",
    "~/Library/Preferences",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingKind {
    SecretValue,
    SecretKey,
    EmailValue,
    HomePathValue,
    MachineValue,
    LinkDuplicate,
    LinkOverlap,
    LinkProtected,
    LinkClone,
    LinkOwnFolder,
    WritesThroughLink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// The entry it applies to: `name`, a setting or Item ID, a link location or `dotfiles.repo`.
    pub at: String,
    pub message: String,
}

impl Finding {
    /// Something a save refuses, because it must not leave this Mac.
    pub fn must_stay_on_this_mac(&self) -> bool {
        matches!(
            self.kind,
            FindingKind::SecretValue
                | FindingKind::SecretKey
                | FindingKind::EmailValue
                | FindingKind::HomePathValue
                | FindingKind::MachineValue
        )
    }

    fn new(kind: FindingKind, at: &str, message: String) -> Self {
        Self {
            kind,
            at: at.to_string(),
            message,
        }
    }
}

/// Every finding for a Config being read. `clone_location` is where this Machine keeps its Dotfiles Repo.
pub fn lint(config: &Config, catalog: &Catalog, clone_location: &str) -> Vec<Finding> {
    let mut found = secret_findings(config, catalog);
    if let Some(dotfiles) = &config.dotfiles {
        found.extend(link_findings(
            dotfiles.links.keys().map(String::as_str),
            clone_location,
        ));
    }
    found.extend(write_through_findings(config, catalog));
    found
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a fixed pattern compiles"))
}

fn secret_key(text: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    regex(
        &CELL,
        r"(?:^|[.-])(?:tokens?|secrets?|passwords?|passwd|pass|pwd|passphrases?|credentials?|api-keys?|apikeys?|private-keys?|auth|license-keys?|sudo)(?:[.-]|$)",
    )
    .is_match(text)
}

fn token(text: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    regex(
        &CELL,
        r"(?:ghp_|gho_|ghs_|ghu_|ghr_|github_pat_|glpat-|xox[abprs]-|AIza|sk_live_|sk_test_|npm_|hf_|pypi-)[A-Za-z0-9_-]{8,}|sk-[A-Za-z0-9_-]{20,}|A(?:KIA|SIA)[0-9A-Z]{16}|-----BEGIN|\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.|\bBearer\s+\S{8,}",
    )
    .is_match(text)
}

fn email(text: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    regex(&CELL, r"^\s*[^@\s]+@[^@\s]+\.[^@\s]+\s*$").is_match(text)
}

fn home_path(text: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    regex(&CELL, r"^\s*(?:file://)?/Users/").is_match(text)
}

fn scan_free_text(found: &mut Vec<Finding>, at: &str, text: &str) {
    if token(text) {
        found.push(Finding::new(
            FindingKind::SecretValue,
            at,
            format!("{at} looks like a token"),
        ));
    } else if email(text) {
        found.push(Finding::new(
            FindingKind::EmailValue,
            at,
            format!("{at} looks like an email address"),
        ));
    } else if home_path(text) {
        found.push(Finding::new(
            FindingKind::HomePathValue,
            at,
            format!("{at} names a home folder, so it won't work on another Mac"),
        ));
    }
}

fn scan_token_only(found: &mut Vec<Finding>, at: &str, text: &str) {
    if token(text) {
        found.push(Finding::new(
            FindingKind::SecretValue,
            at,
            format!("{at} looks like a token"),
        ));
    }
}

fn secret_findings(config: &Config, catalog: &Catalog) -> Vec<Finding> {
    let mut found = Vec::new();
    scan_free_text(&mut found, "name", &config.name);
    for (key, value) in &config.settings {
        if key.starts_with("git.user.") {
            found.push(Finding::new(
                FindingKind::MachineValue,
                key,
                format!("{key} is personal and stays on the Machine"),
            ));
            continue;
        }
        let known = match catalog.lookup(key) {
            Lookup::Found { entry, .. } => Some(entry),
            _ => None,
        };
        if known.is_none() && secret_key(key) {
            found.push(Finding::new(
                FindingKind::SecretKey,
                key,
                format!("{key} looks like a secret's name"),
            ));
        }
        let allowed = known.and_then(|entry| match &entry.recipe {
            Recipe::Setting(setting) => setting.values.as_ref(),
            _ => None,
        });
        let strings: Vec<&String> = match value {
            SettingValue::String(text) => vec![text],
            SettingValue::List(list) => list.iter().collect(),
            _ => vec![],
        };
        for text in strings {
            // A value the Catalog enumerates can't be a secret someone typed. One it only patterns still can.
            if !allowed.is_some_and(|values| values.contains(text)) {
                scan_free_text(&mut found, key, text);
            }
        }
    }
    if let Some(dotfiles) = &config.dotfiles {
        for (target, source) in &dotfiles.links {
            scan_token_only(&mut found, target, target);
            scan_token_only(&mut found, target, source);
        }
        scan_token_only(&mut found, "dotfiles.repo", &dotfiles.repo);
    }
    found
}

/// How macOS's default volume compares names: case-insensitive and normalization-insensitive.
/// Going through uppercase also folds what plain lowercasing leaves alone, such as U+017F to `s`.
pub fn fold(path: &str) -> String {
    let composed = path.nfc().collect::<String>();
    composed
        .to_uppercase()
        .to_lowercase()
        .nfc()
        .collect::<String>()
}

fn overlaps(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
}

fn link_findings<'a>(links: impl Iterator<Item = &'a str>, clone_location: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut names: Vec<&str> = links.collect();
    names.sort_unstable();
    let folded: Vec<String> = names.iter().map(|name| fold(name)).collect();
    for (i, a) in names.iter().enumerate() {
        for (j, b) in names.iter().enumerate().skip(i + 1) {
            if folded[i] == folded[j] {
                found.push(Finding::new(
                    FindingKind::LinkDuplicate,
                    a,
                    format!("{a} and {b} are the same location on this Mac"),
                ));
            } else if overlaps(&folded[i], &folded[j]) {
                found.push(Finding::new(
                    FindingKind::LinkOverlap,
                    a,
                    format!("{a} and {b} overlap, so one would land inside the other"),
                ));
            }
        }
    }
    let clone = fold(clone_location);
    let own = fold(OWN_FOLDER);
    for (name, fold_name) in names.iter().zip(&folded) {
        if overlaps(fold_name, &own) {
            found.push(Finding::new(
                FindingKind::LinkOwnFolder,
                name,
                format!("{name} is at, above or inside Soulcaster's own folder"),
            ));
        }
        if PROTECTED.iter().any(|p| fold(p) == *fold_name) {
            found.push(Finding::new(
                FindingKind::LinkProtected,
                name,
                format!("{name} holds user data, so it can't be replaced by a link"),
            ));
        }
        if overlaps(fold_name, &clone) {
            found.push(Finding::new(
                FindingKind::LinkClone,
                name,
                format!("{name} is at, above or inside the Dotfiles Repo's clone location {clone_location}"),
            ));
        }
    }
    found
}

/// Every link that must not be applied, naming both sides of a clash. A finding names one side,
/// and applying the other would let the file's own spelling choose which link wins.
pub fn refused_links<'a>(
    links: impl Iterator<Item = &'a str>,
    clone_location: &str,
) -> BTreeMap<String, FindingKind> {
    let names: Vec<&str> = links.collect();
    let mut refused = BTreeMap::new();
    for finding in link_findings(names.iter().copied(), clone_location) {
        refused.entry(finding.at).or_insert(finding.kind);
    }
    let folded: Vec<String> = names.iter().map(|name| fold(name)).collect();
    for (i, a) in names.iter().enumerate() {
        for (j, b) in names.iter().enumerate().skip(i + 1) {
            if folded[i] == folded[j] {
                refused
                    .entry((*a).to_string())
                    .or_insert(FindingKind::LinkDuplicate);
                refused
                    .entry((*b).to_string())
                    .or_insert(FindingKind::LinkDuplicate);
            } else if overlaps(&folded[i], &folded[j]) {
                refused
                    .entry((*a).to_string())
                    .or_insert(FindingKind::LinkOverlap);
                refused
                    .entry((*b).to_string())
                    .or_insert(FindingKind::LinkOverlap);
            }
        }
    }
    refused
}

/// Files a Recipe writes to, for the rule that a written value never goes through a link into the repo.
fn written_paths(recipe: &Recipe) -> Vec<String> {
    let stores: Vec<&Store> = match recipe {
        Recipe::Setting(setting) => setting.stores.iter().collect(),
        Recipe::MachineValues(values) => values.values.iter().map(|v| &v.store).collect(),
        Recipe::Install(_) => vec![],
    };
    let mut paths: Vec<String> = stores
        .into_iter()
        .filter_map(|store| match store {
            Store::GitConfig { .. } => Some("~/.gitconfig".to_string()),
            Store::Defaults { domain, .. } if domain.as_str() != "NSGlobalDomain" => {
                Some(format!("~/Library/Preferences/{}.plist", domain.as_str()))
            }
            _ => None,
        })
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn write_through_findings(config: &Config, catalog: &Catalog) -> Vec<Finding> {
    let mut found = Vec::new();
    let Some(dotfiles) = &config.dotfiles else {
        return found;
    };
    let links: Vec<(String, &String)> = dotfiles
        .links
        .keys()
        .map(|target| (fold(target), target))
        .collect();
    for raw in config.items.iter().chain(config.settings.keys()) {
        let Lookup::Found { entry, .. } = catalog.lookup(raw) else {
            continue;
        };
        for path in written_paths(&entry.recipe) {
            let folded = fold(&path);
            if let Some((_, target)) = links.iter().find(|(link, _)| overlaps(&folded, link)) {
                found.push(Finding::new(
                    FindingKind::WritesThroughLink,
                    raw,
                    format!("{raw} writes {path}, which {target} links into the Dotfiles Repo: unavailable here"),
                ));
            }
        }
    }
    found
}
