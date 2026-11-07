//! The review: one page, one pass, everything asked once. See README.md for why it looks like this.

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit};
use serde_json::Value;

use crate::plan::{list, plan, s, strs};
use crate::ui::*;
use crate::{App, Screen};

fn word_count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

pub fn review(app: &App, k: &Kit) -> El {
    let p = plan(&app.plan);
    let warned = !list(p, "signals").is_empty();
    let mut c = col().w_full().gap(18.0).child(where_from(k, p));
    if warned {
        c = c.child(warning(k, p));
    }
    c = c.child(counts(k, p, warned));
    let approvals = approvals_section(app, k, p);
    if let Some(a) = approvals {
        c = c.child(a);
    } else {
        c = c.child(muted_line(k));
    }
    c = c.child(installs(app, k, p)).child(settings(app, k, p)).child(answers(app, k, p));
    if !list(p, "wontApply").is_empty() || refused_links(p) > 0 {
        c = c.child(wont_apply(app, k, p));
    }
    let title = format!("Review {}", s(&p["source"], "path"));
    page(k, &title, "Import", c, Some(footer(app, k, p, warned)))
}

fn muted_line(k: &Kit) -> El {
    card(k, para(k, "Nothing here needs approval. Everything comes from the Catalog or official Homebrew."))
}

fn where_from(k: &Kit, p: &Value) -> El {
    let src = &p["source"];
    let repo = s(src, "pasted").split("/blob/").next().unwrap_or("").trim_start_matches("https://github.com/").to_string();
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(kv(k, "From", col().w_full().gap(2.0)
            .child(strong(k, format!("github.com/{repo}")))
            .child(data(k, format!("{} at commit {}", s(src, "path"), s(src, "commit"))))))
        .child(kv(k, "Name in the file", col().w_full().gap(4.0)
            .child(quoted(k, s(p, "name")))
            .child(muted(k, "Written by whoever made this file. Soulcaster doesn't check names, and doesn't verify or endorse any Config."))));
    if p["nameHasLookalikes"].as_bool() == Some(true) {
        c = c.child(kv(k, "Written out", data(k, s(p, "nameEscaped"))));
    }
    card(k, c.child(muted(k, "Nothing has been installed, changed or run yet.")))
}

fn warning(k: &Kit, p: &Value) -> El {
    let mut c = col().w_full().gap(8.0).child(
        text("This file looks built to mislead you, so everything starts off").font(k.ty.body_medium).color(k.c.danger.fg).w_full().wrap(),
    );
    for sig in list(p, "signals") {
        c = c.child(row().w_full().gap(8.0).items_start().child(text("•").font(k.ty.text_small).color(k.c.danger.fg)).child(col().grow(1.0).min_w(0.0).child(para(k, s(sig, "text")))));
    }
    c = c.child(muted(k, "You can still select parts of it below. If you don't know who made this, close it."));
    rune::el().w_full().padding(14.0).radius(k.radius.md).fill(k.c.danger.bg).border(1.0, k.c.danger.border).child(c)
}

fn counts(k: &Kit, p: &Value, warned: bool) -> El {
    let n = &p["counts"];
    let tile = |num: String, what: &str| {
        col().grow(1.0).gap(0.0).child(text(num).font(k.ty.body_medium).color(k.c.text_base)).child(muted(k, what))
    };
    row()
        .w_full()
        .gap(12.0)
        .child(tile(n["install"].to_string(), if warned { "could install" } else { "to install" }))
        .child(tile(n["settings"].to_string(), if warned { "settings could change" } else { "settings to change" }))
        .child(tile(n["approvals"].to_string(), "need your approval"))
        .child(tile(n["wont"].to_string(), "won't apply"))
}

// ---------------------------------------------------------------- the three approvals

