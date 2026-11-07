//! Drift: one row per difference between the Config and this Mac. Nothing changes until the footer
//! button, and every row starts on Later.

use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Dialog, Kit, TextInput};

use super::kit;
use crate::drift::{COULD_NOT_CHECK, Decision, DriftEntry, DriftKind, UNAVAILABLE, UNTRACKED};
use crate::state::{App, Timer};

const GROUPS: &[(DriftKind, &str)] = &[
    (DriftKind::ChangedHere, "On this Mac"),
    (DriftKind::ChangedInConfig, "In the Config"),
    (DriftKind::ChangedBoth, "Both"),
    (DriftKind::RemovedHere, "Missing here"),
    (DriftKind::DroppedFromConfig, "No longer in the Config"),
    (DriftKind::NewHere, "New here"),
];

const CONFIG_COLUMN: f32 = 74.0;
const MAC_COLUMN: f32 = 106.0;
const CHOICE_COLUMN: f32 = 190.0;

/// The choices a row offers, in order, with what each one does.
fn options(entry: &DriftEntry) -> Vec<(&'static str, Decision, String)> {
    let adopt = |help: &str| ("Adopt", Decision::Adopt, help.to_string());
    let later = ("Later", Decision::Later, "Leave it as it is".to_string());
    let apply_help = if entry.apply_needs_password {
        "Changes this Mac. This one needs your password."
    } else {
        "Changes this Mac to match the Config"
    };
    let apply = ("Apply", Decision::Apply, apply_help.to_string());
    match entry.kind {
        DriftKind::ChangedHere | DriftKind::ChangedInConfig | DriftKind::ChangedBoth => {
            vec![later, adopt("Saves this Mac's value to the Config"), apply]
        }
        DriftKind::RemovedHere => vec![
            later,
            adopt("Takes it out of the Config, for every Mac"),
            apply,
        ],
        DriftKind::DroppedFromConfig => vec![
            later,
            (
                "Untracked",
                Decision::KeepUntracked,
                "Leaves it installed, and stops watching it".to_string(),
            ),
            adopt("Adds it back to the Config"),
            (
                "Remove\u{2026}",
                Decision::Remove,
                "Uninstalls it from this Mac, after you confirm".to_string(),
            ),
        ],
        DriftKind::NewHere => vec![later, adopt("Adds it to the Config")],
    }
}

fn columns(name: El, config: El, mac: El, choice: El) -> El {
    row()
        .child(name.grow(1.15).shrink().min_w(0.0).truncate())
        .child(config.w(CONFIG_COLUMN).truncate())
        .child(mac.w(MAC_COLUMN).truncate())
        .child(row().w(CHOICE_COLUMN).justify_end().child(choice))
}

fn group_heading(words: &str) -> El {
    row()
        .w_full()
        .px(14.0)
        .pt(12.0)
        .pb(2.0)
        .child(kit::group(words.to_string()).role("heading").level(2))
}

