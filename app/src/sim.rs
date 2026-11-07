//! A pretend engine: turns an approved plan and a scenario into the event stream a real Run would
//! produce, on a clock. Deterministic, so the same scene always shows the same screen.
//!
//! It only ever builds data. Nothing here starts a process, reads the disk or opens a socket.

use std::collections::{BTreeMap, VecDeque};

use soulcaster_core::ask::Password;
use soulcaster_core::event::{
    AskKind, AskStatus, ConfigRef, Event, EventKind, LogStream, QueueEntry, QueueKind, RunScope,
    Stamper,
};
use soulcaster_core::lifecycle::{Assurance, FailureKind, ItemState, Stage};
use soulcaster_core::plan::{Plan, PlanEntry, Why};
use soulcaster_core::recipe::{Recipe, Step};

use crate::answers::{Answers, Waits, waits_for};
use crate::engine::Engine;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scenario {
    AllGood,
    Both,
}

/// Which sample Machine the simulated check reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SimMachine {
    Bare,
    HasHomebrew,
}

impl SimMachine {
    pub fn base_present(self) -> bool {
        self == SimMachine::HasHomebrew
    }
}

struct Script {
    at: f64,
    stamper: Stamper,
    out: Vec<(f64, Event)>,
    states: BTreeMap<String, ItemState>,
}

impl Script {
    fn new(start: f64, stamper: Stamper) -> Self {
        Self {
            at: start,
            stamper,
            out: Vec::new(),
            states: BTreeMap::new(),
        }
    }

    fn push(&mut self, kind: EventKind) {
        let event = self.stamper.stamp((self.at * 1000.0) as u64, kind);
        self.out.push((self.at, event));
    }

    fn wait(&mut self, seconds: f64) {
        self.at += seconds;
    }

    /// Moves an Item, filling in `from`, so every event is a legal step of the lifecycle diagram.
    fn state(&mut self, item: &str, to: ItemState) {
        let from = self.states.get(item).cloned().unwrap_or(ItemState::Pending);
        self.states.insert(item.to_string(), to.clone());
        self.push(EventKind::ItemState {
            item: item.to_string(),
            from,
            to,
        });
    }

    fn log(&mut self, item: &str, lines: &[String]) {
        self.push(EventKind::ItemLog {
            item: item.to_string(),
            stream: LogStream::Stdout,
            lines: lines.to_vec(),
        });
    }

    fn progress(&mut self, item: &str, done: u64) {
        self.push(EventKind::ItemProgress {
            item: item.to_string(),
            phase: "Downloading".to_string(),
            bytes_done: done,
            bytes_total: Some(4),
        });
    }

    fn finish(&mut self) {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for state in self.states.values() {
            *counts.entry(state.name().to_string()).or_default() += 1;
        }
        self.wait(0.4);
        self.push(EventKind::RunFinished {
            counts,
            queue: Vec::new(),
        });
    }
}

fn is_setting(entry: &PlanEntry) -> bool {
    matches!(entry.recipe, Recipe::Setting(_))
}

/// Which Item fails in this scenario: openjdk when it's there, because something else waits on it.
fn failing_item<'a>(plan: &'a Plan, answers: &Answers) -> Option<&'a PlanEntry> {
    let usable =
        |e: &&PlanEntry| e.why != Why::Base && !is_setting(e) && waits_for(e, answers).is_none();
    plan.entries
        .iter()
        .find(|e| e.id == "openjdk")
        .filter(usable)
        .or_else(|| plan.entries.iter().filter(usable).nth(3))
}

/// Which Item surprises everyone by needing admin after the password's gone: an official cask the
/// Catalog doesn't say needs it, in the User Stage.
fn surprise_item<'a>(
    plan: &'a Plan,
    failing: Option<&PlanEntry>,
    answers: &Answers,
) -> Option<&'a PlanEntry> {
    let usable = |e: &&PlanEntry| {
        e.stage == Stage::User
            && e.why != Why::Base
            && waits_for(e, answers).is_none()
            && failing.is_none_or(|f| f.id != e.id)
            && e.recipe.taps().is_empty()
            && matches!(e.recipe.steps().first(), Some(Step::BrewCask { .. }))
    };
    plan.entries
        .iter()
        .find(|e| usable(e) && e.id == "docker-desktop")
        .or_else(|| plan.entries.iter().find(usable))
}

/// Everything that needs `failed`, directly or not, in plan order.
fn waiting_on<'a>(plan: &'a Plan, failed: &str) -> Vec<&'a PlanEntry> {
    let mut stuck: Vec<&PlanEntry> = Vec::new();
    for entry in &plan.entries {
        if entry
            .recipe
            .requires()
            .iter()
            .any(|r| r == failed || stuck.iter().any(|s| s.id == *r))
        {
            stuck.push(entry);
        }
    }
    stuck
}

