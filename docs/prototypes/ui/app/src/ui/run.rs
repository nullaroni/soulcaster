//! Set up: watching a Run. Progress, both stages, per-Item states and a log.

use std::borrow::Cow;

use rune::{El, GrowthSpec, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Kit, Progress, SegmentedControl, Spinner};

use super::kit::{self, Tones, icons};
use super::state_words;
use crate::lifecycle::{ItemState, Stage};
use crate::state::{AdminState, App, RunFilter, RunItem, Screen};

/// One row of the Item list: a stage header or an Item. Rebuilt from state each commit, which is
/// cheap at this size, because it's only indices.
#[derive(Clone, Copy)]
enum Row {
    Stage(Stage),
    Item(usize),
}

fn is_problem(item: &RunItem) -> bool {
    matches!(item.state, ItemState::Failed | ItemState::NeedsYou | ItemState::Blocked)
}

fn rows(app: &App) -> Vec<Row> {
    let mut out = Vec::new();
    for stage in [Stage::Admin, Stage::User] {
        let mut any = false;
        for (i, item) in app.run_items.iter().enumerate() {
            if item.stage != stage || (app.run_filter == RunFilter::Problems && !is_problem(item)) {
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

pub fn screen(app: &App, k: &'static Kit, tones: Tones) -> El {
    col()
        .key("run-screen")
        .w_full()
        .h_full()
        .fill(k.c.bg_base)
        .child(header(app, k, tones))
        .child(stages(app, k, tones))
        .child(
            row().key("run-body").w_full().grow(1.0).min_h(0.0).px(16.0).pb(16.0).gap(12.0)
                .child(item_list(app, k, tones))
                .child(log_pane(app, k, tones)),
        )
}

fn header(app: &App, k: &Kit, tones: Tones) -> El {
    let total = app.run_items.len().max(1);
    let done = app.run_items.iter().filter(|i| i.state.is_terminal()).count();
    let current = app.run_items.iter().find(|i| i.state.is_working());
    let title = if app.run_finished {
        "Run finished".to_string()
    } else if let Some(item) = current {
        format!("{} {}", state_words(item), item.name)
    } else {
        "Run starting".to_string()
    };
    let sub = format!("{done} of {total} done");
    let trouble = problem_sentence(app);
    let mut head = col()
        .key("run-header")
        .w_full()
        .px(16.0)
        .pt(16.0)
        .pb(10.0)
        .gap(10.0)
        .child(
            row().w_full().gap(12.0).center_y()
                .child(col().grow(1.0).shrink().min_w(0.0).gap(2.0)
                    .child(text(title).key("run-title").font(rune::TextStyle::new(20.0, 600.0)).color(k.c.text_strong).truncate())
                    .child(text(sub).key("run-progress-text").font(k.ty.text_small).color(k.c.text_muted).truncate()))
                .child(
                    row().key("walk-away").gap(8.0).center_y().px(11.0).h(34.0).radius(8.0).fill(tones.accent_soft).shrink()
                        .child(icon(&icons::MOON, 15.0).color(tones.accent_ink))
                        .child(text(if app.run_finished { "This Machine can sleep" } else { "You can walk away" }).font(k.ty.text_small_medium).color(tones.accent_ink)),
                ),
        )
        .child(Progress::new(Some(done as f32 / total as f32)).label("Run progress").build(k).key("run-progress"));
    if !trouble.is_empty() {
        head = head.child(text(trouble).key("run-problems").font(k.ty.text_small).color(tones.bad).w_full().wrap());
    }
    head
}

/// Names the failures in the header so a long Item list can't hide them.
fn problem_sentence(app: &App) -> String {
    let failed: Vec<&str> = app.run_items.iter().filter(|i| i.state == ItemState::Failed).map(|i| i.name.as_str()).collect();
    let needs: Vec<&str> = app.run_items.iter().filter(|i| i.state == ItemState::NeedsYou).map(|i| i.name.as_str()).collect();
    let blocked = app.run_items.iter().filter(|i| i.state == ItemState::Blocked).count();
    let mut parts = Vec::new();
    if !failed.is_empty() {
        parts.push(format!("{} failed", failed.join(", ")));
    }
    if !needs.is_empty() {
        parts.push(format!("{} joined the Needs-You Queue", needs.join(", ")));
    }
    if blocked > 0 {
        parts.push(format!("{blocked} waiting on a failed Item"));
    }
    if parts.is_empty() { String::new() } else { format!("{}. The rest of this Run carries on.", parts.join(". ")) }
}

fn stage_chip(k: &Kit, tones: Tones, label: &str, done: usize, total: usize, active: bool, finished: bool) -> El {
    let glyph: El = if finished {
        icon(&icons::CHECK_CIRCLE, 16.0).color(tones.good)
    } else if active {
        Spinner::new().size(14.0).build(k)
    } else {
        icon(&icons::DOT, 16.0).color(k.c.icon_faint)
    };
    row().grow(1.0).shrink().min_w(0.0).h(40.0).px(12.0).gap(9.0).center_y().radius(8.0).fill(k.c.layer_01).border(1.0, if active { tones.accent } else { k.c.border_weak })
        .child(glyph)
        .child(text(label.to_string()).font(k.ty.text_small_medium).color(if active || finished { k.c.text_base } else { k.c.text_muted }).grow(1.0).shrink().truncate())
        .child(text(format!("{done}/{total}")).font(k.ty.text_xsmall).color(k.c.text_muted))
}

fn stages(app: &App, k: &Kit, tones: Tones) -> El {
    let count = |stage: Stage| {
        let of = app.run_items.iter().filter(|i| i.stage == stage);
        (of.clone().filter(|i| i.state.is_terminal()).count(), of.count())
    };
    let (admin_done, admin_total) = count(Stage::Admin);
    let (user_done, user_total) = count(Stage::User);
    let user_active = !app.run_finished && app.admin != AdminState::Held && admin_done == admin_total;
    let password = match app.admin {
        AdminState::NotNeeded => None,
        AdminState::Waiting | AdminState::Held => Some((&icons::LOCK_CLOSED, tones.warn, tones.warn_soft, tones.warn_ink, "Password in memory")),
        AdminState::Discarded => Some((&icons::LOCK_OPEN, tones.good, tones.good_soft, tones.good, "Password discarded")),
    };
    let mut bar = row().key("stages").w_full().px(16.0).pb(12.0).gap(8.0).center_y();
    if app.admin != AdminState::NotNeeded {
        bar = bar.child(stage_chip(k, tones, "Admin Stage", admin_done, admin_total, app.admin == AdminState::Held, app.admin == AdminState::Discarded));
    }
    if let Some((glyph, color, fill, ink, words)) = password {
        bar = bar.child(
            row().key("password-state").role("status").label(words).live_region(rune::LiveRegion::Polite).h(40.0).px(12.0).gap(7.0).center_y().radius(8.0).fill(fill)
                .child(icon(glyph, 15.0).color(color))
                .child(text(words).font(k.ty.text_small_medium).color(ink)),
        );
    }
    bar.child(stage_chip(k, tones, "User Stage", user_done, user_total, user_active, app.run_finished))
}

fn state_glyph(item: &RunItem, k: &Kit, tones: Tones) -> El {
    match item.state {
        ItemState::Pending => icon(&icons::DOT, 16.0).color(k.c.icon_faint),
        ItemState::Checking | ItemState::Acting | ItemState::Verifying => Spinner::new().size(14.0).build(k),
        ItemState::Satisfied => icon(&icons::CHECK_CIRCLE, 16.0).color(tones.good),
        ItemState::Unknown => icon(&icons::QUESTION, 16.0).color(k.c.icon_muted),
        ItemState::Differs => icon(&icons::ARROWS_SWAP, 16.0).color(k.c.icon_muted),
        ItemState::NeedsYou => icon(&icons::PERSON, 16.0).color(tones.warn),
        ItemState::Failed => icon(&icons::X_CIRCLE, 16.0).color(tones.bad),
        ItemState::Blocked => icon(&icons::LINK, 16.0).color(k.c.icon_muted),
        ItemState::Skipped => icon(&icons::LINK, 16.0).color(k.c.icon_faint),
    }
}

fn item_row(app: &App, i: usize, k: &Kit, tones: Tones) -> El {
    let item = &app.run_items[i];
    let selected = app.log_item.as_deref() == Some(item.id.as_str());
    let id = item.id.clone();
    let words = state_words(item);
    let ink = match item.state {
        ItemState::Failed => tones.bad,
        ItemState::NeedsYou => tones.warn_ink,
        ItemState::Satisfied => k.c.text_muted,
        ItemState::Pending | ItemState::Blocked | ItemState::Skipped => k.c.text_faint,
        _ => tones.accent_ink,
    };
    let mut name = row().grow(1.0).shrink().min_w(0.0).gap(6.0).center_y().child(text(item.name.clone()).font(k.ty.text_small).color(if item.state == ItemState::Pending { k.c.text_muted } else { k.c.text_base }).shrink().truncate());
    if let Some(by) = &item.pulled_by {
        name = name.child(text(format!("for {by}")).font(k.ty.text_xsmall).color(k.c.text_faint).shrink().truncate());
    }
    let mut r = row()
        .key(format!("run-item-{}", item.id))
        .role("button")
        .label(format!("{}: {words}", item.name))
        .focusable(true)
        .w_full()
        .h(32.0)
        .px(12.0)
        .gap(9.0)
        .center_y()
        .hover(k.c.hover)
        .on_press(move |app: &mut App| {
            let next = if app.log_item.as_deref() == Some(id.as_str()) { None } else { Some(id.clone()) };
            app.set_log_item(next);
        })
        .child(el().size(16.0, 16.0).center().child(state_glyph(item, k, tones)))
        .child(name);
    if selected {
        r = r.fill(k.c.layer_03);
    }
    if let (Some(fraction), true) = (item.progress, item.state == ItemState::Acting) {
        r = r.child(el().w(60.0).child(Progress::new(Some(fraction)).build(k)));
    }
    r.child(text(words).font(k.ty.text_xsmall_medium).color(ink))
}

fn item_list(app: &App, k: &'static Kit, tones: Tones) -> El {
    let problems = app.run_items.iter().filter(|i| is_problem(i)).count();
    let labels = ["Everything".to_string(), format!("Problems ({problems})")];
    let shown = rows(app);
    let version = (app.run_filter == RunFilter::Problems, shown.len());
    let body = col().key("run-list").role("list").label("Items in this Run").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(
        col().key("run-rows").w_full().growth(GrowthSpec::default())
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
                |s: &App, i| {
                    let (k, tones) = kit::theme_now();
                    match rows(s).get(i).copied() {
                        Some(Row::Stage(stage)) => row().w_full().h(30.0).px(12.0).center_y().child(
                            text(if stage == Stage::Admin { "ADMIN STAGE" } else { "USER STAGE" }).font(k.ty.text_xsmall_medium).color(k.c.text_faint),
                        ),
                        Some(Row::Item(x)) => item_row(s, x, k, tones),
                        None => el(),
                    }
                },
            )
            .virtual_rows_version(version),
    );
    let _ = tones;
    kit::card(k).key("run-items").w_frac(0.5).h_full().min_w(0.0)
        .child(row().w_full().h(44.0).px(10.0).gap(8.0).center_y()
            .child(
                SegmentedControl::new(&labels, Some(if app.run_filter == RunFilter::Problems { 1 } else { 0 }))
                    .label("Show")
                    .fit()
                    .on_pick(|app: &mut App, i, _| app.run_filter = if i == 1 { RunFilter::Problems } else { RunFilter::All })
                    .build(k)
                    .key("run-filter"),
            ))
        .child(body)
}

fn log_pane(app: &App, k: &'static Kit, _tones: Tones) -> El {
    let title = match &app.log_item {
        Some(id) => format!("Log: {}", crate::catalog::catalog().name_of(id)),
        None => "Log: everything".to_string(),
    };
    let mut head = row().w_full().h(44.0).px(12.0).gap(8.0).center_y().child(text(title).key("log-title").font(k.ty.text_small_medium).color(k.c.text_base).grow(1.0).shrink().truncate());
    if app.log_item.is_some() {
        head = head.child(rune_kit::Button::new("Show everything").variant(rune_kit::ButtonVariant::GhostMuted).on_press(|app: &mut App| app.set_log_item(None)).build(k).key("log-all"));
    }
    let count = app.log_view.len();
    let lines = col().key("log").role("log").label("Run log").w_full().grow(1.0).min_h(0.0).px(12.0).pb(8.0).scroll(ScrollSpec { stick_bottom: true }).child(
        col().key("log-rows").w_full().growth(GrowthSpec::default())
            .virtual_rows(
                count,
                18.0,
                |s: &App, i| Cow::Owned(format!("l{}", s.log_view.get(i).copied().unwrap_or(i))),
                |s: &App, i| {
                    let (k, tones) = kit::theme_now();
                    let Some(line) = s.log_view.get(i).and_then(|&abs| s.log.get(abs.wrapping_sub(s.log_base))) else { return el() };
                    let t = &line.text;
                    let ink = if t.starts_with("Error") || t.starts_with("curl:") { tones.bad } else if t.starts_with('$') { k.c.text_base } else { k.c.text_muted };
                    text(t.clone()).font(rune::TextStyle { size: 11.0, line_height: 18.0, ..k.ty.mono }).color(ink).selectable(true).truncate().w_full()
                },
            ),
    );
    kit::card(k).key("run-log").grow(1.0).shrink().min_w(0.0).h_full().fill(k.c.layer_01).child(head).child(lines)
}

#[allow(dead_code)]
fn unused(_: Screen) {}
