//! Differences: review Drift per entry. Nothing changes until the person presses Apply choices.

use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, SegmentedControl, TextInput};

use super::kit::{self, Tones, icons};
use crate::drift::{COULD_NOT_CHECK, Decision, DriftEntry, DriftKind, UNAVAILABLE, UNTRACKED};
use crate::state::{App, Timer};

const GROUPS: &[(DriftKind, &str, &str)] = &[
    (DriftKind::ChangedHere, "Changed on this Mac", "Your Config says one thing, and someone changed this Mac since."),
    (DriftKind::ChangedInConfig, "Changed in your Config", "Your Config moved, probably from another Mac. This Mac hasn't caught up."),
    (DriftKind::ChangedBoth, "Changed in both places", "Your Config and this Mac both moved, so Soulcaster can't tell which one you want."),
    (DriftKind::RemovedHere, "Removed from this Mac", "Your Config still lists these, and they're gone from this Mac."),
    (DriftKind::DroppedFromConfig, "No longer in your Config", "Soulcaster installed these earlier. They're still here, and nothing gets uninstalled unless you say so."),
    (DriftKind::NewHere, "New on this Mac", "Installed with Homebrew, and not in your Config."),
];

/// The choices an entry offers, in order, with what each one means.
fn options(entry: &DriftEntry) -> Vec<(&'static str, Decision)> {
    match entry.kind {
        DriftKind::ChangedHere | DriftKind::ChangedInConfig | DriftKind::ChangedBoth => {
            vec![("Later", Decision::Later), ("Adopt", Decision::Adopt), ("Apply", Decision::Apply)]
        }
        DriftKind::RemovedHere => vec![("Later", Decision::Later), ("Adopt", Decision::Adopt), ("Apply", Decision::Apply)],
        DriftKind::DroppedFromConfig => vec![("Later", Decision::Later), ("Untracked", Decision::KeepUntracked), ("Adopt", Decision::Adopt), ("Remove\u{2026}", Decision::Remove)],
        DriftKind::NewHere => vec![("Later", Decision::Later), ("Adopt", Decision::Adopt)],
    }
}

