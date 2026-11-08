//! Import: paste, pick one Config, and the one review page. Everything can be seen before anything
//! happens, and the things that run someone else's code each get their own box.

use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Kit, TextInput};
use soulcaster_repo::import::{PickRow, Picker, Refusal};
use soulcaster_repo::review::{DotfilesCard, LinkRow, Primary, TapCard, Tick};

use super::kit;
use crate::repo_flow::{ImportPhase, ReviewPage, SAMPLES};
use crate::state::App;

const TAP_STAKES: &str = "Homebrew runs code written by this account on this Mac, as you, after your password is gone. It can read what your account can. A later update of an approved package runs the newest formula with no new question, when you choose to update.";
const REPO_STAKES: &str = "These files replace yours at the locations below, and some run by themselves when you use your Mac. This covers this version and this list of links, nothing newer. Existing files are backed up before they're replaced. Leave it off and the repo isn't downloaded.";
const SCRIPT_STAKES: &str = "This one file runs once, as you, after your password is gone. It can run other files in the repo and do anything you can. A newer version needs a new approval.";
const OWN_WORDS: &str = "Yours, and so is this Config, so it starts on";
const FAKE_NOTE: &str = "Soulcaster doesn't verify or endorse any Config";

pub fn screen(app: &App, k: &'static Kit) -> El {
    match &app.repo.import {
        ImportPhase::Paste { error } => paste(app, k, error.as_deref()),
        ImportPhase::Pick(picker) => pick(picker),
        ImportPhase::Refused { from, refusal } => refused(from, refusal),
        ImportPhase::Review(page) => review(page, k),
        ImportPhase::Kept { name, path } => kept(name, path),
    }
}

fn page(key: &str, title: &str, body: El, foot: El) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title(title)))
        .child(
            col()
                .key(format!("{key}-scroll"))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .mt(14.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(foot)
}

fn group_row(key: &str, words: &str) -> El {
    row()
        .key(format!("group-{key}"))
        .w_full()
        .px(14.0)
        .pt(12.0)
        .pb(2.0)
        .child(kit::group(words))
}

/// A row: a name, and whatever goes on its right.
fn plain_row(key: String, name: impl Into<String>, right: El) -> El {
    kit::ruled(
        key,
        row()
            .child(kit::label(name).grow(1.0).shrink().truncate())
            .child(right),
    )
}

fn paint_row(key: String, words: impl Into<String>, color: Color) -> El {
    let words: String = words.into();
    kit::ruled(
        key,
        row().help(words.clone()).child(
            text(words)
                .font(kit::ui_style())
                .color(color)
                .grow(1.0)
                .shrink()
                .wrap(),
        ),
    )
}

/// The outlined form of the big button, for a second choice that can still be pressed.
fn outline<S: 'static>(
    key: &str,
    words: &str,
    enabled: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let mut button = row()
        .key(key.to_string())
        .role("button")
        .label(words.to_string())
        .h(28.0)
        .px(14.0)
        .radius(999.0)
        .center()
        .border(1.0, kit::ink())
        .child(
            text(words.to_string())
                .font(kit::bold_style())
                .color(kit::ink()),
        );
    if enabled {
        button = button
            .focusable(true)
            .focus_ring(2.0, 2.0, kit::forest())
            .hover(kit::ink().alpha(0.06))
            .on_press_tx(f);
    } else {
        button = button.opacity(0.4).disabled(true);
    }
    button
}

