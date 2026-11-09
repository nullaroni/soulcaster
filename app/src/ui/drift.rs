//! Drift: one row per difference between the Config and this Mac. Nothing changes until the footer
//! button, and every row starts on Later.

use macflayer_core::drift::{Choice, RowKind, confirm_remove, held_sentence};
use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Dialog, Kit};

use super::kit;
use crate::drift::{DriftEntry, UNTRACKED, button_word, cell, choice_help};
use crate::drift_flow::{Status, missing, unreadable};
use crate::state::App;

const GROUPS: &[(RowKind, &str)] = &[
    (RowKind::ChangedHere, "On this Mac"),
    (RowKind::ChangedInConfig, "In the Machine Type"),
    (RowKind::Conflict, "Both"),
    (RowKind::RemovedHere, "Missing here"),
    (RowKind::NewInConfig, "New in the Machine Type"),
    (RowKind::Dropped, "No longer in the Machine Type"),
    (RowKind::Extra, "New here"),
];

const CONFIG_COLUMN: f32 = 74.0;
const MAC_COLUMN: f32 = 106.0;
const CHOICE_COLUMN: f32 = 190.0;

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
            kit::label("Machine Type"),
            kit::label("This Mac"),
            kit::label(""),
        ),
    ));
    for (at, note) in app.drift_notes.iter().enumerate() {
        body = body.child(kit::ruled(
            format!("drift-note-{at}"),
            row().child(kit::label(note.clone()).shrink().truncate()),
        ));
    }
    if app.drift.is_empty() && !app.drift_applying {
        let words = if app.drift_status == Some(Status::NoConfig) {
            "Run or save a Machine Type, and this Mac can be compared with it"
        } else {
            "Up to date with your Machine Type."
        };
        body = body.child(kit::ruled(
            "drift-empty".to_string(),
            row().child(kit::label(words)),
        ));
    }
    for (kind, title) in GROUPS {
        let entries: Vec<&DriftEntry> = app.drift.iter().filter(|d| d.row.kind == *kind).collect();
        if entries.is_empty() {
            continue;
        }
        body = body.child(group_heading(title));
        for entry in entries {
            body = body.child(entry_row(entry));
        }
    }
    if app.drift_waiting > 0 {
        body = body.child(group_heading("Waiting to send"));
        body = body.child(kit::ruled(
            "drift-waiting".to_string(),
            row()
                .gap(8.0)
                .help("Saved on this Mac, but not on GitHub yet. Your other Macs can't see this until it's sent.")
                .child(
                    kit::label("Saved on this Mac, but not on GitHub yet.")
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .child(kit::action(
                    "drift-discard".to_string(),
                    "Discard, and keep this Mac as it is",
                    false,
                    |app: &mut App, _| {
                        app.drift_applying = true;
                        app.engine.discard_waiting_saves();
                    },
                ))
                .child(kit::action(
                    "drift-send".to_string(),
                    "Send to GitHub",
                    false,
                    |app: &mut App, _| {
                        app.drift_applying = true;
                        app.engine.send_waiting_saves();
                    },
                )),
        ));
    }
    if !app.drift_held.is_empty() {
        body = body.child(group_heading("Needs an update"));
        for id in &app.drift_held {
            body = body.child(kit::ruled(
                format!("held-{id}"),
                row().child(kit::label(held_sentence(id)).shrink().truncate()),
            ));
        }
    }
    // Apart from Drift, as the Config format requires: no Adopt or Apply here.
    if !app.drift_unavailable.is_empty() {
        body = body.child(group_heading("Unavailable here"));
    }
    for (name, reason) in &app.drift_unavailable {
        body = body.child(kit::ruled(
            format!("unavailable-{name}"),
            row()
                .help(reason.clone())
                .child(kit::label(name.clone()).grow(1.0).shrink().truncate())
                .child(kit::label(reason.clone()).shrink().truncate()),
        ));
    }
    if !app.drift_unknown.is_empty() {
        body = body.child(group_heading("Untouched"));
    }
    for name in &app.drift_unknown {
        body = body.child(kit::ruled(
            format!("unknown-{name}"),
            row()
                .child(kit::label(name.clone()).grow(1.0).shrink().truncate())
                .child(kit::label("Couldn't check")),
        ));
    }
    // Sample only: finding apps outside the Catalog isn't built yet.
    if !app.engine.is_real() {
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
    let row_id = &entry.row.id;
    let mut choices = row()
        .key(format!("choose-{row_id}"))
        .role("radiogroup")
        .label(format!("What to do about {}", entry.row.name))
        .gap(8.0);
    let later = {
        let key = row_id.clone();
        kit::action(
            format!("choose-{row_id}-later"),
            "Later",
            entry.choice.is_none(),
            move |app: &mut App, _| {
                if let Some(d) = app.drift.iter_mut().find(|d| d.row.id == key) {
                    d.choice = None;
                }
            },
        )
        .help("Leave it as it is")
    };
    choices = choices.child(later);
    for choice in entry.row.choices() {
        let key = row_id.clone();
        let button = kit::action(
            format!("choose-{row_id}-{choice:?}"),
            button_word(choice),
            entry.choice == Some(choice),
            move |app: &mut App, _| {
                if choice == Choice::Remove {
                    app.confirm_remove = Some(key.clone());
                    app.remove_understood = false;
                } else if let Some(d) = app.drift.iter_mut().find(|d| d.row.id == key) {
                    d.choice = Some(choice);
                }
            },
        );
        choices = choices.child(button.help(choice_help(&entry.row, choice)));
    }
    kit::ruled(
        format!("entry-{row_id}"),
        columns(
            kit::label(entry.row.name.clone()).help(entry.row.sentence()),
            kit::label(cell(&entry.row, entry.row.in_config.as_ref(), true)),
            kit::label(cell(&entry.row, entry.row.on_mac.as_ref(), false)),
            choices,
        ),
    )
}

fn dialogs(app: &App, k: &'static Kit, layer: &mut Vec<El>) {
    if let Some(key) = &app.confirm_remove {
        let name = app
            .drift
            .iter()
            .find(|d| d.row.id == *key)
            .map_or(String::new(), |d| d.row.name.clone());
        let understood = app.remove_understood;
        let target = key.clone();
        let content = col()
            .w_full()
            .px(16.0)
            .pb(16.0)
            .gap(12.0)
            .child(kit::label(confirm_remove(&name)))
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
                            if let Some(d) = app.drift.iter_mut().find(|d| d.row.id == target) {
                                d.choice = Some(Choice::Remove);
                            }
                            app.confirm_remove = None;
                        },
                    )),
            );
        Dialog::new("remove", format!("Remove {name}"))
            .open(true)
            .fit()
            .width(420.0)
            .on_close(|app: &mut App| app.confirm_remove = None)
            .child(content)
            .build(k, layer);
    }
}