fn brew_command(entry: &PlanEntry) -> (String, String) {
    let step = entry
        .recipe
        .steps()
        .iter()
        .find(|s| s.brew_name().is_some());
    let token = step
        .and_then(Step::brew_name)
        .unwrap_or(&entry.id)
        .to_string();
    let flag = if matches!(step, Some(Step::BrewCask { .. })) {
        "--cask "
    } else {
        ""
    };
    (format!("$ brew install {flag}{token}"), token)
}

fn run_setting(s: &mut Script, entry: &PlanEntry) {
    s.state(&entry.id, ItemState::Checking);
    s.wait(0.05);
    s.state(&entry.id, ItemState::Acting);
    s.log(
        &entry.id,
        &[
            format!("write {} (simulated)", entry.id),
            "restart the app that owns it, once per group".to_string(),
        ],
    );
    s.wait(0.08);
    s.state(&entry.id, ItemState::Verifying);
    s.wait(0.05);
    s.log(
        &entry.id,
        &["read back: matches. That shows it's stored, not that it took effect.".to_string()],
    );
    s.state(
        &entry.id,
        ItemState::Satisfied {
            assurance: Assurance::Stored,
        },
    );
}

fn run_install(s: &mut Script, entry: &PlanEntry, seed: usize, already_here: bool) {
    let (command, token) = brew_command(entry);
    let weight = 0.45 + (seed % 5) as f64 * 0.12;
    s.state(&entry.id, ItemState::Checking);
    s.wait(0.12);
    if already_here {
        s.log(
            &entry.id,
            &[format!("{} is already here. Nothing to do.", entry.name)],
        );
        s.state(
            &entry.id,
            ItemState::Satisfied {
                assurance: Assurance::Proven,
            },
        );
        return;
    }
    s.state(&entry.id, ItemState::Acting);
    s.log(&entry.id, &[command, format!("==> Fetching {token}")]);
    for done in 1..=4 {
        s.wait(weight / 4.0);
        s.progress(&entry.id, done);
    }
    s.log(
        &entry.id,
        &[
            format!("==> Installing {token}"),
            format!("==> Summary: {token} was successfully installed"),
        ],
    );
    s.state(&entry.id, ItemState::Verifying);
    s.wait(0.12);
    s.log(
        &entry.id,
        &["check: installed version found. Confirmed.".to_string()],
    );
    s.state(
        &entry.id,
        ItemState::Satisfied {
            assurance: Assurance::Proven,
        },
    );
}

fn run_failure(s: &mut Script, entry: &PlanEntry, blocked: &[&PlanEntry]) {
    let (command, token) = brew_command(entry);
    s.state(&entry.id, ItemState::Checking);
    s.wait(0.12);
    s.state(&entry.id, ItemState::Acting);
    s.log(&entry.id, &[command, format!("==> Fetching {token}")]);
    for attempt in 1..=2 {
        s.wait(0.7);
        s.log(
            &entry.id,
            &[format!(
                "curl: (28) Operation timed out. Trying again ({attempt} of 2)"
            )],
        );
    }
    s.wait(0.7);
    s.log(
        &entry.id,
        &[
            "curl: (28) Operation timed out".to_string(),
            "Error: gave up after 3 tries".to_string(),
        ],
    );
    s.state(
        &entry.id,
        ItemState::Failed {
            kind: FailureKind::Network,
        },
    );
    for other in blocked {
        s.state(
            &other.id,
            ItemState::Blocked {
                by: entry.id.clone(),
            },
        );
        s.log(
            &other.id,
            &[format!(
                "Not attempted: {} needs {}, which failed.",
                other.name, entry.name
            )],
        );
    }
}

fn queue(
    item: &str,
    kind: QueueKind,
    text: String,
    link: Option<String>,
    needs_password: bool,
) -> EventKind {
    let entry = QueueEntry {
        id: format!("{item}-queue"),
        item: item.to_string(),
        kind,
        text,
        detail: None,
        link,
        needs_password,
    };
    EventKind::QueueOpened { entry }
}

