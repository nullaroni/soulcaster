//! Named starting points for headless screenshots of states a Run reaches slowly, and for
//! `--scene` when looking at one screen. Scenes go through the same event path as a real Run.

use soulcaster_core::config::SettingValue;
use soulcaster_core::lifecycle::ItemState;
use soulcaster_repo::address::ConfigPath;
use soulcaster_repo::review::Tick;

use crate::fleet_flow::{FleetSample, seeded_fleet};
use crate::repo_flow::DotfilesPhase;
use crate::sim::{Scenario, SimEngine, SimMachine};
use crate::state::{App, GithubState, PasswordState, SETTINGS_SECTION, Screen};

fn answered(app: &mut App) {
    app.fill_typical();
    app.github = GithubState::Done;
    app.password_state = PasswordState::Accepted;
    app.selected.remove("rust");
    app.answers.licenses.insert("android-sdk".to_string());
    app.answers.taps.insert("nikitabobko/tap".to_string());
    app.rebuild_rows();
}

/// Lets the simulated clock run until `stop` says so, or the engine has nothing left.
fn run_until(app: &mut App, stop: impl Fn(&App) -> bool) {
    let mut clock = app.clock;
    while !stop(app) {
        let Some(next) = app.next_due() else { break };
        clock += next + 0.001;
        app.tick(clock);
    }
}

/// A Run taken to its end at once, for the screens after it. `trim` runs just before it starts.
fn finished(app: &mut App, scenario: Scenario, trim: fn(&mut App)) {
    answered(app);
    trim(app);
    app.engine = Box::new(SimEngine::new(scenario, SimMachine::Bare));
    app.start_run();
    run_until(app, |_| false);
    app.screen = Screen::Finish;
    app.timers.clear();
    app.clock = 0.0;
}

fn import(app: &mut App, paste: &str) {
    app.screen = Screen::Import;
    app.read_sample(paste);
}

fn pick(app: &mut App, path: &str) {
    app.pick_config(&ConfigPath::parse(path).expect("a fixed path"));
}

