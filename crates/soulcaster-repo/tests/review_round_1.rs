//! Review round 1. These tests are the failures found in that review. They are meant to fail
//! on this branch until the code matches the assertion.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use soulcaster_core::approval::{
    ApprovalStore, ApprovalStoreError, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::address::{ConfigPath, RepoId};
use soulcaster_repo::dotfiles::{DotfilesStep, LinkOutcome, ScriptOutcome, UserStage};
use soulcaster_repo::fake::{FakeGitHub, FileApprovals};
use soulcaster_repo::import::{Fetched, Source};
use soulcaster_repo::review::{DotfilesFacts, Review};
use soulcaster_repo::save::{SaveOutcome, SaveRequest, save};
use soulcaster_repo::tools::Tools;

const SHA: &str = "a1b2c3d000000000000000000000000000000000";
const NEWER: &str = "b1b2c3d000000000000000000000000000000000";
const REPO: &str = "example-stranger/dotfiles";
const KEY: &str = "github.com/example-stranger/dotfiles";
const URL: &str = "https://github.com/example-stranger/dotfiles.git";

fn sha() -> CommitSha {
    CommitSha::parse(SHA).unwrap()
}

fn newer() -> CommitSha {
    CommitSha::parse(NEWER).unwrap()
}

fn catalog() -> Catalog {
    Catalog::parse(include_str!("fixtures/catalog-stub.json")).unwrap()
}

fn fetched(config: Config, owner: &str) -> Fetched {
    Fetched {
        source: Source {
            repo: RepoId::new(owner, "setup").unwrap(),
            path: ConfigPath::parse("configs/x.json").unwrap(),
            commit: sha(),
        },
        config,
    }
}

fn own_facts(commit: CommitSha) -> DotfilesFacts {
    DotfilesFacts {
        commit: Some(commit),
        current_owner: Some("example-you".to_string()),
        script_text: Some("echo hi\n".to_string()),
    }
}

fn review_own(bytes: &[u8], facts: &DotfilesFacts) -> Review {
    Review::build(
        &fetched(Config::parse(bytes).expect("config"), "example-you"),
        Some("example-you"),
        facts,
        &catalog(),
        DEFAULT_CLONE_LOCATION,
    )
}

struct Sandbox {
    _dir: tempfile::TempDir,
    _home: PathBuf,
    tools: Tools,
    gh: FakeGitHub,
    store: FileApprovals,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let clone = home.join("Developer/dotfiles");
    fs::create_dir_all(clone.join(".git")).unwrap();
    fs::create_dir_all(clone.join("zsh")).unwrap();
    fs::write(clone.join("zsh/zshrc"), "export EDITOR=nvim\n").unwrap();
    fs::write(clone.join("setup.sh"), "echo approved\n").unwrap();
    let tools = Tools::fixed(home.clone());
    let gh = FakeGitHub::new();
    gh.on_git("rev-parse", Reply::ok(format!("{SHA}\n")));
    gh.on_git("--get", Reply::ok(format!("{URL}\n")));
    let store = FileApprovals::new(home.join("approvals.json"));
    Sandbox {
        _dir: dir,
        _home: home,
        tools,
        gh,
        store,
    }
}

fn plan(links: &[(&str, &str)], script: Option<&str>) -> DotfilesPlan {
    DotfilesPlan {
        repo: REPO.to_string(),
        links: links
            .iter()
            .map(|(target, source)| (target.to_string(), source.to_string()))
            .collect(),
        refused_links: vec![],
        install_script: script.map(str::to_string),
    }
}

fn approve(store: &dyn ApprovalStore, links: &DotfilesPlan, script: Option<&str>) {
    store
        .write(&Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: sha(),
                links: links.links.clone(),
            }),
            script: script.map(|path| ScriptApproval {
                repo: KEY.to_string(),
                commit: sha(),
                path: path.to_string(),
                ran: false,
            }),
            ..Approvals::default()
        })
        .unwrap();
}

fn step<'a>(
    runner: &'a dyn soulcaster_core::command::CommandRunner,
    tools: &'a Tools,
    store: &'a dyn ApprovalStore,
) -> DotfilesStep<'a> {
    DotfilesStep {
        runner,
        tools,
        store,
        clone_location: DEFAULT_CLONE_LOCATION,
        stamp: "run-1",
    }
}

