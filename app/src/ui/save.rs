//! Save: the selection as a Config, on the person's own GitHub repo. It works with nothing saved
//! yet, and a Run never waits on it. No Git words reach this screen.

use rune::{El, ScrollSpec, col, el, row, text};
use soulcaster_repo::copies::slug;
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveOutcome};

use super::kit;
use crate::repo_flow::SavePhase;
use crate::state::App;

fn line(key: &str, name: &str, value: String) -> El {
    kit::ruled(
        format!("save-{key}"),
        row()
            .child(kit::label(name.to_string()).grow(1.0))
            .child(kit::label(value).shrink().truncate()),
    )
}

fn sentence(key: &str, words: &str, failed: bool) -> El {
    kit::ruled(
        format!("save-{key}"),
        row().help(words.to_string()).child(
            text(words.to_string())
                .font(kit::ui_style())
                .color(if failed { kit::mandarin() } else { kit::ink() })
                .grow(1.0)
                .shrink()
                .wrap(),
        ),
    )
}

pub fn screen(app: &App) -> El {
    let config = app.config();
    let counted = config.items.len() + config.packages.formulae.len() + config.packages.casks.len();
    let mut body = col()
        .key("save-body")
        .w_full()
        .child(line("name", "Machine Type", format!("{:?}", config.name)))
        .child(line(
            "holds",
            "Holds",
            format!("{counted} to install, {} settings", config.settings.len()),
        ))
        .child(line(
            "file",
            "File",
            format!("configs/{}.json", slug(&config.name)),
        ));
    let mut primary = "Save to GitHub";
    match &app.repo.save {
        SavePhase::Ready => {
            body = body.child(line(
                "where",
                "Goes to",
                format!("example-you/{CONFIG_REPO_NAME}, private if it's new"),
            ));
        }
        SavePhase::Blocked(words) => {
            body = body.child(sentence("blocked", words, true));
        }
        SavePhase::Done(SaveOutcome::Sent {
            repo,
            public,
            created_repo,
            ..
        }) => {
            body = body.child(line("saved", "Saved to", repo.full_name()));
            if *created_repo {
                body = body.child(line("made", "New repo", "Private".to_string()));
            }
            if *public {
                body = body.child(sentence(
                    "public",
                    "This Machine Type is public. Anyone can read what you save.",
                    false,
                ));
            }
            primary = "Save again";
        }
        SavePhase::Done(SaveOutcome::AlreadyThere { repo, public, .. }) => {
            body = body.child(line("saved", "Already on", repo.full_name()));
            if *public {
                body = body.child(sentence(
                    "public",
                    "This Machine Type is public. Anyone can read what you save.",
                    false,
                ));
            }
        }
        SavePhase::Done(SaveOutcome::ChangedElsewhere { path }) => {
            body = body.child(sentence(
                "elsewhere",
                &format!("{path} also changed in your Machine Type, so this wasn't saved. Choose which one to keep."),
                true,
            ));
        }
        SavePhase::Done(SaveOutcome::Waiting { why, .. }) => {
            body = body.child(sentence("waiting", why.words(), false));
            primary = "Send to GitHub";
        }
    }
    col()
        .key("save-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Save")))
        .child(
            col()
                .key("save-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .mt(14.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(kit::foot(
            "save-footer",
            el(),
            kit::primary("save-do", primary, true, |app: &mut App, _| {
                app.save_config();
            }),
        ))
}
