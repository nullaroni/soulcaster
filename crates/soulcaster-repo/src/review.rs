//! The one review page, as rules: what a Config would do, where each box starts, what each box
//! allows, and the only way boxes become approvals. These are the Import design's rules, and they
//! read no Machine and no network, so a screen and the CLI show the same thing.

use std::collections::{BTreeMap, BTreeSet};

use soulcaster_core::approval::{
    ApprovalStore, ApprovalStoreError, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::catalog::{Catalog, Lookup};
use soulcaster_core::config::{Config, Dotfiles, SettingValue};
use soulcaster_core::lifecycle::SkipReason;
use soulcaster_core::lint::{FindingKind, fold, refused_links};
use soulcaster_core::plan::{NeedKind, Plan, Why};
use soulcaster_core::recipe::{Effect, Recipe, tap_of};

use crate::address::DotfilesSource;
use crate::import::{Fetched, Source};
use crate::text::{claim, escaped, fakes_a_name, lookalikes, skeleton};

/// A section longer than this starts off.
const BIG_LIST: usize = 100;

/// What GitHub said about the Dotfiles Repo. All of it is empty until someone asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DotfilesFacts {
    pub commit: Option<CommitSha>,
    /// GitHub's answer for who owns the repo now, which a transfer changes.
    pub current_owner: Option<String>,
    pub script_text: Option<String>,
}

