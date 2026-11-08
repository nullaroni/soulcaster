//! Review: everything the selection needs from a person, once, then walk away.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Dialog, Kit, TextInput};
use soulcaster_core::plan::{NeedKind, Plan, PlanConflict};

use super::kit;
use crate::catalog_view::view;
use crate::state::{App, GithubState, PasswordState, Screen};

pub fn screen(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    let plan = app.plan();
    let mut asks = col().key("asks").w_full().mt(14.0);
    let mut left = 0;
    for conflict in &plan.conflicts {
        left += 1;
        asks = asks.child(conflict_row(conflict));
    }
    if plan.needs_admin() && !app.engine.is_real() {
        left += usize::from(app.password_state != PasswordState::Accepted);
        asks = asks.child(password_row(app, k));
    }
    if app.needs_github() {
        left += usize::from(!matches!(
            app.github,
            GithubState::Done | GithubState::Skipped
        ));
        asks = asks.child(github_row(app));
    }
    for entry in plan
        .entries
        .iter()
        .filter(|e| e.needs.iter().any(|n| n.kind == NeedKind::License))
    {
        let on = app.answers.licenses.contains(&entry.id);
        left += usize::from(!on);
        let id = entry.id.clone();
        let words = if on { "Accepted" } else { "Accept" };
        let help = entry
            .needs
            .iter()
            .find(|n| n.kind == NeedKind::License)
            .map_or(String::new(), |n| n.text.clone());
        asks = asks.child(kit::ruled(
            format!("license-{}", entry.id),
            row()
                .help(help)
                .child(
                    kit::label(format!("{} license", entry.name))
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .child(kit::action(
                    format!("license-do-{}", entry.id),
                    words,
                    on,
                    move |app: &mut App, _| {
                        if !app.answers.licenses.remove(&id) {
                            app.answers.licenses.insert(id.clone());
                        }
                    },
                )),
        ));
    }
    for (tap, items) in &plan.taps {
        let on = app.answers.taps.contains(tap);
        left += usize::from(!on);
        let t = tap.clone();
        let names: Vec<String> = items.iter().map(|id| view().name_of(id)).collect();
        asks = asks.child(kit::ruled(
            format!("tap-{}", tap.replace('/', "-")),
            row()
                .help(format!("For {}", names.join(", ")))
                .child(kit::label(tap.clone()).grow(1.0).shrink().truncate())
                .child(kit::action(
                    format!("tap-do-{}", tap.replace('/', "-")),
                    if on { "Allowed" } else { "Allow" },
                    on,
                    move |app: &mut App, _| {
                        if !app.answers.taps.remove(&t) {
                            app.answers.taps.insert(t.clone());
                        }
                    },
                )),
        ));
    }

    if app.show_config {
        Dialog::new("config", format!("Your Config: {}", app.config_name))
            .open(true)
            .width(560.0)
            .on_close(|app: &mut App| app.show_config = false)
            .child(
                col()
                    .key("config-json-scroll")
                    .w_full()
                    .h(290.0)
                    .px(16.0)
                    .pb(16.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(
                        text(app.config().to_canonical_json())
                            .key("config-json")
                            .font(kit::log_style())
                            .color(kit::ink())
                            .selectable(true),
                    ),
            )
            .build(k, layer);
    }

    col()
        .key("review-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Review")))
        .child(
            col()
                .key("review-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(asks),
        )
        .child(footer(app, &plan, left))
}

fn name_of(id: &str) -> String {
    view().name_of(id)
}

fn conflict_row(conflict: &PlanConflict) -> El {
    let (a, b) = (name_of(&conflict.item), name_of(&conflict.other));
    let pick = |keep: &str, drop: &str| {
        let drop = drop.to_string();
        let words = name_of(keep);
        kit::action(
            format!("keep-{keep}"),
            &words,
            false,
            move |app: &mut App, _| app.drop_entry(&drop),
        )
    };
    let mut content = row()
        .child(
            kit::label(format!("{a} or {b}"))
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .child(
            row()
                .gap(8.0)
                .child(pick(&conflict.item, &conflict.other))
                .child(pick(&conflict.other, &conflict.item)),
        );
    if let Some(reason) = &conflict.reason {
        content = content.help(reason.clone());
    }
    kit::ruled(format!("ask-conflict-{}", conflict.item), content)
}

fn password_row(app: &App, k: &Kit) -> El {
    let control = match app.password_state {
        PasswordState::Accepted => kit::group("Held"),
        PasswordState::Checking => kit::label("Checking"),
        state => {
            let input = TextInput::new(&app.password)
                .secure()
                .plain()
                .label("Your Mac password")
                .invalid(state == PasswordState::Wrong)
                .on_edit(|app: &mut App| &mut app.password)
                .on_submit(|app: &mut App, _| app.check_password())
                .build(k)
                .key("password-field");
            let mut field = col()
                .w(120.0)
                .child(input.h(18.0))
                .child(el().w_full().h(1.0).fill(if state == PasswordState::Wrong {
                    kit::mandarin()
                } else {
                    kit::ink()
                }));
            if state == PasswordState::Wrong {
                field = field.help("That password didn't work.");
            }
            field
        }
    };
    kit::ruled(
        "ask-password".to_string(),
        row().child(kit::label("Password").grow(1.0)).child(control),
    )
}

fn github_row(app: &App) -> El {
    let control = match &app.github {
        GithubState::Ready => row()
            .gap(8.0)
            .child(kit::action(
                "github-open".to_string(),
                "Sign in",
                false,
                |app: &mut App, _| app.start_github(),
            ))
            .child(kit::action(
                "github-skip".to_string(),
                "Later",
                false,
                |app: &mut App, _| app.github = GithubState::Skipped,
            )),
        GithubState::Waiting { code } => row()
            .gap(8.0)
            .child(
                text(code.clone())
                    .key("github-code")
                    .font(kit::bold_style())
                    .color(kit::ink())
                    .selectable(true),
            )
            .child(kit::label("Waiting")),
        GithubState::Done => kit::group("Signed in"),
        GithubState::Skipped => kit::action(
            "github-undo".to_string(),
            "Later",
            true,
            |app: &mut App, _| app.github = GithubState::Ready,
        ),
    };
    kit::ruled(
        "ask-github".to_string(),
        row().child(kit::label("GitHub").grow(1.0)).child(control),
    )
}

fn footer(app: &App, plan: &Plan, left: usize) -> El {
    let ready = app.ready_to_start(plan);
    let words = if left == 0 {
        "Nothing left".to_string()
    } else {
        format!("{left} left")
    };
    let status = row()
        .gap(16.0)
        .center_y()
        .child(
            text(words)
                .key("review-status")
                .font(kit::ui_style())
                .color(kit::ink())
                .live_region(rune::LiveRegion::Polite),
        )
        .child(kit::action(
            "view-config".to_string(),
            "Config",
            false,
            |app: &mut App, _| app.show_config = true,
        ))
        .child(kit::action(
            "review-back".to_string(),
            "Back",
            false,
            |app: &mut App, _| app.screen = Screen::Catalog,
        ));
    kit::foot(
        "review-footer",
        status,
        kit::primary("start-run", "Start", ready, |app: &mut App, _| {
            app.start_run()
        }),
    )
}
