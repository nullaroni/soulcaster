//! `soulcaster import` and `soulcaster save`, on the same crate the window uses. They print plain
//! words and take the runner as a value, so everything but the last line of `main` runs on a fake.
//! Import keeps a copy only when asked and records no approvals: only a Run's start does that.

use std::fmt::Write;

use soulcaster_core::approval::Approvals;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::CommandRunner;
use soulcaster_core::config::Config;
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_repo::address::ConfigPath;
use soulcaster_repo::copies::keep_import;
use soulcaster_repo::import::{Fetched, ImportError, Importer, Opened, Picker};
use soulcaster_repo::review::{Primary, Review};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveOutcome, SaveRequest, save};
use soulcaster_repo::tools::Tools;

/// What a command came to: its words, and which exit code they go with.
#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    Ok(String),
    /// Refused: bad input, or a file that failed. Exit code 2.
    Refused(String),
    /// Kept on this Mac and not sent. Exit code 1.
    Waiting(String),
}

pub struct ImportArgs<'a> {
    pub paste: &'a str,
    pub config: Option<&'a str>,
    pub keep: bool,
}

pub fn import(
    runner: &dyn CommandRunner,
    tools: &Tools,
    catalog: &Catalog,
    args: &ImportArgs<'_>,
) -> Done {
    let importer = Importer::connect(runner, tools, catalog, DEFAULT_CLONE_LOCATION);
    let opened = match importer.open(args.paste) {
        Ok(opened) => opened,
        Err(error) => return refused(&error),
    };
    let fetched = match opened {
        Opened::One(fetched) => *fetched,
        Opened::Pick(picker) => match pick(&importer, &picker, args.config) {
            Ok(Some(fetched)) => fetched,
            Ok(None) => return Done::Ok(print_picker(&picker)),
            Err(done) => return done,
        },
    };
    let review = importer.review(&fetched);
    let mut text = print_review(&review);
    if args.keep {
        match keep_import(&tools.home, &fetched) {
            Ok(path) => {
                let _ = writeln!(
                    text,
                    "\nKept a copy at {}. No approval was recorded.",
                    path.display()
                );
            }
            Err(error) => return Done::Refused(error.to_string()),
        }
    } else {
        text.push_str("\nNothing was kept. Pass --keep to keep a copy on this Mac.\n");
    }
    Done::Ok(text)
}

fn refused(error: &ImportError) -> Done {
    match error {
        ImportError::Refused(refusal) => Done::Refused(format!(
            "{}\nThe whole file was refused, and nothing was kept.\n{}",
            refusal.words(),
            refusal.detail
        )),
        other => Done::Refused(other.to_string()),
    }
}

fn pick(
    importer: &Importer<'_>,
    picker: &Picker,
    wanted: Option<&str>,
) -> Result<Option<Fetched>, Done> {
    let Some(wanted) = wanted else {
        return Ok(None);
    };
    let path = ConfigPath::parse(wanted)
        .ok_or_else(|| Done::Refused("That isn't a Config in this repo.".to_string()))?;
    importer
        .pick(picker, &path)
        .map(Some)
        .map_err(|error| refused(&error))
}

fn print_picker(picker: &Picker) -> String {
    let mut text = format!(
        "{} at {} has several Configs. A Machine follows one, so pick with --config <path>.\n",
        picker.repo.full_name(),
        picker.commit.short()
    );
    for row in &picker.rows {
        match &row.refused {
            Some(refusal) => {
                let _ = writeln!(
                    text,
                    "  {}  refused: {}",
                    row.path.as_str(),
                    refusal.words()
                );
            }
            None => {
                let _ = writeln!(
                    text,
                    "  {}  {:?}: {} to install, {} settings, {} need your approval",
                    row.path.as_str(),
                    row.name.as_deref().unwrap_or(""),
                    row.install,
                    row.settings,
                    row.approvals
                );
            }
        }
    }
    if picker.truncated {
        text.push_str("  and more, which aren't shown.\n");
    }
    text
}

