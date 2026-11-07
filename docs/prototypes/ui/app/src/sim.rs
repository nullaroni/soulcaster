//! A pretend engine: turns an approved plan and a scenario into the event stream a real Run
//! would produce, with times. Pure and deterministic, so the same script always shows the same
//! screens, and the tests below can check it against the lifecycle's rules.
//!
//! It only ever builds data. Nothing here starts a process, reads the disk or opens a socket.

use crate::catalog::catalog;
use crate::lifecycle::{Assurance, Event, FailureKind, ItemState, QueueEntry, QueueKind, Stage, Timed};
use crate::plan::{Plan, PlanEntry, Waits, Why};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    AllGood,
    OneFailure,
    Surprise,
    Both,
}

impl Scenario {
    fn fails(self) -> bool {
        matches!(self, Scenario::OneFailure | Scenario::Both)
    }
    fn surprises(self) -> bool {
        matches!(self, Scenario::Surprise | Scenario::Both)
    }
}

struct Script {
    at: f64,
    seq: u64,
    out: Vec<Timed>,
}

impl Script {
    fn push(&mut self, event: Event) {
        self.seq += 1;
        self.out.push(Timed { seq: self.seq, at: self.at, event });
    }
    fn wait(&mut self, seconds: f64) {
        self.at += seconds;
    }
    fn state(&mut self, entry: &PlanEntry, to: ItemState) {
        self.push(Event::ItemChanged { item: entry.id.clone(), to, assurance: None, failure: None, blocked_by: None, acted: false });
    }
    fn log(&mut self, entry: &PlanEntry, lines: &[String]) {
        self.push(Event::ItemLog { item: entry.id.clone(), lines: lines.to_vec() });
    }
}

/// Which Item fails in this scenario: openjdk when it's there, because something else waits on it.
fn failing_item(plan: &Plan) -> Option<&PlanEntry> {
    let usable = |e: &&PlanEntry| e.why != Why::Base && !e.is_setting && e.waits_for.is_none();
    plan.entries.iter().find(|e| e.id == "openjdk").filter(usable).or_else(|| plan.entries.iter().filter(usable).nth(3))
}

/// Which Item surprises everyone by needing admin after the password's gone: one the Catalog
/// doesn't say needs it, in the User Stage.
fn surprise_item<'a>(plan: &'a Plan, failing: Option<&PlanEntry>) -> Option<&'a PlanEntry> {
    let usable = |e: &&PlanEntry| {
        e.stage == Stage::User
            && e.why != Why::Base
            && !e.is_setting
            && e.waits_for.is_none()
            && failing.is_none_or(|f| f.id != e.id)
            && catalog().item(&e.id).is_some_and(|i| i.install.kind == "cask" && i.tap.is_none())
    };
    plan.entries.iter().find(|e| usable(e) && e.id == "docker-desktop").or_else(|| plan.entries.iter().find(|e| usable(e)))
}

fn install_lines(entry: &PlanEntry) -> Vec<String> {
    let cat = catalog();
    let token = cat.item(&entry.id).and_then(|i| i.install.token.clone()).unwrap_or_else(|| entry.id.clone());
    let cask = cat.item(&entry.id).is_some_and(|i| i.install.kind == "cask");
    let flag = if cask { "--cask " } else { "" };
    vec![
        format!("$ brew install {flag}{token}"),
        format!("==> Fetching {token}"),
        format!("==> Downloading https://formulae.brew.sh/api/{}/{token}.json", if cask { "cask" } else { "formula" }),
        "######################################################################## 100.0%".to_string(),
        format!("==> Installing {token}"),
        format!("==> Summary: {token} was successfully installed"),
    ]
}

fn run_setting(s: &mut Script, entry: &PlanEntry, speed: f64) {
    s.state(entry, ItemState::Checking);
    s.wait(0.05 * speed);
    s.state(entry, ItemState::Acting);
    s.log(entry, &[format!("write {} (simulated)", entry.id), "restart the app that owns it, once per group".to_string()]);
    s.wait(0.08 * speed);
    s.state(entry, ItemState::Verifying);
    s.wait(0.05 * speed);
    s.log(entry, &["read back: matches. That shows it's stored, not that it took effect.".to_string()]);
    s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::Satisfied, assurance: Some(Assurance::Stored), failure: None, blocked_by: None, acted: true });
}

