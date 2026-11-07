//! Finish: what's left after a Run, in one place, and what comes up later.

use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, Spinner, TextInput};

use super::kit::{self, Tones, icons};
use crate::catalog::catalog;
use crate::lifecycle::{ItemState, QueueEntry, QueueKind};
use crate::state::{App, RunItem, Screen};

pub fn screen(app: &App, k: &'static Kit, tones: Tones, layer: &mut Vec<El>) -> El {
    let count = |f: &dyn Fn(&RunItem) -> bool| app.run_items.iter().filter(|i| f(i)).count();
    let ready = count(&|i| i.state == ItemState::Satisfied);
    let failed: Vec<&RunItem> = app.run_items.iter().filter(|i| i.state == ItemState::Failed).collect();
    let blocked = count(&|i| i.state == ItemState::Blocked);
    let working = count(&|i| i.state.is_working());
    let left = app.queue.len() + failed.len() + blocked;

    let (glyph, color, title, sub) = if !app.run_started {
        (&icons::INFO, k.c.icon_muted, "No Run yet".to_string(), "Pick Items in the Catalog, then start a Run.".to_string())
    } else if left == 0 {
        (&icons::CHECK_CIRCLE, tones.good, "Your Mac is ready".to_string(), format!("All {ready} are done. This Mac now follows \u{201c}{}\u{201d}.", app.config_name))
    } else {
        (&icons::PERSON, tones.warn, format!("Almost done: {left} {} left for you", if left == 1 { "thing" } else { "things" }), format!("{ready} are ready. These stay here until you finish them, even if you quit and come back."))
    };

    let mut body = col().key("finish-body").w_full().max_w(760.0).self_center().px(24.0).pt(16.0).pb(20.0).gap(12.0)
        .child(
            row().w_full().gap(12.0).center_y()
                .child(icon(glyph, 34.0).color(color))
                .child(col().grow(1.0).shrink().gap(2.0).child(text(title).key("finish-title").font(rune::TextStyle::new(22.0, 600.0)).color(k.c.text_strong).role("heading").level(1).wrap().w_full()).child(text(sub).font(k.ty.text_small).color(k.c.text_muted).wrap().w_full())),
        )
        .child(tally(app, ready, k, tones));

    if working > 0 {
        body = body.child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(text("Working on it. This only touches what you just finished.").font(k.ty.text_small).color(k.c.text_muted)));
    }
    if !app.queue.is_empty() {
        let mut list = col().w_full().gap(0.0);
        for (i, entry) in app.queue.iter().enumerate() {
            list = list.child(queue_row(entry, i > 0, k, tones));
        }
        body = body.child(section(k, "Needs-You Queue", "Only a person can do these. They stay here until you finish them, even if you quit.", kit::card(k).key("queue").child(list)));
    }
    if !failed.is_empty() || blocked > 0 {
        let mut list = col().w_full();
        for (i, item) in failed.iter().enumerate() {
            list = list.child(failed_row(app, item, i > 0, k, tones));
        }
        for item in app.run_items.iter().filter(|i| i.state == ItemState::Blocked) {
            list = list.child(blocked_row(item, k, tones));
        }
        body = body.child(section(k, "These didn't work", "Everything else still went ahead.", kit::card(k).key("failed").child(list)));
    }
    if left == 0 && app.run_started {
        body = body.child(
            kit::card(k).key("all-done").padding(14.0).gap(8.0)
                .child(text("Soulcaster's job here is done").font(k.ty.text_small_medium).color(k.c.text_strong))
                .child(text("If this Machine and its Config disagree, that shows up as Drift. Nothing changes until you Adopt or Apply.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap())
                .child(row().child(Button::new("Open Drift").variant(ButtonVariant::Neutral).on_press(|app: &mut App| app.screen = Screen::Drift).build(k).key("open-drift"))),
        );
    }
    let first_open = app.plan().first_open;
    if !first_open.is_empty() && app.run_started {
        let mut list = col().w_full().padding(12.0).gap(9.0);
        for (name, detail) in &first_open {
            list = list.child(
                row().w_full().gap(9.0).items_start()
                    .child(kit::tile(k, name, 22.0))
                    .child(col().grow(1.0).shrink().gap(1.0).child(text(name.clone()).font(k.ty.text_small_medium).color(k.c.text_base)).child(text(detail.clone()).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).w_full().wrap())),
            );
        }
        body = body.child(section(k, "When you first open them", "Nothing to do now. These apps will ask for themselves.", kit::card(k).key("first-open").child(list)));
    }
    if app.settings.keys().any(|id| id.starts_with("keyboard.") || id.starts_with("trackpad.")) && app.run_started {
        body = body.child(
            kit::card(k).key("logout-note").padding(12.0)
                .child(row().w_full().gap(9.0).center_y()
                    .child(icon(&icons::INFO, 15.0).color(k.c.icon_muted))
                    .child(text("Some keyboard and trackpad settings only take effect after you log out and back in.").font(k.ty.text_xsmall).color(k.c.text_muted).grow(1.0).shrink().wrap())
                    .child(Button::new("Log out later").variant(ButtonVariant::GhostMuted).build(k))),
        );
    }

    if let Some(id) = &app.scoped_password_for {
        let name = app.queue.iter().find(|q| q.id == *id).map_or(String::new(), |q| catalog().name_of(&q.item));
        let mut content = col().w_full().px(16.0).pb(16.0).gap(10.0)
            .child(
                TextInput::new(&app.scoped_password).secure().label("Your Mac password").placeholder("Password").leading_icon(k.glyph.lock).invalid(app.scoped_wrong)
                    .on_edit(|app: &mut App| &mut app.scoped_password).on_submit(|app: &mut App, _| app.submit_scoped_password()).build(k).key("scoped-password"),
            );
        if app.scoped_wrong {
            content = content.child(text("That password didn't work. Nothing changed.").key("scoped-error").font(k.ty.text_small).color(tones.bad));
        }
        content = content.child(row().w_full().justify_end().gap(8.0)
            .child(Button::new("Not now").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| { app.scoped_password.clear(); app.scoped_password_for = None; }).build(k).key("scoped-cancel"))
            .child(kit::primary(k, tones, "scoped-submit", "Finish", true, |app: &mut App, _| app.submit_scoped_password())));
        Dialog::new("scoped", format!("Password for {name}"))
            .description("Used for this one step, then discarded. It isn't saved anywhere.")
            .open(true).fit().width(420.0)
            .on_close(|app: &mut App| { app.scoped_password.clear(); app.scoped_password_for = None; })
            .child(content)
            .build(k, layer);
    }

    col().key("finish-screen").w_full().h_full().fill(k.c.bg_base).scroll(ScrollSpec { stick_bottom: false }).child(body)
}

