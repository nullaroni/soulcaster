//! The window: title bar with the path a person walks, the current screen, and the strip that says
//! the engine is simulated.

mod catalog;
mod dotfiles;
mod drift;
mod finish;
mod import;
pub mod kit;
mod review;
mod run;
mod save;

use rune::{Cx, El, col, el, icon, row, text};
use rune_kit::Toast;
use soulcaster_core::lifecycle::ItemState;

use crate::catalog_view::view as catalog_view;
use crate::state::{App, RunItem, Screen};

pub const TITLEBAR: f32 = 44.0;

pub fn view(app: &App, cx: &mut Cx<App>) -> El {
    let k = kit::theme();
    let mut layer: Vec<El> = Vec::new();
    let inset = cx.chrome_insets();
    let body = match app.screen {
        Screen::Catalog => catalog::screen(app, k, &mut layer),
        Screen::Review => review::screen(app, k, &mut layer),
        Screen::Run => run::screen(app),
        Screen::Finish => finish::screen(app, k, &mut layer),
        Screen::Drift => drift::screen(app, k, &mut layer),
        Screen::Import => import::screen(app, k),
        Screen::Save => save::screen(app),
        Screen::Dotfiles => dotfiles::screen(app),
    };
    if let Some(message) = &app.toast {
        layer.push(
            el().key("toast-layer")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .justify_end()
                .items_center()
                .pb(64.0)
                .child(Toast::new("toast", message.clone()).build(k)),
        );
    }
    let base = col()
        .key("window")
        .w_full()
        .h_full()
        .fill(kit::paper())
        .child(titlebar(app, inset.leading))
        .child(
            col()
                .key("screen")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(body),
        )
        .child(simulated_strip());
    rune_kit::root(base, layer)
}

/// Whether a step of the path can be walked to right now. Going back is free until a Run starts.
/// After that, Catalog and Review are history.
fn can_walk(app: &App, step: usize) -> bool {
    match step {
        0 | 1 => !app.run_started,
        2 => app.run_started,
        _ => app.run_finished,
    }
}

fn titlebar(app: &App, leading: f32) -> El {
    let steps = ["Catalog", "Review", "Run", "Needs you"];
    let current = match app.screen {
        Screen::Catalog => Some(0),
        Screen::Review => Some(1),
        Screen::Run => Some(2),
        Screen::Finish => Some(3),
        Screen::Drift | Screen::Import | Screen::Save | Screen::Dotfiles => None,
    };
    let mut path = row()
        .key("path")
        .role("navigation")
        .label("Path")
        .center_y()
        .gap(12.0);
    for (i, step) in steps.iter().enumerate() {
        let on = current == Some(i);
        path = path.child(if !on && can_walk(app, i) {
            kit::action(format!("step-{i}"), step, false, move |app: &mut App, _| {
                app.screen = match i {
                    0 => Screen::Catalog,
                    1 => Screen::Review,
                    2 => Screen::Run,
                    _ => Screen::Finish,
                };
            })
        } else {
            let style = if on {
                kit::bold_style()
            } else {
                kit::ui_style()
            };
            row()
                .key(format!("step-{i}"))
                .px(2.0)
                .child(
                    text(*step)
                        .font(style)
                        .color(if on { kit::forest() } else { kit::ink() }),
                )
        });
    }
    let on_drift = app.screen == Screen::Drift;
    let running = app.run_started && !app.run_finished;
    let drift = if running {
        row()
            .key("nav-drift")
            .px(2.0)
            .help("Only one thing can change this Mac at a time.")
            .child(text("Drift").font(kit::ui_style()).color(kit::dim()))
    } else {
        kit::action(
            "nav-drift".to_string(),
            "Drift",
            on_drift,
            |app: &mut App, _| {
                app.screen = if app.screen != Screen::Drift {
                    Screen::Drift
                } else if app.run_finished {
                    Screen::Finish
                } else {
                    Screen::Catalog
                };
            },
        )
    };
    col()
        .key("titlebar")
        .w_full()
        .child(
            row()
                .window_drag()
                .w_full()
                .h(TITLEBAR - 1.0)
                .pl(leading.max(0.0) + 14.0)
                .pr(14.0)
                .gap(14.0)
                .center_y()
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .min_w(132.0)
                        .child(icon(&kit::MARK, 16.0).color(kit::forest()))
                        .child(text("Soulcaster").font(kit::bold_style()).color(kit::ink())),
                )
                .child(path)
                .child(el().grow(1.0))
                .child(nav_action(app, Screen::Import, "Import", running))
                .child(nav_action(app, Screen::Save, "Save", false))
                .child(nav_action(app, Screen::Dotfiles, "Dotfiles", running))
                .child(drift),
        )
        .child(kit::hairline())
}

/// A place beside the path. Saving is always here, because it never gets in a Run's way. Import and
/// the Dotfiles Repo can change what a Run does, so they wait while one is going.
fn nav_action(app: &App, to: Screen, words: &str, unavailable: bool) -> El {
    let key = format!("nav-{}", words.to_lowercase());
    if unavailable {
        return row()
            .key(key)
            .px(2.0)
            .help("Only one thing can change this Mac at a time.")
            .child(text(words.to_string()).font(kit::ui_style()).color(kit::dim()));
    }
    kit::action(key, words, app.screen == to, move |app: &mut App, _| {
        app.screen = if app.screen != to {
            to
        } else if app.run_finished {
            Screen::Finish
        } else {
            Screen::Catalog
        };
    })
}

fn simulated_strip() -> El {
    col()
        .key("simulated-strip")
        .w_full()
        .child(kit::hairline())
        .child(
            row().w_full().h(21.0).px(14.0).center_y().child(
                text("Simulated. Nothing here touches your Mac.")
                    .font(kit::ui_style())
                    .color(kit::dim()),
            ),
        )
}

/// How a state reads on screen. The lifecycle's names stay in code; people get plain words.
pub fn state_words(item: &RunItem) -> String {
    match &item.state {
        ItemState::Pending => "Waiting".to_string(),
        ItemState::Checking => "Checking".to_string(),
        ItemState::Acting if item.is_setting => "Setting".to_string(),
        ItemState::Acting => "Installing".to_string(),
        ItemState::Verifying => "Confirming".to_string(),
        ItemState::Satisfied { .. } if item.is_setting => "Set".to_string(),
        ItemState::Satisfied { .. } if item.acted => "Installed".to_string(),
        ItemState::Satisfied { .. } => "Already here".to_string(),
        ItemState::Unknown => "Couldn't check".to_string(),
        ItemState::Differs => "Differs".to_string(),
        ItemState::NeedsYou { .. } => "Needs you".to_string(),
        ItemState::Failed { .. } => "Failed".to_string(),
        ItemState::Blocked { by } => format!("Waiting on {}", catalog_view().name_of(by)),
        ItemState::Skipped { .. } => "Skipped".to_string(),
    }
}