fn run_install(s: &mut Script, entry: &PlanEntry, seed: usize) {
    let lines = install_lines(entry);
    let weight = 0.45 + (seed % 5) as f64 * 0.12;
    s.state(entry, ItemState::Checking);
    s.wait(0.12);
    if entry.already_ok {
        s.log(entry, &[format!("{} is already here. Nothing to do.", entry.name)]);
        s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::Satisfied, assurance: Some(Assurance::Proven), failure: None, blocked_by: None, acted: false });
        return;
    }
    s.state(entry, ItemState::Acting);
    s.log(entry, &lines[..3]);
    for step in 1..=4 {
        s.wait(weight / 4.0);
        s.push(Event::ItemProgress { item: entry.id.clone(), phase: "Downloading".to_string(), fraction: step as f32 / 4.0 });
    }
    s.log(entry, &lines[3..]);
    s.state(entry, ItemState::Verifying);
    s.wait(0.12);
    s.log(entry, &["check: installed version found. Confirmed.".to_string()]);
    s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::Satisfied, assurance: Some(Assurance::Proven), failure: None, blocked_by: None, acted: true });
}

fn run_failure(s: &mut Script, entry: &PlanEntry, blocked: &[&PlanEntry]) {
    s.state(entry, ItemState::Checking);
    s.wait(0.12);
    s.state(entry, ItemState::Acting);
    let token = catalog().item(&entry.id).and_then(|i| i.install.token.clone()).unwrap_or_else(|| entry.id.clone());
    s.log(entry, &[format!("$ brew install {token}"), format!("==> Fetching {token}")]);
    for attempt in 1..=2 {
        s.wait(0.7);
        s.log(entry, &[format!("curl: (28) Operation timed out. Trying again ({attempt} of 2)")]);
    }
    s.wait(0.7);
    s.log(entry, &["curl: (28) Operation timed out".to_string(), "Error: gave up after 3 tries. The rest of the setup carries on.".to_string()]);
    s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::Failed, assurance: None, failure: Some(FailureKind::Network), blocked_by: None, acted: true });
    for other in blocked {
        s.push(Event::ItemChanged { item: other.id.clone(), to: ItemState::Blocked, assurance: None, failure: None, blocked_by: Some(entry.id.clone()), acted: false });
        s.log(other, &[format!("Not attempted: {} needs {}, which failed.", other.name, entry.name)]);
    }
}

fn run_surprise(s: &mut Script, entry: &PlanEntry) {
    s.state(entry, ItemState::Checking);
    s.wait(0.12);
    s.state(entry, ItemState::Acting);
    let token = catalog().item(&entry.id).and_then(|i| i.install.token.clone()).unwrap_or_else(|| entry.id.clone());
    s.log(entry, &[format!("$ HOMEBREW_NO_SUDO=1 brew install --cask {token}"), format!("==> Installing {token}")]);
    s.wait(0.6);
    s.log(entry, &[
        "Error: this cask needs sudo, and the password is already gone.".to_string(),
        "Added to your list. The next setup will put it with the password steps.".to_string(),
    ]);
    s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::NeedsYou, assurance: None, failure: Some(FailureKind::NeedsAdmin), blocked_by: None, acted: true });
    s.push(Event::QueueOpened(QueueEntry {
        id: format!("needs-admin-{}", entry.id),
        item: entry.id.clone(),
        kind: QueueKind::NeedsAdmin,
        title: format!("Finish installing {}", entry.name),
        detail: format!("{} asked for your password after Soulcaster had discarded it, so it stopped there. Everything else carried on. Enter your password once more to finish it.", entry.name),
        action: "Enter password and finish".to_string(),
        needs_password: true,
    }));
}

fn run_waiting(s: &mut Script, entry: &PlanEntry, waits: &Waits) {
    s.state(entry, ItemState::Checking);
    s.wait(0.1);
    let (kind, title, detail, action, needs_password) = match waits {
        Waits::Person { title, detail, action, apple_account } => {
            (QueueKind::Person, title.clone(), detail.clone(), action.clone(), *apple_account)
        }
        Waits::License => (
            QueueKind::License,
            format!("Accept the {} license", entry.name),
            "You didn't accept this before the setup started, so it stopped there. Everything else carried on.".to_string(),
            "Accept license".to_string(),
            false,
        ),
        Waits::Tap(tap) => (
            QueueKind::Tap,
            format!("Allow {tap} for {}", entry.name),
            format!("{} comes from {tap}, not Homebrew's official list, and you didn't allow it before the setup started.", entry.name),
            "Allow and install".to_string(),
            false,
        ),
    };
    s.log(entry, &[format!("Waiting for you: {title}")]);
    s.push(Event::ItemChanged { item: entry.id.clone(), to: ItemState::NeedsYou, assurance: None, failure: None, blocked_by: None, acted: false });
    s.push(Event::QueueOpened(QueueEntry { id: format!("{}-gate", entry.id), item: entry.id.clone(), kind, title, detail, action, needs_password }));
}