fn approvals_section(app: &App, k: &Kit, p: &Value) -> Option<El> {
    let taps = list(p, "taps");
    let has_dotfiles = !p["dotfiles"].is_null();
    if taps.is_empty() && !has_dotfiles {
        return None;
    }
    let mut c = col().w_full().gap(10.0);
    // Blocked lines first: they're what a person is most likely to misread.
    for t in taps.iter().filter(|t| t["blocked"].as_bool() == Some(true)) {
        c = c.child(blocked_tap(k, t));
    }
    for t in taps.iter().filter(|t| t["blocked"].as_bool() != Some(true)) {
        c = c.child(tap_card(app, k, t));
    }
    if has_dotfiles {
        c = c.child(repo_card(app, k, &p["dotfiles"])).child(script_card(app, k, &p["dotfiles"]));
    }
    Some(section(k, "Can run someone else's code", "Each of these stays off unless you turn it on. Leave them off and the Run still does everything else.", c))
}

fn tap_card(app: &App, k: &Kit, t: &Value) -> El {
    let tap = s(t, "tap").to_string();
    let owner = s(t, "owner");
    let on = app.boxes.taps.get(&tap).copied().unwrap_or(false);
    let names: Vec<&str> = list(t, "packages").iter().map(|x| s(x, "short")).collect();
    let label = format!("Install {} from {}", names.join(", "), tap);
    let key = format!("approve-tap-{tap}");
    let tap2 = tap.clone();
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(
            Checkbox::new(on)
                .label(label)
                .on_toggle(move |a: &mut App| {
                    let v = a.boxes.taps.entry(tap2.clone()).or_insert(false);
                    *v = !*v;
                })
                .build(k)
                .key(key),
        )
        .child(kv(k, "What this runs", para(k, format!("Homebrew runs install code written by {owner} on this Mac, as you. Soulcaster and Homebrew haven't reviewed it."))))
        .child(kv(k, "What's at stake", para(k, "It runs after your password is gone, so it can't use it. It can still read what your account can, such as SSH keys and project folders, use your GitHub sign-in, and leave something behind that runs later.")))
        .child(kv(k, "Later", para(k, format!("{owner} can change this code at any time. When you choose to update, the newest version runs with no new question."))))
        .child(kv(k, "If you leave it off", para(k, format!("{} won't be installed. Everything else continues.", names.join(", ")))));
    for pk in list(t, "packages") {
        let mut line = format!("{} comes from {}.", s(pk, "name"), s(pk, "via"));
        if let Some(twin) = pk["sameAsCatalog"].as_str() {
            line.push_str(&format!(" This isn't the Catalog's {twin}."));
        }
        c = c.child(muted(k, line));
    }
    risky(k, c.child(muted(k, format!("Read it first: {}", s(t, "formulaUrl")))), on)
}

fn blocked_tap(k: &Kit, t: &Value) -> El {
    let owner = s(t, "owner");
    let p = list(t, "packages").iter().find(|x| x["sameAsBase"].as_bool() == Some(true)).unwrap_or(&Value::Null);
    let short = s(p, "short");
    let c = col()
        .w_full()
        .gap(6.0)
        .child(text(format!("Not offered: {}", s(p, "name"))).font(k.ty.text_small_medium).color(k.c.danger.fg).w_full().wrap())
        .child(para(k, format!("This isn't the {short} Soulcaster installs on every Mac. {owner} wrote it and gave it the same name, so you can't approve it here. It stays in your copy until you delete the line, and it never installs.")))
        .child(muted(k, format!("{}.", s(p, "via"))));
    rune::el().w_full().padding(14.0).radius(k.radius.md).fill(k.c.layer_01).border(1.0, k.c.danger.border).child(c)
}

