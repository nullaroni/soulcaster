//! From a pasted link to the review: paste, read, choose, and every way that can fail.

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, Kit, Spinner, TextInput};

use crate::plan::{plan, s};
use crate::ui::*;
use crate::{App, Screen};

#[derive(Clone, Copy, PartialEq)]
pub enum Failure {
    Offline,
    NotVisible,
    NotVisibleSignedIn,
    RateLimited,
    NoConfigs,
}

const SAMPLES: &[(&str, &str)] = &[
    ("Clean", "clean"),
    ("One tap", "one-tap"),
    ("Stranger's Dotfiles Repo", "stranger-dotfiles"),
    ("Hostile", "hostile"),
    ("Bidi in name", "refused-bidi"),
    ("Self-approval", "refused-self-approval"),
];

pub fn paste(app: &App, k: &Kit) -> El {
    let mut samples = row().w_full().gap(6.0).flex_wrap();
    for &(label, id) in SAMPLES {
        samples = samples.child(
            Button::new(label)
                .variant(ButtonVariant::Outline)
                .on_press(move |a: &mut App| {
                    if id.starts_with("refused") { a.screen = Screen::Refused(id.into()) } else { a.open_review(id) }
                })
                .build(k),
        );
    }
    let body = col()
        .w_full()
        .gap(14.0)
        .child(para(k, "Paste owner/repo, or a link to a Config. You'll get your own copy, and you'll see everything it would do before anything happens."))
        .child(
            row()
                .w_full()
                .gap(8.0)
                .child(col().grow(1.0).min_w(0.0).child(
                    TextInput::new(&app.paste)
                        .placeholder("owner/repo or https://…")
                        .on_edit(|a: &mut App| &mut a.paste)
                        .build(k),
                ))
                .child(
                    Button::new("Read Config")
                        .variant(ButtonVariant::Contrast)
                        .on_press(|a: &mut App| a.screen = Screen::Choose)
                        .build(k),
                ),
        )
        .child(muted(k, "Reading a Config runs nothing from it. Your copy doesn't follow its source: you'll be told when the source changes, and nothing changes unless you choose."))
        .child(card(k, col().w_full().gap(8.0).child(muted(k, "PROTOTYPE: nothing is fetched. These open the sample Configs.")).child(samples)));
    page(k, "Import a Config", "Import", body, None)
}

pub fn reading(k: &Kit, source: &str) -> El {
    let step = |done: bool, line: &str| {
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(if done { text("✓").font(k.ty.text_small).color(k.c.text_muted) } else { Spinner::new().size(14.0).build(k) })
            .child(para(k, line))
    };
    let body = col()
        .w_full()
        .gap(10.0)
        .child(data(k, source))
        .child(step(true, "Found the repo at commit 3f9a1c2. Everything below is read at that commit."))
        .child(step(true, "Found 3 Configs in configs/"))
        .child(step(false, "Checking each against the Config format"))
        .child(muted(k, "Reading runs nothing from these files, and nothing is saved until you choose."));
    page(k, "Reading", "Import", body, None)
}

pub fn choose(k: &Kit) -> El {
    let mut rows = col().w_full().gap(8.0);
    for (path, id) in [("configs/laptop.json", "clean"), ("configs/mobile.json", "one-tap"), ("configs/terminal.json", "stranger-dotfiles")] {
        let p = plan(id);
        let approvals = p["counts"]["approvals"].as_u64().unwrap_or(0);
        rows = rows.child(card(
            k,
            row()
                .w_full()
                .center_y()
                .gap(12.0)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(data(k, path))
                        .child(muted(k, format!("Name in the file: \"{}\"", s(p, "name"))))
                        .child(muted(k, format!(
                            "{} to install, {} settings, {}",
                            p["counts"]["install"], p["counts"]["settings"],
                            match approvals { 0 => "nothing needs approval".to_string(), 1 => "1 part needs approval".to_string(), n => format!("{n} parts need approval") }
                        ))),
                )
                .child(Button::new("Review").on_press(move |a: &mut App| a.open_review(id)).build(k)),
        ));
    }
    let body = col().w_full().gap(10.0)
        .child(para(k, "This repo has 3 Configs at commit 3f9a1c2. A Machine follows one at a time, so pick one."))
        .child(rows);
    page(k, "Pick a Config from example-dev/mac-setup", "Import", body, Some(back(k)))
}

fn back(k: &Kit) -> El {
    row().w_full().child(Button::new("Back").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k))
}