/// The review page, for a terminal. Every string from the file is quoted or was checked by the
/// schema first, so it can't move the cursor or rewrite what's above it.
fn print_review(review: &Review) -> String {
    let boxes = review.starting_boxes(&Approvals::default());
    let mut text = String::new();
    let line = |text: &mut String, words: String| {
        text.push_str(&words);
        text.push('\n');
    };
    line(&mut text, format!("Name in the file: {:?}", review.name));
    if review.name_has_lookalikes {
        line(&mut text, format!("Written out: {}", review.name_escaped));
    }
    line(
        &mut text,
        format!(
            "From: github.com/{}, {}, at {}",
            review.source.repo.full_name(),
            review.source.path.as_str(),
            review.source.commit.short()
        ),
    );
    text.push_str("Soulcaster doesn't verify or endorse any Config. Nothing has been installed, changed or run.\n");
    if !review.signals.is_empty() {
        text.push_str("\nThis file looks built to mislead you, so everything starts off.\n");
        for signal in &review.signals {
            line(&mut text, format!("  {}", signal.text));
        }
    }
    let word = if review.signals.is_empty() {
        ("to install", "settings to change")
    } else {
        ("could install", "settings could change")
    };
    line(
        &mut text,
        format!(
            "\n{} {}, {} {}, {} need your approval, {} won't apply",
            review.install_count(),
            word.0,
            review.settings.len(),
            word.1,
            review.approval_count(),
            review.wont_apply_count()
        ),
    );
    print_approvals(review, &mut text);
    if !review.wont_apply.is_empty() {
        text.push_str("\nWon't apply\n");
        for row in &review.wont_apply {
            line(&mut text, format!("  {}: {}", row.id, row.text));
        }
    }
    let button = match review.primary(&boxes) {
        Primary::Close => "Close without importing".to_string(),
        Primary::Start(words) => words,
    };
    line(
        &mut text,
        format!("\nIn the window, the button here says: {button}"),
    );
    text
}

fn print_approvals(review: &Review, text: &mut String) {
    if review.taps.is_empty() && review.dotfiles.is_none() {
        text.push_str("\nNothing here needs approval. Everything comes from the Catalog or official Homebrew.\n");
        return;
    }
    text.push_str("\nCan run someone else's code. Each stays off until you turn it on.\n");
    for tap in review.taps.iter().filter(|tap| tap.blocked) {
        for package in tap.packages.iter().filter(|p| p.same_as_base) {
            let _ = writeln!(text, "  Not offered: {} ({})", package.name, package.via);
        }
    }
    for tap in review.taps.iter().filter(|tap| !tap.blocked) {
        let _ = writeln!(text, "  Tap {}, code by {}", tap.tap, tap.owner);
        for package in &tap.packages {
            let _ = writeln!(text, "    {} ({})", package.name, package.via);
        }
    }
    if let Some(card) = &review.dotfiles {
        let at = card.commit.as_ref().map_or("unknown", |c| c.short());
        let whose = if card.own {
            "yours"
        } else if card.is_github {
            "not your account's"
        } else {
            "not on GitHub, so nobody can say whose"
        };
        let _ = writeln!(text, "  Dotfiles Repo {:?} at {at}, {whose}", card.repo);
        for link in &card.links {
            let status = link
                .refused
                .map(|why| format!("  refused: {why}"))
                .or(link.note.map(|note| format!("  {note}")))
                .unwrap_or_default();
            let _ = writeln!(
                text,
                "    {} <- {}{status}",
                link.target_escaped, link.source
            );
        }
        if let Some(script) = &card.script {
            match &card.script_text {
                Some(lines) => {
                    let _ = writeln!(text, "  Install script {script}, all of it:");
                    for script_line in lines.lines() {
                        let _ = writeln!(text, "    | {script_line}");
                    }
                }
                None => {
                    let _ = writeln!(
                        text,
                        "  Install script {script} can't be shown, so it can't be approved."
                    );
                }
            }
        }
    }
}