fn run_surprise(s: &mut Script, entry: &PlanEntry) {
    let (command, token) = brew_command(entry);
    s.state(&entry.id, ItemState::Checking);
    s.wait(0.12);
    s.state(&entry.id, ItemState::Acting);
    s.log(&entry.id, &[command, format!("==> Installing {token}")]);
    s.wait(0.6);
    s.log(
        &entry.id,
        &["Error: this cask needs sudo, and the password is already gone.".to_string()],
    );
    s.state(
        &entry.id,
        ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin),
        },
    );
    s.push(queue(
        &entry.id,
        QueueKind::NeedsAdmin,
        format!(
            "{} asked for your password after Soulcaster had discarded it.",
            entry.name
        ),
        None,
        true,
    ));
}

fn run_waiting(s: &mut Script, entry: &PlanEntry, waits: Waits) {
    s.state(&entry.id, ItemState::Checking);
    s.wait(0.1);
    s.state(&entry.id, ItemState::NeedsYou { cause: None });
    let kind = match waits {
        Waits::Person { text, link } => {
            let needs_password = entry.recipe.has_admin_step();
            queue(&entry.id, QueueKind::Person, text, link, needs_password)
        }
        Waits::License => queue(
            &entry.id,
            QueueKind::License,
            format!("Accept the {} license.", entry.name),
            None,
            false,
        ),
        Waits::Tap(tap) => queue(
            &entry.id,
            QueueKind::Tap,
            format!("Allow {tap} for {}.", entry.name),
            None,
            false,
        ),
    };
    s.push(kind);
}

/// The whole Run, in the lifecycle's order: check, Admin Stage, discard, User Stage, finish.
fn schedule(
    plan: &Plan,
    answers: &Answers,
    scenario: Scenario,
    machine: SimMachine,
    start: f64,
    stamper: Stamper,
) -> Vec<(f64, Event)> {
    let mut s = Script::new(start, stamper);
    s.push(EventKind::RunStarted {
        run_id: "simulated".to_string(),
        scope: RunScope::Full,
        config: ConfigRef {
            name: plan.config_name.clone(),
            ..ConfigRef::default()
        },
        catalog_version: soulcaster_core::catalog::Catalog::shipped()
            .version()
            .to_string(),
        engine_version: "simulated".to_string(),
        macos_version: None,
        plan: plan.summary(),
    });
    s.wait(0.4);

    let fails = scenario == Scenario::Both;
    let failing = fails.then(|| failing_item(plan, answers)).flatten();
    let surprise = fails
        .then(|| surprise_item(plan, failing, answers))
        .flatten();
    let blocked: Vec<&PlanEntry> = failing.map(|f| waiting_on(plan, &f.id)).unwrap_or_default();

    if plan.needs_admin() {
        let names: Vec<&str> = plan
            .entries
            .iter()
            .filter(|e| e.stage == Stage::Admin)
            .map(|e| e.name.as_str())
            .collect();
        s.push(EventKind::AdminHeld {
            why: format!("{} need it", names.join(", ")),
        });
    }
    let mut seed = 0;
    let mut run_one = |s: &mut Script, entry: &PlanEntry| {
        seed += 1;
        let already_here = machine.base_present() && entry.why == Why::Base;
        if failing.is_some_and(|f| f.id == entry.id) {
            run_failure(s, entry, &blocked);
        } else if blocked.iter().any(|b| b.id == entry.id) {
            // Already reported when its requirement failed.
        } else if surprise.is_some_and(|x| x.id == entry.id) {
            run_surprise(s, entry);
        } else if let Some(waits) = waits_for(entry, answers) {
            run_waiting(s, entry, waits);
        } else if is_setting(entry) {
            run_setting(s, entry);
        } else {
            run_install(s, entry, seed, already_here);
        }
    };
    for entry in plan.entries.iter().filter(|e| e.stage == Stage::Admin) {
        run_one(&mut s, entry);
    }
    if plan.needs_admin() {
        s.wait(0.2);
        s.push(EventKind::AdminDiscarded {
            why: "The Admin Stage finished".to_string(),
        });
        s.wait(0.5);
    }
    for entry in plan.entries.iter().filter(|e| e.stage == Stage::User) {
        run_one(&mut s, entry);
    }
    s.finish();
    s.out
}

/// Stands in for the real engine until the wiring ticket replaces it.
pub struct SimEngine {
    pub scenario: Scenario,
    pub machine: SimMachine,
    pending: VecDeque<(f64, Event)>,
    now: f64,
    last_seq: u64,
}

impl SimEngine {
    pub fn new(scenario: Scenario, machine: SimMachine) -> Self {
        Self {
            scenario,
            machine,
            pending: VecDeque::new(),
            now: 0.0,
            last_seq: 0,
        }
    }