fn git(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let output = soulcaster_repo::process::runner()
        .run(
            &Command::new("/usr/bin/git")
                .args(args.iter().copied())
                .cwd(cwd)
                .env("HOME", home.display().to_string())
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            &mut |_, _| {},
        )
        .expect("git starts");
    assert!(
        output.is_success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout_text()
}

/// A real repo already at the approved commit, under a temporary home. No fetch.
struct RealRepo {
    _dir: tempfile::TempDir,
    home: PathBuf,
    tools: Tools,
    store: FileApprovals,
    commit: CommitSha,
}

fn real_repo(files: &[(&str, &str)]) -> RealRepo {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let clone = home.join("Developer/dotfiles");
    fs::create_dir_all(&clone).unwrap();
    git(&home, &clone, &["init", "--quiet", "--initial-branch=main"]);
    git(&home, &clone, &["remote", "add", "origin", URL]);
    for (path, text) in files {
        let at = clone.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, text).unwrap();
    }
    git(&home, &clone, &["add", "--all"]);
    git(&home, &clone, &["commit", "--quiet", "-m", "approved"]);
    let commit = CommitSha::parse(git(&home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    let mut tools = Tools::fixed(home.clone());
    tools.git = Some(PathBuf::from("/usr/bin/git"));
    tools.zsh = PathBuf::from("/bin/zsh");
    let store = FileApprovals::new(home.join("approvals.json"));
    RealRepo {
        _dir: dir,
        home,
        tools,
        store,
        commit,
    }
}

#[test]
fn your_own_repo_does_not_start_selected_for_a_newer_commit_or_one_more_link() {
    let own = include_bytes!("fixtures/import/own-dotfiles.json");
    let facts = own_facts(sha());
    let first = review_own(own, &facts);
    let boxes = first.starting_boxes(&Approvals::default());
    assert!(
        boxes.repo,
        "the first review of your own repo starts the repo on"
    );
    let approvals = first.approvals_after_run(&boxes, &Approvals::default());

    let newer_review = review_own(own, &own_facts(newer()));
    let newer_on = newer_review.starting_boxes(&approvals).repo;

    let mut value: serde_json::Value = serde_json::from_slice(own).unwrap();
    value["dotfiles"]["links"]["~/.zshenv"] = "zsh/zshenv".into();
    let extra = serde_json::to_vec(&value).unwrap();
    let extra_review = review_own(&extra, &facts);
    let extra_on = extra_review.starting_boxes(&approvals).repo;

    assert!(
        !newer_on && !extra_on,
        "your own repo started selected for a newer commit ({newer_on}) and for one more link ({extra_on})"
    );
}

#[test]
fn a_dangerous_location_keeps_its_note_when_only_the_case_changes() {
    let json = br#"{
        "schemaVersion": 1,
        "name": "Work",
        "dotfiles": {
            "repo": "example-stranger/dotfiles",
            "links": {
                "~/.ZSHENV": "zshenv",
                "~/.Gitconfig": "gitconfig",
                "~/library/launchagents/evil.plist": "evil.plist"
            }
        }
    }"#;
    let review = Review::build(
        &fetched(
            Config::parse(json).expect("schema accepts these paths"),
            "example-stranger",
        ),
        Some("example-you"),
        &DotfilesFacts {
            commit: Some(sha()),
            current_owner: Some("example-stranger".to_string()),
            script_text: None,
        },
        &catalog(),
        DEFAULT_CLONE_LOCATION,
    );
    let note = |target: &str| {
        review
            .dotfiles
            .as_ref()
            .unwrap()
            .links
            .iter()
            .find(|link| link.target == target)
            .unwrap_or_else(|| panic!("{target} missing"))
            .note
    };
    assert_eq!(
        note("~/.ZSHENV"),
        Some("Runs for every zsh, even in scripts")
    );
    assert_eq!(
        note("~/.Gitconfig"),
        Some("Can run programs during git commands and redirect clones")
    );
    assert_eq!(
        note("~/library/launchagents/evil.plist"),
        Some("Starts a program every time you log in")
    );
}

#[test]
fn a_mixed_alphabet_repo_path_starts_the_page_off() {
    let es = '\u{0441}';
    let json = format!(
        r#"{{"schemaVersion":1,"name":"Work","dotfiles":{{"repo":"example-you/dotfiles","links":{{"~/.zshrc":"zsh/zshr{es}"}}}}}}"#
    );
    let review = Review::build(
        &fetched(
            Config::parse(json.as_bytes()).expect("schema accepts the path"),
            "example-you",
        ),
        Some("example-you"),
        &own_facts(sha()),
        &catalog(),
        DEFAULT_CLONE_LOCATION,
    );
    let row = review
        .dotfiles
        .as_ref()
        .unwrap()
        .links
        .iter()
        .find(|link| link.target == "~/.zshrc")
        .unwrap();
    assert!(
        row.lookalike,
        "source {} is shown as a plain path",
        row.source
    );
    assert!(
        review
            .signals
            .iter()
            .any(|signal| signal.code == "link-lookalike"),
        "signals: {:?}",
        review
            .signals
            .iter()
            .map(|signal| signal.code)
            .collect::<Vec<_>>()
    );
    assert!(
        !review.starting_boxes(&Approvals::default()).repo,
        "your own repo started selected while a link path mixes alphabets"
    );
}

#[test]
fn a_cut_off_diff_cannot_be_approved() {
    let sandbox = sandbox();
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], None);
    approve(&sandbox.store, &links, None);
    let marker = "TAIL_NOT_SHOWN";
    let body = format!("{}{marker}", "x".repeat(300 * 1024));
    sandbox.gh.on_git("diff", Reply::ok(body));
    let preview = step(&sandbox.gh, &sandbox.tools, &sandbox.store)
        .preview_move(&links, &newer())
        .expect("diff exit 0 became a preview");
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{NEWER}\n")));
    let moved =
        step(&sandbox.gh, &sandbox.tools, &sandbox.store).move_after_review(&links, &preview);
    assert!(
        preview.text().contains(marker) || moved.is_err(),
        "a {}-byte preview dropped the end of the diff and was accepted: {moved:?}",
        preview.text().len()
    );
}