pub fn screen(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    let mut body = col().key("drift-body").w_full().mt(10.0).child(kit::ruled(
        "drift-head".to_string(),
        columns(
            kit::label(""),
            kit::label("Config"),
            kit::label("This Mac"),
            kit::label(""),
        ),
    ));
    if app.drift.is_empty() {
        body = body.child(kit::ruled(
            "drift-empty".to_string(),
            row().child(kit::label("This Mac matches the Config")),
        ));
    }
    for (kind, title) in GROUPS {
        let entries: Vec<&DriftEntry> = app.drift.iter().filter(|d| d.kind == *kind).collect();
        if entries.is_empty() {
            continue;
        }
        body = body.child(group_heading(title));
        for entry in entries {
            body = body.child(entry_row(entry));
        }
    }
    // Apart from Drift, as the Config format requires: no Adopt or Apply here.
    body = body.child(group_heading("Unavailable here"));
    for u in UNAVAILABLE {
        body = body.child(kit::ruled(
            format!("unavailable-{}", u.name),
            row()
                .help(u.reason)
                .child(kit::label(u.name).grow(1.0).shrink().truncate())
                .child(kit::label(u.reason).shrink().truncate()),
        ));
    }
    body = body.child(group_heading("Untouched"));
    body = body.child(kit::ruled(
        "dotfiles".to_string(),
        row()
            .child(
                kit::label("your-github-username/dotfiles")
                    .grow(1.0)
                    .shrink()
                    .truncate(),
            )
            .child(kit::label("2 behind")),
    ));
    for name in COULD_NOT_CHECK {
        body = body.child(kit::ruled(
            format!("unknown-{name}"),
            row()
                .child(kit::label(*name).grow(1.0).shrink().truncate())
                .child(kit::label("Couldn't check")),
        ));
    }
    let shown = app.show_untracked;
    body = body.child(kit::ruled(
        "untracked".to_string(),
        row()
            .child(kit::label("Untracked").grow(1.0))
            .child(kit::action(
                "untracked-toggle".to_string(),
                if shown { "Hide" } else { "Show" },
                shown,
                |app: &mut App, _| app.show_untracked = !app.show_untracked,
            )),
    ));
    if shown {
        for name in UNTRACKED {
            body = body.child(kit::ruled(
                format!("untracked-{name}"),
                row().child(kit::label(*name)),
            ));
        }
    }

    dialogs(app, k, layer);

    col()
        .key("drift-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Drift")))
        .child(
            col()
                .key("drift-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(footer(app))
}

fn entry_row(entry: &DriftEntry) -> El {
    let mut choice = row()
        .key(format!("choose-{}", entry.key))
        .role("radiogroup")
        .label(format!("What to do about {}", entry.name))
        .gap(8.0);
    for (words, decision, help) in options(entry) {
        let key = entry.key.clone();
        let button = kit::action(
            format!("choose-{}-{words}", entry.key),
            words,
            entry.decision == decision,
            move |app: &mut App, _| {
                if decision == Decision::Remove {
                    app.confirm_remove = Some(key.clone());
                    app.remove_understood = false;
                } else if let Some(d) = app.drift.iter_mut().find(|d| d.key == key) {
                    d.decision = decision;
                }
            },
        );
        choice = choice.child(button.help(help));
    }
    kit::ruled(
        format!("entry-{}", entry.key),
        columns(
            kit::label(entry.name.clone()),
            kit::label(entry.in_config.clone()),
            kit::label(entry.on_mac.clone()),
            choice,
        ),
    )
}

fn dialogs(app: &App, k: &'static Kit, layer: &mut Vec<El>) {
    if let Some(key) = &app.confirm_remove {
        let name = app
            .drift
            .iter()
            .find(|d| d.key == *key)
            .map_or(String::new(), |d| d.name.clone());
        let understood = app.remove_understood;
        let target = key.clone();
        let content = col()
            .w_full()
            .px(16.0)
            .pb(16.0)
            .gap(12.0)
            .child(
                row()
                    .key("remove-understood")
                    .role("checkbox")
                    .label(format!("Yes, uninstall {name} from this Mac"))
                    .checked(understood)
                    .focusable(true)
                    .w_full()
                    .min_h(30.0)
                    .gap(10.0)
                    .center_y()
                    .on_press(|app: &mut App| app.remove_understood = !app.remove_understood)
                    .child(kit::check_box(k, understood, false))
                    .child(kit::label(format!("Yes, uninstall {name} from this Mac"))),
            )
            .child(
                row()
                    .w_full()
                    .justify_end()
                    .center_y()
                    .gap(12.0)
                    .child(kit::action(
                        "remove-cancel".to_string(),
                        "Keep it",
                        false,
                        |app: &mut App, _| app.confirm_remove = None,
                    ))
                    .child(kit::primary(
                        "remove-confirm",
                        &format!("Remove {name}"),
                        understood,
                        move |app: &mut App, _| {
                            if let Some(d) = app.drift.iter_mut().find(|d| d.key == target) {
                                d.decision = Decision::Remove;
                            }
                            app.confirm_remove = None;
                        },
                    )),
            );
        Dialog::new("remove", format!("Remove {name} from this Mac?"))
            .open(true)
            .fit()
            .width(420.0)
            .on_close(|app: &mut App| app.confirm_remove = None)
            .child(content)
            .build(k, layer);
    }
    if app.drift_password_open {
        let close = |app: &mut App| {
            app.drift_password.clear();
            app.drift_password_open = false;
        };
        let content = col()
            .w_full()
            .px(16.0)
            .pb(16.0)
            .gap(12.0)
            .child(
                TextInput::new(&app.drift_password)
                    .secure()
                    .label("Your Mac password")
                    .on_edit(|app: &mut App| &mut app.drift_password)
                    .on_submit(|app: &mut App, _| apply_with_password(app))
                    .build(k)
                    .key("drift-password"),
            )
            .child(
                row()
                    .w_full()
                    .justify_end()
                    .center_y()
                    .gap(12.0)
                    .child(kit::action(
                        "drift-password-cancel".to_string(),
                        "Cancel",
                        false,
                        move |app: &mut App, _| close(app),
                    ))
                    .child(kit::primary(
                        "drift-password-submit",
                        "Apply",
                        true,
                        |app: &mut App, _| apply_with_password(app),
                    )),
            );
        Dialog::new("drift-pw", "Your Mac password")
            .open(true)
            .fit()
            .width(380.0)
            .on_close(close)
            .child(content)
            .build(k, layer);
    }
}

fn apply_with_password(app: &mut App) {
    if app.drift_password.text.is_empty() {
        return;
    }
    app.drift_password.clear();
    app.drift_password_open = false;
    begin_apply(app);
}

fn begin_apply(app: &mut App) {
    app.drift_applying = true;
    app.after(1.4, Timer::DriftApplied);
}

fn footer(app: &App) -> El {
    let count = |f: &dyn Fn(Decision) -> bool| app.drift.iter().filter(|d| f(d.decision)).count();
    let to_config = count(&|d| d == Decision::Adopt);
    let to_mac = count(&|d| matches!(d, Decision::Apply | Decision::Remove));
    let stop = count(&|d| d == Decision::KeepUntracked);
    let needs_password = app.drift.iter().any(|d| {
        (d.decision == Decision::Apply && d.apply_needs_password) || d.decision == Decision::Remove
    });
    let words = if app.drift_applying {
        "Working\u{2026}"
    } else if to_config > 0 && to_mac > 0 {
        "Adopt and Apply"
    } else if to_config > 0 {
        "Adopt"
    } else if to_mac > 0 {
        "Apply"
    } else if stop > 0 {
        "Leave Untracked"
    } else {
        "Apply"
    };
    let enabled = to_config + to_mac + stop > 0 && !app.drift_applying;
    let button = kit::primary("drift-apply", words, enabled, move |app: &mut App, _| {
        if needs_password {
            app.drift_password_open = true;
            app.drift_password.focused = true;
        } else {
            begin_apply(app);
        }
    });
    kit::foot(
        "drift-footer",
        text(String::new()).font(kit::ui_style()),
        button,
    )
}