fn repo_card(app: &App, k: &Kit, d: &Value) -> El {
    let repo = s(d, "repo");
    let commit = d["commit"].as_str().unwrap_or("");
    let own = d["own"].as_bool() == Some(true);
    let links = list(d, "links");
    let live: Vec<&Value> = links.iter().filter(|l| l["refused"].is_null()).collect();
    let mut c = col()
        .w_full()
        .gap(8.0)
        .child(
            Checkbox::new(app.boxes.repo)
                .label(format!("Link {} from {repo} at commit {commit}", word_count(live.len(), "file", "files")))
                .on_toggle(|a: &mut App| {
                    a.boxes.repo = !a.boxes.repo;
                    if !a.boxes.repo {
                        a.boxes.script = false;
                    }
                })
                .build(k)
                .key("approve-repo"),
        );
    if own && app.signed_in {
        c = c.child(para(k, "This repo belongs to your GitHub account, example-you, and so does this Config, so this starts on. Turn it off if you like."));
    } else if own {
        c = c.child(row().w_full().gap(10.0).center_y()
            .child(col().grow(1.0).min_w(0.0).child(para(k, "This may be yours, but Soulcaster can't tell until you sign in, so it starts off.")))
            .child(Button::new("Sign in to GitHub").on_press(|a: &mut App| { a.signed_in = true; let id = a.plan.clone(); a.open_review(&id) }).build(k)));
    } else if d["isGitHub"].as_bool() == Some(true) {
        c = c.child(para(k, format!("This repo doesn't belong to your GitHub account. {} controls what's in it.", s(d, "ownerLabel"))));
    } else {
        c = c.child(para(k, format!("This repo isn't on GitHub. It's on {}, so Soulcaster can't tell who owns it.", s(d, "host"))));
    }
    c = c
        .child(kv(k, "What this runs", para(k, "These files replace yours at the locations below, and some run by themselves when you use your Mac. No script is needed.")))
        .child(kv(k, "What's at stake", para(k, "Code in these files can do what you can: read your SSH keys and project folders, use your GitHub sign-in, and wait for the next time you type your password.")))
        .child(kv(k, "Later", para(k, format!("This covers commit {commit} and this list of links, nothing newer. Soulcaster shows you before it moves. If you pull by hand, links follow what's on disk."))))
        .child(kv(k, "If you leave it off", para(k, "The repo isn't downloaded and no file is linked. Everything else continues.")));
    let mut table = col().w_full().gap(6.0);
    for l in live {
        table = table.child(link_row(k, l));
    }
    c = c.child(table).child(muted(k, "A link with no note can still matter, because many programs run what they read. Your existing files are backed up before they're replaced."));
    risky(k, c, app.boxes.repo)
}

fn link_row(k: &Kit, l: &Value) -> El {
    let mut info = col().grow(1.0).min_w(0.0).gap(0.0);
    if l["lookalike"].as_bool() == Some(true) {
        info = info.child(colored(k, "Mixes alphabets. This isn't the file it looks like.", k.c.danger.fg));
    } else if let Some(n) = l["note"].as_str() {
        info = info.child(text(n.to_string()).font(k.ty.text_xsmall_medium).color(k.c.warning.fg).w_full().wrap());
    }
    row()
        .w_full()
        .items_start()
        .gap(12.0)
        .child(col().w(420.0).child(data(k, format!("{}  ←  {}", s(l, "targetEscaped"), s(l, "source")))))
        .child(info)
}

fn script_card(app: &App, k: &Kit, d: &Value) -> El {
    let Some(script) = d["script"].as_str() else { return rune::el() };
    let commit = d["commit"].as_str().unwrap_or("");
    let repo = s(d, "repo");
    let readable = d["scriptText"].is_string();
    let enabled = app.boxes.repo && readable;
    let mut c = col().w_full().gap(8.0).child(
        Checkbox::new(app.boxes.script)
            .label(format!("Also run {script} once, at commit {commit}"))
            .disabled(!enabled)
            .on_toggle(|a: &mut App| a.boxes.script = !a.boxes.script)
            .build(k)
            .key("approve-script"),
    );
    if !readable {
        return risky(k, c.child(para(k, format!("Soulcaster can't show this file: {repo} isn't on GitHub, and reading it elsewhere needs Git, which isn't on this Mac yet. A script you can't read can't be approved, so it stays off."))), false);
    }
    if !app.boxes.repo {
        c = c.child(para(k, "Turn the Dotfiles Repo on first. This script lives inside it."));
    }
    c = c
        .child(kv(k, "What this runs", para(k, "This one file, as you, after your password is gone. It can run other files in the repo, so treat this as approving all of them at this commit.")))
        .child(kv(k, "What's at stake", para(k, "It can do anything you can, including install software, change any of your files and send them anywhere.")))
        .child(kv(k, "Later", para(k, "It runs once for this commit. A newer commit needs a new approval. If it fails, the rest of the Run still finishes.")))
        .child(kv(k, "If you leave it off", para(k, "The script doesn't run. The links above aren't affected.")));
    let text_lines = s(d, "scriptText");
    let n = text_lines.lines().count();
    let mut box_ = col().w_full().gap(0.0);
    for line in text_lines.lines() {
        box_ = box_.child(data(k, line));
    }
    c = c.child(muted(k, format!("All {n} lines of {script}:")))
        .child(rune::el().w_full().padding(8.0).radius(k.radius.sm).fill(k.c.layer_02).border(1.0, k.c.border_weak).child(box_));
    risky(k, c, app.boxes.script)
}