fn paste(app: &App, k: &Kit, error: Option<&str>) -> El {
    let input = TextInput::new(&app.repo.paste)
        .plain()
        .placeholder("owner/repo, or a link")
        .label("Repo or link to Import")
        .on_edit(|app: &mut App| &mut app.repo.paste)
        .on_submit(|app: &mut App, _| app.read_paste())
        .build(k)
        .key("import-paste");
    let field = col()
        .key("import-field")
        .w_full()
        .px(14.0)
        .pb(4.0)
        .child(input.h(22.0))
        .child(el().w_full().h(1.0).fill(if error.is_some() {
            kit::mandarin()
        } else {
            kit::ink()
        }));
    let mut body = col().key("import-body").w_full().child(field);
    if let Some(error) = error {
        body = body.child(paint_row(
            "import-error".to_string(),
            error.to_string(),
            kit::mandarin(),
        ));
    }
    if !app.engine.is_real() {
        body = body.child(group_row("samples", "Simulated repos"));
    }
    for sample in SAMPLES.iter().filter(|_| !app.engine.is_real()) {
        let name = (*sample).to_string();
        body = body.child(plain_row(
            format!("sample-{sample}"),
            *sample,
            kit::action(
                format!("sample-read-{sample}"),
                "Read",
                false,
                move |app: &mut App, _| app.read_sample(&name),
            ),
        ));
    }
    page(
        "import-screen",
        "Import",
        body,
        kit::foot(
            "import-footer",
            el(),
            kit::primary(
                "import-read",
                "Read",
                !app.repo.paste.text.trim().is_empty(),
                |app: &mut App, _| app.read_paste(),
            ),
        ),
    )
}

fn pick_row(row_data: &PickRow) -> El {
    let path = row_data.path.clone();
    let name = row_data.path.as_str().to_string();
    let key = format!("pick-{name}");
    let content = match &row_data.refused {
        Some(refusal) => row()
            .help(refusal.words().to_string())
            .child(kit::label(name).grow(1.0).shrink().truncate())
            .child(text("Refused").font(kit::ui_style()).color(kit::mandarin())),
        None => row()
            .child(
                kit::label(format!(
                    "{name}  {:?}",
                    row_data.name.as_deref().unwrap_or("")
                ))
                .grow(1.0)
                .shrink()
                .truncate(),
            )
            .child(kit::label(format!(
                "{} to install, {} settings, {} to approve",
                row_data.install, row_data.settings, row_data.approvals
            )))
            .child(kit::action(
                format!("pick-do-{}", path.as_str()),
                "Review",
                false,
                move |app: &mut App, _| app.pick_config(&path),
            )),
    };
    kit::ruled(key, content)
}

fn pick(picker: &Picker) -> El {
    let mut body = col().key("pick-body").w_full().child(plain_row(
        "pick-from".to_string(),
        "From",
        kit::label(format!(
            "github.com/{} at {}",
            picker.repo.full_name(),
            picker.commit.short()
        )),
    ));
    body = body.child(group_row("pick", "Pick one"));
    for row_data in &picker.rows {
        body = body.child(pick_row(row_data));
    }
    page(
        "pick-screen",
        "Import",
        body,
        kit::foot(
            "pick-footer",
            kit::action(
                "pick-back".to_string(),
                "Back",
                false,
                |app: &mut App, _| {
                    app.close_import();
                },
            ),
            el(),
        ),
    )
}

fn refused(from: &str, refusal: &Refusal) -> El {
    let body = col()
        .key("refused-body")
        .w_full()
        .child(plain_row(
            "refused-from".to_string(),
            "From",
            kit::label(soulcaster_repo::text::escaped(from)),
        ))
        .child(paint_row(
            "refused-why".to_string(),
            refusal.words(),
            kit::mandarin(),
        ))
        .child(
            col()
                .key("refused-detail")
                .w_full()
                .px(14.0)
                .py(10.0)
                .child(
                    text(refusal.detail.clone())
                        .font(kit::log_style())
                        .color(kit::ink())
                        .selectable(true)
                        .w_full()
                        .wrap(),
                ),
        );
    page(
        "refused-screen",
        "Refused",
        body,
        kit::foot(
            "refused-footer",
            kit::label("Nothing was kept"),
            kit::primary("refused-back", "Back", true, |app: &mut App, _| {
                app.close_import();
            }),
        ),
    )
}

fn kept(name: &str, path: &str) -> El {
    let body = col()
        .key("kept-body")
        .w_full()
        .child(plain_row(
            "kept-name".to_string(),
            "Name in the file",
            kit::label(format!("{name:?}")),
        ))
        .child(plain_row(
            "kept-path".to_string(),
            "Kept as",
            kit::label(path),
        ))
        .child(plain_row(
            "kept-approvals".to_string(),
            "Approvals",
            kit::label("None recorded"),
        ));
    page(
        "kept-screen",
        "Kept",
        body,
        kit::foot(
            "kept-footer",
            el(),
            kit::primary("kept-back", "Done", true, |app: &mut App, _| {
                app.close_import();
            }),
        ),
    )
}