fn tally(app: &App, ready: usize, k: &Kit, tones: Tones) -> El {
    let installed = app.run_items.iter().filter(|i| i.state == ItemState::Satisfied && i.acted && !i.is_setting).count();
    let here = app.run_items.iter().filter(|i| i.state == ItemState::Satisfied && !i.acted).count();
    let set = app.run_items.iter().filter(|i| i.state == ItemState::Satisfied && i.is_setting).count();
    let _ = ready;
    let chip = |value: usize, label: &str, color: rune::Color| {
        col().grow(1.0).shrink().min_w(0.0).padding(10.0).gap(1.0).radius(8.0).fill(k.c.layer_01).border(1.0, k.c.border_weak)
            .child(text(value.to_string()).font(rune::TextStyle::new(18.0, 600.0)).color(color))
            .child(text(label.to_string()).font(k.ty.text_xsmall).color(k.c.text_muted).truncate())
    };
    let failed = app.run_items.iter().filter(|i| i.state == ItemState::Failed).count();
    let mut r = row().key("tally").w_full().gap(8.0).child(chip(installed, "installed", k.c.text_strong)).child(chip(set, "settings set", k.c.text_strong));
    if here > 0 {
        r = r.child(chip(here, "already here", k.c.text_strong));
    }
    r.child(chip(app.queue.len(), "need you", if app.queue.is_empty() { k.c.text_strong } else { tones.warn_ink })).child(chip(failed, "failed", if failed == 0 { k.c.text_strong } else { tones.bad }))
}