#[test]
fn a_failed_diff_is_not_a_preview() {
    let sandbox = sandbox();
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], None);
    approve(&sandbox.store, &links, None);
    sandbox
        .gh
        .on_git("diff", Reply::fails(2, "fatal: bad object"));
    let preview = step(&sandbox.gh, &sandbox.tools, &sandbox.store).preview_move(&links, &newer());
    assert!(
        preview.is_err(),
        "a failed diff became a preview: {:?}",
        preview.ok().map(|item| item.text().to_string())
    );
}

#[test]
fn a_dirty_worktree_at_the_approved_commit_is_not_linked() {
    let repo = real_repo(&[("zsh/zshrc", "approved-bytes\n")]);
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], None);
    repo.store
        .write(&Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: repo.commit.clone(),
                links: links.links.clone(),
            }),
            ..Approvals::default()
        })
        .unwrap();
    let runner = soulcaster_repo::process::runner();
    fs::write(
        repo.home.join("Developer/dotfiles/zsh/zshrc"),
        "dirty-bytes\n",
    )
    .unwrap();
    let report = step(&runner, &repo.tools, &repo.store)
        .apply_links(&links, UserStage::password_is_gone())
        .unwrap_or_else(|error| panic!("apply failed before the link decision: {error}"));
    let row = report
        .rows
        .iter()
        .find(|row| row.target == "~/.zshrc")
        .unwrap();
    assert!(
        matches!(row.outcome, LinkOutcome::Refused(_)),
        "HEAD matched and the dirty bytes were linked: {row:?}"
    );
    let linked = repo.home.join(".zshrc");
    if linked.exists() {
        assert_ne!(
            fs::read_to_string(&linked).unwrap_or_default(),
            "dirty-bytes\n"
        );
    }
}

#[test]
fn assume_unchanged_cannot_hide_a_different_install_script() {
    let repo = real_repo(&[("setup.sh", "echo approved\n"), ("zsh/zshrc", "ok\n")]);
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], Some("setup.sh"));
    let clone = repo.home.join("Developer/dotfiles");
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();
    git(
        &repo.home,
        &clone,
        &["update-index", "--assume-unchanged", "setup.sh"],
    );
    let status = git(&repo.home, &clone, &["status", "--porcelain=v1"]);
    assert!(
        status.trim().is_empty(),
        "the fixture should be hidden from status, got {status:?}"
    );
    repo.store
        .write(&Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: repo.commit.clone(),
                links: links.links.clone(),
            }),
            script: Some(ScriptApproval {
                repo: KEY.to_string(),
                commit: repo.commit.clone(),
                path: "setup.sh".to_string(),
                ran: false,
            }),
            ..Approvals::default()
        })
        .unwrap();
    let runner = soulcaster_repo::process::runner();
    let outcome = step(&runner, &repo.tools, &repo.store).run_script(
        &links,
        UserStage::password_is_gone(),
        &mut |_, _| {},
    );
    let marker = clone.join("pwned-marker");
    assert_eq!(
        (outcome, marker.exists()),
        (ScriptOutcome::HeldForReview, false),
        "status hid the edit and the other bytes ran"
    );
}

#[test]
fn the_install_script_does_not_run_twice_when_recording_it_fails() {
    let sandbox = sandbox();
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], Some("setup.sh"));
    let store = FailWrite {
        inner: Mutex::new(Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: sha(),
                links: links.links.clone(),
            }),
            script: Some(ScriptApproval {
                repo: KEY.to_string(),
                commit: sha(),
                path: "setup.sh".to_string(),
                ran: false,
            }),
            ..Approvals::default()
        }),
    };
    let once = step(&sandbox.gh, &sandbox.tools, &store).run_script(
        &links,
        UserStage::password_is_gone(),
        &mut |_, _| {},
    );
    assert_eq!(once, ScriptOutcome::Ran { success: true }, "{once:?}");
    let twice = step(&sandbox.gh, &sandbox.tools, &store).run_script(
        &links,
        UserStage::password_is_gone(),
        &mut |_, _| {},
    );
    assert_eq!(
        twice,
        ScriptOutcome::AlreadyRan,
        "a failed record let the script run again: {twice:?}"
    );
}

