//! Answer once: everything the selection needs from a person, in one place, then walk away.

use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, Spinner, Switch, TextInput};

use super::kit::{self, Tones, icons};
use crate::catalog::{BASE, catalog};
use crate::plan::{ConflictAsk, Plan, Waits, Why};
use crate::state::{App, GithubState, PasswordState, Screen, Timer};

pub fn screen(app: &App, k: &'static Kit, tones: Tones, layer: &mut Vec<El>) -> El {
    let plan = app.plan();
    let mut asks = col().key("asks").w_full().gap(12.0);
    let mut n = 0;
    let mut number = || {
        n += 1;
        n
    };
    for conflict in &plan.conflicts {
        asks = asks.child(conflict_card(number(), conflict, k, tones));
    }
    if plan.needs_password {
        asks = asks.child(password_card(number(), app, &plan, k, tones));
    }
    if plan.needs_github {
        asks = asks.child(github_card(number(), app, k, tones));
    }
    if !plan.licenses.is_empty() {
        asks = asks.child(licenses_card(number(), app, &plan, k, tones));
    }
    if !plan.taps.is_empty() {
        asks = asks.child(taps_card(number(), app, &plan, k, tones));
    }
    let questions = n;
    if questions == 0 {
        asks = asks.child(nothing_needed(k, tones));
    }

    let lead = if questions == 0 {
        "This selection doesn't need anything from you.".to_string()
    } else {
        "Answer these now and nothing else will ask you anything until the end.".to_string()
    };
    let left = col().key("review-left").grow(1.0).shrink().min_w(0.0).gap(14.0)
        .child(kit::heading(k, "Before the Run", &lead))
        .child(asks);
    let right = plan_panel(app, &plan, k, tones);

    if app.show_config {
        Dialog::new("config", format!("Your Config: {}", app.config_name))
            .description("This is the file Soulcaster saves from your choices. It's plain JSON in your GitHub repo, and it holds no passwords.")
            .open(true)
            .width(560.0)
            .on_close(|app: &mut App| app.show_config = false)
            .child(col().key("config-json-scroll").w_full().h(290.0).px(16.0).pb(16.0).child(
                col().w_full().grow(1.0).min_h(0.0).radius(8.0).fill(k.c.layer_02).scroll(ScrollSpec { stick_bottom: false }).child(
                    text(app.config_json()).key("config-json").font(k.ty.mono).color(k.c.text_base).selectable(true).padding(12.0),
                ),
            ))
            .build(k, layer);
    }

    col()
        .key("review-screen")
        .w_full()
        .h_full()
        .fill(k.c.bg_base)
        .child(
            col().key("review-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(
                row().w_full().padding(24.0).gap(24.0).items_start().child(left).child(right),
            ),
        )
        .child(footer(app, &plan, k, tones))
}

fn ask_card(key: &str, number: usize, title: &str, done: bool, k: &Kit, tones: Tones) -> (El, El) {
    let badge = if done {
        el().size(22.0, 22.0).center().child(icon(&icons::CHECK_CIRCLE, 20.0).color(tones.good))
    } else {
        el().size(22.0, 22.0).radius(11.0).center().fill(tones.accent_soft).child(text(number.to_string()).font(k.ty.text_xsmall_medium).color(tones.accent_ink))
    };
    let head = row().gap(9.0).center_y().child(badge).child(text(title.to_string()).font(k.ty.body_medium).color(k.c.text_strong).role("heading").level(2));
    (kit::card(k).key(key.to_string()).padding(14.0).gap(9.0), head)
}

fn conflict_card(number: usize, conflict: &ConflictAsk, k: &Kit, tones: Tones) -> El {
    let cat = catalog();
    let (name_a, name_b) = (cat.name_of(&conflict.a), cat.name_of(&conflict.b));
    let (card, head) = ask_card(&format!("ask-conflict-{}", conflict.a), number, &format!("Choose {name_a} or {name_b}"), false, k, tones);
    let pick = |keep: &str, drop: &str, name: &str| {
        let (keep, drop) = (keep.to_string(), drop.to_string());
        Button::new(format!("Keep {name}")).variant(ButtonVariant::Outline).on_press(move |app: &mut App| {
            app.answers.dropped.insert(drop.clone());
            app.selected.remove(&drop);
            let _ = &keep;
        }).build(k).key(format!("keep-{}", name.to_lowercase().replace(' ', "-")))
    };
    card.child(head)
        .child(text(format!("These two can't be installed together: {}.", conflict.reason.trim_end_matches('.').to_lowercase())).font(k.ty.text_small).color(k.c.text_muted).w_full().wrap())
        .child(row().gap(8.0).child(pick(&conflict.a, &conflict.b, &name_a)).child(pick(&conflict.b, &conflict.a, &name_b)))
}