// ---------------------------------------------------------------- what needs no approval

fn installs(app: &App, k: &Kit, p: &Value) -> El {
    let mut c = col().w_full().gap(10.0);
    let catalog = list(p, "catalog");
    if !catalog.is_empty() {
        let mut inner = col().w_full().gap(6.0).child(
            Checkbox::new(app.boxes.catalog)
                .label(format!("{} from the Catalog", word_count(catalog.len(), "Item", "Items")))
                .on_toggle(|a: &mut App| a.boxes.catalog = !a.boxes.catalog)
                .build(k)
                .key("safe-catalog"),
        );
        let titles: Vec<String> = catalog.iter().map(|i| s(i, "title").to_string()).collect();
        inner = inner.child(para(k, titles.join(", ")));
        for i in catalog {
            if let Some(n) = i["note"].as_str() {
                inner = inner.child(muted(k, format!("{}: {n}", s(i, "title"))));
            }
        }
        c = c.child(card(k, inner));
    }
    c = c.child(package_list(app, k, p, "officialFormulae", "formulae", "official Homebrew formula", "official Homebrew formulae"));
    c = c.child(package_list(app, k, p, "officialCasks", "casks", "official Homebrew cask", "official Homebrew casks"));
    let pulled = list(p, "pulledIn");
    let mut quiet = col().w_full().gap(4.0);
    for q in pulled {
        quiet = quiet.child(muted(k, format!("{} comes along because {} needs it.", s(q, "title"), s(q, "by"))));
    }
    quiet = quiet.child(muted(k, "Command Line Tools, Homebrew and the GitHub CLI: every Run installs these, whatever this Config says."));
    section(k, "Will install", "Needs no approval. Everything here comes from the Catalog or official Homebrew.", c.child(quiet))
}

fn package_list(app: &App, k: &Kit, p: &Value, field: &str, big_key: &str, one: &str, many: &str) -> El {
    let names = strs(p, field);
    if names.is_empty() {
        return rune::el();
    }
    let on = if big_key == "formulae" { app.boxes.formulae } else { app.boxes.casks };
    let big = strs(p, "bigLists").contains(&big_key);
    let key = format!("safe-{big_key}");
    let formulae = big_key == "formulae";
    let mut inner = col().w_full().gap(6.0).child(
        Checkbox::new(on)
            .label(if names.len() == 1 { format!("1 {one}") } else { format!("{} {many}", names.len()) })
            .on_toggle(move |a: &mut App| if formulae { a.boxes.formulae = !a.boxes.formulae } else { a.boxes.casks = !a.boxes.casks })
            .build(k)
            .key(key),
    );
    if big {
        inner = inner.child(text(format!("That's far more than a Config usually lists, so it starts off. Anything that isn't official Homebrew has already been pulled out above.")).font(k.ty.text_xsmall_medium).color(k.c.warning.fg).w_full().wrap());
    }
    let shown: Vec<&str> = if big && !app.show_all { names.iter().take(14).copied().collect() } else { names.clone() };
    let mut joined = shown.join(", ");
    if shown.len() < names.len() {
        joined.push_str(&format!(", and {} more", names.len() - shown.len()));
    }
    inner = inner.child(data(k, joined));
    if big && !app.show_all {
        inner = inner.child(row().child(Button::new(format!("Show all {}", names.len())).variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.show_all = true).build(k)));
    }
    card(k, inner)
}