/// The one line that says where this comparison stands.
fn status_words(app: &App) -> String {
    let rows = app.drift.len();
    match &app.drift_status {
        _ if app.drift_applying => "Working\u{2026}".to_string(),
        Some(Status::Unreadable) => unreadable(),
        Some(Status::Missing { path }) => missing(path),
        Some(Status::Offline { .. }) => {
            "Can't reach GitHub right now. Showing your Machine Type as of the last time it was read."
                .to_string()
        }
        Some(Status::NoConfig) => String::new(),
        _ if rows == 0 => "Up to date with your Machine Type.".to_string(),
        _ => format!("{rows} differences to review."),
    }
}

fn footer(app: &App) -> El {
    let count = |matches: &dyn Fn(Choice) -> bool| {
        app.drift
            .iter()
            .filter(|d| d.choice.is_some_and(matches))
            .count()
    };
    let to_config = count(&|c| matches!(c, Choice::Adopt | Choice::AddBack));
    let to_mac = count(&|c| matches!(c, Choice::Apply | Choice::Remove));
    let stop = count(&|c| matches!(c, Choice::Keep | Choice::KeepHere | Choice::NotHere));
    let words = if app.drift_applying {
        "Working\u{2026}"
    } else if to_config > 0 && to_mac > 0 {
        "Save and Apply"
    } else if to_config > 0 {
        "Save"
    } else if to_mac > 0 {
        "Apply"
    } else if stop > 0 {
        "Done"
    } else {
        "Apply"
    };
    let blocked = app
        .drift_status
        .as_ref()
        .is_some_and(Status::blocks_changes);
    let enabled = to_config + to_mac + stop > 0 && !app.drift_applying && !blocked;
    let button = kit::primary("drift-apply", words, enabled, |app: &mut App, _| {
        app.carry_out_drift();
    });
    let again = kit::action(
        "drift-check".to_string(),
        "Check again",
        false,
        |app: &mut App, _| app.check_drift(),
    );
    let mut left = row().gap(12.0).center_y();
    if app.engine.is_real() {
        left = left.child(again);
    }
    kit::foot(
        "drift-footer",
        left.child(
            text(status_words(app))
                .font(kit::ui_style())
                .color(kit::dim()),
        ),
        button,
    )
}