/// The whole Run, in the lifecycle's order: check, Admin Stage, discard, User Stage, finish.
pub fn schedule(plan: &Plan, scenario: Scenario, touch_id: bool) -> Vec<Timed> {
    let mut s = Script { at: 0.0, seq: 0, out: Vec::new() };
    s.push(Event::RunStarted { items: plan.entries.iter().map(|e| e.id.clone()).collect() });
    if touch_id {
        s.push(Event::RunWarning { text: "Touch ID is on for sudo, so macOS may ask for your fingerprint during the first part.".to_string() });
    }
    s.wait(0.4);

    let failing = scenario.fails().then(|| failing_item(plan)).flatten();
    let surprise = scenario.surprises().then(|| surprise_item(plan, failing)).flatten();
    let blocked: Vec<&PlanEntry> = failing
        .map(|f| plan.entries.iter().filter(|e| catalog().requires(&e.id).contains(&f.id)).collect())
        .unwrap_or_default();

    let admin: Vec<&PlanEntry> = plan.entries.iter().filter(|e| e.stage == Stage::Admin).collect();
    if plan.needs_password {
        let why = format!("{} need it", plan.admin_names.join(", "));
        s.push(Event::AdminHeld { why });
    }
    let mut seed = 0;
    let mut go = |s: &mut Script, entry: &PlanEntry| {
        seed += 1;
        if failing.is_some_and(|f| f.id == entry.id) {
            run_failure(s, entry, &blocked);
        } else if blocked.iter().any(|b| b.id == entry.id) {
            // Already reported when its requirement failed.
        } else if surprise.is_some_and(|x| x.id == entry.id) {
            run_surprise(s, entry);
        } else if let Some(waits) = &entry.waits_for {
            run_waiting(s, entry, waits);
        } else if entry.is_setting {
            run_setting(s, entry, 1.0);
        } else {
            run_install(s, entry, seed);
        }
    };
    for entry in &admin {
        go(&mut s, entry);
    }
    if plan.needs_password {
        s.wait(0.2);
        s.push(Event::AdminDiscarded);
        s.wait(0.5);
    }
    for entry in plan.entries.iter().filter(|e| e.stage == Stage::User) {
        go(&mut s, entry);
    }
    s.wait(0.4);
    s.push(Event::RunFinished);
    s.out
}