/// A sign a file is built to mislead. Finding one starts everything off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    pub code: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapPackage {
    pub name: String,
    pub short: String,
    pub via: String,
    pub same_as_base: bool,
    pub same_as_catalog: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapCard {
    pub tap: String,
    pub owner: String,
    pub packages: Vec<TapPackage>,
    /// A package here shares a Base Item's name, so no box is offered.
    pub blocked: bool,
    pub formula_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkRow {
    pub target: String,
    pub target_escaped: String,
    /// The path as it is, because that's what gets linked. Never show this one.
    pub source: String,
    pub source_escaped: String,
    /// What this location does by itself. It explains and doesn't decide.
    pub note: Option<&'static str>,
    pub lookalike: bool,
    pub refused: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DotfilesCard {
    pub repo: String,
    /// What an approval is tied to. `None` when the address isn't a repo anyone can read.
    pub key: Option<String>,
    pub host: Option<String>,
    pub is_github: bool,
    pub commit: Option<CommitSha>,
    pub own: bool,
    pub names_your_repo: bool,
    pub links: Vec<LinkRow>,
    pub script: Option<String>,
    pub script_text: Option<String>,
}

impl DotfilesCard {
    /// The links a person is approving: every one that wasn't refused.
    pub fn live_links(&self) -> BTreeMap<String, String> {
        self.links
            .iter()
            .filter(|link| link.refused.is_none())
            .map(|link| (link.target.clone(), link.source.clone()))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRow {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PulledIn {
    pub name: String,
    pub by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingRow {
    pub id: String,
    pub title: String,
    pub after: String,
    pub effect: &'static str,
    /// A list value replaces the whole list.
    pub whole_list: bool,
    pub if_repo_on: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseRow {
    pub id: String,
    pub name: String,
    pub text: String,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WontApply {
    pub id: String,
    pub text: String,
}

/// Everything the one page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub source: Source,
    pub name: String,
    pub name_escaped: String,
    pub name_has_lookalikes: bool,
    pub signals: Vec<Signal>,
    pub taps: Vec<TapCard>,
    pub dotfiles: Option<DotfilesCard>,
    pub catalog: Vec<CatalogRow>,
    pub official_formulae: Vec<String>,
    pub official_casks: Vec<String>,
    pub big_formulae: bool,
    pub big_casks: bool,
    pub pulled_in: Vec<PulledIn>,
    pub settings: Vec<SettingRow>,
    pub licenses: Vec<LicenseRow>,
    pub later: Vec<String>,
    pub wont_apply: Vec<WontApply>,
    config: Config,
    /// Which taps each listed Item loads, by the ID the file spells it with.
    item_taps: BTreeMap<String, Vec<String>>,
}

/// The state of every box. Nothing makes one but [`Review::starting_boxes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boxes {
    pub catalog: bool,
    pub formulae: bool,
    pub casks: bool,
    pub settings: bool,
    pub licenses: BTreeSet<String>,
    pub taps: BTreeMap<String, bool>,
    pub repo: bool,
    pub script: bool,
}

/// One box a person can press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tick {
    Catalog,
    Formulae,
    Casks,
    Settings,
    License(String),
    Tap(String),
    Repo,
    Script,
}

/// What the page's filled-in button is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Primary {
    /// Start, in words that say what's being left off.
    Start(String),
    Close,
}

/// What a Run from the current boxes would do. The one thing that turns boxes into work.
#[derive(Clone, Debug, PartialEq)]
pub struct RunContents {
    /// The Config to plan a Run from. The kept copy stays as the file was.
    pub config: Config,
    pub taps: Vec<String>,
    pub packages: Vec<String>,
    pub linked: Vec<String>,
    pub clone: Option<(String, CommitSha)>,
    pub script: Option<String>,
    pub licenses: BTreeSet<String>,
}

impl RunContents {
    pub fn is_empty(&self) -> bool {
        self.config.items.is_empty()
            && self.config.packages.is_empty()
            && self.config.settings.is_empty()
            && self.config.dotfiles.is_none()
    }
}

fn link_note(target: &str) -> Option<&'static str> {
    // This volume treats `~/.ZSHENV` and `~/.zshenv` as one file, so the notes do too.
    let folded = fold(target);
    let target = folded.as_str();
    match target {
        "~/.zshenv" => Some("Runs for every zsh, even in scripts"),
        "~/.zshrc" | "~/.zprofile" | "~/.zlogin" | "~/.bashrc" | "~/.bash_profile"
        | "~/.profile" => Some("Runs whenever a terminal opens"),
        "~/.gitconfig" | "~/.config/git" | "~/.config/git/config" => {
            Some("Can run programs during git commands and redirect clones")
        }
        "~/.ssh/authorized_keys" | "~/.ssh/authorized_keys2" => {
            Some("Lets whoever holds these keys log in to this Mac")
        }
        "~/.ssh" | "~/.ssh/config" => Some("Decides where SSH connects, and can run programs"),
        "~/.local/bin/sudo" => Some("Can stand in for sudo and capture your password"),
        "~/library/launchagents" => Some("Starts a program every time you log in"),
        _ if target.starts_with("~/library/launchagents/") => {
            Some("Starts a program every time you log in")
        }
        "~/.local/bin" | "~/bin" => Some("Can stand in for a command you type"),
        _ if target.starts_with("~/.local/bin/") || target.starts_with("~/bin/") => {
            Some("Can stand in for a command you type")
        }
        _ => None,
    }
}

fn refused_words(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::LinkProtected => "This folder holds your data, so a link can't replace it",
        FindingKind::LinkOverlap => "Overlaps another link, so one would land inside the other",
        FindingKind::LinkDuplicate => "Same location as another link on this Mac",
        FindingKind::LinkClone => "Would write into or over the Dotfiles Repo's own folder",
        FindingKind::LinkOwnFolder => "Would write into Soulcaster's own folder",
        _ => "Looks like a secret",
    }
}

fn effect_words(effect: Option<Effect>) -> &'static str {
    match effect {
        Some(Effect::RestartDock) => "Dock restarts",
        Some(Effect::RestartFinder) => "Finder restarts",
        Some(Effect::RestartSystemuiserver | Effect::RestartControlcenter) => "Menu bar restarts",
        Some(Effect::RestartAffectedApps) => "When apps restart",
        Some(Effect::LogOut) => "After you log out",
        Some(Effect::None) | None => "Right away",
    }
}

fn show_value(value: &SettingValue) -> String {
    match value {
        SettingValue::Bool(true) => "On".to_string(),
        SettingValue::Bool(false) => "Off".to_string(),
        SettingValue::Number(number) => number.to_string(),
        SettingValue::String(text) => escaped(text),
        SettingValue::List(list) => format!("{} entries", list.len()),
    }
}

fn base_names(catalog: &Catalog) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for base in catalog.base_items() {
        names.insert(base.id().to_string());
        for key in &base.covers_packages {
            if let Some((_, name)) = key.split_once(':') {
                names.insert(name.to_string());
            }
        }
    }
    names
}