/// Drops the approval on the third read, which is the last read before the script runs, then
/// watches whether the trailing write puts that approval back.
struct RevokeAfterSnapshot {
    inner: Mutex<Approvals>,
    reads: AtomicUsize,
}

impl ApprovalStore for RevokeAfterSnapshot {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        let n = self.reads.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.inner.lock().expect("lock");
        let snapshot = guard.clone();
        if n == 2 {
            *guard = Approvals::default();
        }
        Ok(snapshot)
    }

    fn write(&self, approvals: &Approvals) -> Result<(), ApprovalStoreError> {
        *self.inner.lock().expect("lock") = approvals.clone();
        Ok(())
    }
}

#[test]
fn finishing_the_script_does_not_restore_a_revoked_approval() {
    let sandbox = sandbox();
    let links = plan(&[("~/.zshrc", "zsh/zshrc")], Some("setup.sh"));
    let store = RevokeAfterSnapshot {
        inner: Mutex::new(Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: sha(),
                links: links.links.clone(),
            }),
            script: Some(ScriptApproval {
                repo: KEY.to_string(),
                commit: sha(),
                path: "setup.sh".to_string(),
                ran: false,
            }),
            ..Approvals::default()
        }),
        reads: AtomicUsize::new(0),
    };
    let outcome = step(&sandbox.gh, &sandbox.tools, &store).run_script(
        &links,
        UserStage::password_is_gone(),
        &mut |_, _| {},
    );
    assert_eq!(outcome, ScriptOutcome::Ran { success: true }, "{outcome:?}");
    let after = store.read().expect("the record can be read");
    assert!(
        after.dotfiles.is_none() && after.script.is_none(),
        "the script wrote its old snapshot back over the revocation: {after:?}"
    );
}

struct FailWrite {
    inner: Mutex<Approvals>,
}

impl ApprovalStore for FailWrite {
    fn read(&self) -> Result<Approvals, ApprovalStoreError> {
        Ok(self.inner.lock().expect("lock").clone())
    }

    fn write(&self, _: &Approvals) -> Result<(), ApprovalStoreError> {
        Err(ApprovalStoreError("disk full".to_string()))
    }
}

#[test]
fn a_save_does_not_overwrite_a_different_config_with_the_same_slug() {
    let dir = tempfile::tempdir().unwrap();
    let mut tools = Tools::fixed(fs::canonicalize(dir.path()).unwrap());
    tools.gh = None;
    tools.git = None;
    let runner = FakeGitHub::new();
    let first =
        Config::parse(br#"{"schemaVersion":1,"name":"My Mac","items":["ghostty"]}"#).unwrap();
    let second =
        Config::parse(br#"{"schemaVersion":1,"name":"My-Mac","items":["ghostty"]}"#).unwrap();
    let catalog = Catalog::shipped();
    let outcome = save(
        &runner,
        &tools,
        catalog,
        &SaveRequest {
            config: &first,
            repo_name: "soulcaster-configs",
            base: None,
        },
    )
    .unwrap();
    let SaveOutcome::Waiting { local, .. } = outcome else {
        panic!("{outcome:?}");
    };
    let original = fs::read(&local).unwrap();
    let second_outcome = save(
        &runner,
        &tools,
        catalog,
        &SaveRequest {
            config: &second,
            repo_name: "soulcaster-configs",
            base: None,
        },
    );
    let after = fs::read(&local).unwrap();
    assert_eq!(
        after,
        original,
        "saving \"My-Mac\" replaced \"My Mac\" at {}: {second_outcome:?}",
        local.display()
    );
}

#[test]
fn an_ambiguous_approval_record_approves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.json");
    // A repeated link key. Struct fields already fail closed; the link map must too, because
    // serde otherwise keeps the last value and that value is what a later Run links.
    let body = format!(
        r#"{{"dotfiles":{{"repo":"{KEY}","commit":"{SHA}","links":{{"~/.zshrc":"safe.sh","~/.zshrc":"evil.sh"}}}}}}"#
    );
    fs::write(&path, body).unwrap();
    let store = FileApprovals::new(path);
    match store.read() {
        Err(_) => {}
        Ok(approvals) => {
            let linked = approvals
                .dotfiles
                .as_ref()
                .and_then(|held| held.links.get("~/.zshrc"))
                .map(String::as_str);
            assert_ne!(
                linked,
                Some("evil.sh"),
                "the last duplicate key was approved: {linked:?}"
            );
        }
    }
}