type Scene = (&'static str, fn(&mut App));

pub const SCENES: &[Scene] = &[
    ("fleet", |a| a.open_fleet()),
    ("fleet-one", |a| {
        a.repo.sim.fleet = seeded_fleet(FleetSample::One);
        a.open_fleet();
    }),
    ("fleet-empty", |a| {
        a.repo.sim.fleet = seeded_fleet(FleetSample::Empty);
        a.open_fleet();
    }),
    ("catalog", |a| a.screen = Screen::Catalog),
    ("catalog-picked", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
    }),
    ("catalog-browsers", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.show_section(1);
    }),
    ("catalog-editors", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.show_section(2);
    }),
    ("catalog-cloud", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.show_section(10);
    }),
    ("catalog-settings", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.show_section(SETTINGS_SECTION);
    }),
    ("catalog-search", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.search.text = "git".to_string();
        a.rebuild_rows();
    }),
    ("catalog-selected", |a| {
        a.screen = Screen::Catalog;
        a.fill_typical();
        a.only_selected = true;
        a.rebuild_rows();
    }),
    ("review", |a| {
        a.fill_typical();
        a.screen = Screen::Review;
    }),
    ("review-answered", |a| {
        answered(a);
        a.screen = Screen::Review;
    }),
    ("review-nothing-needed", |a| {
        a.base_present = true;
        for id in ["neovim", "ripgrep", "jq", "fzf"] {
            a.selected.insert(id.to_string());
        }
        a.github = GithubState::Done;
        a.screen = Screen::Review;
    }),
    // Mid-Run, with a failure already on screen and another Item still installing. One failed Item
    // never stops the rest, so the shot has to show both at once.
    ("run", |a| {
        answered(a);
        a.start_run();
        run_until(a, |a| {
            let failed = a
                .run_items
                .iter()
                .any(|i| matches!(i.state, ItemState::Failed { .. }));
            failed
                && a.run_items.iter().any(|i| {
                    i.state == ItemState::Acting
                        && i.progress.is_some_and(|p| (0.25..0.9).contains(&p))
                })
        });
        if let Some(id) = a
            .run_items
            .iter()
            .find(|i| matches!(i.state, ItemState::Failed { .. }))
            .map(|i| i.id.clone())
        {
            a.set_log_item(Some(id));
        }
        // The snapshot's own clock starts at zero, which is before anything left is due.
        a.clock = 0.0;
    }),
    ("finish", |a| finished(a, Scenario::Both, |_| {})),
    ("finish-clean", |a| {
        finished(a, Scenario::AllGood, |a| {
            // Xcode and AeroSpace always stop for a person, so a clean Run leaves them out.
            a.selected.remove("xcode");
            a.selected.remove("aerospace");
        });
    }),
    ("drift", |a| a.screen = Screen::Drift),
    ("run-real-asks", |a| {
        a.start_run();
        a.engine = Box::new(AskingEngine);
        a.screen = Screen::Run;
    }),
    ("import", |a| a.screen = Screen::Import),
    ("import-failed", |a| import(a, "example-dev/private-setup")),
    ("import-bad-link", |a| {
        import(a, "https://github.com.evil.example/a/b")
    }),
    ("import-pick", |a| import(a, "example-dev/mac-setup")),
    ("import-review-clean", |a| {
        import(a, "example-dev/mac-setup");
        pick(a, "configs/laptop.json");
    }),
    ("import-review-one-tap", |a| {
        import(a, "example-dev/one-config")
    }),
    ("import-review-stranger", |a| {
        import(a, "example-stranger/setup")
    }),
    ("import-review-stranger-on", |a| {
        import(a, "example-stranger/setup");
        a.review_tick(&Tick::Repo);
        a.review_tick(&Tick::Script);
    }),
    ("import-review-own", |a| import(a, "example-you/setup")),
    ("import-review-hostile", |a| {
        import(a, "s0ulcaster-official/verified-configs")
    }),
    ("import-refused", |a| import(a, "example-dev/odd-config")),
    ("import-kept", |a| {
        import(a, "example-you/setup");
        a.keep_copy();
    }),
    ("save", |a| {
        a.fill_typical();
        a.screen = Screen::Save;
    }),
    ("save-sent", |a| {
        a.fill_typical();
        a.screen = Screen::Save;
        a.save_config();
    }),
    ("save-waiting", |a| {
        a.fill_typical();
        a.screen = Screen::Save;
        a.repo.sim.github.go_offline();
        a.save_config();
    }),
    ("save-blocked", |a| {
        a.settings.insert(
            "example.note".to_string(),
            SettingValue::String("ghp_abcdefghijklmnopqrstuvwxyz".to_string()),
        );
        a.screen = Screen::Save;
        a.save_config();
    }),
    ("dotfiles", |a| a.screen = Screen::Dotfiles),
    ("dotfiles-applied", |a| {
        a.screen = Screen::Dotfiles;
        a.apply_dotfiles();
    }),
    ("dotfiles-behind", |a| {
        a.screen = Screen::Dotfiles;
        a.review_dotfiles_update();
    }),
    ("dotfiles-rewritten", |a| {
        a.screen = Screen::Dotfiles;
        a.repo.dotfiles = DotfilesPhase::Rewritten;
    }),
];

/// For the look of a real Run's questions. It starts nothing.
struct AskingEngine;

impl crate::engine::Engine for AskingEngine {
    fn start_run(&mut self, _: &soulcaster_core::plan::Plan, _: &crate::answers::Answers) {}

    fn start_scoped(&mut self, _: &str, _: &[String], _: Option<&str>) {}

    fn submit_password(&mut self, _: soulcaster_core::ask::Password) {}

    fn start_github_sign_in(&mut self) -> String {
        String::new()
    }

    fn poll(&mut self, _: f64) -> Vec<soulcaster_core::event::Event> {
        Vec::new()
    }

    fn next_due(&self, _: f64) -> Option<f64> {
        None
    }

    fn is_real(&self) -> bool {
        true
    }

    fn password_request(&self) -> Option<soulcaster_core::ask::PasswordRequest> {
        Some(soulcaster_core::ask::PasswordRequest {
            why: "the admin steps".to_string(),
            last_was_wrong: false,
        })
    }

    fn github_code(&self) -> Option<(String, String)> {
        Some((
            "WDJB-MJHT".to_string(),
            "https://github.com/login/device".to_string(),
        ))
    }
}
