//! The app's state and everything that changes it. Screens only read this and call its methods.
//! A Run's state changes only through `apply`, one event at a time, as the real engine will send them.

use std::collections::{BTreeMap, BTreeSet};

use rune::TextEdit;
use soulcaster_core::ask::Password;
use soulcaster_core::config::{Config, Packages, SettingValue};
use soulcaster_core::event::{AskKind, AskStatus, Event, EventKind, QueueEntry, RunScope};
use soulcaster_core::lifecycle::{ItemState, Stage};
use soulcaster_core::plan::{Plan, Why};
use soulcaster_core::recipe::ValueType;

use crate::answers::Answers;
use crate::catalog_view::{surface_name, view};
use crate::drift::{Decision, DriftEntry};
use crate::engine::Engine;
use crate::sim::{Scenario, SimEngine, SimMachine};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Catalog,
    Review,
    Run,
    Finish,
    Drift,
    Import,
    Save,
    Dotfiles,
}

#[derive(Clone, PartialEq, Debug)]
pub enum CatRow {
    Base(usize),
    Section(usize),
    Group(String),
    Item(usize),
    Surface(String),
    Setting(usize),
    Package(String),
    AddPackage(String),
    Empty,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PasswordState {
    Empty,
    Checking,
    Wrong,
    Accepted,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GithubState {
    Ready,
    Waiting { code: String },
    Done,
    Skipped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdminState {
    NotNeeded,
    Held,
    Discarded,
}

pub struct RunItem {
    pub id: String,
    pub name: String,
    pub stage: Stage,
    pub is_setting: bool,
    pub state: ItemState,
    /// It got as far as `acting`, so "already here" isn't true of it.
    pub acted: bool,
    pub progress: Option<f32>,
}

pub struct LogLine {
    pub item: String,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Timer {
    DriftApplied,
    GoToFinish,
    ClearToast,
}

/// The bound on the log kept in memory. The real one writes to disk and keeps a tail like this.
const LOG_CAP: usize = 20_000;

pub struct App {
    pub engine: Box<dyn Engine>,
    pub screen: Screen,
    pub clock: f64,

    // Catalog
    pub selected: BTreeSet<String>,
    pub settings: BTreeMap<String, SettingValue>,
    pub packages: BTreeSet<String>,
    pub open_select: Option<String>,
    pub search: TextEdit,
    pub only_selected: bool,
    pub rows: Vec<CatRow>,
    pub rows_version: u64,
    pub current_section: usize,
    pub config_name: String,

    // Review
    pub base_present: bool,
    pub answers: Answers,
    pub password: TextEdit,
    pub password_state: PasswordState,
    pub github: GithubState,
    pub show_config: bool,

    // Run
    pub timers: Vec<(f64, Timer)>,
    pub run_items: Vec<RunItem>,
    pub admin: AdminState,
    pub run_started: bool,
    pub run_finished: bool,
    pub log: Vec<LogLine>,
    pub log_base: usize,
    pub log_view: Vec<usize>,
    pub log_item: Option<String>,

    // Needs you
    pub queue: Vec<QueueEntry>,
    pub scoped_password_for: Option<String>,
    pub scoped_password: TextEdit,
    pub run_password: TextEdit,
    pub run_asks: (bool, bool),
    /// The Dotfiles Repo from an imported Config, for the Run to apply.
    pub dotfiles: Option<soulcaster_core::config::Dotfiles>,
    dotfiles_for: Option<String>,
    pub scoped_wrong: bool,

    // Drift
    pub drift: Vec<DriftEntry>,
    pub confirm_remove: Option<String>,
    pub remove_understood: bool,
    pub drift_password_open: bool,
    pub drift_password: TextEdit,
    pub drift_applying: bool,
    pub show_untracked: bool,

    // Save, Import and the Dotfiles Repo
    pub repo: crate::repo_flow::RepoFlow,

    pub toast: Option<String>,
}

pub fn init() -> App {
    // A real Run changes this Mac, so it's opt-in until real-Mac verification has passed.
    if std::env::args().any(|arg| arg == "--real")
        && let Some(home) = std::env::var_os("HOME")
    {
        let home = std::path::PathBuf::from(home);
        let mut app = with_engine(Box::new(crate::real::RealEngine::new(home.clone())));
        app.repo = crate::repo_flow::RepoFlow::real(home);
        return app;
    }
    with_engine(Box::new(SimEngine::new(Scenario::Both, SimMachine::Bare)))
}

pub fn with_engine(engine: Box<dyn Engine>) -> App {
    let mut app = App {
        engine,
        screen: Screen::Catalog,
        clock: 0.0,
        selected: BTreeSet::new(),
        settings: BTreeMap::new(),
        packages: BTreeSet::new(),
        open_select: None,
        search: TextEdit::new(""),
        only_selected: false,
        rows: Vec::new(),
        rows_version: 0,
        current_section: 0,
        config_name: "My Mac".to_string(),
        base_present: false,
        answers: Answers::default(),
        password: TextEdit::new("").secure(),
        password_state: PasswordState::Empty,
        github: GithubState::Ready,
        show_config: false,
        timers: Vec::new(),
        run_items: Vec::new(),
        admin: AdminState::NotNeeded,
        run_started: false,
        run_finished: false,
        log: Vec::new(),
        log_base: 0,
        log_view: Vec::new(),
        log_item: None,
        queue: Vec::new(),
        scoped_password_for: None,
        scoped_password: TextEdit::new("").secure(),
        run_password: TextEdit::new("").secure(),
        run_asks: (false, false),
        dotfiles: None,
        dotfiles_for: None,
        scoped_wrong: false,
        drift: crate::drift::sample(),
        confirm_remove: None,
        remove_understood: false,
        drift_password_open: false,
        drift_password: TextEdit::new("").secure(),
        drift_applying: false,
        show_untracked: false,
        repo: crate::repo_flow::RepoFlow::simulated(),
        toast: None,
    };
    app.rebuild_rows();
    app
}

/// The picks behind the screenshots.
pub const TYPICAL: &[&str] = &[
    "visual-studio-code",
    "neovim",
    "ghostty",
    "starship",
    "tmux",
    "ripgrep",
    "fd",
    "fzf",
    "bat",
    "eza",
    "jq",
    "zoxide",
    "git-lfs",
    "lazygit",
    "delta",
    "fnm",
    "uv",
    "go",
    "rust",
    "rustup",
    "kotlin",
    "android-sdk",
    "orbstack",
    "postgresql",
    "tableplus",
    "claude-code",
    "codex",
    "cursor",
    "tailscale",
    "firefox",
    "raycast",
    "aerospace",
    "1password",
    "slack",
    "jetbrains-mono",
    "xcode",
];

pub fn typical_settings() -> Vec<(&'static str, SettingValue)> {
    vec![
        ("dock.autohide", SettingValue::Bool(true)),
        ("dock.position", SettingValue::String("left".to_string())),
        ("dock.show-recents", SettingValue::Bool(false)),
        ("finder.show-path-bar", SettingValue::Bool(true)),
        ("finder.show-status-bar", SettingValue::Bool(true)),
        ("git.pull-rebase", SettingValue::Bool(true)),
        ("git.push-auto-setup-remote", SettingValue::Bool(true)),
        ("keyboard.press-and-hold", SettingValue::Bool(false)),
        ("keyboard.key-repeat", SettingValue::Number(2.into())),
        ("trackpad.tap-to-click", SettingValue::Bool(true)),
    ]
}

impl App {
    /// The Config this selection saves, in the Config format's shape.
    pub fn config(&self) -> Config {
        let mut config = self.selection();
        // Only the exact selection that was reviewed with it.
        if self.dotfiles_for.as_deref() == Some(config.to_canonical_json().as_str()) {
            config.dotfiles = self.dotfiles.clone();
        }
        config
    }

    /// Records what the imported Dotfiles Repo was reviewed with.
    pub fn keep_dotfiles(&mut self, dotfiles: Option<soulcaster_core::config::Dotfiles>) {
        self.dotfiles_for = dotfiles
            .is_some()
            .then(|| self.selection().to_canonical_json());
        self.dotfiles = dotfiles;
    }

    fn selection(&self) -> Config {
        let (casks, formulae): (Vec<&String>, Vec<&String>) =
            self.packages.iter().partition(|p| p.starts_with("cask:"));
        Config {
            schema: None,
            schema_version: soulcaster_core::config::SCHEMA_VERSION,
            name: self.config_name.clone(),
            items: self.selected.iter().cloned().collect(),
            packages: Packages {
                formulae: formulae.into_iter().cloned().collect(),
                casks: casks
                    .into_iter()
                    .map(|c| c.trim_start_matches("cask:").to_string())
                    .collect(),
            },
            settings: self.settings.clone(),
            dotfiles: None,
        }
    }

    pub fn plan(&self) -> Plan {
        let mut plan = Plan::new(&self.config(), view().catalog);
        if self.base_present {
            // A simulated check found the Base Items here, so they need no admin work.
            plan.assign_stages(|entry| {
                if entry.why == Why::Base {
                    Stage::User
                } else {
                    entry.stage
                }
            });
        }
        plan
    }

    pub fn needs_github(&self) -> bool {
        !self.base_present && !self.engine.is_real()
    }

    pub fn fill_typical(&mut self) {
        for id in TYPICAL {
            if view().catalog.entry(id).is_some() {
                self.selected.insert((*id).to_string());
            }
        }
        for (id, value) in typical_settings() {
            self.settings.insert(id.to_string(), value);
        }
        self.rebuild_rows();
    }

    pub fn toggle_item(&mut self, id: &str) {
        if !self.selected.remove(id) {
            self.selected.insert(id.to_string());
        }
        if self.only_selected {
            self.rebuild_rows();
        }
    }

    pub fn toggle_setting(&mut self, id: &str) {
        if self.settings.remove(id).is_none() {
            let setting = view().setting(id).expect("a known setting");
            let value = match setting.value_type {
                ValueType::Boolean => SettingValue::Bool(true),
                ValueType::Integer => {
                    SettingValue::Number(setting.minimum.unwrap_or(0).max(2).into())
                }
                _ => SettingValue::String(
                    setting
                        .values
                        .as_ref()
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or_default(),
                ),
            };
            self.settings.insert(id.to_string(), value);
        }
        if self.only_selected {
            self.rebuild_rows();
        }
    }

    /// Adds a Homebrew package by name, if the Config format allows that name.
    pub fn add_package(&mut self, name: &str) {
        let probe = serde_json::json!({"schemaVersion": 1, "name": "probe", "packages": {"formulae": [name]}});
        if Config::parse(probe.to_string().as_bytes()).is_ok() {
            self.packages.insert(name.to_string());
        }
        self.search.clear();
        self.rebuild_rows();
    }

    /// Keeps one side of a conflict, by dropping the other from the selection.
    pub fn drop_entry(&mut self, id: &str) {
        self.selected.remove(id);
        if let Some(name) = id.strip_prefix("formula:") {
            self.packages.remove(name);
        } else if let Some(name) = id.strip_prefix("cask:") {
            self.packages.remove(&format!("cask:{name}"));
        }
        self.rebuild_rows();
    }

    pub fn set_filter_selected(&mut self, on: bool, tx: &mut rune::Tx) {
        self.only_selected = on;
        self.rebuild_rows();
        tx.scroll_to_instant("catalog-list", 0.0, 0.0);
    }

    pub fn jump_to_section(&mut self, section: usize, tx: &mut rune::Tx) {
        if self.only_selected || !self.search.text.is_empty() {
            self.only_selected = false;
            self.search.clear();
            self.rebuild_rows();
        }
        self.current_section = section;
        let key = match section {
            usize::MAX => "row-surface-first".to_string(),
            ALWAYS_SECTION => "row-base-0".to_string(),
            _ => format!("row-section-{section}"),
        };
        tx.scroll_into_view(key);
    }

    /// The flat list the Catalog's virtual rows read. Rebuilt on search and filter changes, not on
    /// every commit: a toggle only changes what a row draws, not which rows exist.
    pub fn rebuild_rows(&mut self) {
        let cat = view();
        let query = self.search.text.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let item_shown = |i: usize| {
            let entry = cat.items[i];
            let listing = entry.listing.as_ref().expect("a listed Item");
            let hit = matches(&listing.name)
                || matches(&listing.description)
                || matches(entry.id())
                || listing.group.as_deref().is_some_and(matches);
            hit && (!self.only_selected || self.selected.contains(entry.id()))
        };
        let browsing = query.is_empty() && !self.only_selected;
        let mut rows = Vec::new();
        if browsing {
            for i in 0..cat.catalog.base_items().count() {
                rows.push(CatRow::Base(i));
            }
        }
        for (s, section) in cat.catalog.sections().iter().enumerate() {
            let section_of = |i: usize| {
                cat.items[i]
                    .listing
                    .as_ref()
                    .expect("a listed Item")
                    .section
                    == section.id
            };
            let indices: Vec<usize> = (0..cat.items.len())
                .filter(|&i| section_of(i) && item_shown(i))
                .collect();
            if indices.is_empty() {
                continue;
            }
            rows.push(CatRow::Section(s));
            let group_of = |i: usize| {
                cat.items[i]
                    .listing
                    .as_ref()
                    .and_then(|l| l.group.as_deref())
            };
            let mut groups: Vec<Option<&str>> = Vec::new();
            for &i in &indices {
                if !groups.contains(&group_of(i)) {
                    groups.push(group_of(i));
                }
            }
            for group in groups {
                if let Some(name) = group {
                    rows.push(CatRow::Group(name.to_string()));
                }
                let (competing, plain): (Vec<usize>, Vec<usize>) = indices
                    .iter()
                    .copied()
                    .filter(|&i| group_of(i) == group)
                    .partition(|&i| browsing && cat.choice_of(cat.items[i].id()).is_some());
                rows.extend(plain.into_iter().map(CatRow::Item));
                // Competing options sit together, under a name for what they compete at.
                let mut shown: Vec<&str> = Vec::new();
                for i in competing {
                    let choice = cat
                        .choice_of(cat.items[i].id())
                        .expect("partitioned on this");
                    if shown.contains(&choice.id.as_str()) {
                        continue;
                    }
                    shown.push(&choice.id);
                    rows.push(CatRow::Group(choice.title.clone()));
                    for member in &choice.members {
                        if let Some(at) = cat.items.iter().position(|x| x.id() == member) {
                            rows.push(CatRow::Item(at));
                        }
                    }
                }
            }
        }
        let mut surfaces: Vec<&'static str> = Vec::new();
        for (i, setting) in cat.settings.iter().enumerate() {
            let surface = surface_name(&setting.id);
            let shown = (matches(&setting.title) || matches(&setting.id) || matches(surface))
                && (!self.only_selected || self.settings.contains_key(&setting.id));
            if !shown {
                continue;
            }
            if !surfaces.contains(&surface) {
                surfaces.push(surface);
                rows.push(CatRow::Surface(surface.to_string()));
            }
            rows.push(CatRow::Setting(i));
        }
        for package in &self.packages {
            if matches(package) {
                rows.push(CatRow::Package(package.clone()));
            }
        }
        if !query.is_empty() && !self.only_selected {
            rows.push(CatRow::AddPackage(query.clone()));
        }
        if rows.is_empty() {
            rows.push(CatRow::Empty);
        }
        self.rows = rows;
        self.rows_version += 1;
    }

    pub fn selection_count(&self) -> usize {
        self.selected.len() + self.settings.len() + self.packages.len()
    }

    pub fn after(&mut self, seconds: f64, timer: Timer) {
        self.timers.push((self.clock + seconds, timer));
    }

    pub fn check_password(&mut self) {
        if self.password.text.is_empty() {
            return;
        }
        self.password_state = PasswordState::Checking;
        let typed = std::mem::take(&mut self.password.text);
        self.password.clear();
        self.engine.submit_password(Password::new(typed));
    }

    /// The Run is waiting on this. A real engine checks it and may ask again.
    pub fn submit_run_password(&mut self) {
        if self.run_password.text.is_empty() {
            return;
        }
        let typed = std::mem::take(&mut self.run_password.text);
        self.run_password.clear();
        self.engine.submit_password(Password::new(typed));
    }

    pub fn start_github(&mut self) {
        let code = self.engine.start_github_sign_in();
        self.github = GithubState::Waiting { code };
    }

    pub fn ready_to_start(&self, plan: &Plan) -> bool {
        // A real Run asks when it knows it needs to.
        let password = self.engine.is_real()
            || !plan.needs_admin()
            || self.password_state == PasswordState::Accepted;
        let github =
            !self.needs_github() || matches!(self.github, GithubState::Done | GithubState::Skipped);
        password && github && plan.conflicts.is_empty()
    }

    pub fn start_run(&mut self) {
        let plan = self.plan();
        // An answer from an earlier Review only counts if this plan shows that tap.
        let mut answers = self.answers.clone();
        answers.taps.retain(|tap| plan.taps.contains_key(tap));
        self.engine.start_run(&plan, &answers);
        self.screen = Screen::Run;
        self.pump();
    }

    fn pump(&mut self) {
        for event in self.engine.poll(self.clock) {
            self.apply(event);
        }
    }

    /// The only way a Run's state changes.
    pub fn apply(&mut self, event: Event) {
        match event.kind {
            EventKind::RunStarted { scope, plan, .. } => {
                self.run_started = true;
                self.run_finished = false;
                if scope == RunScope::Full {
                    self.run_items = plan
                        .into_iter()
                        .map(|item| RunItem {
                            id: item.id,
                            name: item.name,
                            stage: item.stage,
                            is_setting: item.is_setting,
                            state: ItemState::Pending,
                            acted: false,
                            progress: None,
                        })
                        .collect();
                    self.admin = AdminState::NotNeeded;
                    self.log.clear();
                    self.log_base = 0;
                    self.log_view.clear();
                    self.log_item = None;
                    self.queue.clear();
                }
            }
            EventKind::AskState {
                ask: AskKind::Password,
                state,
            } => {
                // For a real engine `waiting` only means it's about to ask.
                self.password_state = match state {
                    AskStatus::Done => PasswordState::Accepted,
                    AskStatus::Skipped => PasswordState::Empty,
                    AskStatus::Waiting if self.engine.is_real() => PasswordState::Empty,
                    AskStatus::Waiting => PasswordState::Wrong,
                };
                if self.scoped_password_for.is_some() {
                    self.finish_scoped_password(state);
                }
            }
            EventKind::AskState {
                ask: AskKind::Github,
                state,
            } => {
                if state == AskStatus::Done {
                    self.github = GithubState::Done;
                }
            }
            EventKind::AdminHeld { .. } => self.admin = AdminState::Held,
            EventKind::AdminDiscarded { .. } => self.admin = AdminState::Discarded,
            EventKind::ItemState { item, to, .. } => {
                if let Some(entry) = self.run_items.iter_mut().find(|i| i.id == item) {
                    entry.acted |= to == ItemState::Acting;
                    if to.is_terminal() {
                        entry.progress = None;
                    }
                    entry.state = to;
                }
            }
            EventKind::ItemStep { .. } => {}
            EventKind::ItemProgress {
                item,
                bytes_done,
                bytes_total,
                ..
            } => {
                if let (Some(entry), Some(total)) = (
                    self.run_items.iter_mut().find(|i| i.id == item),
                    bytes_total,
                ) {
                    entry.progress = Some(bytes_done as f32 / total.max(1) as f32);
                }
            }
            EventKind::ItemLog { item, lines, .. } => {
                for text in lines {
                    self.push_log(&item, text);
                }
            }
            EventKind::QueueOpened { entry } => self.queue.push(entry),
            EventKind::QueueClosed { id } => self.queue.retain(|q| q.id != id),
            EventKind::RunWarning { .. } => {}
            EventKind::RunFinished { .. } => {
                self.run_finished = true;
                if self.screen == Screen::Run {
                    // Whoever walked away comes back to what's left, not to a finished log.
                    self.after(1.2, Timer::GoToFinish);
                }
            }
        }
    }

    fn push_log(&mut self, item: &str, text: String) {
        let abs = self.log_base + self.log.len();
        if self.log_item.as_deref().is_none_or(|f| f == item) {
            self.log_view.push(abs);
        }
        self.log.push(LogLine {
            item: item.to_string(),
            text,
        });
        if self.log.len() > LOG_CAP {
            let drop = self.log.len() - LOG_CAP + LOG_CAP / 10;
            self.log.drain(..drop);
            self.log_base += drop;
            let base = self.log_base;
            self.log_view.retain(|&n| n >= base);
        }
    }

    pub fn set_log_item(&mut self, item: Option<String>) {
        self.log_item = item;
        self.log_view.clear();
        let base = self.log_base;
        for (i, line) in self.log.iter().enumerate() {
            if self.log_item.as_deref().is_none_or(|f| f == line.item) {
                self.log_view.push(base + i);
            }
        }
    }

    /// A retry, or a finished queue entry: a smaller Run over this Item and what waits on it.
    pub fn start_scoped(&mut self, item: &str, queue_id: Option<&str>) {
        let waiting: Vec<String> = self
            .run_items
            .iter()
            .filter(|i| matches!(&i.state, ItemState::Blocked { by } if by == item))
            .map(|i| i.id.clone())
            .collect();
        self.engine.start_scoped(item, &waiting, queue_id);
    }

    pub fn finish_queue_entry(&mut self, id: &str) {
        let Some(entry) = self.queue.iter().find(|q| q.id == id).cloned() else {
            return;
        };
        if entry.needs_password && !self.engine.is_real() {
            self.scoped_password_for = Some(id.to_string());
            self.scoped_wrong = false;
            self.scoped_password.focused = true;
        } else {
            self.start_scoped(&entry.item, Some(id));
        }
    }

    pub fn submit_scoped_password(&mut self) {
        if self.scoped_password.text.is_empty() {
            return;
        }
        let typed = std::mem::take(&mut self.scoped_password.text);
        self.scoped_password.clear();
        self.engine.submit_password(Password::new(typed));
    }

    fn finish_scoped_password(&mut self, state: AskStatus) {
        if state == AskStatus::Waiting {
            self.scoped_wrong = true;
            return;
        }
        if let Some(id) = self.scoped_password_for.take()
            && let Some(entry) = self.queue.iter().find(|q| q.id == id).cloned()
        {
            self.start_scoped(&entry.item, Some(&id));
        }
    }

    pub fn close_drift_entries(&mut self) {
        self.drift.retain(|d| d.decision == Decision::Later);
        self.drift_applying = false;
        self.toast = Some("Done".to_string());
        self.after(4.0, Timer::ClearToast);
    }

    /// Runs before every frame, with virtual time when headless.
    pub fn tick(&mut self, clock: f64) -> bool {
        self.clock = clock;
        let mut changed = false;
        let mut i = 0;
        while i < self.timers.len() {
            if self.timers[i].0 <= clock {
                let (_, timer) = self.timers.remove(i);
                self.fire(timer);
                changed = true;
            } else {
                i += 1;
            }
        }
        for event in self.engine.poll(clock) {
            self.apply(event);
            changed = true;
        }
        // These come from the Run's thread between events, so a change here needs a redraw too.
        let asks = (
            self.engine.password_request().is_some(),
            self.engine.github_code().is_some(),
        );
        if asks != self.run_asks {
            // What was typed for a question that has ended isn't for the next one.
            if !asks.0 {
                self.run_password.replace_text("");
                self.run_password.clear();
            }
            self.run_asks = asks;
            changed = true;
        }
        changed
    }

    /// Seconds until the next thing is due, for `wake_after`.
    pub fn next_due(&self) -> Option<f64> {
        let timer = self
            .timers
            .iter()
            .map(|t| t.0 - self.clock)
            .fold(f64::INFINITY, f64::min);
        let engine = self.engine.next_due(self.clock).unwrap_or(f64::INFINITY);
        let next = timer.min(engine);
        next.is_finite().then_some(next.max(0.0))
    }

    fn fire(&mut self, timer: Timer) {
        match timer {
            Timer::DriftApplied => self.close_drift_entries(),
            Timer::GoToFinish => {
                if self.screen == Screen::Run {
                    self.screen = Screen::Finish;
                }
            }
            Timer::ClearToast => self.toast = None,
        }
    }
}

/// `current_section` for the Base Items at the top of the list.
pub const ALWAYS_SECTION: usize = usize::MAX - 1;

#[cfg(test)]
mod tests {
    mod real_mode {
        use std::sync::{Arc, Mutex};

        use soulcaster_core::ask::PasswordRequest;

        use super::*;
        use crate::answers::Answers;

        #[derive(Default)]
        struct Shared {
            asking: Mutex<Option<PasswordRequest>>,
            got: Mutex<Option<String>>,
            scoped: Mutex<u32>,
            /// Taps `start_run` handed over that the plan itself doesn't list.
            outside_taps: Mutex<Vec<String>>,
        }

        /// Behaves like the real engine to the window, and runs nothing.
        struct Stand(Arc<Shared>);

        impl Engine for Stand {
            fn start_run(&mut self, plan: &Plan, answers: &Answers) {
                *self.0.outside_taps.lock().unwrap() = answers
                    .taps
                    .iter()
                    .filter(|tap| !plan.taps.contains_key(tap.as_str()))
                    .cloned()
                    .collect();
            }

            fn start_scoped(&mut self, _: &str, _: &[String], _: Option<&str>) {
                *self.0.scoped.lock().unwrap() += 1;
            }

            fn submit_password(&mut self, password: Password) {
                *self.0.got.lock().unwrap() = Some(password.expose().to_string());
            }

            fn start_github_sign_in(&mut self) -> String {
                String::new()
            }

            fn poll(&mut self, _: f64) -> Vec<Event> {
                Vec::new()
            }

            fn next_due(&self, _: f64) -> Option<f64> {
                None
            }

            fn is_real(&self) -> bool {
                true
            }

            fn password_request(&self) -> Option<PasswordRequest> {
                self.0.asking.lock().unwrap().clone()
            }
        }

        fn app() -> (App, Arc<Shared>) {
            let shared = Arc::new(Shared::default());
            (with_engine(Box::new(Stand(Arc::clone(&shared)))), shared)
        }

        #[test]
        fn review_doesnt_ask_for_what_the_run_will_ask_for() {
            let (mut app, _) = app();
            app.selected.insert("ripgrep".to_string());
            let plan = app.plan();
            assert!(plan.needs_admin());
            assert!(!app.needs_github());
            assert!(app.ready_to_start(&plan));
        }

        #[test]
        fn the_engine_about_to_ask_isnt_a_wrong_password() {
            let (mut app, _) = app();
            let mut stamper = soulcaster_core::event::Stamper::new();
            app.apply(stamper.stamp(
                0,
                EventKind::AskState {
                    ask: AskKind::Password,
                    state: AskStatus::Waiting,
                },
            ));
            assert_eq!(app.password_state, PasswordState::Empty);
        }

        #[test]
        fn a_question_from_the_run_redraws_and_the_answer_goes_back() {
            let (mut app, shared) = app();
            assert!(!app.tick(1.0));
            *shared.asking.lock().unwrap() = Some(PasswordRequest {
                why: "x".to_string(),
                last_was_wrong: false,
            });
            assert!(app.tick(2.0), "the question didn't redraw the window");
            assert!(!app.tick(3.0), "it should only redraw when it changes");
            app.run_password.replace_text("secret");
            app.submit_run_password();
            assert_eq!(shared.got.lock().unwrap().as_deref(), Some("secret"));
            assert!(
                app.run_password.text.is_empty(),
                "the field kept the password"
            );
        }

        #[test]
        fn finishing_a_queue_entry_doesnt_ask_first_when_the_run_will() {
            let (mut app, shared) = app();
            app.queue.push(QueueEntry {
                id: "x:y".to_string(),
                item: "x".to_string(),
                kind: soulcaster_core::event::QueueKind::NeedsAdmin,
                text: "t".to_string(),
                detail: None,
                link: None,
                needs_password: true,
            });
            app.finish_queue_entry("x:y");
            assert!(app.scoped_password_for.is_none());
            assert_eq!(*shared.scoped.lock().unwrap(), 1);
        }

        #[test]
        fn a_password_from_a_finished_question_is_not_waiting_for_the_next_one() {
            let (mut app, shared) = app();
            *shared.asking.lock().unwrap() = Some(PasswordRequest {
                why: "the admin steps".to_string(),
                last_was_wrong: false,
            });
            assert!(app.tick(1.0));
            app.run_password.replace_text("secret");
            // Later, or the question ending any other way, drops the request.
            *shared.asking.lock().unwrap() = None;
            app.tick(2.0);
            *shared.asking.lock().unwrap() = Some(PasswordRequest {
                why: "a later admin step".to_string(),
                last_was_wrong: false,
            });
            app.tick(3.0);
            app.submit_run_password();
            assert!(
                shared.got.lock().unwrap().is_none(),
                "the password typed for a question that already ended was sent to the next one"
            );
            assert!(
                app.run_password.text.is_empty(),
                "the field still holds the password after the question ended"
            );
        }

        #[test]
        fn a_tap_from_an_earlier_review_is_not_approved_with_a_plan_that_doesnt_show_it() {
            let (mut app, shared) = app();
            app.selected.insert("aerospace".to_string());
            app.answers.taps.insert("nikitabobko/tap".to_string());
            assert!(
                app.plan().taps.contains_key("nikitabobko/tap"),
                "aerospace should be the catalog item that asks for this tap"
            );
            app.screen = Screen::Catalog;
            app.selected.remove("aerospace");
            app.packages.clear();
            app.settings.clear();
            app.selected.insert("jq".to_string());
            assert!(!app.plan().taps.contains_key("nikitabobko/tap"));
            app.start_run();
            assert!(
                shared.outside_taps.lock().unwrap().is_empty(),
                "start handed the engine a tap this plan doesn't show: {:?}",
                shared.outside_taps.lock().unwrap()
            );
        }
    }

    use soulcaster_core::catalog::Catalog;
    use soulcaster_core::event::{EventKind, Stamper};

    use super::*;

    fn event(kind: EventKind) -> Event {
        Stamper::new().stamp(0, kind)
    }

    #[test]
    fn the_config_a_selection_saves_parses_under_the_real_rules() {
        let mut app = init();
        app.fill_typical();
        app.add_package("tree");
        let text = app.config().to_canonical_json();
        let parsed = Config::parse(text.as_bytes()).unwrap();
        assert!(parsed.items.iter().any(|i| i == "ghostty"));
        assert_eq!(parsed.packages.formulae, ["tree"]);
    }

    #[test]
    fn a_name_the_config_format_refuses_isnt_added() {
        let mut app = init();
        app.add_package("evil; curl example.com");
        assert!(app.packages.is_empty());
    }

    #[test]
    fn every_catalog_item_shows_a_row_and_every_row_key_section_exists() {
        let app = init();
        let items = app
            .rows
            .iter()
            .filter(|r| matches!(r, CatRow::Item(_)))
            .count();
        assert_eq!(items, 243, "each listed Item appears exactly once");
        assert_eq!(
            view().catalog.sections().len(),
            Catalog::shipped().sections().len()
        );
    }

    #[test]
    fn the_run_screen_is_built_only_from_events() {
        let mut app = init();
        app.fill_typical();
        let plan = app.plan();
        app.apply(event(EventKind::RunStarted {
            run_id: "x".to_string(),
            scope: RunScope::Full,
            config: Default::default(),
            catalog_version: String::new(),
            engine_version: String::new(),
            macos_version: None,
            plan: plan.summary(),
        }));
        assert_eq!(app.run_items.len(), plan.entries.len());
        assert!(app.run_started);
        app.apply(event(EventKind::ItemState {
            item: "homebrew".to_string(),
            from: ItemState::Checking,
            to: ItemState::Acting,
        }));
        app.apply(event(EventKind::ItemProgress {
            item: "homebrew".to_string(),
            phase: "x".to_string(),
            bytes_done: 1,
            bytes_total: Some(4),
        }));
        let brew = app.run_items.iter().find(|i| i.id == "homebrew").unwrap();
        assert!(brew.acted);
        assert_eq!(brew.progress, Some(0.25));
    }

    #[test]
    fn the_password_box_empties_as_soon_as_it_is_submitted() {
        let mut app = init();
        app.password.text = "hunter2".to_string();
        app.check_password();
        assert!(app.password.text.is_empty());
        assert_eq!(app.password_state, PasswordState::Checking);
        app.tick(1.0);
        assert_eq!(app.password_state, PasswordState::Accepted);
    }

    #[test]
    fn a_finished_run_takes_you_to_needs_you() {
        let mut app = init();
        app.fill_typical();
        app.start_run();
        let mut clock = 0.0;
        while let Some(next) = app.next_due() {
            clock += next + 0.001;
            app.tick(clock);
        }
        assert!(app.run_finished);
        assert_eq!(app.screen, Screen::Finish);
        assert!(!app.queue.is_empty());
    }
}
