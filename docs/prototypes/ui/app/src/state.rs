//! The app's state and everything that changes it. Screens only read this and call its methods.

use std::collections::{BTreeMap, BTreeSet};

use rune::TextEdit;

use crate::catalog::{Catalog, catalog, surface_name};
use crate::drift::{Decision, DriftEntry};
use crate::lifecycle::{Assurance, Event, FailureKind, ItemState, QueueEntry, Stage, Timed};
use crate::plan::{Answers, Machine, Plan, make_plan};
use crate::sim::{self, Scenario};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Catalog,
    Review,
    Run,
    Finish,
    Drift,
}

#[derive(Clone, PartialEq, Debug)]
pub enum CatRow {
    Base,
    Section(usize),
    Group(String),
    Choice(usize),
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GithubState {
    /// The bootstrap `gh` is still downloading. It's the very first thing a Run does.
    Preparing,
    Ready,
    Waiting,
    Done,
    Skipped,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdminState {
    NotNeeded,
    Waiting,
    Held,
    Discarded,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunFilter {
    All,
    Problems,
}

pub struct RunItem {
    pub id: String,
    pub name: String,
    pub stage: Stage,
    pub is_setting: bool,
    pub pulled_by: Option<String>,
    pub state: ItemState,
    pub assurance: Option<Assurance>,
    pub failure: Option<FailureKind>,
    pub blocked_by: Option<String>,
    pub acted: bool,
    pub progress: Option<f32>,
}

pub struct LogLine {
    pub item: String,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Timer {
    GhReady,
    PasswordChecked,
    GithubApproved,
    DriftApplied,
    GoToFinish,
    ClearToast,
}

/// The bound on the log kept in memory. The real one writes to disk and keeps a tail like this.
const LOG_CAP: usize = 20_000;

pub struct App {
    pub screen: Screen,
    pub clock: f64,

    // Choose
    pub selected: BTreeSet<String>,
    pub settings: BTreeMap<String, String>,
    pub packages: BTreeSet<String>,
    pub text_values: BTreeMap<String, TextEdit>,
    pub open_select: Option<String>,
    pub search: TextEdit,
    pub only_selected: bool,
    pub rows: Vec<CatRow>,
    pub rows_version: u64,
    pub current_section: usize,
    pub config_name: String,

    // Review
    pub machine: Machine,
    pub answers: Answers,
    pub password: TextEdit,
    pub password_state: PasswordState,
    pub github: GithubState,
    pub show_config: bool,

    // Run
    pub scenario: Scenario,
    pub speed: f64,
    pub timers: Vec<(f64, Timer)>,
    pub schedule: Vec<Timed>,
    pub cursor: usize,
    pub sim_time: f64,
    pub last_clock: f64,
    pub run_items: Vec<RunItem>,
    pub admin: AdminState,
    pub admin_why: String,
    pub run_started: bool,
    pub run_finished: bool,
    pub run_filter: RunFilter,
    pub log: Vec<LogLine>,
    pub log_base: usize,
    pub log_view: Vec<usize>,
    pub log_item: Option<String>,
    pub warnings: Vec<String>,
    pub stress: bool,

    // Finish
    pub queue: Vec<QueueEntry>,
    pub closed: Vec<QueueEntry>,
    pub scoped_password_for: Option<String>,
    pub scoped_password: TextEdit,
    pub scoped_wrong: bool,

    // Drift
    pub drift: Vec<DriftEntry>,
    pub confirm_remove: Option<String>,
    pub remove_understood: bool,
    pub drift_password_open: bool,
    pub drift_password: TextEdit,
    pub drift_applying: bool,
    pub show_untracked: bool,

    pub toast: Option<String>,
}

pub fn init() -> App {
    let mut app = App {
        screen: Screen::Catalog,
        clock: 0.0,
        selected: BTreeSet::new(),
        settings: BTreeMap::new(),
        packages: BTreeSet::new(),
        text_values: BTreeMap::new(),
        open_select: None,
        search: TextEdit::new(""),
        only_selected: false,
        rows: Vec::new(),
        rows_version: 0,
        current_section: 0,
        config_name: "My Mac".to_string(),
        machine: Machine::Bare,
        answers: Answers::default(),
        password: TextEdit::new("").secure(),
        password_state: PasswordState::Empty,
        github: GithubState::Preparing,
        show_config: false,
        scenario: Scenario::Both,
        speed: 1.0,
        timers: Vec::new(),
        schedule: Vec::new(),
        cursor: 0,
        sim_time: 0.0,
        last_clock: 0.0,
        run_items: Vec::new(),
        admin: AdminState::NotNeeded,
        admin_why: String::new(),
        run_started: false,
        run_finished: false,
        run_filter: RunFilter::All,
        log: Vec::new(),
        log_base: 0,
        log_view: Vec::new(),
        log_item: None,
        warnings: Vec::new(),
        stress: false,
        queue: Vec::new(),
        closed: Vec::new(),
        scoped_password_for: None,
        scoped_password: TextEdit::new("").secure(),
        scoped_wrong: false,
        drift: crate::drift::sample(),
        confirm_remove: None,
        remove_understood: false,
        drift_password_open: false,
        drift_password: TextEdit::new("").secure(),
        drift_applying: false,
        show_untracked: false,
        toast: None,
    };
    app.rebuild_rows();
    app
}

/// The picks behind the "typical developer" prototype button and the screenshots.
pub const TYPICAL: &[&str] = &[
    "visual-studio-code", "neovim", "ghostty", "starship", "tmux", "ripgrep", "fd", "fzf", "bat", "eza", "jq", "zoxide", "git-lfs", "lazygit", "delta",
    "fnm", "uv", "go", "rust", "kotlin", "android-sdk", "orbstack", "postgresql", "tableplus", "claude-code", "codex", "cursor", "tailscale",
    "firefox", "raycast", "aerospace", "1password", "slack", "jetbrains-mono", "xcode",
];
pub const TYPICAL_SETTINGS: &[(&str, &str)] = &[
    ("dock.autohide", "true"), ("dock.position", "left"), ("dock.show-recents", "false"), ("finder.show-path-bar", "true"),
    ("finder.show-status-bar", "true"), ("git.pull-rebase", "true"), ("git.push-auto-setup-remote", "true"),
    ("keyboard.press-and-hold", "false"), ("keyboard.key-repeat", "2"), ("trackpad.tap-to-click", "true"),
];

impl App {
    pub fn cat(&self) -> &'static Catalog {
        catalog()
    }

    pub fn plan(&self) -> Plan {
        make_plan(&self.selected, &self.settings, self.machine, &self.answers)
    }

    pub fn fill_typical(&mut self) {
        for id in TYPICAL {
            if catalog().item(id).is_some() {
                self.selected.insert((*id).to_string());
            }
        }
        for (id, value) in TYPICAL_SETTINGS {
            self.settings.insert((*id).to_string(), (*value).to_string());
        }
        self.rebuild_rows();
    }

    pub fn toggle_item(&mut self, id: &str) {
        if !self.selected.remove(id) {
            self.selected.insert(id.to_string());
            self.answers.dropped.remove(id);
        }
        if self.only_selected {
            self.rebuild_rows();
        }
    }

    pub fn toggle_setting(&mut self, id: &str) {
        if self.settings.remove(id).is_none() {
            let setting = catalog().setting(id).expect("known setting");
            let value = match setting.value_type.as_str() {
                "boolean" => "true".to_string(),
                "integer" => setting.minimum.unwrap_or(0).max(2).to_string(),
                _ => setting.values.first().cloned().unwrap_or_else(|| self.text_of(id)),
            };
            self.settings.insert(id.to_string(), value);
        }
        if self.only_selected {
            self.rebuild_rows();
        }
    }

    fn text_of(&self, id: &str) -> String {
        self.text_values.get(id).map_or(String::new(), |t| t.text.clone())
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
        let key = if section == usize::MAX { "row-surface-first".to_string() } else { format!("row-section-{section}") };
        tx.scroll_into_view(key);
    }

    /// The flat list the Catalog's virtual rows read. Rebuilt on search and filter changes, not on
    /// every commit: a toggle only changes what a row draws, not which rows exist.
    pub fn rebuild_rows(&mut self) {
        let cat = catalog();
        let query = self.search.text.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let item_shown = |i: usize| {
            let item = &cat.items[i];
            let hit = matches(&item.name) || matches(&item.description) || matches(&item.id) || item.group.as_deref().is_some_and(&matches);
            hit && (!self.only_selected || self.selected.contains(&item.id))
        };
        let browsing = query.is_empty() && !self.only_selected;
        let mut rows = Vec::new();
        if browsing {
            rows.push(CatRow::Base);
        }
        for (s, section) in cat.sections.iter().enumerate() {
            let indices: Vec<usize> = (0..cat.items.len()).filter(|&i| cat.items[i].section == section.id && item_shown(i)).collect();
            if indices.is_empty() {
                continue;
            }
            rows.push(CatRow::Section(s));
            let mut groups: Vec<Option<&str>> = Vec::new();
            for &i in &indices {
                let group = cat.items[i].group.as_deref();
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            for group in groups {
                if let Some(name) = group {
                    rows.push(CatRow::Group(name.to_string()));
                }
                let mut shown_choices = Vec::new();
                let mut plain = Vec::new();
                for &i in indices.iter().filter(|&&i| cat.items[i].group.as_deref() == group) {
                    match cat.choices.iter().position(|c| c.members.contains(&cat.items[i].id)) {
                        Some(c) if browsing => {
                            if !shown_choices.contains(&c) {
                                shown_choices.push(c);
                            }
                        }
                        _ => plain.push(i),
                    }
                }
                for i in plain {
                    rows.push(CatRow::Item(i));
                }
                // Competing options sit together, under the note that explains them.
                for c in shown_choices {
                    rows.push(CatRow::Choice(c));
                    for member in &cat.choices[c].members {
                        if let Some(i) = cat.items.iter().position(|x| x.id == *member && x.section == section.id) {
                            rows.push(CatRow::Item(i));
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

    pub fn go_to_review(&mut self) {
        self.screen = Screen::Review;
        if self.github == GithubState::Preparing && !self.timers.iter().any(|t| t.1 == Timer::GhReady) {
            self.after(1.4, Timer::GhReady);
        }
    }

    pub fn check_password(&mut self) {
        if self.password.text.is_empty() {
            return;
        }
        // The prototype has no sudo to ask. "wrong" stands in for a rejected password.
        self.password_state = if self.password.text == "wrong" { PasswordState::Wrong } else { PasswordState::Checking };
        if self.password_state == PasswordState::Checking {
            self.after(0.6, Timer::PasswordChecked);
        }
        self.password.clear();
    }

    pub fn ready_to_start(&self, plan: &Plan) -> bool {
        let password = !plan.needs_password || self.password_state == PasswordState::Accepted;
        let github = !plan.needs_github || matches!(self.github, GithubState::Done | GithubState::Skipped);
        password && github && plan.conflicts.is_empty()
    }

    pub fn start_run(&mut self) {
        let plan = self.plan();
        self.schedule = sim::schedule(&plan, self.scenario, self.machine.touch_id());
        if self.stress {
            self.add_stress_logs();
        }
        self.cursor = 0;
        self.sim_time = 0.0;
        self.last_clock = self.clock;
        self.run_items = plan
            .entries
            .iter()
            .map(|e| RunItem {
                id: e.id.clone(),
                name: e.name.clone(),
                stage: e.stage,
                is_setting: e.is_setting,
                pulled_by: match &e.why {
                    crate::plan::Why::PulledIn { by } => Some(by.clone()),
                    _ => None,
                },
                state: ItemState::Pending,
                assurance: None,
                failure: None,
                blocked_by: None,
                acted: false,
                progress: None,
            })
            .collect();
        self.admin = if plan.needs_password { AdminState::Waiting } else { AdminState::NotNeeded };
        self.admin_why = plan.admin_names.join(", ");
        self.run_started = true;
        self.run_finished = false;
        self.run_filter = RunFilter::All;
        self.log.clear();
        self.log_base = 0;
        self.log_view.clear();
        self.log_item = None;
        self.queue.clear();
        self.closed.clear();
        self.warnings.clear();
        self.screen = Screen::Run;
    }

    /// Floods the log, to measure what a chatty Run costs the UI.
    fn add_stress_logs(&mut self) {
        let end = self.schedule.last().map_or(0.0, |t| t.at);
        let item = self.schedule.iter().find_map(|t| match &t.event {
            Event::ItemChanged { item, .. } => Some(item.clone()),
            _ => None,
        });
        let Some(item) = item else { return };
        let mut at = 0.0;
        let mut seq = 1_000_000;
        // 500 lines per batch every 100 ms: the design's batching, at 5,000 lines a second.
        while at < end {
            let lines = (0..500).map(|n| format!("stress line {seq}-{n}: compiling something with a fairly long path/to/a/file.rs")).collect();
            self.schedule.push(Timed { seq, at, event: Event::ItemLog { item: item.clone(), lines } });
            seq += 1;
            at += 0.1;
        }
        self.schedule.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::RunStarted { .. } => {}
            Event::AdminHeld { .. } => self.admin = AdminState::Held,
            Event::AdminDiscarded => self.admin = AdminState::Discarded,
            Event::ItemChanged { item, to, assurance, failure, blocked_by, acted } => {
                if let Some(entry) = self.run_items.iter_mut().find(|i| i.id == item) {
                    entry.state = to;
                    entry.assurance = assurance;
                    entry.failure = failure;
                    entry.blocked_by = blocked_by;
                    entry.acted = entry.acted || acted;
                    if to.is_terminal() {
                        entry.progress = None;
                    }
                }
            }
            Event::ItemProgress { item, fraction, .. } => {
                if let Some(entry) = self.run_items.iter_mut().find(|i| i.id == item) {
                    entry.progress = Some(fraction);
                }
            }
            Event::ItemLog { item, lines } => {
                for text in lines {
                    self.push_log(&item, text);
                }
            }
            Event::QueueOpened(entry) => self.queue.push(entry),
            Event::QueueClosed { id } => {
                if let Some(at) = self.queue.iter().position(|q| q.id == id) {
                    let entry = self.queue.remove(at);
                    self.closed.push(entry);
                }
            }
            Event::RunWarning { text } => self.warnings.push(text),
            Event::RunFinished => {
                self.run_finished = true;
                // Whoever walked away comes back to what's left, not to a finished log.
                self.after(1.2, Timer::GoToFinish);
            }
        }
    }

    fn push_log(&mut self, item: &str, text: String) {
        let abs = self.log_base + self.log.len();
        if self.log_item.as_deref().is_none_or(|f| f == item) {
            self.log_view.push(abs);
        }
        self.log.push(LogLine { item: item.to_string(), text });
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
        let name = catalog().name_of(item);
        let waiting: Vec<(String, String)> = self
            .run_items
            .iter()
            .filter(|i| i.state == ItemState::Blocked && i.blocked_by.as_deref() == Some(item))
            .map(|i| (i.id.clone(), i.name.clone()))
            .collect();
        let seq = self.schedule.last().map_or(0, |t| t.seq);
        self.schedule.extend(sim::scoped(item, &name, &waiting, queue_id, self.sim_time, seq));
        self.schedule.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    pub fn finish_queue_entry(&mut self, id: &str) {
        let Some(entry) = self.queue.iter().find(|q| q.id == id).cloned() else { return };
        if entry.needs_password {
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
        if self.scoped_password.text == "wrong" {
            self.scoped_wrong = true;
            self.scoped_password.clear();
            return;
        }
        self.scoped_password.clear();
        if let Some(id) = self.scoped_password_for.take() {
            if let Some(entry) = self.queue.iter().find(|q| q.id == id).cloned() {
                self.start_scoped(&entry.item, Some(&id));
            }
        }
    }

    pub fn close_drift_entries(&mut self) {
        self.drift.retain(|d| d.decision == Decision::Later);
        self.drift_applying = false;
        self.toast = Some("Done. Your choices were applied, and everything else is unchanged.".to_string());
        self.after(4.0, Timer::ClearToast);
    }

    /// The prototype's timer. Runs before every frame, with virtual time when headless.
    pub fn tick(&mut self, clock: f64) -> Option<f64> {
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
        if self.run_started {
            self.sim_time += (clock - self.last_clock) * self.speed;
            self.last_clock = clock;
            while self.cursor < self.schedule.len() && self.schedule[self.cursor].at <= self.sim_time {
                let event = self.schedule[self.cursor].event.clone();
                self.cursor += 1;
                self.apply(event);
                changed = true;
            }
        }
        changed.then_some(clock)
    }

    /// Seconds until the next thing is due, for `wake_after`.
    pub fn next_due(&self) -> Option<f64> {
        let timer = self.timers.iter().map(|t| t.0 - self.clock).fold(f64::INFINITY, f64::min);
        let sim = self.schedule.get(self.cursor).map_or(f64::INFINITY, |t| (t.at - self.sim_time) / self.speed);
        let next = timer.min(sim);
        next.is_finite().then_some(next.max(0.0))
    }

    fn fire(&mut self, timer: Timer) {
        match timer {
            Timer::GhReady => self.github = GithubState::Ready,
            Timer::PasswordChecked => self.password_state = PasswordState::Accepted,
            Timer::GithubApproved => self.github = GithubState::Done,
            Timer::DriftApplied => self.close_drift_entries(),
            Timer::GoToFinish => {
                if self.screen == Screen::Run {
                    self.screen = Screen::Finish;
                }
            }
            Timer::ClearToast => self.toast = None,
        }
    }

    /// The Config this selection saves, in the Config format's shape.
    pub fn config_json(&self) -> String {
        let cat = catalog();
        let (mut formulae, mut casks) = (Vec::new(), Vec::new());
        for package in &self.packages {
            if package.starts_with("cask:") {
                casks.push(package.trim_start_matches("cask:").to_string());
            } else {
                formulae.push(package.clone());
            }
        }
        let settings: serde_json::Map<String, serde_json::Value> = self
            .settings
            .iter()
            .map(|(id, value)| {
                let kind = cat.setting(id).map_or("string", |s| s.value_type.as_str());
                let json = match kind {
                    "boolean" => serde_json::Value::Bool(value == "true"),
                    "integer" => value.parse::<i64>().map_or(serde_json::Value::Null, Into::into),
                    _ => serde_json::Value::String(value.clone()),
                };
                (id.clone(), json)
            })
            .collect();
        let mut root = serde_json::Map::new();
        root.insert("schemaVersion".to_string(), 1.into());
        root.insert("name".to_string(), self.config_name.clone().into());
        if !self.selected.is_empty() {
            root.insert("items".to_string(), self.selected.iter().filter(|id| !self.answers.dropped.contains(*id)).cloned().collect());
        }
        if !self.packages.is_empty() {
            root.insert("packages".to_string(), serde_json::json!({ "formulae": formulae, "casks": casks }));
        }
        if !settings.is_empty() {
            root.insert("settings".to_string(), settings.into());
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default()
    }
}