/// A retry, or finishing a queue entry: a smaller Run over one Item and whatever waits on it.
pub fn scoped(item: &str, name: &str, waiting: &[(String, String)], queue_id: Option<&str>, start_at: f64, seq: u64) -> Vec<Timed> {
    let mut s = Script { at: start_at, seq, out: Vec::new() };
    let entry = |id: &str, name: &str| PlanEntry {
        id: id.to_string(),
        name: name.to_string(),
        stage: Stage::User,
        why: Why::Selected,
        is_setting: false,
        already_ok: false,
        waits_for: None,
    };
    run_install(&mut s, &entry(item, name), 2);
    if let Some(id) = queue_id {
        s.push(Event::QueueClosed { id: id.to_string() });
    }
    for (id, name) in waiting {
        run_install(&mut s, &entry(id, name), 3);
    }
    s.out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::plan::{Answers, Machine, make_plan};

    fn plan_for(ids: &[&str], machine: Machine, answers: &Answers) -> Plan {
        let selected: BTreeSet<String> = ids.iter().map(|s| s.to_string()).collect();
        let settings: BTreeMap<String, String> = [("dock.autohide".to_string(), "true".to_string())].into();
        make_plan(&selected, &settings, machine, answers)
    }

    fn sample(machine: Machine) -> Plan {
        plan_for(&["ghostty", "android-sdk", "tailscale", "aerospace", "xcode", "neovim", "docker-desktop"], machine, &Answers::default())
    }

    const SCENARIOS: [Scenario; 4] = [Scenario::AllGood, Scenario::OneFailure, Scenario::Surprise, Scenario::Both];

    #[test]
    fn every_item_reaches_a_terminal_state_in_every_scenario() {
        for scenario in SCENARIOS {
            let plan = sample(Machine::Bare);
            let events = schedule(&plan, scenario, false);
            for entry in &plan.entries {
                let last = events.iter().rev().find_map(|t| match &t.event {
                    Event::ItemChanged { item, to, .. } if *item == entry.id => Some(*to),
                    _ => None,
                });
                assert!(last.is_some_and(ItemState::is_terminal), "{} ended at {last:?} in {scenario:?}", entry.id);
            }
            assert!(matches!(events.last().unwrap().event, Event::RunFinished));
        }
    }

    #[test]
    fn the_password_is_gone_before_any_user_stage_item_starts() {
        let plan = sample(Machine::Bare);
        let events = schedule(&plan, Scenario::Both, false);
        let discarded = events.iter().position(|t| matches!(t.event, Event::AdminDiscarded)).expect("discard");
        for t in &events[..discarded] {
            if let Event::ItemChanged { item, .. } = &t.event {
                let entry = plan.entries.iter().find(|e| e.id == *item).unwrap();
                assert_eq!(entry.stage, Stage::Admin, "{item} ran while the password was held");
            }
        }
    }

    #[test]
    fn third_party_tap_items_never_run_in_the_admin_stage() {
        let mut answers = Answers::default();
        answers.taps.insert("koekeishiya/formulae".to_string());
        let plan = plan_for(&["yabai", "aerospace"], Machine::Bare, &answers);
        for entry in &plan.entries {
            if catalog().item(&entry.id).and_then(|i| i.tap.as_ref()).is_some() {
                assert_eq!(entry.stage, Stage::User, "{}", entry.id);
            }
        }
    }

    #[test]
    fn a_failure_never_stops_the_rest() {
        let plan = sample(Machine::Bare);
        let events = schedule(&plan, Scenario::OneFailure, false);
        let failed = events.iter().position(|t| matches!(t.event, Event::ItemChanged { to: ItemState::Failed, .. })).expect("a failure");
        assert!(events[failed..].iter().any(|t| matches!(t.event, Event::ItemChanged { to: ItemState::Satisfied, .. })));
    }

    #[test]
    fn what_fails_blocks_only_what_waits_on_it() {
        let plan = sample(Machine::Bare);
        let events = schedule(&plan, Scenario::OneFailure, false);
        let blocked: Vec<_> = events.iter().filter_map(|t| match &t.event {
            Event::ItemChanged { item, to: ItemState::Blocked, blocked_by, .. } => Some((item.clone(), blocked_by.clone())),
            _ => None,
        }).collect();
        assert_eq!(blocked, vec![("android-sdk".to_string(), Some("openjdk".to_string()))]);
    }

    #[test]
    fn a_surprise_goes_to_the_queue_after_the_password_is_gone() {
        let plan = sample(Machine::Bare);
        let events = schedule(&plan, Scenario::Surprise, false);
        let discarded = events.iter().position(|t| matches!(t.event, Event::AdminDiscarded)).unwrap();
        let opened = events.iter().position(|t| matches!(&t.event, Event::QueueOpened(q) if q.kind == QueueKind::NeedsAdmin)).expect("queue entry");
        assert!(opened > discarded);
    }

    #[test]
    fn the_password_is_asked_for_only_when_the_plan_has_admin_work() {
        let bare = plan_for(&["neovim"], Machine::Bare, &Answers::default());
        assert!(bare.needs_password, "a bare Mac installs Command Line Tools and Homebrew");
        let ready = plan_for(&["neovim"], Machine::HasHomebrew, &Answers::default());
        assert!(!ready.needs_password, "nothing here needs admin");
        assert!(!ready.needs_github);
        let cask = plan_for(&["tailscale"], Machine::HasHomebrew, &Answers::default());
        assert!(cask.needs_password, "Tailscale's installer needs admin");
    }

    #[test]
    fn xcode_holds_nothing_open_while_it_waits_for_a_person() {
        let plan = plan_for(&["xcode"], Machine::HasHomebrew, &Answers::default());
        let xcode = plan.entries.iter().find(|e| e.id == "xcode").unwrap();
        assert_eq!(xcode.stage, Stage::User);
        assert!(!plan.needs_password);
    }

    #[test]
    fn events_are_in_order() {
        let events = schedule(&sample(Machine::Bare), Scenario::Both, false);
        for pair in events.windows(2) {
            assert!(pair[0].seq < pair[1].seq);
            assert!(pair[0].at <= pair[1].at);
        }
    }

    #[test]
    fn conflicts_come_from_the_catalog() {
        let plan = plan_for(&["rust", "rustup"], Machine::Bare, &Answers::default());
        assert_eq!(plan.conflicts.len(), 1);
    }

    #[test]
    fn the_catalog_loads_at_the_size_the_screens_must_handle() {
        let n = catalog().items.len();
        assert!((150..=300).contains(&n), "{n} Items");
        for item in &catalog().items {
            assert!(catalog().sections.iter().any(|s| s.id == item.section), "{} has no section", item.id);
        }
    }
}
