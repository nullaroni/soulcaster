//! Run: the current Item, progress, every Item's state, and the log beside them.

use std::borrow::Cow;

use macflayer_core::lifecycle::{ItemState, Stage};
use rune::{El, GrowthSpec, ScrollSpec, col, el, row, text};
use rune_kit::TextInput;

use super::{kit, state_words};
use crate::catalog_view::view;
use crate::state::{AdminState, App};

/// One row of the Item list: a stage heading or an Item. Rebuilt from state each commit, which is
/// cheap at this size, because it's only indices.
#[derive(Clone, Copy)]
enum Row {
    Stage(Stage),
    Item(usize),
}

fn rows(app: &App) -> Vec<Row> {
    let mut out = Vec::new();
    for stage in [Stage::Admin, Stage::User] {
        let mut any = false;
        for (i, item) in app.run_items.iter().enumerate() {
            if item.stage != stage {
                continue;
            }
            if !any {
                out.push(Row::Stage(stage));
                any = true;
            }
            out.push(Row::Item(i));
        }
    }
    out
}

pub fn screen(app: &App) -> El {
    col()
        .key("run-screen")
        .w_full()
        .h_full()
        .child(header(app))
        .child(asks(app))
        .child(body(app))
}

/// What the Run is waiting on from the person. A real engine asks here, when it knows it needs to.
fn asks(app: &App) -> El {
    let mut strip = col().key("run-asks").w_full();
    if let Some(request) = app.engine.password_request() {
        let k = kit::theme();
        let input = TextInput::new(&app.run_password)
            .secure()
            .plain()
            .label("Your Mac password")
            .invalid(request.last_was_wrong)
            .on_edit(|app: &mut App| &mut app.run_password)
            .on_submit(|app: &mut App, _| app.submit_run_password())
            .build(k)
            .key("run-password-field");
        let field = col()
            .w(160.0)
            .child(input.h(18.0))
            .child(el().w_full().h(1.0).fill(if request.last_was_wrong {
                kit::mandarin()
            } else {
                kit::ink()
            }));
        let words = if request.last_was_wrong {
            "That password didn't work"
        } else {
            "Your Mac password, for the admin steps"
        };
        strip = strip.child(kit::ruled(
            "run-ask-password".to_string(),
            row()
                .gap(8.0)
                .child(kit::label(words).grow(1.0))
                .child(field)
                .child(kit::action(
                    "run-password-later".to_string(),
                    "Later",
                    false,
                    |app: &mut App, _| app.engine.decline_password(),
                )),
        ));
    }
    if let Some((code, address)) = app.engine.github_code() {
        strip = strip.child(kit::ruled(
            "run-ask-github".to_string(),
            row()
                .gap(8.0)
                .child(kit::label(format!("Sign in at {address}")).grow(1.0))
                .child(
                    text(code)
                        .key("github-code")
                        .font(kit::bold_style())
                        .color(kit::ink())
                        .selectable(true),
                ),
        ));
    }
    strip
}

fn header(app: &App) -> El {
    let total = app.run_items.len().max(1);
    let done = app
        .run_items
        .iter()
        .filter(|i| i.state.is_terminal())
        .count();
    let title = if app.run_finished {
        "Finished".to_string()
    } else {
        app.run_items
            .iter()
            .find(|i| i.state.is_working())
            .map_or("Starting".to_string(), |item| item.name.clone())
    };
    let fraction = done as f32 / total as f32;
    col()
        .key("run-header")
        .w_full()
        .child(kit::title_pad(
            row()
                .gap(10.0)
                .items_end()
                .child(kit::title(&title).key("run-title").shrink().truncate())
                .child(kit::label(format!("{done} of {total}")).key("run-progress-text")),
        ))
        .child(
            row()
                .key("run-progress")
                .role("progressbar")
                .label("Run progress")
                .mx(16.0)
                .mt(14.0)
                .h(2.0)
                .fill(kit::ink().alpha(0.15))
                .child(el().h(2.0).w_frac(fraction).fill(kit::forest())),
        )
}

fn body(app: &App) -> El {
    row()
        .key("run-body")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .mt(12.0)
        .child(item_list(app))
        .child(log_pane(app))
}

fn admin_words(app: &App) -> Option<&'static str> {
    match app.admin {
        AdminState::NotNeeded => None,
        AdminState::Held => Some("Password held"),
        AdminState::Discarded => Some("Password discarded"),
    }
}