pub fn screen(app: &App, k: &'static Kit, tones: Tones, layer: &mut Vec<El>) -> El {
    let mut body = col().key("drift-body").w_full().max_w(860.0).self_center().padding(24.0).gap(16.0).child(
        row().w_full().gap(12.0).center_y()
            .child(col().grow(1.0).shrink().child(kit::heading(k, "Drift", "Where this Config and this Machine disagree. Nothing changes until you Adopt or Apply, one line at a time.")))
            .child(
                row().gap(7.0).center_y().px(10.0).h(30.0).radius(7.0).fill(k.c.layer_02)
                    .child(icon(&icons::GITHUB, 13.0).color(k.c.icon_muted))
                    .child(text("octocat/mac-config \u{b7} My Mac").font(k.ty.text_xsmall).color(k.c.text_muted)),
            ),
    );
    if app.drift.is_empty() {
        body = body.child(
            kit::card(k).key("drift-empty").padding(18.0).gap(4.0)
                .child(kit::status_line(k, &icons::CHECK_CIRCLE, tones.good, "This Mac matches your Config"))
                .child(text("\u{201c}Matches\u{201d} means the values read back the same. For settings, it doesn't prove macOS still honors them.").font(k.ty.text_xsmall).color(k.c.text_muted)),
        );
    }
    for (kind, title, sub) in GROUPS {
        let entries: Vec<&DriftEntry> = app.drift.iter().filter(|d| d.kind == *kind).collect();
        if entries.is_empty() {
            continue;
        }
        let mut list = col().w_full();
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                list = list.child(el().w_full().h(1.0).fill(k.c.border_weak));
            }
            list = list.child(entry_row(entry, k, tones));
        }
        body = body.child(group(k, title, sub, entries.len(), kit::card(k).key(format!("drift-{kind:?}")).child(list)));
    }

    // Apart from Drift, as the Config format requires: no Save or Change here.
    let mut unavailable = col().w_full().padding(12.0).gap(9.0);
    for u in UNAVAILABLE {
        unavailable = unavailable.child(
            row().w_full().gap(9.0).items_start().child(icon(&icons::LINK, 14.0).color(k.c.icon_faint).mt(1.0))
                .child(col().grow(1.0).shrink().gap(1.0).child(text(u.name).font(k.ty.text_small_medium).color(k.c.text_muted)).child(text(u.reason).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).w_full().wrap())),
        );
    }
    body = body.child(el().w_full().h(1.0).fill(k.c.border_base).mt(4.0));
    body = body.child(group(k, "Unavailable here", "These stay in your Config for your other Macs. They aren't differences, so there's nothing to choose.", UNAVAILABLE.len(), kit::card(k).key("drift-unavailable").child(unavailable)));

    let dotfiles = row().w_full().padding(12.0).gap(9.0).center_y()
        .child(icon(&icons::BRANCH, 15.0).color(k.c.icon_muted))
        .child(col().grow(1.0).shrink().gap(1.0).child(text("octocat/dotfiles").font(k.ty.text_small_medium).color(k.c.text_base)).child(text("2 commits behind, 1 file not pushed. Soulcaster only reports this. Use Git to sort it out.").font(k.ty.text_xsmall).color(k.c.text_muted)));
    let could_not = row().w_full().padding(12.0).gap(9.0).center_y()
        .child(icon(&icons::QUESTION, 15.0).color(k.c.icon_muted))
        .child(col().grow(1.0).shrink().gap(1.0).child(text(format!("{} couldn't be checked", COULD_NOT_CHECK.len())).font(k.ty.text_small_medium).color(k.c.text_base)).child(text(format!("{}. Not counted as differences, and Soulcaster won't act on them.", COULD_NOT_CHECK.join(", "))).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap()));
    let mut untracked = col().key("untracked").w_full().padding(12.0).gap(6.0).child(
        row().key("untracked-toggle").role("button").label("Untracked apps").expanded(app.show_untracked).focusable(true).w_full().gap(9.0).center_y().on_press(|app: &mut App| app.show_untracked = !app.show_untracked)
            .child(icon(&icons::WINDOW, 15.0).color(k.c.icon_muted))
            .child(col().grow(1.0).shrink().gap(1.0).child(text(format!("{} Untracked", UNTRACKED.len())).font(k.ty.text_small_medium).color(k.c.text_base)).child(text("Installed outside anything Soulcaster manages. It leaves them alone, and they can't be Adopted.").font(k.ty.text_xsmall).color(k.c.text_muted)))
            .child(text(if app.show_untracked { "Hide" } else { "Show" }).font(k.ty.text_xsmall_medium).color(tones.accent_ink)),
    );
    if app.show_untracked {
        untracked = untracked.child(text(UNTRACKED.join(", ")).font(k.ty.text_xsmall.lh(16.0)).color(k.c.text_muted).w_full().wrap().pl(24.0));
    }
    body = body.child(
        group(k, "Just so you know", "No action needed.", 0, kit::card(k).key("drift-info")
            .child(dotfiles).child(el().w_full().h(1.0).fill(k.c.border_weak))
            .child(could_not).child(el().w_full().h(1.0).fill(k.c.border_weak))
            .child(untracked)),
    );

    dialogs(app, k, tones, layer);

    col()
        .key("drift-screen")
        .w_full()
        .h_full()
        .fill(k.c.bg_base)
        .child(col().key("drift-scroll").w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(body))
        .child(footer(app, k, tones))
}

fn group(k: &Kit, title: &str, sub: &str, n: usize, body: El) -> El {
    let mut head = row().gap(7.0).center_y().child(text(title.to_string()).font(k.ty.body_medium).color(k.c.text_strong).role("heading").level(2));
    if n > 0 {
        head = head.child(text(n.to_string()).font(k.ty.text_xsmall_medium).color(k.c.text_muted));
    }
    col().w_full().gap(6.0).child(col().w_full().gap(1.0).child(head).child(text(sub.to_string()).font(k.ty.text_xsmall).color(k.c.text_muted).w_full().wrap())).child(body)
}

fn value_box(k: &Kit, label: &str, value: &str, emphasized: bool, tones: Tones) -> El {
    col().w(104.0).gap(1.0)
        .child(text(label.to_string()).font(k.ty.text_xsmall).color(k.c.text_faint))
        .child(text(value.to_string()).font(k.ty.text_small_medium).color(if emphasized { tones.accent_ink } else { k.c.text_base }).truncate())
}