fn host_lookalike(host: &str) -> bool {
    host != "github.com"
        && (skeleton(host).contains("github")
            || host
                .split(['.', ':'])
                .any(|label| label.starts_with("xn--")))
}

fn registrable(host: &str) -> String {
    let host = host.split(':').next().unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    labels[labels.len().saturating_sub(2)..].join(".")
}

impl Review {
    pub fn build(
        fetched: &Fetched,
        signed_in: Option<&str>,
        facts: &DotfilesFacts,
        catalog: &Catalog,
        clone_location: &str,
    ) -> Self {
        let config = &fetched.config;
        let plan = Plan::with_clone_location(config, catalog, clone_location);
        let (taps, item_taps) = tap_cards(config, &plan, catalog);
        let dotfiles = config
            .dotfiles
            .as_ref()
            .map(|d| dotfiles_card(d, &plan, fetched, signed_in, facts, clone_location));
        let mut review = Self {
            source: fetched.source.clone(),
            name: config.name.clone(),
            name_escaped: escaped(&config.name),
            name_has_lookalikes: !lookalikes(&config.name).is_empty(),
            signals: Vec::new(),
            catalog: Vec::new(),
            official_formulae: official(&config.packages.formulae),
            official_casks: official(&config.packages.casks),
            big_formulae: false,
            big_casks: false,
            pulled_in: Vec::new(),
            settings: Vec::new(),
            licenses: Vec::new(),
            later: Vec::new(),
            wont_apply: Vec::new(),
            taps,
            dotfiles,
            config: config.clone(),
            item_taps,
        };
        review.big_formulae = review.official_formulae.len() > BIG_LIST;
        review.big_casks = review.official_casks.len() > BIG_LIST;
        review.fill_from_plan(&plan, catalog);
        review.signals = review.find_signals(fetched);
        review
    }

    fn fill_from_plan(&mut self, plan: &Plan, catalog: &Catalog) {
        for entry in &plan.entries {
            let package = entry.id.starts_with("formula:") || entry.id.starts_with("cask:");
            match (&entry.why, &entry.recipe) {
                (Why::Selected, Recipe::Setting(_)) | (Why::Base, _) => {}
                (Why::Selected, _) if package || !entry.recipe.taps().is_empty() => {}
                (Why::Selected, _) => self.catalog.push(CatalogRow {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                }),
                (Why::PulledIn { by }, _) => self.pulled_in.push(PulledIn {
                    name: entry.name.clone(),
                    by: by.clone(),
                }),
            }
            if let (Recipe::Setting(setting), Some(value)) = (&entry.recipe, &entry.value) {
                self.settings.push(SettingRow {
                    id: entry.id.clone(),
                    title: setting.title.clone(),
                    after: show_value(value),
                    effect: effect_words(setting.effect),
                    whole_list: matches!(value, SettingValue::List(_)),
                    if_repo_on: None,
                });
            }
            for need in &entry.needs {
                match need.kind {
                    NeedKind::License => self.licenses.push(LicenseRow {
                        id: entry.id.clone(),
                        name: entry.name.clone(),
                        text: need.text.clone(),
                        link: need.link.clone(),
                    }),
                    NeedKind::Person | NeedKind::AppleAccount => {
                        self.later.push(need.text.clone());
                    }
                    _ => {}
                }
            }
        }
        for skipped in &plan.skipped {
            if skipped.reason == SkipReason::WritesThroughLink {
                self.add_write_through_setting(&skipped.raw, catalog);
            } else {
                self.wont_apply.push(WontApply {
                    id: escaped(&skipped.raw),
                    text: escaped(&skipped.note),
                });
            }
        }
        self.add_always_installed(catalog);
        for conflict in &plan.conflicts {
            self.wont_apply.push(WontApply {
                id: conflict.item.clone(),
                text: format!(
                    "{} and {} can't both be installed. Neither is, until you pick one.",
                    conflict.item, conflict.other
                ),
            });
        }
    }

