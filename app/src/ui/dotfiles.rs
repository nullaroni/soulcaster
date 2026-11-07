//! The Dotfiles Repo: which files link where, what happened when they did, and any newer version,
//! shown before anything moves. In a real Run the links happen during the Run, with no button: the
//! one here drives the same step on a temporary folder, until the wiring ticket connects it.

use rune::{El, ScrollSpec, col, el, row, text};
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{LinkOutcome, LinkReport, MovePreview};

use super::kit;
use crate::repo_flow::{DotfilesPhase, RepoFlow};
use crate::state::App;

fn link_row(target: &str, source: &str, status: &str, failed: bool) -> El {
    kit::ruled(
        format!("dot-{target}"),
        row()
            .help(status.to_string())
            .child(
                text(format!(
                    "{}  \u{2190}  {}",
                    soulcaster_repo::text::escaped(target),
                    soulcaster_repo::text::escaped(source)
                ))
                .font(kit::log_style())
                .color(kit::ink())
                .grow(1.0)
                .shrink()
                .truncate(),
            )
            .child(
                text(status.to_string())
                    .font(kit::ui_style())
                    .color(if failed { kit::mandarin() } else { kit::ink() }),
            ),
    )
}

fn plan_rows(plan: &DotfilesPlan, mut body: El, status: &str) -> El {
    for (target, source) in &plan.links {
        body = body.child(link_row(target, source, status, false));
    }
    body
}

fn report_rows(report: &LinkReport, mut body: El) -> El {
    for row_data in &report.rows {
        let (status, failed) = match &row_data.outcome {
            LinkOutcome::Linked { backup: Some(_) } => {
                ("Linked, old file backed up".to_string(), false)
            }
            LinkOutcome::Linked { backup: None } => ("Linked".to_string(), false),
            LinkOutcome::AlreadyLinked => ("Already linked".to_string(), false),
            LinkOutcome::Refused(why) => (why.to_string(), true),
        };
        body = body.child(link_row(
            &row_data.target,
            &row_data.source,
            &status,
            failed,
        ));
    }
    body
}

fn sentence(key: &str, words: &str, failed: bool) -> El {
    kit::ruled(
        format!("dot-{key}"),
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

fn preview_block(preview: &MovePreview, body: El) -> El {
    let words = if preview.is_complete() {
        "Dotfiles Repo: updates are available on GitHub."
    } else {
        "These updates are too long to show in full, so they can't be approved here."
    };
    body.child(sentence("behind", words, !preview.is_complete()))
        .child(
            col()
                .key("dot-diff")
                .w_full()
                .fill(kit::ink())
                .px(14.0)
                .py(10.0)
                .child(
                    text(preview.text().to_string())
                        .font(kit::log_style())
                        .color(kit::paper())
                        .selectable(true)
                        .w_full()
                        .wrap(),
                ),
        )
}

pub fn screen(app: &App) -> El {
    let plan = RepoFlow::dotfiles_plan();
    let mut body = col().key("dot-body").w_full().child(kit::ruled(
        "dot-repo".to_string(),
        row()
            .child(kit::label("Dotfiles Repo").grow(1.0))
            .child(kit::label(plan.repo.clone())),
    ));
    let footer = match &app.repo.dotfiles {
        DotfilesPhase::Approved => {
            body = plan_rows(&plan, body, "");
            kit::primary("dot-apply", "Link", true, |app: &mut App, _| {
                app.apply_dotfiles();
            })
        }
        DotfilesPhase::Applied(report) => {
            body = report_rows(report, body);
            kit::primary(
                "dot-review",
                "Review the updates",
                true,
                |app: &mut App, _| {
                    app.review_dotfiles_update();
                },
            )
        }
        DotfilesPhase::Failed(words) => {
            body = body.child(sentence("failed", words, true));
            kit::primary("dot-apply", "Link", true, |app: &mut App, _| {
                app.apply_dotfiles();
            })
        }
        DotfilesPhase::Behind(preview) => {
            body = preview_block(preview, body);
            kit::primary(
                "dot-move",
                "Move to this version",
                preview.is_complete(),
                |app: &mut App, _| {
                    app.move_dotfiles();
                },
            )
        }
        DotfilesPhase::Rewritten => {
            body = body.child(sentence(
                "rewritten",
                "Someone replaced the history, so Soulcaster can't show what changed. Treat this as a repo you've never seen.",
                true,
            ));
            kit::primary(
                "dot-stay",
                "Stay on this version",
                true,
                |app: &mut App, _| {
                    app.repo.dotfiles = DotfilesPhase::Approved;
                },
            )
        }
    };
    col()
        .key("dot-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Dotfiles")))
        .child(
            col()
                .key("dot-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .mt(14.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(kit::foot("dot-footer", el(), footer))
}
