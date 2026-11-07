//! Needs you: where a finished Run drops you. What only a person can finish, what failed, and what
//! is waiting on what failed.

use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Dialog, Kit, TextInput};
use soulcaster_core::event::{QueueEntry, QueueKind};
use soulcaster_core::lifecycle::ItemState;

use super::{kit, state_words};
use crate::catalog_view::view;
use crate::state::{App, RunItem, Screen};

pub fn screen(app: &App, k: &'static Kit, layer: &mut Vec<El>) -> El {
    let mut list = col().key("needs-you").w_full().mt(14.0);
    for entry in &app.queue {
        list = list.child(queue_row(entry));
    }
    for item in app
        .run_items
        .iter()
        .filter(|i| matches!(i.state, ItemState::Failed { .. }))
    {
        list = list.child(failed_row(item));
    }
    for item in app
        .run_items
        .iter()
        .filter(|i| matches!(i.state, ItemState::Blocked { .. }))
    {
        list = list.child(kit::ruled(
            format!("blocked-{}", item.id),
            row()
                .child(kit::label(item.name.clone()).grow(1.0).shrink().truncate())
                .child(kit::label(state_words(item))),
        ));
    }
    for item in app.run_items.iter().filter(|i| i.state.is_working()) {
        list = list.child(kit::ruled(
            format!("working-{}", item.id),
            row()
                .child(kit::label(item.name.clone()).grow(1.0).shrink().truncate())
                .child(kit::group(state_words(item))),
        ));
    }
    let nothing_left = app.queue.is_empty()
        && !app.run_items.iter().any(|i| {
            matches!(
                i.state,
                ItemState::Failed { .. } | ItemState::Blocked { .. }
            ) || i.state.is_working()
        });
    if nothing_left {
        let words = if app.run_started {
            "Nothing left"
        } else {
            "No Run yet"
        };
        let mut content = row().child(kit::label(words).grow(1.0));
        if app.run_started {
            content = content.child(kit::action(
                "open-drift".to_string(),
                "Drift",
                false,
                |app: &mut App, _| app.screen = Screen::Drift,
            ));
        }
        list = list.child(kit::ruled("nothing-left".to_string(), content));
    }

    if let Some(id) = &app.scoped_password_for {
        password_dialog(app, id, k, layer);
    }

    col()
        .key("finish-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Needs you").key("finish-title")))
        .child(
            col()
                .key("finish-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

/// The row's name, from what the queue entry asks of the person.
fn queue_title(entry: &QueueEntry) -> String {
    let name = view().name_of(&entry.item);
    let link = entry.link.as_deref().unwrap_or("");
    match entry.kind {
        QueueKind::NeedsAdmin => format!("Finish {name}"),
        QueueKind::License => format!("{name} license"),
        QueueKind::Tap | QueueKind::Conflict => name,
        QueueKind::Person if link.starts_with("macappstore:") => format!("Get {name}"),
        QueueKind::Person if link.starts_with("x-apple.systempreferences:") => {
            format!("Allow {name}")
        }
        QueueKind::Person => name,
    }
}

fn queue_action(entry: &QueueEntry) -> &'static str {
    let link = entry.link.as_deref().unwrap_or("");
    match entry.kind {
        QueueKind::NeedsAdmin => "Enter password",
        QueueKind::License => "Accept",
        QueueKind::Tap => "Allow",
        QueueKind::Conflict => "Choose",
        QueueKind::Person if link.starts_with("macappstore:") => "App Store",
        QueueKind::Person if link.starts_with("x-apple.systempreferences:") => "System Settings",
        QueueKind::Person => "Done",
    }
}

fn queue_row(entry: &QueueEntry) -> El {
    let id = entry.id.clone();
    kit::ruled(
        format!("queue-{}", entry.id),
        row()
            .help(entry.text.clone())
            .child(kit::label(queue_title(entry)).grow(1.0).shrink().truncate())
            .child(kit::action(
                format!("queue-do-{}", entry.id),
                queue_action(entry),
                false,
                move |app: &mut App, _| app.finish_queue_entry(&id),
            )),
    )
}

fn failed_row(item: &RunItem) -> El {
    let (retry, log) = (item.id.clone(), item.id.clone());
    kit::ruled(
        format!("failed-{}", item.id),
        row()
            .child(kit::label(item.name.clone()).grow(1.0).shrink().truncate())
            .child(
                row()
                    .gap(12.0)
                    .child(kit::action(
                        format!("log-{}", item.id),
                        "Log",
                        false,
                        move |app: &mut App, _| {
                            app.set_log_item(Some(log.clone()));
                            app.screen = Screen::Run;
                        },
                    ))
                    .child(kit::failed_action(
                        format!("retry-{}", item.id),
                        "Try again",
                        move |app: &mut App, _| app.start_scoped(&retry, None),
                    )),
            ),
    )
}

fn password_dialog(app: &App, id: &str, k: &'static Kit, layer: &mut Vec<El>) {
    let name = app
        .queue
        .iter()
        .find(|q| q.id == *id)
        .map_or(String::new(), |q| view().name_of(&q.item));
    let close = |app: &mut App| {
        app.scoped_password.clear();
        app.scoped_password_for = None;
    };
    let mut content = col().w_full().px(16.0).pb(16.0).gap(10.0).child(
        TextInput::new(&app.scoped_password)
            .secure()
            .label("Your Mac password")
            .invalid(app.scoped_wrong)
            .on_edit(|app: &mut App| &mut app.scoped_password)
            .on_submit(|app: &mut App, _| app.submit_scoped_password())
            .build(k)
            .key("scoped-password"),
    );
    if app.scoped_wrong {
        content = content.child(
            text("That password didn't work")
                .key("scoped-error")
                .font(kit::ui_style())
                .color(kit::mandarin()),
        );
    }
    content = content.child(
        row()
            .w_full()
            .justify_end()
            .center_y()
            .gap(12.0)
            .child(kit::action(
                "scoped-cancel".to_string(),
                "Not now",
                false,
                move |app: &mut App, _| close(app),
            ))
            .child(kit::primary(
                "scoped-submit",
                "Finish",
                true,
                |app: &mut App, _| app.submit_scoped_password(),
            )),
    );
    Dialog::new("scoped", format!("Password for {name}"))
        .open(true)
        .fit()
        .width(380.0)
        .on_close(close)
        .child(content)
        .build(k, layer);
}