fn settings(app: &App, k: &Kit, p: &Value) -> El {
    let rows = list(p, "settings");
    if rows.is_empty() {
        return rune::el();
    }
    let header = |a: &str, b: &str, c: &str, d: &str| {
        row()
            .w_full()
            .gap(12.0)
            .child(col().grow(1.0).min_w(0.0).child(colored(k, a, k.c.text_muted)))
            .child(col().w(150.0).child(colored(k, b, k.c.text_muted)))
            .child(col().w(100.0).child(colored(k, c, k.c.text_muted)))
            .child(col().w(120.0).child(colored(k, d, k.c.text_muted)))
    };
    let mut t = col().w_full().gap(8.0).child(
        Checkbox::new(app.boxes.settings)
            .label(word_count(rows.len(), "setting", "settings"))
            .on_toggle(|a: &mut App| a.boxes.settings = !a.boxes.settings)
            .build(k)
            .key("safe-settings"),
    ).child(header("Setting", "Now", "After", "Takes effect"));
    for r in rows {
        let note = s(r, "nowNote");
        let now = if note.is_empty() { s(r, "now").to_string() } else { format!("{} ({note})", s(r, "now")) };
        t = t.child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(col().grow(1.0).min_w(0.0).gap(0.0).child(para(k, s(r, "title"))).child(text(s(r, "id").to_string()).font(k.ty.mono).color(k.c.text_faint)))
                .child(col().w(150.0).child(para(k, now)))
                .child(col().w(100.0).child(strong(k, s(r, "new"))))
                .child(col().w(120.0).child(muted(k, s(r, "effect")))),
        );
        if r["replaces"].is_object() {
            let leaves = strs(&r["replaces"], "leaves").join(", ");
            let after = strs(&r["replaces"], "after").join(", ");
            t = t.child(
                rune::el().w_full().padding(8.0).radius(k.radius.sm).fill(k.c.warning.bg).child(
                    para(k, format!("This replaces your whole Dock. These leave it: {leaves}. The Dock will then hold only: {after}. The apps stay installed."))
                ),
            );
        }
        if let Some(n) = r["ifRepoOn"].as_str() {
            t = t.child(muted(k, n));
        }
    }
    section(k, "Will change settings", "Needs no approval. Soulcaster only writes settings it knows, and it records the old value first so you can go back.", card(k, t))
}

fn answers(app: &App, k: &Kit, p: &Value) -> El {
    let mut c = col().w_full().gap(10.0);
    for n in list(p, "needs") {
        if s(n, "kind") == "license" {
            c = c.child(card(k, col().w_full().gap(4.0)
                .child(Checkbox::new(app.boxes.license)
                    .label(format!("I accept the {} licenses", s(n, "item")))
                    .on_toggle(|a: &mut App| a.boxes.license = !a.boxes.license)
                    .build(k)
                    .key("answer-license"))
                .child(muted(k, format!("Read them: {}", s(n, "link"))))
                .child(muted(k, format!("Only you can accept these, and a Config can't. Leave it off and {} waits in your Needs-You Queue while everything else continues.", s(n, "item"))))));
        }
    }
    let later: Vec<&Value> = list(p, "needs").iter().filter(|n| s(n, "kind") == "later").collect();
    let mut asks = col().w_full().gap(4.0)
        .child(muted(k, "In the first minute of the Run: your Mac password, because this Mac needs admin work, and GitHub sign-in in your browser. Then you can walk away."));
    for n in later {
        let cond = n["onlyIf"].as_str().map(|t| format!(" (only if you approve {t})")).unwrap_or_default();
        asks = asks.child(muted(k, format!("After the Run, in your Needs-You Queue{cond}: {}", s(n, "text"))));
    }
    section(k, "What you'll be asked", "", c.child(asks))
}

fn refused_links(p: &Value) -> usize {
    list(&p["dotfiles"], "links").iter().filter(|l| !l["refused"].is_null()).count()
}