/// A row with a box. The whole row presses. A rule on its left marks what can run someone else's
/// code, and it darkens only while the box is on.
// One argument per thing a row shows. A struct would only rename them at three call sites.
#[allow(clippy::too_many_arguments)]
fn tick_row(
    k: &Kit,
    key: String,
    words: String,
    on: bool,
    enabled: bool,
    risky: bool,
    help: &str,
    tick: Tick,
) -> El {
    let mut content = row()
        .role("checkbox")
        .label(words.clone())
        .checked(on)
        .help(help.to_string())
        .child(kit::check_box(k, on, false).opacity(if enabled { 1.0 } else { 0.35 }));
    if risky {
        content = content.child(el().w(3.0).h(18.0).fill(if on {
            kit::forest()
        } else {
            kit::ink().alpha(0.22)
        }));
    }
    content = content.child(
        text(words)
            .font(if on && risky {
                kit::bold_style()
            } else {
                kit::ui_style()
            })
            .color(if enabled { kit::ink() } else { kit::dim() })
            .grow(1.0)
            .shrink()
            .truncate(),
    );
    if enabled {
        content = content
            .focusable(true)
            .focus_ring(1.5, -1.5, kit::forest())
            .hover(kit::ink().alpha(0.05))
            .on_press(move |app: &mut App| app.review_tick(&tick));
    } else {
        content = content.disabled(true);
    }
    kit::ruled(key, content)
}