fn stage_row(app: &App, stage: Stage) -> El {
    let name = if stage == Stage::Admin {
        "Admin Stage"
    } else {
        "User Stage"
    };
    let mut r = row()
        .w_full()
        .px(14.0)
        .pt(12.0)
        .pb(2.0)
        .child(kit::group(name))
        .child(el().grow(1.0));
    if stage == Stage::Admin
        && let Some(words) = admin_words(app)
    {
        r = r.child(
            kit::group(words)
                .key("password-state")
                .live_region(rune::LiveRegion::Polite),
        );
    }
    r
}

fn item_row(app: &App, i: usize) -> El {
    let item = &app.run_items[i];
    let id = item.id.clone();
    let words = state_words(item);
    let working = item.state.is_working();
    let failed = matches!(item.state, ItemState::Failed { .. });
    let (style, color) = if failed {
        (kit::bold_style(), kit::mandarin())
    } else if working {
        (kit::bold_style(), kit::forest())
    } else {
        (kit::ui_style(), kit::ink())
    };
    let name_style = if working {
        kit::bold_style()
    } else {
        kit::ui_style()
    };
    let name_color = if working { kit::forest() } else { kit::ink() };
    let mut content = row()
        .role("button")
        .label(format!("{}: {words}", item.name))
        .focusable(true)
        .focus_ring(1.5, -1.5, kit::forest())
        .hover(kit::ink().alpha(0.05))
        .on_press(move |app: &mut App| {
            let next = if app.log_item.as_deref() == Some(id.as_str()) {
                None
            } else {
                Some(id.clone())
            };
            app.set_log_item(next);
        })
        .child(
            text(item.name.clone())
                .font(name_style)
                .color(name_color)
                .grow(1.0)
                .shrink()
                .truncate(),
        );
    if app.log_item.as_deref() == Some(item.id.as_str()) {
        content = content.fill(kit::ink().alpha(0.05));
    }
    kit::ruled(
        format!("run-item-{}", item.id),
        content.child(text(words).font(style).color(color)),
    )
}

fn item_list(app: &App) -> El {
    let shown = rows(app);
    col()
        .key("run-items")
        .role("list")
        .label("Items in this Run")
        .w_frac(0.5)
        .h_full()
        .min_w(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("run-rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    shown.len(),
                    32.0,
                    |s: &App, i| {
                        Cow::Owned(match rows(s).get(i) {
                            Some(Row::Stage(Stage::Admin)) => "stage-admin".to_string(),
                            Some(Row::Stage(Stage::User)) => "stage-user".to_string(),
                            Some(Row::Item(x)) => format!("r-{}", s.run_items[*x].id),
                            None => format!("gone-{i}"),
                        })
                    },
                    |s: &App, i| match rows(s).get(i).copied() {
                        Some(Row::Stage(stage)) => stage_row(s, stage),
                        Some(Row::Item(x)) => item_row(s, x),
                        None => el(),
                    },
                )
                .virtual_rows_version(shown.len()),
        )
}

fn log_line(text_of: &str) -> El {
    let trouble = text_of.starts_with("Error") || text_of.starts_with("curl:");
    let lead = ["$ ", "==> "]
        .iter()
        .find(|lead| text_of.starts_with(**lead));
    let style = kit::log_style();
    match lead {
        Some(lead) => row()
            .w_full()
            .child(text(lead.to_string()).font(style).color(kit::dim()))
            .child(
                text(text_of[lead.len()..].to_string())
                    .font(style)
                    .color(kit::paper())
                    .selectable(true)
                    .shrink()
                    .truncate(),
            ),
        None => text(text_of.to_string())
            .font(style)
            .color(if trouble {
                kit::mandarin()
            } else {
                kit::paper()
            })
            .selectable(true)
            .truncate()
            .w_full(),
    }
}

fn log_pane(app: &App) -> El {
    let title = app
        .log_item
        .as_ref()
        .map_or("Everything".to_string(), |id| view().name_of(id));
    let lines = col()
        .key("log")
        .role("log")
        .label("Run log")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(14.0)
        .pb(16.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(
            col()
                .key("log-rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    app.log_view.len(),
                    17.0,
                    |s: &App, i| {
                        Cow::Owned(format!("l{}", s.log_view.get(i).copied().unwrap_or(i)))
                    },
                    |s: &App, i| {
                        let Some(line) = s
                            .log_view
                            .get(i)
                            .and_then(|&abs| s.log.get(abs.wrapping_sub(s.log_base)))
                        else {
                            return el();
                        };
                        log_line(&line.text)
                    },
                ),
        );
    col()
        .key("run-log")
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .h_full()
        .fill(kit::ink())
        .child(
            row().w_full().px(14.0).pt(12.0).pb(6.0).child(
                text(title)
                    .key("log-title")
                    .font(kit::bold_style())
                    .color(kit::dim())
                    .shrink()
                    .truncate(),
            ),
        )
        .child(lines)
}