    /// The core plan skips this whenever a link could catch the write. Here it stays a row, because
    /// leaving the Dotfiles Repo off means nothing is linked and the setting applies.
    fn add_write_through_setting(&mut self, raw: &str, catalog: &Catalog) {
        let (Some(value), Lookup::Found { entry, .. }) =
            (self.config.settings.get(raw), catalog.lookup(raw))
        else {
            return;
        };
        let Recipe::Setting(setting) = &entry.recipe else {
            return;
        };
        self.settings.push(SettingRow {
            id: entry.id().to_string(),
            title: setting.title.clone(),
            after: show_value(value),
            effect: effect_words(setting.effect),
            whole_list: matches!(value, SettingValue::List(_)),
            if_repo_on: Some(
                "Skipped if you turn the Dotfiles Repo on: it would write into a linked file",
            ),
        });
    }

    fn add_always_installed(&mut self, catalog: &Catalog) {
        let words = "Every Run installs this anyway, so this line does nothing.";
        for raw in &self.config.items {
            if let Lookup::Found { entry, .. } = catalog.lookup(raw)
                && entry.recipe.is_base()
            {
                self.wont_apply.push(WontApply {
                    id: raw.clone(),
                    text: words.to_string(),
                });
            }
        }
        let lists = [
            ("formula", &self.config.packages.formulae),
            ("cask", &self.config.packages.casks),
        ];
        for (prefix, names) in lists {
            for name in names {
                if catalog
                    .owner_of(&format!("{prefix}:{name}"))
                    .is_some_and(|owner| owner.recipe.is_base())
                {
                    self.wont_apply.push(WontApply {
                        id: name.clone(),
                        text: words.to_string(),
                    });
                }
            }
        }
    }

    fn find_signals(&self, fetched: &Fetched) -> Vec<Signal> {
        let mut out = Vec::new();
        let mut signal = |code, text: String| out.push(Signal { code, text });
        let letters: String = lookalikes(&self.name).into_iter().collect();
        if !letters.is_empty() {
            signal(
                "name-lookalike",
                format!(
                    "The name mixes alphabets so its letters pass for others: {}",
                    escaped(&letters)
                ),
            );
        }
        if let Some(word) = claim(&self.name) {
            signal(
                "name-claim",
                format!(
                    "The name says \"{}\". Soulcaster doesn't verify or endorse any Config.",
                    escaped(&word)
                ),
            );
        }
        let owner = fetched.source.repo.owner();
        if let Some(faked) = fakes_a_name(owner) {
            signal(
                "source-lookalike",
                format!(
                    "The account {owner} has a name like {faked}, and nothing connects it to {faked}."
                ),
            );
        }
        for tap in &self.taps {
            if let Some(faked) = fakes_a_name(&tap.owner) {
                signal(
                    "tap-lookalike",
                    format!(
                        "The tap {} has a name like {faked}, and isn't from {faked}.",
                        tap.tap
                    ),
                );
            }
            for package in &tap.packages {
                if package.same_as_base {
                    signal(
                        "same-as-base",
                        format!(
                            "{} shares its name with {}, which Soulcaster always installs. It's a different program.",
                            package.name, package.short
                        ),
                    );
                }
            }
        }
        if let Some(dotfiles) = &self.dotfiles {
            if let Some(host) = dotfiles.host.as_deref()
                && host_lookalike(host)
            {
                signal(
                    "host-lookalike",
                    format!(
                        "The Dotfiles Repo's address starts like GitHub, but the site is {}.",
                        escaped(&registrable(host))
                    ),
                );
            }
            for link in &dotfiles.links {
                if link.lookalike {
                    signal(
                        "link-lookalike",
                        format!(
                            "A link mixes alphabets: {} from {}",
                            link.target_escaped, link.source_escaped
                        ),
                    );
                }
            }
        }
        out
    }