fn password_card(number: usize, app: &App, plan: &Plan, k: &Kit, tones: Tones) -> El {
    let done = app.password_state == PasswordState::Accepted;
    let (card, head) = ask_card("ask-password", number, "Your Mac password", done, k, tones);
    let names = plan.admin_names.join(", ");
    let mut card = card.child(head).child(
        text(format!("Only these need it: {names}. Soulcaster keeps it in memory, never on disk, and discards it as soon as they finish."))
            .font(k.ty.text_small).color(k.c.text_muted).w_full().wrap(),
    );
    match app.password_state {
        PasswordState::Accepted => {
            card = card.child(kit::status_line(k, &icons::LOCK_CLOSED, tones.good, "Checked. Held in memory until those steps finish."));
        }
        PasswordState::Checking => {
            card = card.child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(text("Checking with macOS").font(k.ty.text_small).color(k.c.text_muted)));
        }
        state => {
            let wrong = state == PasswordState::Wrong;
            let input = TextInput::new(&app.password)
                .secure()
                .label("Your Mac password")
                .placeholder("Password")
                .leading_icon(k.glyph.lock)
                .invalid(wrong)
                .on_edit(|app: &mut App| &mut app.password)
                .on_submit(|app: &mut App, _| app.check_password())
                .build(k)
                .key("password-field");
            let has_text = !app.password.text.is_empty();
            card = card.child(
                row().w_full().gap(8.0).center_y().child(input.grow(1.0).max_w(340.0))
                    .child(Button::new("Check").variant(ButtonVariant::Neutral).disabled(!has_text).on_press(|app: &mut App| app.check_password()).build(k).key("password-check")),
            );
            if wrong {
                card = card.child(text("That password didn't work. Nothing has started. Try again.").key("password-error").font(k.ty.text_small).color(tones.bad).live_region(rune::LiveRegion::Polite));
            }
            card = card.child(text("Prototype: don't type your real password. Any text passes, and \u{201c}wrong\u{201d} shows the rejected state.").font(k.ty.text_xsmall).color(tones.warn_ink));
        }
    }
    if app.machine.touch_id() {
        card = card.child(
            row().w_full().gap(7.0).items_start().padding(9.0).radius(7.0).fill(tones.warn_soft)
                .child(icon(&icons::WARN, 14.0).color(tones.warn))
                .child(text("This Mac uses Touch ID for sudo, so stay nearby for the first part. It may ask for your fingerprint. If nobody answers, that step waits for you at the end and the rest carries on.").font(k.ty.text_xsmall).color(tones.warn_ink).grow(1.0).shrink().wrap()),
        );
    }
    card
}

