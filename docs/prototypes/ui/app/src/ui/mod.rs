//! The window: title bar with the steps, the current screen, and the prototype strip.

mod catalog;
mod drift;
mod finish;
pub mod kit;
mod review;
mod run;

use rune::{Cx, El, col, el, icon, row, text};
use rune_kit::Toast;

use crate::lifecycle::ItemState;
use crate::plan::Machine;
use crate::sim::Scenario;
use crate::state::{App, Screen};
use kit::{Tones, icons};

pub const TITLEBAR: f32 = 52.0;

pub fn view(app: &App, cx: &mut Cx<App>) -> El {
    let (k, tones) = kit::theme(cx.system_dark());
    let mut layer: Vec<El> = Vec::new();
    let inset = cx.chrome_insets();
    let body = match app.screen {
        Screen::Catalog => catalog::screen(app, k, tones, &mut layer),
        Screen::Review => review::screen(app, k, tones, &mut layer),
        Screen::Run => run::screen(app, k, tones),
        Screen::Finish => finish::screen(app, k, tones, &mut layer),
        Screen::Drift => drift::screen(app, k, tones, &mut layer),
    };
    if let Some(message) = &app.toast {
        layer.push(
            el().key("toast-layer").absolute(0.0, 0.0).w_full().h_full().justify_end().items_center().pb(64.0)
                .child(Toast::new("toast", message.clone()).lead(icon(&icons::CHECK_CIRCLE, 16.0).color(tones.good)).build(k)),
        );
    }
    let base = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(titlebar(app, k, tones, inset.leading))
        .child(col().key("screen").w_full().grow(1.0).min_h(0.0).child(body))
        .child(prototype_strip(app, k, tones));
    rune_kit::root(base, layer)
}

fn step_state(app: &App, step: usize) -> (bool, bool) {
    let current = match app.screen {
        Screen::Catalog => 0,
        Screen::Review => 1,
        Screen::Run => 2,
        Screen::Finish => 3,
        Screen::Drift => usize::MAX,
    };
    let reached = if app.run_finished {
        4
    } else if app.run_started {
        3
    } else if current == 1 {
        2
    } else {
        1
    };
    (current == step, step < reached)
}

fn titlebar(app: &App, k: &rune_kit::Kit, tones: Tones, leading: f32) -> El {
    let steps = ["Catalog", "Review", "Run", "Needs you"];
    let mut stepper = row().key("stepper").role("navigation").label("Setup steps").center_y().gap(6.0);
    for (i, label) in steps.iter().enumerate() {
        let (current, reached) = step_state(app, i);
        let done = reached && !current && (app.run_started || i < 1) && !(i == 3 && !app.run_finished);
        let circle = el().size(20.0, 20.0).radius(10.0).center().fill(if current { tones.accent } else if done { tones.accent_soft } else { k.c.layer_03 }).child(if done {
            icon(k.glyph.check, 11.0).color(tones.accent_ink)
        } else {
            text((i + 1).to_string()).font(k.ty.small_medium).color(if current { tones.on_accent } else { k.c.text_muted })
        });
        // Going back is free until a Run starts. After that, the Choose and Answer steps are history.
        let can_go = reached && !current && !(app.run_started && i < 2);
        let mut chip = row()
            .key(format!("step-{i}"))
            .role("button")
            .label(format!("Step {}: {label}", i + 1))
            .h(30.0)
            .px(8.0)
            .gap(7.0)
            .center_y()
            .radius(7.0)
            .disabled(!can_go && !current)
            .child(circle)
            .child(text(*label).font(if current { k.ty.text_small_medium } else { k.ty.text_small }).color(if current || reached { k.c.text_base } else { k.c.text_muted }));
        if can_go {
            chip = chip.hover(k.c.hover).focusable(true).on_press_tx(move |app: &mut App, _| match i {
                0 => app.screen = Screen::Catalog,
                1 => app.go_to_review(),
                2 => app.screen = Screen::Run,
                _ => app.screen = Screen::Finish,
            });
        }
        stepper = stepper.child(chip);
        if i + 1 < steps.len() {
            stepper = stepper.child(el().w(18.0).h(1.0).fill(k.c.border_base));
        }
    }

    let drift_count = app.drift.len();
    let on_drift = app.screen == Screen::Drift;
    let running = app.run_started && !app.run_finished;
    let mut drift = row()
        .key("nav-drift")
        .role("button")
        .label(format!("Drift, {drift_count}"))
        .h(30.0)
        .px(10.0)
        .gap(7.0)
        .center_y()
        .radius(7.0)
        .fill(if on_drift { k.c.layer_03 } else { rune::Color::TRANSPARENT })
        .child(icon(&icons::ARROWS_SWAP, 15.0).color(if running { k.c.icon_faint } else { k.c.icon_base }))
        .child(text("Drift").font(k.ty.text_small).color(if running { k.c.text_faint } else { k.c.text_base }));
    if drift_count > 0 {
        drift = drift.child(kit::count_badge(k, tones, drift_count));
    }
    if running {
        drift = drift.disabled(true).help("Only one thing can change this Mac at a time. Available when this setup finishes.");
    } else {
        drift = drift.hover(k.c.hover).focusable(true).on_press(move |app: &mut App| {
            app.screen = if app.screen == Screen::Drift {
                if app.run_finished { Screen::Finish } else { Screen::Catalog }
            } else {
                Screen::Drift
            };
        });
    }

    row()
        .key("titlebar")
        .window_drag()
        .w_full()
        .h(TITLEBAR)
        .pl(leading.max(12.0) + 12.0)
        .pr(12.0)
        .center_y()
        .fill(k.c.bg_base)
        .border(0.0, k.c.border_base)
        .child(
            row().gap(8.0).center_y().w(190.0)
                .child(el().size(22.0, 22.0).radius(6.0).center().linear_gradient(135.0, [(0.0, tones.accent), (1.0, tones.accent_deep)]).child(icon(&icons::SPARK, 14.0).color(tones.on_accent)))
                .child(text("Soulcaster").font(k.ty.body_medium).color(k.c.text_base)),
        )
        .child(el().grow(1.0))
        .child(stepper)
        .child(el().grow(1.0))
        .child(row().w(190.0).justify_end().child(drift))
}