fn entry_row(entry: &DriftEntry, k: &'static Kit, tones: Tones) -> El {
    let opts = options(entry);
    let labels: Vec<&str> = opts.iter().map(|o| o.0).collect();
    let selected = opts.iter().position(|o| o.1 == entry.decision);
    let key = entry.key.clone();
    let picks: Vec<Decision> = opts.iter().map(|o| o.1).collect();
    let choose = SegmentedControl::new(&labels, selected)
        .label(format!("What to do about {}", entry.name))
        .fit()
        .on_pick(move |app: &mut App, i, _| {
            let decision = picks[i];
            if decision == Decision::Remove {
                app.confirm_remove = Some(key.clone());
                app.remove_understood = false;
            } else if let Some(d) = app.drift.iter_mut().find(|d| d.key == key) {
                d.decision = decision;
            }
        })
        .build(k)
        .key(format!("choose-{}", entry.key));
    let moved_here = matches!(entry.kind, DriftKind::ChangedHere | DriftKind::ChangedBoth | DriftKind::RemovedHere | DriftKind::NewHere);
    let moved_config = matches!(entry.kind, DriftKind::ChangedInConfig | DriftKind::ChangedBoth | DriftKind::DroppedFromConfig);
    let mut values = row().gap(8.0).center_y().child(value_box(k, "Your Config", &entry.in_config, moved_config, tones)).child(value_box(k, "This Mac", &entry.on_mac, moved_here, tones));
    if let (DriftKind::ChangedBoth, Some(last)) = (entry.kind, &entry.last_applied) {
        values = values.child(value_box(k, "Last set here", last, false, tones));
    }
    let consequence = match entry.decision {
        Decision::Later => None,
        Decision::Adopt => Some(match entry.kind {
            DriftKind::RemovedHere => "Takes it out of your Config. Your other Macs will see that as a difference too.".to_string(),
            _ => "Saves this to your Config, as a commit on GitHub.".to_string(),
        }),
        Decision::Apply => Some(if entry.apply_needs_password { "Changes this Mac. This one needs your password, once, when you apply.".to_string() } else { "Changes this Mac to match your Config.".to_string() }),
        Decision::KeepUntracked => Some("Leaves it installed. Soulcaster stops watching it.".to_string()),
        Decision::Remove => Some("Uninstalls it from this Mac when you apply. You confirmed this.".to_string()),
    };
    // One line. Wrapping the Adopt/Apply control dropped it under the card's clip.
    let mut c = col().key(format!("entry-{}", entry.key)).w_full().padding(12.0).gap(8.0).child(
        row().w_full().gap(10.0).center_y()
            .child(col().w(176.0).shrink().min_w(0.0).gap(1.0).child(text(entry.name.clone()).font(k.ty.text_small_medium).color(k.c.text_base).wrap().w_full()))
            .child(values.shrink())
            .child(el().grow(1.0).min_w(8.0))
            .child(choose),
    );
    if let Some(words) = consequence {
        c = c.child(text(words).key(format!("consequence-{}", entry.key)).font(k.ty.text_xsmall).color(if entry.decision == Decision::Remove { tones.bad } else { tones.accent_ink }));
    }
    c
}