    fn enqueue(&mut self, events: Vec<(f64, Event)>) {
        self.last_seq = events.last().map_or(self.last_seq, |(_, event)| event.seq);
        self.pending.extend(events);
        self.pending
            .make_contiguous()
            .sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    fn one_event(&mut self, delay: f64, kind: EventKind) {
        let mut stamper = Stamper::after(self.last_seq);
        let at = self.now + delay;
        let event = stamper.stamp((at * 1000.0) as u64, kind);
        self.enqueue(vec![(at, event)]);
    }
}

impl Engine for SimEngine {
    fn start_run(&mut self, plan: &Plan, answers: &Answers) {
        let events = schedule(
            plan,
            answers,
            self.scenario,
            self.machine,
            self.now,
            Stamper::after(self.last_seq),
        );
        self.enqueue(events);
    }

    fn start_scoped(&mut self, item: &str, waiting: &[String], queue_id: Option<&str>) {
        let mut s = Script::new(self.now, Stamper::after(self.last_seq));
        let mut items = vec![item.to_string()];
        items.extend_from_slice(waiting);
        s.push(EventKind::RunStarted {
            run_id: "simulated".to_string(),
            scope: RunScope::Items {
                items: items.clone(),
            },
            config: ConfigRef::default(),
            catalog_version: String::new(),
            engine_version: "simulated".to_string(),
            macos_version: None,
            plan: Vec::new(),
        });
        for id in &items {
            s.state(id, ItemState::Checking);
            s.wait(0.12);
            s.state(id, ItemState::Acting);
            s.log(
                id,
                &[
                    format!("$ brew install {id}"),
                    format!("==> Installing {id}"),
                ],
            );
            s.wait(0.6);
            s.state(id, ItemState::Verifying);
            s.wait(0.12);
            s.state(
                id,
                ItemState::Satisfied {
                    assurance: Assurance::Proven,
                },
            );
            if id == item
                && let Some(queue_id) = queue_id
            {
                s.push(EventKind::QueueClosed {
                    id: queue_id.to_string(),
                });
            }
        }
        s.finish();
        self.enqueue(s.out);
    }

    fn submit_password(&mut self, password: Password) {
        // There is no sudo to ask here. "wrong" stands in for a refused password.
        let state = if password.expose() == "wrong" {
            AskStatus::Waiting
        } else {
            AskStatus::Done
        };
        self.one_event(
            0.6,
            EventKind::AskState {
                ask: AskKind::Password,
                state,
            },
        );
    }

    fn start_github_sign_in(&mut self) -> String {
        self.one_event(
            2.6,
            EventKind::AskState {
                ask: AskKind::Github,
                state: AskStatus::Done,
            },
        );
        "WDJB-MJHT".to_string()
    }

    fn poll(&mut self, now: f64) -> Vec<Event> {
        self.now = now.max(self.now);
        let mut out = Vec::new();
        while self
            .pending
            .front()
            .is_some_and(|(due, _)| *due <= self.now)
        {
            out.push(self.pending.pop_front().expect("checked the front").1);
        }
        out
    }