fn github_card(number: usize, app: &App, k: &Kit, tones: Tones) -> El {
    let done = matches!(app.github, GithubState::Done | GithubState::Skipped);
    let (card, head) = ask_card("ask-github", number, "Approve GitHub in your browser", done, k, tones);
    let mut card = card.child(head);
    match app.github {
        GithubState::Preparing => {
            card = card
                .child(text("You approve this on github.com. Soulcaster never sees your GitHub password.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap())
                .child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(text("Getting the GitHub CLI ready").font(k.ty.text_small).color(k.c.text_muted)));
        }
        GithubState::Ready => {
            card = card
                .child(text("You approve this on github.com. Soulcaster never sees your GitHub password.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap())
                .child(
                    row().gap(8.0).center_y()
                        .child(Button::new("Open GitHub").icon(&icons::GITHUB).variant(ButtonVariant::Neutral).on_press_tx(|app: &mut App, _| {
                            app.github = GithubState::Waiting;
                            app.after(2.6, Timer::GithubApproved);
                        }).build(k).key("github-open"))
                        .child(Button::new("Skip for now").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| app.github = GithubState::Skipped).build(k).key("github-skip")),
                );
        }
        GithubState::Waiting => {
            card = card
                .child(row().gap(10.0).center_y()
                    .child(text("WDJB-MJHT").key("github-code").font(rune::TextStyle::new(18.0, 600.0)).color(k.c.text_strong).selectable(true))
                    .child(text("Enter this code on the page that opened.").font(k.ty.text_small).color(k.c.text_muted)))
                .child(row().gap(8.0).center_y().child(Spinner::new().size(14.0).build(k)).child(text("Waiting for you to approve. This updates by itself.").font(k.ty.text_small).color(k.c.text_muted)))
                .child(text("Prototype: no browser opens, and this approves itself in a moment.").font(k.ty.text_xsmall).color(tones.warn_ink));
        }
        GithubState::Done => card = card.child(kit::status_line(k, &icons::GITHUB, tones.good, "Signed in as octocat")),
        GithubState::Skipped => {
            card = card.child(row().gap(10.0).center_y()
                .child(kit::status_line(k, &icons::INFO, k.c.icon_muted, "Skipped. Anything that needs GitHub will wait for you at the end."))
                .child(Button::new("Undo").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| app.github = GithubState::Ready).build(k).key("github-undo")));
        }
    }
    card
}

fn licenses_card(number: usize, app: &App, plan: &Plan, k: &Kit, tones: Tones) -> El {
    let all = plan.licenses.iter().all(|(id, _)| app.answers.licenses.contains(id));
    let (card, head) = ask_card("ask-licenses", number, "Accept licenses", all, k, tones);
    let mut card = card.child(head).child(text("Soulcaster won't accept a license for you. One you leave off waits at the end, and the rest carries on.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap());
    for (id, name) in &plan.licenses {
        let on = app.answers.licenses.contains(id);
        let id2 = id.clone();
        card = card.child(
            row().key(format!("license-{id}")).role("checkbox").label(format!("I accept the {name} license")).checked(on).focusable(true).w_full().min_h(32.0).gap(9.0).center_y().radius(6.0).hover(k.c.hover)
                .on_press(move |app: &mut App| {
                    if !app.answers.licenses.remove(&id2) {
                        app.answers.licenses.insert(id2.clone());
                    }
                })
                .child(kit::check_mark(k, tones, on, false))
                .child(text(format!("I accept the {name} license")).font(k.ty.text_small).color(k.c.text_base).grow(1.0))
                .child(text("Read it").font(k.ty.text_xsmall_medium).color(tones.accent_ink)),
        );
    }
    card
}

fn taps_card(number: usize, app: &App, plan: &Plan, k: &Kit, tones: Tones) -> El {
    let all = plan.taps.iter().all(|(t, _)| app.answers.taps.contains(t));
    let (card, head) = ask_card("ask-taps", number, "Allow third-party sources", all, k, tones);
    let mut card = card.child(head).child(text("These come from outside Homebrew's official list, and their code runs on your Mac. They only run after your password is gone.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap());
    for (tap, names) in &plan.taps {
        let on = app.answers.taps.contains(tap);
        let t = tap.clone();
        card = card.child(
            row().key(format!("tap-{}", tap.replace('/', "-"))).w_full().min_h(36.0).gap(10.0).center_y()
                .child(icon(&icons::SHIELD, 15.0).color(k.c.icon_muted))
                .child(col().grow(1.0).shrink().child(text(tap.clone()).font(k.ty.text_small_medium).color(k.c.text_base)).child(text(format!("For {}", names.join(", "))).font(k.ty.text_xsmall).color(k.c.text_muted)))
                .child(Switch::new(on).label(format!("Allow {tap}")).hide_label().on_toggle(move |app: &mut App| {
                    if !app.answers.taps.remove(&t) {
                        app.answers.taps.insert(t.clone());
                    }
                }).build(k)),
        );
    }
    card
}

fn nothing_needed(k: &Kit, tones: Tones) -> El {
    kit::card(k).key("ask-nothing").padding(16.0).gap(6.0)
        .child(kit::status_line(k, &icons::CHECK_CIRCLE, tones.good, "No password, no sign-in, no licenses"))
        .child(text("Nothing you picked needs your password, because Command Line Tools and Homebrew are already on this Mac, and you're already signed in to GitHub.").font(k.ty.text_small).color(k.c.text_muted).w_full().wrap())
}

fn plan_panel(app: &App, plan: &Plan, k: &Kit, tones: Tones) -> El {
    let apps = plan.count_where(|e| e.why == Why::Selected && !e.is_setting);
    let settings = plan.count_where(|e| e.is_setting);
    let pulled: Vec<String> = plan.entries.iter().filter_map(|e| match &e.why {
        Why::PulledIn { by } => Some(format!("{} (for {by})", e.name)),
        _ => None,
    }).collect();
    let stat = |value: String, label: &str| {
        row().w_full().gap(8.0).center_y().child(text(value).font(k.ty.text_small_medium).color(k.c.text_strong).w(26.0)).child(text(label.to_string()).font(k.ty.text_small).color(k.c.text_muted).grow(1.0).shrink().wrap())
    };
    let mut summary = kit::card(k).key("plan-summary").padding(14.0).gap(7.0)
        .child(text("What will happen").font(k.ty.text_small_medium).color(k.c.text_strong))
        .child(stat(apps.to_string(), "apps and tools you picked"))
        .child(stat(settings.to_string(), "settings you picked"))
        .child(stat(BASE.len().to_string(), if app.machine.base_present() { "Base Items, already here" } else { "Base Items" }));
    if !pulled.is_empty() {
        summary = summary.child(stat(pulled.len().to_string(), &format!("needed by your picks: {}", pulled.join(", "))));
    }
    summary = summary.child(el().h(2.0)).child(
        Button::new("View your Config file").icon(&icons::DOC).variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| app.show_config = true).build(k).key("view-config"),
    );

    let mut panel = col().key("review-right").w(320.0).gap(12.0).child(summary);

    let waiting: Vec<_> = plan.waiting().collect();
    if !waiting.is_empty() {
        let mut card = kit::card(k).key("plan-waits").padding(14.0).gap(9.0)
            .child(text("Needs-You Queue").font(k.ty.text_small_medium).color(k.c.text_strong))
            .child(text("Only a person can do these. They wait until the end of the Run and don't hold anything up.").font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap());
        for entry in waiting {
            let (glyph, words) = match entry.waits_for.as_ref().expect("filtered") {
                Waits::Person { title, apple_account, .. } => (if *apple_account { &icons::APPLE } else { &icons::PERSON }, title.clone()),
                Waits::License => (&icons::DOC, format!("Accept the {} license", entry.name)),
                Waits::Tap(tap) => (&icons::SHIELD, format!("{}: {tap} isn't allowed", entry.name)),
            };
            card = card.child(row().w_full().gap(8.0).items_start().child(icon(glyph, 13.0).color(tones.warn).mt(2.0)).child(text(words).font(k.ty.text_xsmall.lh(17.0)).color(k.c.text_base).grow(1.0).shrink().wrap()));
        }
        panel = panel.child(card);
    }
    if !plan.first_open.is_empty() {
        let mut card = kit::card(k).key("plan-first-open").padding(14.0).gap(7.0)
            .child(text("These come up when you first open them").font(k.ty.text_small_medium).color(k.c.text_strong));
        let mut names: Vec<&str> = plan.first_open.iter().map(|(n, _)| n.as_str()).collect();
        names.dedup();
        card = card.child(text(names.join(", ")).font(k.ty.text_xsmall.lh(17.0)).color(k.c.text_muted).w_full().wrap());
        panel = panel.child(card);
    }
    panel
}

fn footer(app: &App, plan: &Plan, k: &Kit, tones: Tones) -> El {
    let ready = app.ready_to_start(plan);
    let (title, sub) = if ready {
        ("After you start the Run, you can walk away.".to_string(), "Soulcaster keeps this Machine awake. One failed Item never stops the rest.".to_string())
    } else {
        let mut left = Vec::new();
        if !plan.conflicts.is_empty() {
            left.push("pick between the conflicting apps");
        }
        if plan.needs_password && app.password_state != PasswordState::Accepted {
            left.push("enter your Mac password");
        }
        if plan.needs_github && !matches!(app.github, GithubState::Done | GithubState::Skipped) {
            left.push("approve GitHub");
        }
        ("Almost there".to_string(), format!("Still to do: {}.", left.join(", ")))
    };
    row()
        .key("review-footer")
        .w_full()
        .h(64.0)
        .px(24.0)
        .gap(12.0)
        .center_y()
        .fill(k.c.bg_base)
        .child(Button::new("Back").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| app.screen = Screen::Catalog).build(k).key("review-back"))
        .child(
            row().grow(1.0).shrink().min_w(0.0).gap(10.0).center_y().pl(6.0)
                .child(icon(&icons::MOON, 18.0).color(if ready { tones.accent_ink } else { k.c.icon_faint }))
                .child(col().grow(1.0).shrink().min_w(0.0).gap(1.0)
                    .child(text(title).key("review-status").font(k.ty.text_small_medium).color(k.c.text_strong).live_region(rune::LiveRegion::Polite))
                    .child(text(sub).font(k.ty.text_xsmall).color(k.c.text_muted).truncate())),
        )
        .child(kit::primary(k, tones, "start-run", "Start Run", ready, |app: &mut App, _| app.start_run()))
}