    pub fn fetched_config(&self) -> &Config {
        &self.config
    }

    pub fn install_count(&self) -> usize {
        self.catalog.len()
            + self.official_formulae.len()
            + self.official_casks.len()
            + self.pulled_in.len()
    }

    /// Parts with a box of their own. A blocked tap has no box, so it isn't counted.
    pub fn approval_count(&self) -> usize {
        let taps = self.taps.iter().filter(|tap| !tap.blocked).count();
        let dotfiles = self
            .dotfiles
            .as_ref()
            .map_or(0, |d| 1 + usize::from(d.script.is_some()));
        taps + dotfiles
    }

    pub fn wont_apply_count(&self) -> usize {
        let links = self.dotfiles.as_ref().map_or(0, |d| {
            d.links.iter().filter(|link| link.refused.is_some()).count()
        });
        self.wont_apply.len() + links
    }

    /// What the page opens with: off unless this Machine already approved exactly this, or it's
    /// your own repo. One signal turns everything off, whatever was approved before.
    pub fn starting_boxes(&self, approvals: &Approvals) -> Boxes {
        let safe = self.signals.is_empty();
        let mut boxes = Boxes {
            catalog: safe && !self.catalog.is_empty(),
            formulae: safe && !self.big_formulae && !self.official_formulae.is_empty(),
            casks: safe && !self.big_casks && !self.official_casks.is_empty(),
            settings: safe && !self.settings.is_empty(),
            licenses: BTreeSet::new(),
            taps: self
                .taps
                .iter()
                .map(|tap| {
                    let approved = safe
                        && !tap.blocked
                        && tap
                            .packages
                            .iter()
                            .all(|p| approvals.packages.contains(&p.name));
                    (tap.tap.clone(), approved)
                })
                .collect(),
            repo: false,
            script: false,
        };
        if let Some(dotfiles) = &self.dotfiles
            && safe
            && let (Some(key), Some(commit)) = (&dotfiles.key, &dotfiles.commit)
        {
            let same = approvals.dotfiles.as_ref().is_some_and(|held| {
                held.repo == *key && held.commit == *commit && held.links == dotfiles.live_links()
            });
            // Your own repo starts on only the first time. With an approval on record, a newer
            // commit or one more link is a different approval, and the move screen handles it.
            boxes.repo = same || (dotfiles.own && approvals.dotfiles.is_none());
            let script_same = approvals.script.as_ref().is_some_and(|held| {
                held.repo == *key
                    && held.commit == *commit
                    && Some(&held.path) == dotfiles.script.as_ref()
            });
            boxes.script = boxes.repo && script_same && dotfiles.script_text.is_some();
        }
        boxes
    }

    /// Whether a box can change. An approval is never implied by another one.
    pub fn can_tick(&self, boxes: &Boxes, tick: &Tick) -> bool {
        match tick {
            Tick::Catalog => !self.catalog.is_empty(),
            Tick::Formulae => !self.official_formulae.is_empty(),
            Tick::Casks => !self.official_casks.is_empty(),
            Tick::Settings => !self.settings.is_empty(),
            Tick::License(id) => self.licenses.iter().any(|license| license.id == *id),
            Tick::Tap(name) => self.taps.iter().any(|t| t.tap == *name && !t.blocked),
            Tick::Repo => self.dotfiles.as_ref().is_some_and(|d| d.commit.is_some()),
            Tick::Script => {
                boxes.repo
                    && self
                        .dotfiles
                        .as_ref()
                        .is_some_and(|d| d.script.is_some() && d.script_text.is_some())
            }
        }
    }