    fn next_due(&self, now: f64) -> Option<f64> {
        self.pending.front().map(|(due, _)| (due - now).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use soulcaster_core::catalog::Catalog;
    use soulcaster_core::config::Config;

    use super::*;

    fn plan_for(items: &[&str]) -> Plan {
        let config = json!({"schemaVersion": 1, "name": "test", "items": items, "settings": {"dock.autohide": true}});
        Plan::new(
            &Config::parse(config.to_string().as_bytes()).unwrap(),
            Catalog::shipped(),
        )
    }

    fn sample() -> Plan {
        plan_for(&[
            "ghostty",
            "android-sdk",
            "tailscale",
            "aerospace",
            "xcode",
            "neovim",
            "docker-desktop",
        ])
    }

    fn run(plan: &Plan, scenario: Scenario) -> Vec<Event> {
        let mut engine = SimEngine::new(scenario, SimMachine::Bare);
        engine.start_run(plan, &Answers::default());
        engine.poll(10_000.0)
    }

    fn state_changes(events: &[Event]) -> Vec<(&String, &ItemState, &ItemState)> {
        events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::ItemState { item, from, to } => Some((item, from, to)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_item_reaches_a_terminal_state_in_every_scenario() {
        for scenario in [Scenario::AllGood, Scenario::Both] {
            let plan = sample();
            let events = run(&plan, scenario);
            for entry in &plan.entries {
                let last = state_changes(&events)
                    .into_iter()
                    .rev()
                    .find(|(item, ..)| **item == entry.id);
                assert!(
                    last.is_some_and(|(_, _, to)| to.is_terminal()),
                    "{} in {scenario:?}",
                    entry.id
                );
            }
            assert!(matches!(
                events.last().unwrap().kind,
                EventKind::RunFinished { .. }
            ));
        }
    }

    #[test]
    fn every_state_change_is_a_legal_step_of_the_lifecycle() {
        for (item, from, to) in state_changes(&run(&sample(), Scenario::Both)) {
            assert!(from.can_move_to(to), "{item}: {from:?} -> {to:?}");
        }
    }

    #[test]
    fn the_password_is_gone_before_any_user_stage_item_starts() {
        let plan = sample();
        let events = run(&plan, Scenario::Both);
        let discarded = events
            .iter()
            .position(|e| matches!(e.kind, EventKind::AdminDiscarded { .. }))
            .expect("discard");
        for (item, ..) in state_changes(&events[..discarded]) {
            let entry = plan.entries.iter().find(|e| e.id == *item).unwrap();
            assert_eq!(
                entry.stage,
                Stage::Admin,
                "{item} ran while the password was held"
            );
        }
    }

    #[test]
    fn a_failure_never_stops_the_rest() {
        let events = run(&sample(), Scenario::Both);
        let failed = events
            .iter()
            .position(|e| {
                matches!(
                    &e.kind,
                    EventKind::ItemState {
                        to: ItemState::Failed { .. },
                        ..
                    }
                )
            })
            .expect("a failure");
        assert!(events[failed..].iter().any(|e| matches!(
            &e.kind,
            EventKind::ItemState {
                to: ItemState::Satisfied { .. },
                ..
            }
        )));
    }

    #[test]
    fn what_fails_blocks_only_what_waits_on_it() {
        let events = run(&sample(), Scenario::Both);
        let blocked: Vec<(String, String)> = state_changes(&events)
            .into_iter()
            .filter_map(|(item, _, to)| match to {
                ItemState::Blocked { by } => Some((item.clone(), by.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            blocked,
            vec![("android-sdk".to_string(), "openjdk".to_string())]
        );
    }

    #[test]
    fn a_surprise_goes_to_the_queue_after_the_password_is_gone() {
        let events = run(&sample(), Scenario::Both);
        let discarded = events
            .iter()
            .position(|e| matches!(e.kind, EventKind::AdminDiscarded { .. }))
            .unwrap();
        let opened = events
            .iter()
            .position(|e| matches!(&e.kind, EventKind::QueueOpened { entry } if entry.kind == QueueKind::NeedsAdmin))
            .expect("queue entry");
        assert!(opened > discarded);
    }

    #[test]
    fn events_are_in_order_and_start_with_the_plan() {
        let plan = sample();
        let events = run(&plan, Scenario::Both);
        assert!(
            matches!(&events[0].kind, EventKind::RunStarted { plan: items, .. } if items.len() == plan.entries.len())
        );
        for pair in events.windows(2) {
            assert!(pair[0].seq < pair[1].seq);
            assert!(pair[0].at_ms <= pair[1].at_ms);
        }
    }

    #[test]
    fn nothing_arrives_before_it_is_due() {
        let mut engine = SimEngine::new(Scenario::AllGood, SimMachine::Bare);
        engine.start_run(&sample(), &Answers::default());
        assert_eq!(engine.poll(0.0).len(), 1, "only run.started is due at once");
        assert!(engine.next_due(0.0).unwrap() > 0.0);
    }

    #[test]
    fn a_retry_reaches_satisfied_and_closes_its_queue_entry() {
        let mut engine = SimEngine::new(Scenario::AllGood, SimMachine::Bare);
        engine.start_scoped(
            "openjdk",
            &["android-sdk".to_string()],
            Some("openjdk-queue"),
        );
        let events = engine.poll(100.0);
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.kind, EventKind::QueueClosed { id } if id == "openjdk-queue"))
        );
        let last: BTreeMap<&String, &ItemState> = state_changes(&events)
            .into_iter()
            .map(|(item, _, to)| (item, to))
            .collect();
        assert!(matches!(
            last[&"android-sdk".to_string()],
            ItemState::Satisfied { .. }
        ));
    }

    #[test]
    fn a_wrong_password_asks_again_and_a_right_one_is_done() {
        let mut engine = SimEngine::new(Scenario::AllGood, SimMachine::Bare);
        engine.submit_password(Password::new("wrong".to_string()));
        engine.submit_password(Password::new("right".to_string()));
        let states: Vec<AskStatus> = engine
            .poll(5.0)
            .into_iter()
            .filter_map(|e| match e.kind {
                EventKind::AskState { state, .. } => Some(state),
                _ => None,
            })
            .collect();
        assert_eq!(states, [AskStatus::Waiting, AskStatus::Done]);
    }
}