pub struct SaveArgs<'a> {
    pub path: &'a str,
    pub repo: Option<&'a str>,
}

pub fn save_config(
    runner: &dyn CommandRunner,
    tools: &Tools,
    catalog: &Catalog,
    args: &SaveArgs<'_>,
) -> Done {
    let bytes = match std::fs::read(args.path) {
        Ok(bytes) => bytes,
        Err(error) => return Done::Refused(format!("{}: {error}", args.path)),
    };
    let config = match Config::parse(&bytes) {
        Ok(config) => config,
        Err(error) => return Done::Refused(format!("{}: {error}", args.path)),
    };
    let request = SaveRequest {
        config: &config,
        repo_name: args.repo.unwrap_or(CONFIG_REPO_NAME),
        base: None,
    };
    match save(runner, tools, catalog, &request) {
        Err(error) => Done::Refused(error.to_string()),
        Ok(SaveOutcome::Sent {
            repo,
            path,
            public,
            created_repo,
            ..
        }) => {
            let mut text = format!("Saved {path} to {}.\n", repo.full_name());
            if created_repo {
                text.push_str("That's a new private repo on your GitHub account.\n");
            }
            if public {
                text.push_str("This Config is public. Anyone can read what you save.\n");
            }
            Done::Ok(text)
        }
        Ok(SaveOutcome::AlreadyThere { repo, path, .. }) => Done::Ok(format!(
            "{} already has this at {path}.\n",
            repo.full_name()
        )),
        Ok(SaveOutcome::ChangedElsewhere { path }) => Done::Waiting(format!(
            "{path} also changed on GitHub, so this wasn't saved. Choose which one to keep."
        )),
        Ok(SaveOutcome::Waiting { why, local }) => {
            Done::Waiting(format!("{} It's at {}.", why.words(), local.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use soulcaster_core::command::Reply;
    use soulcaster_repo::fake::{FakeGitHub, RepoShape};

    use super::*;

    const SHA: &str = "3f9a1c2000000000000000000000000000000000";

    fn tools(dir: &tempfile::TempDir) -> Tools {
        Tools::fixed(dir.path().to_path_buf())
    }

    fn github() -> FakeGitHub {
        let gh = FakeGitHub::new();
        gh.sign_in("example-you", 42);
        gh.add_repo("example-dev", "mac-setup", RepoShape::public("main", SHA));
        gh.add_file(
            "example-dev",
            "mac-setup",
            SHA,
            "configs/hostile.json",
            r#"{"schemaVersion":1,"name":"Verified safe Аpple","packages":{"casks":["evil-tools/tap/gh"]},
                "dotfiles":{"repo":"https://github.com.evil.example/a/b","links":{"~/.zshenv":"z","~/Documents":"d"}}}"#
                .as_bytes(),
        );
        gh
    }

    fn run_import(gh: &FakeGitHub, tools: &Tools, paste: &str, keep: bool) -> Done {
        import(
            gh,
            tools,
            Catalog::shipped(),
            &ImportArgs {
                paste,
                config: None,
                keep,
            },
        )
    }

    #[test]
    fn a_hostile_config_is_printed_as_data_with_close_as_the_button() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(&dir);
        let Done::Ok(text) = run_import(&github(), &tools, "example-dev/mac-setup", false) else {
            panic!();
        };
        assert!(text.contains("looks built to mislead"), "{text}");
        assert!(
            text.contains("Written out: Verified safe \\u0410pple"),
            "{text}"
        );
        assert!(text.contains("Not offered: evil-tools/tap/gh"), "{text}");
        assert!(text.contains("~/Documents <- d  refused"), "{text}");
        assert!(
            text.contains("~/.zshenv <- z  Runs for every zsh"),
            "{text}"
        );
        assert!(
            text.contains("the button here says: Close without importing"),
            "{text}"
        );
        assert!(text.contains("Nothing was kept"), "{text}");
        assert!(!dir.path().join("Library").exists(), "nothing was written");
    }

    #[test]
    fn keep_writes_one_copy_in_soulcasters_folder_and_no_approval() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(&dir);
        let Done::Ok(text) = run_import(&github(), &tools, "example-dev/mac-setup", true) else {
            panic!();
        };
        assert!(text.contains("No approval was recorded"), "{text}");
        let folder = dir
            .path()
            .join("Library/Application Support/Soulcaster/imports");
        let names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".source.json")));
        let kept = names.iter().find(|n| !n.ends_with(".source.json")).unwrap();
        let bytes = std::fs::read(folder.join(kept)).unwrap();
        let config = Config::parse(&bytes).expect("the copy is a valid Config");
        assert!(
            !String::from_utf8_lossy(&bytes).contains("example-dev"),
            "no provenance in the file"
        );
        assert_eq!(bytes, config.to_canonical_json().into_bytes());
    }

    #[test]
    fn several_configs_are_listed_and_one_is_picked_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(&dir);
        let gh = github();
        gh.add_file(
            "example-dev",
            "mac-setup",
            SHA,
            "configs/fine.json",
            br#"{"schemaVersion":1,"name":"Fine"}"#,
        );
        let Done::Ok(text) = run_import(&gh, &tools, "example-dev/mac-setup", false) else {
            panic!();
        };
        assert!(text.contains("pick with --config"), "{text}");
        assert!(text.contains("configs/fine.json"), "{text}");
        let Done::Ok(text) = import(
            &gh,
            &tools,
            Catalog::shipped(),
            &ImportArgs {
                paste: "example-dev/mac-setup",
                config: Some("configs/fine.json"),
                keep: false,
            },
        ) else {
            panic!();
        };
        assert!(text.contains("\"Fine\""), "{text}");
        assert!(matches!(
            import(
                &gh,
                &tools,
                Catalog::shipped(),
                &ImportArgs {
                    paste: "example-dev/mac-setup",
                    config: Some("../../etc/passwd.json"),
                    keep: false,
                },
            ),
            Done::Refused(_)
        ));
    }

    #[test]
    fn save_says_what_happened_and_waits_when_it_has_to() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(&dir);
        let path = dir.path().join("mine.json");
        std::fs::write(&path, br#"{"schemaVersion":1,"name":"My Mac"}"#).unwrap();
        let args = SaveArgs {
            path: path.to_str().unwrap(),
            repo: None,
        };
        let gh = FakeGitHub::new();
        gh.sign_in("example-you", 42);
        gh.on_git("rev-parse", Reply::ok(format!("{SHA}\n")));
        let Done::Ok(text) = save_config(&gh, &tools, Catalog::shipped(), &args) else {
            panic!();
        };
        assert!(text.contains("example-you/soulcaster-configs"), "{text}");
        assert!(text.contains("new private repo"), "{text}");

        let offline = FakeGitHub::new();
        offline.go_offline();
        assert!(matches!(
            save_config(&offline, &tools, Catalog::shipped(), &args),
            Done::Waiting(_)
        ));
    }

    #[test]
    fn a_repo_name_that_would_be_a_flag_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let tools = tools(&dir);
        let path = dir.path().join("mine.json");
        std::fs::write(&path, br#"{"schemaVersion":1,"name":"My Mac"}"#).unwrap();
        let gh = FakeGitHub::new();
        gh.sign_in("example-you", 42);
        let done = save_config(
            &gh,
            &tools,
            Catalog::shipped(),
            &SaveArgs {
                path: path.to_str().unwrap(),
                repo: Some("--upload-pack=evil"),
            },
        );
        assert!(matches!(done, Done::Waiting(_)), "{done:?}");
        assert!(
            !gh.calls()
                .iter()
                .any(|c| c.args.iter().any(|a| a.contains("upload-pack")))
        );
    }
}