    pub fn toggle(&self, boxes: &mut Boxes, tick: &Tick) {
        if !self.can_tick(boxes, tick) {
            return;
        }
        let flip = |on: &mut bool| *on = !*on;
        match tick {
            Tick::Catalog => flip(&mut boxes.catalog),
            Tick::Formulae => flip(&mut boxes.formulae),
            Tick::Casks => flip(&mut boxes.casks),
            Tick::Settings => flip(&mut boxes.settings),
            Tick::License(id) => {
                if !boxes.licenses.remove(id) {
                    boxes.licenses.insert(id.clone());
                }
            }
            Tick::Tap(name) => flip(boxes.taps.entry(name.clone()).or_insert(false)),
            Tick::Repo => {
                flip(&mut boxes.repo);
                if !boxes.repo {
                    boxes.script = false;
                }
            }
            Tick::Script => flip(&mut boxes.script),
        }
    }

    /// Turns on what needs no approval, for a person who has read a warning and wants the rest.
    pub fn select_what_needs_no_approval(&self, boxes: &mut Boxes) {
        boxes.catalog = !self.catalog.is_empty();
        boxes.settings = !self.settings.is_empty();
        boxes.formulae = !self.big_formulae && !self.official_formulae.is_empty();
        boxes.casks = !self.big_casks && !self.official_casks.is_empty();
    }

    fn tap_on(&self, boxes: &Boxes, name: &str) -> bool {
        boxes.taps.get(name).copied().unwrap_or(false)
            && self.taps.iter().any(|tap| tap.tap == name && !tap.blocked)
    }

    /// The whole file, narrowed to what the boxes allow. A box forced on by mistake still can't
    /// install a blocked tap or run a script nobody could read.
    pub fn run_contents(&self, boxes: &Boxes) -> RunContents {
        let mut config = self.config.clone();
        config.items.retain(|raw| match self.item_taps.get(raw) {
            Some(taps) => taps.iter().all(|tap| self.tap_on(boxes, tap)),
            None => boxes.catalog,
        });
        let keep = |name: &String, official_on: bool| match tap_of(name) {
            Some(tap) => self.tap_on(boxes, &tap),
            None => official_on,
        };
        config.packages.formulae.retain(|n| keep(n, boxes.formulae));
        config.packages.casks.retain(|n| keep(n, boxes.casks));
        if !boxes.settings {
            config.settings.clear();
        }
        config.dotfiles = None;

        let mut out = RunContents {
            config,
            taps: Vec::new(),
            packages: Vec::new(),
            linked: Vec::new(),
            clone: None,
            script: None,
            licenses: boxes.licenses.clone(),
        };
        for tap in &self.taps {
            if self.tap_on(boxes, &tap.tap) {
                out.taps.push(tap.tap.clone());
                out.packages
                    .extend(tap.packages.iter().map(|p| p.name.clone()));
            }
        }
        if let Some(card) = &self.dotfiles
            && boxes.repo
            && let (Some(key), Some(commit)) = (&card.key, &card.commit)
        {
            let links = card.live_links();
            out.clone = Some((key.clone(), commit.clone()));
            out.linked = links.keys().cloned().collect();
            if boxes.script && card.script_text.is_some() {
                out.script = card.script.clone();
            }
            out.config.dotfiles = Some(Dotfiles {
                repo: card.repo.clone(),
                links,
                install_script: out.script.clone(),
            });
        }
        out
    }

    pub fn nothing_selected(&self, boxes: &Boxes) -> bool {
        self.run_contents(boxes).is_empty()
    }

    /// Start Run: the one call that records approvals. Keep a copy never reaches this.
    pub fn record_start(
        &self,
        boxes: &Boxes,
        store: &dyn ApprovalStore,
    ) -> Result<RunContents, ApprovalStoreError> {
        let approvals = self.approvals_after_run(boxes, &store.read()?);
        store.write(&approvals)?;
        Ok(self.run_contents(boxes))
    }