fn tap_rows(page_data: &ReviewPage, k: &Kit, tap: &TapCard) -> Vec<El> {
    if tap.blocked {
        return tap
            .packages
            .iter()
            .filter(|p| p.same_as_base)
            .map(|package| {
                kit::ruled(
                    format!("blocked-{}", package.name),
                    row()
                        .help(format!(
                            "This isn't the {} Soulcaster installs on every Mac. {} wrote it and gave it the same name, so you can't approve it here. It stays in your copy and never installs.",
                            package.short, tap.owner
                        ))
                        .child(kit::label(package.name.clone()).grow(1.0).shrink().truncate())
                        .child(text("Not offered").font(kit::bold_style()).color(kit::mandarin())),
                )
            })
            .collect();
    }
    let names: Vec<&str> = tap.packages.iter().map(|p| p.short.as_str()).collect();
    let on = page_data.boxes.taps.get(&tap.tap).copied().unwrap_or(false);
    let help = format!(
        "{TAP_STAKES} Read it first: {}. Came from: {}.",
        tap.formula_url,
        tap.packages
            .iter()
            .map(|p| format!("{} ({})", p.name, p.via))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut rows = vec![tick_row(
        k,
        format!("tap-{}", tap.tap),
        format!("{} from {}", names.join(", "), tap.tap),
        on,
        true,
        true,
        &help,
        Tick::Tap(tap.tap.clone()),
    )];
    for package in &tap.packages {
        rows.push(plain_row(
            format!("tap-via-{}", package.name),
            package.name.clone(),
            kit::label(package.via.clone()),
        ));
    }
    rows
}

fn link_row(link: &LinkRow) -> El {
    let (note, color) = match (link.refused, link.lookalike, link.note) {
        (Some(why), _, _) => (why, kit::mandarin()),
        (None, true, _) => (
            "Mixes alphabets. This isn't the file it looks like",
            kit::mandarin(),
        ),
        (None, false, Some(note)) => (note, kit::ink()),
        (None, false, None) => ("", kit::ink()),
    };
    kit::ruled(
        format!("link-{}", link.target_escaped),
        row()
            .help(if note.is_empty() {
                "A link with no note can still matter, because many programs run what they read."
                    .to_string()
            } else {
                note.to_string()
            })
            .child(
                text(format!(
                    "{}  \u{2190}  {}",
                    link.target_escaped, link.source_escaped
                ))
                .font(kit::log_style())
                .color(kit::ink())
                .grow(1.0)
                .shrink()
                .truncate(),
            )
            .child(text(note.to_string()).font(kit::ui_style()).color(color)),
    )
}

fn dotfiles_rows(page_data: &ReviewPage, k: &Kit, card: &DotfilesCard, list: &mut Vec<El>) {
    let at = card.commit.as_ref().map_or("unknown", |c| c.short());
    let live = card.links.iter().filter(|l| l.refused.is_none()).count();
    let whose = if card.own {
        OWN_WORDS.to_string()
    } else if card.names_your_repo {
        "This names your repo, but someone else wrote these links".to_string()
    } else if card.is_github {
        "Someone else controls what's in it".to_string()
    } else {
        format!(
            "Not on GitHub. It's on {}, so nobody can say whose",
            card.host.as_deref().unwrap_or("another site")
        )
    };
    list.push(tick_row(
        k,
        "approve-repo".to_string(),
        format!(
            "Link {live} files from {} at {at}",
            soulcaster_repo::text::escaped(&card.repo)
        ),
        page_data.boxes.repo,
        card.commit.is_some(),
        true,
        &format!("{whose}. {REPO_STAKES}"),
        Tick::Repo,
    ));
    list.push(plain_row(
        "repo-whose".to_string(),
        "Whose",
        kit::label(whose),
    ));
    for link in &card.links {
        list.push(link_row(link));
    }
    let Some(script) = &card.script else {
        return;
    };
    let can = page_data.boxes.repo && card.script_text.is_some();
    list.push(tick_row(
        k,
        "approve-script".to_string(),
        format!("Also run {script} once, at {at}"),
        page_data.boxes.script,
        can,
        true,
        SCRIPT_STAKES,
        Tick::Script,
    ));
    match &card.script_text {
        Some(lines) => list.push(
            col()
                .key("script-text")
                .w_full()
                .fill(kit::ink())
                .px(14.0)
                .py(10.0)
                .child(
                    text(lines.clone())
                        .font(kit::log_style())
                        .color(kit::paper())
                        .selectable(true)
                        .w_full()
                        .wrap(),
                ),
        ),
        None => list.push(plain_row(
            "script-unshown".to_string(),
            script.clone(),
            kit::label("Can't be shown, so it can't be approved"),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn package_rows(
    page_data: &ReviewPage,
    k: &Kit,
    kind: &str,
    names: &[String],
    on: bool,
    big: bool,
    tick: Tick,
    list: &mut Vec<El>,
) {
    if names.is_empty() {
        return;
    }
    list.push(tick_row(
        k,
        format!("safe-{kind}"),
        format!("{} official Homebrew {kind}", names.len()),
        on,
        true,
        false,
        if big {
            "That's far more than a Config usually lists, so it starts off."
        } else {
            "From official Homebrew. Needs no approval."
        },
        tick,
    ));
    let shown = big && !page_data.show_all;
    let mut joined = if shown {
        names[..14.min(names.len())].join(", ")
    } else {
        names.join(", ")
    };
    if shown && names.len() > 14 {
        joined.push_str(&format!(", and {} more", names.len() - 14));
    }
    let mut content = row().child(
        text(joined)
            .font(kit::log_style())
            .color(kit::ink())
            .grow(1.0)
            .shrink()
            .wrap(),
    );
    if shown {
        content = content.child(kit::action(
            format!("show-all-{kind}"),
            "Show all",
            false,
            |app: &mut App, _| app.review_show_all(),
        ));
    }
    list.push(kit::ruled(format!("names-{kind}"), content));
}

fn review(page_data: &ReviewPage, k: &Kit) -> El {
    let review = &page_data.review;
    let boxes = &page_data.boxes;
    let warned = !review.signals.is_empty();
    let mut list: Vec<El> = Vec::new();

    list.push(plain_row(
        "from".to_string(),
        "From",
        kit::label(format!(
            "github.com/{}  {}  at {}",
            review.source.repo.full_name(),
            review.source.path.as_str(),
            review.source.commit.short()
        )),
    ));
    list.push(kit::ruled(
        "name".to_string(),
        row()
            .help(FAKE_NOTE.to_string())
            .child(kit::label("Name in the file").grow(1.0))
            .child(kit::label(format!("{:?}", review.name)).shrink().truncate()),
    ));
    if review.name_has_lookalikes {
        list.push(plain_row(
            "name-written".to_string(),
            "Written out",
            kit::label(review.name_escaped.clone()).shrink().truncate(),
        ));
    }
    list.push(plain_row(
        "not-verified".to_string(),
        FAKE_NOTE,
        kit::label("Nothing has been installed yet"),
    ));

    if warned {
        list.push(
            row()
                .key("group-warning")
                .w_full()
                .px(14.0)
                .pt(12.0)
                .pb(2.0)
                .child(
                    text("Looks built to mislead you, so everything starts off")
                        .font(kit::bold_style())
                        .color(kit::mandarin()),
                ),
        );
        for (i, signal) in review.signals.iter().enumerate() {
            list.push(paint_row(
                format!("signal-{i}"),
                signal.text.clone(),
                kit::ink(),
            ));
        }
    }

    if review.taps.is_empty() && review.dotfiles.is_none() {
        list.push(plain_row(
            "no-approvals".to_string(),
            "Nothing here needs approval",
            kit::label("From the Catalog or official Homebrew"),
        ));
    } else {
        list.push(group_row("code", "Can run someone else's code"));
        for tap in review.taps.iter().filter(|t| t.blocked) {
            list.extend(tap_rows(page_data, k, tap));
        }
        for tap in review.taps.iter().filter(|t| !t.blocked) {
            list.extend(tap_rows(page_data, k, tap));
        }
        if let Some(card) = &review.dotfiles {
            dotfiles_rows(page_data, k, card, &mut list);
        }
    }

    list.push(group_row(
        "install",
        if warned {
            "Could install"
        } else {
            "Will install"
        },
    ));
    if !review.catalog.is_empty() {
        let names: Vec<&str> = review.catalog.iter().map(|c| c.name.as_str()).collect();
        list.push(tick_row(
            k,
            "safe-catalog".to_string(),
            format!("{} from the Catalog", review.catalog.len()),
            boxes.catalog,
            true,
            false,
            &names.join(", "),
            Tick::Catalog,
        ));
        list.push(kit::ruled(
            "catalog-names".to_string(),
            row().child(kit::label(names.join(", ")).grow(1.0).shrink().wrap()),
        ));
    }
    package_rows(
        page_data,
        k,
        "formulae",
        &review.official_formulae,
        boxes.formulae,
        review.big_formulae,
        Tick::Formulae,
        &mut list,
    );
    package_rows(
        page_data,
        k,
        "casks",
        &review.official_casks,
        boxes.casks,
        review.big_casks,
        Tick::Casks,
        &mut list,
    );
    for pulled in &review.pulled_in {
        list.push(plain_row(
            format!("pulled-{}", pulled.name),
            pulled.name.clone(),
            kit::label(format!("Needed by {}", pulled.by)),
        ));
    }
    list.push(plain_row(
        "always".to_string(),
        "Command Line Tools, Homebrew, GitHub CLI",
        kit::label("Every Run"),
    ));

    if !review.settings.is_empty() {
        list.push(group_row(
            "settings",
            if warned {
                "Could change settings"
            } else {
                "Will change settings"
            },
        ));
        list.push(tick_row(
            k,
            "safe-settings".to_string(),
            format!("{} settings", review.settings.len()),
            boxes.settings,
            true,
            false,
            "Soulcaster only writes settings it knows.",
            Tick::Settings,
        ));
        for setting in &review.settings {
            let mut content = row()
                .child(
                    kit::label(setting.title.clone())
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .child(kit::label(setting.after.clone()))
                .child(row().w(130.0).child(kit::label(setting.effect)));
            if let Some(note) = setting.if_repo_on {
                content = content.help(note.to_string());
            } else if setting.whole_list {
                content = content.help("This replaces the whole list.".to_string());
            }
            list.push(kit::ruled(format!("setting-{}", setting.id), content));
        }
    }

    list.push(group_row("asked", "What you'll be asked"));
    for license in &review.licenses {
        list.push(tick_row(
            k,
            format!("license-{}", license.id),
            format!("Accept the {} license", license.name),
            boxes.licenses.contains(&license.id),
            true,
            false,
            &license.text,
            Tick::License(license.id.clone()),
        ));
    }
    list.push(plain_row(
        "asked-first".to_string(),
        "Password and GitHub sign-in",
        kit::label("On the next screen"),
    ));
    for (i, words) in review.later.iter().enumerate() {
        list.push(kit::ruled(
            format!("later-{i}"),
            row()
                .help(words.clone())
                .child(kit::label(words.clone()).grow(1.0).shrink().truncate())
                .child(kit::label("After the Run")),
        ));
    }

    let refused_links = review
        .dotfiles
        .iter()
        .flat_map(|d| d.links.iter())
        .any(|l| l.refused.is_some());
    if !review.wont_apply.is_empty() || refused_links {
        list.push(group_row("wont", "Won't apply"));
        for row_data in &review.wont_apply {
            list.push(kit::ruled(
                format!("wont-{}", row_data.id),
                row()
                    .help(row_data.text.clone())
                    .child(
                        row()
                            .w(240.0)
                            .child(kit::label(row_data.id.clone()).shrink().truncate()),
                    )
                    .child(
                        kit::label(row_data.text.clone())
                            .grow(1.0)
                            .shrink()
                            .truncate(),
                    ),
            ));
        }
    }

    let mut body = col().key("review-body").w_full();
    for item in list {
        body = body.child(item);
    }
    let words = format!(
        "{} {}, {} {}, {} to approve, {} won't apply",
        review.install_count(),
        if warned {
            "could install"
        } else {
            "to install"
        },
        review.settings.len(),
        // A warned page's actions leave less room, and this line can't afford to be cut off.
        if warned {
            "settings"
        } else {
            "settings to change"
        },
        review.approval_count(),
        review.wont_apply_count()
    );
    col()
        .key("import-review-screen")
        .w_full()
        .h_full()
        .child(kit::title_pad(kit::title("Review")))
        .child(
            col()
                .key("import-review-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .mt(14.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(body),
        )
        .child(review_foot(page_data, words))
}

fn review_foot(page_data: &ReviewPage, counts: String) -> El {
    let review = &page_data.review;
    let boxes = &page_data.boxes;
    let nothing = review.nothing_selected(boxes);
    let sentence = review.sentence(boxes);
    let left = col()
        .key("review-words")
        .shrink()
        .help(sentence.clone())
        .child(
            text(counts)
                .font(kit::ui_style())
                .color(kit::ink())
                .truncate(),
        )
        .child(
            text(sentence)
                .font(kit::ui_style())
                .color(kit::ink())
                .truncate()
                .live_region(rune::LiveRegion::Polite),
        );
    let right = match review.primary(boxes) {
        Primary::Close => row()
            .gap(12.0)
            .center_y()
            .child(kit::action(
                "select-safe".to_string(),
                "Select what needs no approval",
                false,
                |app: &mut App, _| app.review_select_safe(),
            ))
            .child(outline(
                "start-import",
                &review.start_words(boxes),
                !nothing,
                |app: &mut App, _| app.start_from_review(),
            ))
            .child(kit::primary(
                "close-import",
                "Close without importing",
                true,
                |app: &mut App, _| app.close_import(),
            )),
        Primary::Start(words) => row()
            .gap(12.0)
            .center_y()
            .child(kit::action(
                "cancel-import".to_string(),
                "Cancel",
                false,
                |app: &mut App, _| app.close_import(),
            ))
            .child(kit::action(
                "keep-copy".to_string(),
                "Keep a copy",
                false,
                |app: &mut App, _| app.keep_copy(),
            ))
            .child(kit::primary(
                "start-import",
                &words,
                !nothing,
                |app: &mut App, _| app.start_from_review(),
            )),
    };
    kit::foot("import-review-footer", left, right)
}