fn section(k: &Kit, title: &str, sub: &str, body: El) -> El {
    col().w_full().gap(7.0)
        .child(col().gap(1.0).child(text(title.to_string()).font(k.ty.body_medium).color(k.c.text_strong).role("heading").level(2)).child(text(sub.to_string()).font(k.ty.text_xsmall).color(k.c.text_muted)))
        .child(body)
}

fn queue_row(entry: &QueueEntry, divider: bool, k: &Kit, tones: Tones) -> El {
    let id = entry.id.clone();
    let glyph = match entry.kind {
        QueueKind::Person if entry.needs_password => &icons::APPLE,
        QueueKind::Person => &icons::PERSON,
        QueueKind::License => &icons::DOC,
        QueueKind::Tap => &icons::SHIELD,
        QueueKind::NeedsAdmin => &icons::KEY,
    };
    let mut r = row().key(format!("queue-{}", entry.id)).w_full().px(12.0).py(8.0).gap(11.0).center_y()
        .child(el().size(30.0, 30.0).radius(8.0).center().fill(tones.warn_soft).child(icon(glyph, 15.0).color(tones.warn_ink)))
        .child(col().grow(1.0).shrink().min_w(0.0).gap(2.0)
            .child(text(entry.title.clone()).font(k.ty.text_small_medium).color(k.c.text_base).wrap().w_full())
            .child(text(entry.detail.clone()).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).wrap().w_full()))
        .child(Button::new(entry.action.clone()).variant(ButtonVariant::Neutral).on_press(move |app: &mut App| app.finish_queue_entry(&id)).build(k).key(format!("queue-do-{}", entry.id)));
    if divider {
        r = col().w_full().child(el().w_full().h(1.0).fill(k.c.border_weak)).child(r);
    }
    r
}

fn failed_row(app: &App, item: &RunItem, divider: bool, k: &Kit, tones: Tones) -> El {
    let id = item.id.clone();
    let id2 = item.id.clone();
    let fix = item.failure.map_or("", |f| f.fix());
    let _ = app;
    let mut r = row().key(format!("failed-{}", item.id)).w_full().px(12.0).py(8.0).gap(11.0).center_y()
        .child(el().size(30.0, 30.0).radius(8.0).center().fill(tones.bad_soft).child(icon(&icons::X_CIRCLE, 16.0).color(tones.bad)))
        .child(col().grow(1.0).shrink().min_w(0.0).gap(2.0)
            .child(text(item.name.clone()).font(k.ty.text_small_medium).color(k.c.text_base))
            .child(text(fix.to_string()).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).wrap().w_full()))
        .child(Button::new("Show log").variant(ButtonVariant::GhostMuted).on_press(move |app: &mut App| { app.set_log_item(Some(id2.clone())); app.screen = Screen::Run; }).build(k).key(format!("log-{}", item.id)))
        .child(Button::new("Try again").variant(ButtonVariant::Neutral).on_press(move |app: &mut App| app.start_scoped(&id, None)).build(k).key(format!("retry-{}", item.id)));
    if divider {
        r = col().w_full().child(el().w_full().h(1.0).fill(k.c.border_weak)).child(r);
    }
    r
}

fn blocked_row(item: &RunItem, k: &Kit, _tones: Tones) -> El {
    let by = item.blocked_by.as_deref().map_or(String::new(), |b| catalog().name_of(b));
    col().w_full().child(el().w_full().h(1.0).fill(k.c.border_weak)).child(
        row().key(format!("blocked-{}", item.id)).w_full().px(12.0).py(8.0).gap(11.0).center_y()
            .child(el().size(30.0, 30.0).radius(8.0).center().fill(k.c.layer_03).child(icon(&icons::LINK, 16.0).color(k.c.icon_muted)))
            .child(col().grow(1.0).shrink().gap(2.0)
                .child(text(item.name.clone()).font(k.ty.text_small_medium).color(k.c.text_base))
                .child(text(format!("Not attempted, because it needs {by}. It goes ahead by itself when {by} works.")).font(k.ty.text_xsmall).color(k.c.text_muted).wrap().w_full())),
    )
}