    /// What Start Run writes. It is the only path from boxes to approvals.
    pub fn approvals_after_run(&self, boxes: &Boxes, existing: &Approvals) -> Approvals {
        let run = self.run_contents(boxes);
        let mut out = existing.clone();
        out.packages.extend(run.packages.iter().cloned());
        out.dotfiles = None;
        out.script = None;
        if let (Some((repo, commit)), Some(card)) = (&run.clone, &self.dotfiles) {
            out.dotfiles = Some(DotfilesApproval {
                repo: repo.clone(),
                commit: commit.clone(),
                links: card.live_links(),
            });
            if let Some(path) = &run.script {
                let ran = existing.script.as_ref().is_some_and(|held| {
                    held.repo == *repo && held.commit == *commit && held.path == *path && held.ran
                });
                out.script = Some(ScriptApproval {
                    repo: repo.clone(),
                    commit: commit.clone(),
                    path: path.clone(),
                    ran,
                });
            }
        }
        out
    }

    fn names_on_off(&self, boxes: &Boxes) -> (Vec<String>, Vec<String>) {
        let (mut on, mut off) = (Vec::new(), Vec::new());
        for tap in self.taps.iter().filter(|tap| !tap.blocked) {
            if self.tap_on(boxes, &tap.tap) {
                on.push(tap.tap.clone());
            } else {
                off.push(tap.tap.clone());
            }
        }
        if let Some(card) = &self.dotfiles {
            let repo = format!("the Dotfiles Repo {}", escaped(&card.repo));
            if boxes.repo {
                on.push(repo);
            } else {
                off.push(repo);
            }
            if let Some(script) = &card.script {
                if boxes.script {
                    on.push(script.clone());
                } else {
                    off.push(script.clone());
                }
            }
        }
        (on, off)
    }

    /// The button's words. A misleading file's filled-in button is Close, whatever the boxes say.
    pub fn primary(&self, boxes: &Boxes) -> Primary {
        if !self.signals.is_empty() {
            return Primary::Close;
        }
        Primary::Start(self.start_words(boxes))
    }

    pub fn start_words(&self, boxes: &Boxes) -> String {
        let (on, off) = self.names_on_off(boxes);
        match (on.len(), off.len()) {
            (0, 0) => "Start".to_string(),
            (0, _) => "Start without them".to_string(),
            (1, _) => "Start with 1 approval".to_string(),
            (n, _) => format!("Start with {n} approvals"),
        }
    }

    /// What a press would do, said before it's pressed.
    pub fn sentence(&self, boxes: &Boxes) -> String {
        if self.nothing_selected(boxes) {
            return "Nothing is selected, so a Run would have nothing to do.".to_string();
        }
        let (on, off) = self.names_on_off(boxes);
        match (on.is_empty(), off.is_empty()) {
            (true, true) => "Nothing here needs approval.".to_string(),
            (true, false) => format!("Left off: {}.", off.join(", ")),
            (false, true) => format!("Allowing: {}.", on.join(", ")),
            (false, false) => format!("Allowing: {}. Left off: {}.", on.join(", "), off.join(", ")),
        }
    }
}

fn official(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| tap_of(name).is_none())
        .cloned()
        .collect()
}

fn short_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