fn wont_apply(app: &App, k: &Kit, p: &Value) -> El {
    let mut c = col().w_full().gap(8.0);
    for l in list(&p["dotfiles"], "links").iter().filter(|l| !l["refused"].is_null()) {
        c = c.child(wont_row(k, s(l, "targetEscaped"), "Link refused", s(l, "refused")));
    }
    for w in list(p, "wontApply") {
        let kind = match s(w, "kind") {
            "unknown" | "unknown-setting" => "Unknown",
            "retired" => "Retired",
            "base" => "Always installed",
            "bad-value" => "Bad value",
            "machine-value" => "Personal value",
            "secret-key" => "Looks like a secret",
            _ => "Skipped",
        };
        c = c.child(wont_row(k, s(w, "id"), kind, s(w, "text")));
    }
    let _ = app;
    section(k, "Won't apply", "Skipped and reported. These are kept in your copy, and the rest of the Config carries on.", card(k, c))
}

fn wont_row(k: &Kit, id: &str, kind: &str, why: &str) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(col().w(260.0).child(data(k, id)))
        .child(col().w(110.0).child(muted(k, kind)))
        .child(col().grow(1.0).min_w(0.0).child(para(k, why)))
}

// ---------------------------------------------------------------- what a press does, said before it's pressed

fn approved_names(app: &App, p: &Value) -> (Vec<String>, Vec<String>) {
    let (mut on, mut off) = (vec![], vec![]);
    for t in list(p, "taps") {
        if t["blocked"].as_bool() == Some(true) {
            continue;
        }
        let name = s(t, "tap").to_string();
        if app.boxes.taps.get(&name).copied().unwrap_or(false) { on.push(name) } else { off.push(name) }
    }
    if !p["dotfiles"].is_null() {
        let d = &p["dotfiles"];
        let repo = format!("the Dotfiles Repo {}", s(d, "repo"));
        if app.boxes.repo { on.push(repo) } else { off.push(repo) }
        if let Some(script) = d["script"].as_str() {
            if app.boxes.script { on.push(script.to_string()) } else { off.push(script.to_string()) }
        }
    }
    (on, off)
}

pub fn sentence(app: &App, p: &Value) -> String {
    let (on, off) = approved_names(app, p);
    if app.nothing_selected() {
        return "Nothing is selected, so a Run would have nothing to do.".into();
    }
    match (on.is_empty(), off.is_empty()) {
        (true, true) => "Nothing here needs approval.".into(),
        (true, false) => format!("Left off: {}. The rest runs without them.", off.join(", ")),
        (false, true) => format!("You're allowing code from: {}.", on.join(", ")),
        (false, false) => format!("You're allowing code from: {}. Left off: {}.", on.join(", "), off.join(", ")),
    }
}

fn footer(app: &App, k: &Kit, p: &Value, warned: bool) -> El {
    let (on, off) = approved_names(app, p);
    let label = if on.is_empty() && !off.is_empty() {
        "Start Run without them".to_string()
    } else if !on.is_empty() {
        format!("Start Run with {}", word_count(on.len(), "approval", "approvals"))
    } else {
        "Start Run".to_string()
    };
    let start = Button::new(label)
        .variant(if warned { ButtonVariant::Outline } else { ButtonVariant::Contrast })
        .disabled(app.nothing_selected())
        .on_press(|a: &mut App| a.screen = Screen::FirstMinute)
        .build(k)
        .key("start-run");
    let mut buttons = row().w_full().gap(8.0).center_y();
    if warned {
        buttons = buttons
            .child(Button::new("Select what needs no approval").variant(ButtonVariant::Ghost).on_press(|a: &mut App| {
                let id = a.plan.clone();
                let p = plan(&id);
                let big = strs(p, "bigLists");
                a.boxes.catalog = true;
                a.boxes.settings = true;
                a.boxes.formulae = !big.contains(&"formulae");
                a.boxes.casks = !big.contains(&"casks");
            }).build(k).key("select-safe"))
            .child(rune::spacer())
            .child(start)
            .child(Button::new("Close without importing").variant(ButtonVariant::Contrast).on_press(|a: &mut App| a.screen = Screen::Paste).build(k));
    } else {
        buttons = buttons
            .child(Button::new("Cancel").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k))
            .child(rune::spacer())
            .child(Button::new("Keep a copy, don't run").on_press(|a: &mut App| a.screen = Screen::Save).build(k).key("keep-copy"))
            .child(start);
    }
    col().w_full().gap(8.0).child(para(k, sentence(app, p))).child(buttons)
}