pub fn failed(k: &Kit, f: Failure) -> El {
    let source = "example-dev/private-setup";
    let (title, lines, primary): (String, Vec<&str>, Option<&str>) = match f {
        Failure::Offline => ("Soulcaster couldn't reach GitHub".into(), vec!["Check your connection and try again. What you pasted is still here.", "Nothing was read, and nothing on this Mac changed."], Some("Try again")),
        Failure::NotVisible => (format!("Soulcaster can't see {source}"), vec![
            "It doesn't exist, or it's private. GitHub gives the same answer for both when you aren't signed in, so Soulcaster can't tell which.",
            "Sign in to try again as you. Signing in happens in your browser, and your password is never typed into Soulcaster.",
        ], Some("Sign in to GitHub")),
        Failure::NotVisibleSignedIn => (format!("example-you can't see {source}"), vec![
            "It doesn't exist, or this account doesn't have access. Check the spelling, or ask the owner for access.",
            "Nothing was read, and nothing on this Mac changed.",
        ], Some("Use another GitHub account")),
        Failure::RateLimited => ("GitHub is limiting requests from this network".into(), vec![
            "Signing in gets a higher limit. Otherwise you can try again after 14:32.",
            "Nothing was read, and nothing on this Mac changed.",
        ], Some("Sign in to GitHub")),
        Failure::NoConfigs => ("No Configs in example-dev/notes".into(), vec![
            "Soulcaster looks for Configs in configs/*.json. If this one lives somewhere else, paste a link to the file.",
        ], None),
    };
    let mut c = col().w_full().gap(8.0).child(data(k, source_for(f)));
    for l in lines {
        c = c.child(para(k, l));
    }
    let mut buttons = row().w_full().gap(8.0).child(Button::new("Change what I pasted").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k));
    if let Some(p) = primary {
        buttons = buttons.child(rune::spacer()).child(Button::new(p).variant(ButtonVariant::Contrast).on_press(|a: &mut App| a.screen = Screen::Choose).build(k));
    }
    page(k, &title, "Import", card(k, c), Some(buttons))
}

fn source_for(f: Failure) -> &'static str {
    match f {
        Failure::NoConfigs => "example-dev/notes",
        Failure::Offline | Failure::RateLimited => "example-dev/mac-setup",
        _ => "example-dev/private-setup",
    }
}

pub fn refused(k: &Kit, id: &str) -> El {
    let p = plan(id);
    let r = &p["refused"];
    let source = &p["source"];
    let (title, why): (&str, String) = match s(r, "code") {
        "bad-text" => ("Soulcaster won't read this file", "Its name has characters that change how text reads on screen, so what you'd see wouldn't be what the file says. Written out, it says:".into()),
        "unknown-field" => ("Soulcaster won't read this file", "It has fields a Config can't have. A Config can't approve anything for you and can't carry commands, so these can't be ignored and read past:".into()),
        "parser" => ("Soulcaster won't read this file", "It says the same thing twice. Programs disagree on which one counts, so what you'd review might not be what runs:".into()),
        _ => ("This Config needs a newer Soulcaster", "It was written for a format this version doesn't know, so it can't be shown accurately. Update Soulcaster and try again. Nothing is wrong with this file as far as Soulcaster can tell.".into()),
    };
    let newer = s(r, "code") == "newer-version";
    let mut c = col()
        .w_full()
        .gap(10.0)
        .child(kv(k, "From", data(k, format!("{}/{} {} at commit {}", s(source, "host"), s(source, "pasted"), s(source, "path"), s(source, "commit")))))
        .child(para(k, why));
    if !newer {
        c = c.child(quoted_detail(k, s(r, "detail")));
    }
    c = c.child(muted(k, if newer { "Nothing was kept, and nothing on this Mac changed." } else { "All of the file is refused, not just this part. Nothing was kept, and nothing on this Mac changed." }));
    let buttons = row().w_full().gap(8.0).child(rune::spacer()).child(
        Button::new(if newer { "Update Soulcaster" } else { "Close" }).variant(ButtonVariant::Contrast).on_press(|a: &mut App| a.screen = Screen::Paste).build(k),
    );
    page(k, title, "Import", card(k, c), Some(buttons))
}

fn quoted_detail(k: &Kit, d: &str) -> El {
    rune::el().w_full().padding(8.0).radius(k.radius.sm).fill(k.c.layer_02).border(1.0, k.c.border_weak).child(data(k, d))
}
