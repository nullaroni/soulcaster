//! Named starting points for headless screenshots of states a script reaches slowly, and for
//! `--scene` when looking at one screen.

use crate::lifecycle::{Event, ItemState};
use crate::plan::Machine;
use crate::sim::Scenario;
use crate::state::{App, GithubState, PasswordState, Screen, init};

fn answered(app: &mut App) {
    app.fill_typical();
    app.github = GithubState::Done;
    app.password_state = PasswordState::Accepted;
    app.answers.dropped.insert("rust".to_string());
    app.answers.licenses.insert("android-sdk".to_string());
    app.answers.taps.insert("nikitabobko/tap".to_string());
}

/// Moves a Run to its end at once, for the screens after it.
fn finished(app: &mut App, scenario: Scenario) {
    answered(app);
    app.scenario = scenario;
    app.start_run();
    let events: Vec<_> = app.schedule.iter().map(|t| t.event.clone()).collect();
    for event in events {
        app.apply(event);
    }
    app.cursor = app.schedule.len();
    app.timers.clear();
    app.screen = Screen::Finish;
}

pub const SCENES: &[(&str, fn(&mut App))] = &[
    ("catalog", |_| {}),
    ("catalog-picked", |a| a.fill_typical()),
    ("review", |a| {
        a.fill_typical();
        a.go_to_review();
    }),
    ("review-answered", |a| {
        answered(a);
        a.screen = Screen::Review;
    }),
    ("review-nothing-needed", |a| {
        a.machine = Machine::HasHomebrew;
        for id in ["neovim", "ripgrep", "jq", "fzf"] {
            a.selected.insert(id.to_string());
        }
        a.github = GithubState::Done;
        a.screen = Screen::Review;
    }),
    // Mid-Run, with a failure already on screen and another Item still installing.
    // One failed Item never stops the rest; the shot has to show both at once.
    ("run", |a| {
        answered(a);
        a.scenario = Scenario::Both;
        a.start_run();
        let events: Vec<Event> = a.schedule.iter().map(|t| t.event.clone()).collect();
        let mut applied = 0usize;
        let mut seen_fail = false;
        for event in &events {
            if matches!(event, Event::RunFinished) {
                break;
            }
            a.apply(event.clone());
            applied += 1;
            if a.run_items.iter().any(|i| i.state == ItemState::Failed) {
                seen_fail = true;
            }
            let painting = a.run_items.iter().any(|i| i.state == ItemState::Acting && i.progress.is_some_and(|p| (0.25..0.9).contains(&p)));
            if seen_fail && painting {
                break;
            }
        }
        a.cursor = applied;
        if let Some(next) = a.schedule.get(applied) {
            a.sim_time = (next.at - 1e-4).max(0.0);
        }
        a.last_clock = 0.0;
        a.timers.clear();
        a.screen = Screen::Run;
        a.run_finished = false;
        if let Some(id) = a.run_items.iter().find(|i| i.state == ItemState::Failed).map(|i| i.id.clone()) {
            a.set_log_item(Some(id));
        }
    }),
    ("finish", |a| finished(a, Scenario::Both)),
    ("finish-clean", |a| {
        a.answers.licenses.insert("xcode".to_string());
        finished(a, Scenario::AllGood);
    }),
    ("drift", |a| a.screen = Screen::Drift),
];

/// Everything selected and a flooded log: what the heaviest Run costs the UI.
pub fn stress_init() -> App {
    let mut app = init();
    let cat = crate::catalog::catalog();
    for item in &cat.items {
        app.selected.insert(item.id.clone());
    }
    for (a, b) in [("rustup", "rust"), ("docker-desktop", "rancher"), ("mysql", "mariadb"), ("redis", "valkey")] {
        let _ = a;
        app.answers.dropped.insert(b.to_string());
    }
    app.github = GithubState::Done;
    app.password_state = PasswordState::Accepted;
    app.stress = true;
    app.speed = 1.0;
    app.rebuild_rows();
    if !std::env::args().any(|a| a == "stress-catalog") {
        app.start_run();
    }
    debug_assert!(app.run_items.iter().all(|i| i.state != ItemState::Failed));
    app
}