fn prototype_strip(app: &App, k: &rune_kit::Kit, tones: Tones) -> El {
    let machine = [("Bare", Machine::Bare), ("Touch ID", Machine::BareTouchId), ("Homebrew", Machine::HasHomebrew)];
    let scenario = [("Good", Scenario::AllGood), ("Failure", Scenario::OneFailure), ("Surprise", Scenario::Surprise), ("Both", Scenario::Both)];
    let speed = [("1×", 1.0), ("4×", 4.0), ("16×", 16.0)];

    let mut machine_row = row().key("proto-machine").gap(2.0).center_y();
    for (i, (label, value)) in machine.iter().enumerate() {
        let value = *value;
        machine_row = machine_row.child(kit::mini_seg(k, tones, format!("proto-machine-{i}"), label, app.machine == value, !app.run_started, move |app: &mut App| {
            app.machine = value;
            app.password_state = crate::state::PasswordState::Empty;
            app.github = if value.github_signed_in() { crate::state::GithubState::Done } else if app.github == crate::state::GithubState::Done { crate::state::GithubState::Ready } else { app.github };
        }));
    }
    let mut scenario_row = row().key("proto-scenario").gap(2.0).center_y();
    for (i, (label, value)) in scenario.iter().enumerate() {
        let value = *value;
        scenario_row = scenario_row.child(kit::mini_seg(k, tones, format!("proto-scenario-{i}"), label, app.scenario == value, !app.run_started, move |app: &mut App| app.scenario = value));
    }
    let mut speed_row = row().key("proto-speed").gap(2.0).center_y();
    for (i, (label, value)) in speed.iter().enumerate() {
        let value = *value;
        speed_row = speed_row.child(kit::mini_seg(k, tones, format!("proto-speed-{i}"), label, app.speed == value, true, move |app: &mut App| app.speed = value));
    }
    let small = k.ty.text_xsmall;
    let lead = if app.run_started && app.run_items.iter().any(|i| !i.state.is_terminal()) { "Simulating" } else { "Prototype" };
    row()
        .key("prototype-strip")
        .w_full()
        .h(30.0)
        .px(12.0)
        .gap(14.0)
        .center_y()
        .fill(k.c.bg_deep)
        .child(
            row().gap(6.0).center_y()
                .child(el().size(6.0, 6.0).radius(3.0).fill(tones.warn))
                .child(text(format!("{lead}: nothing here touches your Mac")).font(small).color(k.c.text_muted)),
        )
        .child(el().grow(1.0))
        .child(text("Mac").font(small).color(k.c.text_faint))
        .child(machine_row)
        .child(text("Scenario").font(small).color(k.c.text_faint))
        .child(scenario_row)
        .child(text("Speed").font(small).color(k.c.text_faint))
        .child(speed_row)
}

/// How a state reads on screen. The lifecycle's names stay in code; people get plain words.
pub fn state_words(item: &crate::state::RunItem) -> String {
    match item.state {
        ItemState::Pending => "Waiting".to_string(),
        ItemState::Checking => "Checking".to_string(),
        ItemState::Acting => if item.is_setting { "Setting".to_string() } else { "Installing".to_string() },
        ItemState::Verifying => "Confirming".to_string(),
        ItemState::Satisfied if item.is_setting => "Set".to_string(),
        ItemState::Satisfied if item.acted => "Installed".to_string(),
        ItemState::Satisfied => "Already here".to_string(),
        ItemState::Unknown => "Couldn't check".to_string(),
        ItemState::Differs => "Differs".to_string(),
        ItemState::NeedsYou => "Needs you".to_string(),
        ItemState::Failed => "Failed".to_string(),
        ItemState::Blocked => match &item.blocked_by {
            Some(by) => format!("Waiting on {}", crate::catalog::catalog().name_of(by)),
            None => "Waiting on something else".to_string(),
        },
        ItemState::Skipped => "Skipped".to_string(),
    }
}