fn tap_cards(
    config: &Config,
    plan: &Plan,
    catalog: &Catalog,
) -> (Vec<TapCard>, BTreeMap<String, Vec<String>>) {
    let base = base_names(catalog);
    let mut packages: BTreeMap<String, Vec<TapPackage>> = BTreeMap::new();
    let lists = [
        ("packages.formulae", &config.packages.formulae),
        ("packages.casks", &config.packages.casks),
    ];
    for (field, names) in lists {
        for (line, name) in names.iter().enumerate() {
            let Some(tap) = tap_of(name) else { continue };
            let short = short_name(name);
            let twin = catalog
                .entry(&short)
                .filter(|entry| !entry.recipe.is_base())
                .map(|entry| entry.name().to_string());
            packages.entry(tap).or_default().push(TapPackage {
                name: name.clone(),
                via: format!("line {} of {} in {field}", line + 1, names.len()),
                same_as_base: base.contains(&short),
                same_as_catalog: twin,
                short,
            });
        }
    }
    // A Catalog Item that loads a tap is the same kind of card, whoever suggested it.
    let mut item_taps: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for raw in &config.items {
        let Lookup::Found { entry, .. } = catalog.lookup(raw) else {
            continue;
        };
        let taps = entry.recipe.taps();
        if !taps.is_empty() && plan.entries.iter().any(|e| e.id == entry.id()) {
            item_taps.insert(raw.clone(), taps);
        }
    }
    for entry in &plan.entries {
        if entry.id.starts_with("formula:") || entry.id.starts_with("cask:") {
            continue;
        }
        for step in entry.recipe.steps() {
            let Some(name) = step.brew_name() else {
                continue;
            };
            let Some(tap) = tap_of(name) else { continue };
            let row = packages.entry(tap).or_default();
            if !row.iter().any(|p| p.name == name) {
                row.push(TapPackage {
                    name: name.to_string(),
                    short: short_name(name),
                    via: format!("the Catalog's {}, listed in this Config", entry.name),
                    same_as_base: false,
                    same_as_catalog: None,
                });
            }
        }
    }
    let cards = packages
        .into_iter()
        .map(|(tap, packages)| {
            let (owner, name) = tap.split_once('/').unwrap_or((&tap, ""));
            TapCard {
                owner: owner.to_string(),
                blocked: packages.iter().any(|p| p.same_as_base),
                formula_url: format!("https://github.com/{owner}/homebrew-{name}"),
                packages,
                tap: tap.clone(),
            }
        })
        .collect();
    (cards, item_taps)
}

fn dotfiles_card(
    dotfiles: &Dotfiles,
    plan: &Plan,
    fetched: &Fetched,
    signed_in: Option<&str>,
    facts: &DotfilesFacts,
    clone_location: &str,
) -> DotfilesCard {
    // A repo the plan dropped, because its address looks like a token, can't be approved at all.
    let source = plan
        .dotfiles
        .as_ref()
        .and_then(|_| DotfilesSource::parse(&dotfiles.repo));
    let clashes = refused_links(dotfiles.links.keys().map(String::as_str), clone_location);
    let secret: BTreeSet<&String> = plan
        .dotfiles
        .iter()
        .flat_map(|d| d.refused_links.iter())
        .collect();
    let mut links: Vec<LinkRow> = dotfiles
        .links
        .iter()
        .map(|(target, repo_path)| LinkRow {
            target: target.clone(),
            target_escaped: escaped(target),
            source: repo_path.clone(),
            source_escaped: escaped(repo_path),
            note: link_note(target),
            lookalike: !lookalikes(target).is_empty() || !lookalikes(repo_path).is_empty(),
            refused: clashes
                .get(target)
                .map(|kind| refused_words(*kind))
                .or_else(|| secret.contains(target).then_some("Looks like a secret")),
        })
        .collect();
    links.sort_by_key(|l| {
        (
            l.refused.is_some(),
            !(l.note.is_some() || l.lookalike),
            l.target.clone(),
        )
    });
    let login = signed_in.map(str::to_ascii_lowercase);
    let github_owner = source
        .as_ref()
        .and_then(DotfilesSource::github)
        .map(|repo| repo.owner().to_ascii_lowercase());
    let names_your_repo = login.is_some() && github_owner == login;
    // The ADR's rule, plus one narrowing: someone else's Config naming your repo still wrote that
    // list of links. GitHub's current answer decides, so a transferred repo doesn't still count.
    let own = names_your_repo
        && fetched.source.repo.owner().to_ascii_lowercase() == *login.as_deref().unwrap_or("")
        && facts.current_owner.as_deref().map(str::to_ascii_lowercase) == login;
    DotfilesCard {
        repo: dotfiles.repo.clone(),
        key: source.as_ref().map(DotfilesSource::key),
        host: source.as_ref().map(|s| s.host().to_string()),
        is_github: source.as_ref().is_some_and(|s| s.github().is_some()),
        commit: facts.commit.clone().filter(|_| source.is_some()),
        own,
        names_your_repo,
        links,
        script: dotfiles.install_script.clone(),
        script_text: facts.script_text.clone(),
    }
}