fn dialogs(app: &App, k: &'static Kit, tones: Tones, layer: &mut Vec<El>) {
    if let Some(key) = &app.confirm_remove {
        let name = app.drift.iter().find(|d| d.key == *key).map_or(String::new(), |d| d.name.clone());
        let understood = app.remove_understood;
        let target = key.clone();
        let content = col().w_full().px(16.0).pb(16.0).gap(12.0)
            .child(
                row().key("remove-understood").role("checkbox").label(format!("Yes, uninstall {name} from this Mac")).checked(understood).focusable(true).w_full().min_h(30.0).gap(9.0).center_y().on_press(|app: &mut App| app.remove_understood = !app.remove_understood)
                    .child(kit::check_mark(k, tones, understood, false))
                    .child(text(format!("Yes, uninstall {name} from this Mac")).font(k.ty.text_small).color(k.c.text_base)),
            )
            .child(
                row().w_full().justify_end().gap(8.0)
                    .child(Button::new("Keep it").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| app.confirm_remove = None).build(k).key("remove-cancel"))
                    .child(Button::new(format!("Remove {name}")).variant(ButtonVariant::Danger).disabled(!understood).on_press(move |app: &mut App| {
                        if let Some(d) = app.drift.iter_mut().find(|d| d.key == target) {
                            d.decision = Decision::Remove;
                        }
                        app.confirm_remove = None;
                    }).build(k).key("remove-confirm")),
            );
        Dialog::new("remove", format!("Remove {name} from this Mac?"))
            .description("This uninstalls the app. Its data may stay behind. Your other Macs aren't affected, because a Config can't ask a Mac to remove anything.")
            .open(true).fit().width(460.0)
            .on_close(|app: &mut App| app.confirm_remove = None)
            .child(content)
            .build(k, layer);
    }
    if app.drift_password_open {
        let content = col().w_full().px(16.0).pb(16.0).gap(12.0)
            .child(TextInput::new(&app.drift_password).secure().label("Your Mac password").placeholder("Password").leading_icon(k.glyph.lock).on_edit(|app: &mut App| &mut app.drift_password).on_submit(|app: &mut App, _| apply_with_password(app)).build(k).key("drift-password"))
            .child(row().w_full().justify_end().gap(8.0)
                .child(Button::new("Cancel").variant(ButtonVariant::GhostMuted).on_press(|app: &mut App| { app.drift_password.clear(); app.drift_password_open = false; }).build(k).key("drift-password-cancel"))
                .child(kit::primary(k, tones, "drift-password-submit", "Apply", true, |app: &mut App, _| apply_with_password(app))));
        Dialog::new("drift-pw", "Your Mac password")
            .description("One of your choices needs it. It's used for that step and then discarded.")
            .open(true).fit().width(420.0)
            .on_close(|app: &mut App| { app.drift_password.clear(); app.drift_password_open = false; })
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

fn footer(app: &App, k: &Kit, tones: Tones) -> El {
    let count = |f: &dyn Fn(Decision) -> bool| app.drift.iter().filter(|d| f(d.decision)).count();
    let to_config = count(&|d| d == Decision::Adopt);
    let to_mac = count(&|d| matches!(d, Decision::Apply | Decision::Remove));
    let stop = count(&|d| d == Decision::KeepUntracked);
    let total = to_config + to_mac + stop;
    let needs_password = app.drift.iter().any(|d| (d.decision == Decision::Apply && d.apply_needs_password) || d.decision == Decision::Remove);
    let mut bits = Vec::new();
    if to_mac > 0 {
        bits.push(format!("{to_mac} to Apply"));
    }
    if to_config > 0 {
        bits.push(format!("{to_config} to Adopt"));
    }
    if stop > 0 {
        bits.push(format!("{stop} left Untracked"));
    }
    let line = if total == 0 { "Nothing chosen yet. Anything left on Later stays as it is.".to_string() } else { format!("{}. The rest stays as it is.", bits.join(", ")) };
    let action = if app.drift_applying {
        "Working\u{2026}".to_string()
    } else if to_config > 0 && to_mac > 0 {
        "Adopt and Apply".to_string()
    } else if to_mac > 0 {
        "Apply".to_string()
    } else if to_config > 0 {
        "Adopt".to_string()
    } else if stop > 0 {
        "Leave Untracked".to_string()
    } else {
        "Adopt and Apply".to_string()
    };
    row()
        .key("drift-footer")
        .w_full()
        .h(64.0)
        .px(24.0)
        .gap(12.0)
        .center_y()
        .fill(k.c.bg_base)
        .child(text(line).key("drift-summary").font(k.ty.text_small).color(k.c.text_muted).grow(1.0).shrink().truncate().live_region(rune::LiveRegion::Polite))
        .child(kit::primary(k, tones, "drift-apply", &action, total > 0 && !app.drift_applying, move |app: &mut App, _| {
            if needs_password {
                app.drift_password_open = true;
                app.drift_password.focused = true;
            } else {
                begin_apply(app);
            }
        }))
}
