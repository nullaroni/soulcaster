//! Review round 6. A test here fails on this commit for the reason in its name.

use std::fs;
use std::path::{Path, PathBuf};

use macflayer_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use macflayer_core::command::{Command, CommandRunner};
use macflayer_core::lint::DEFAULT_CLONE_LOCATION;
use macflayer_core::plan::DotfilesPlan;
use macflayer_repo::dotfiles::{DotfilesStep, LinkOutcome, LinkRefusal, ScriptOutcome, UserStage};
use macflayer_repo::fake::FileApprovals;
use macflayer_repo::tools::Tools;

const URL: &str = "https://github.com/example-stranger/dotfiles.git";
const KEY: &str = "github.com/example-stranger/dotfiles";

fn git(home: &Path, cwd: &Path, args: &[&str]) -> String {
    let output = macflayer_repo::process::runner()
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

fn step<'a>(repo: &'a RealRepo, runner: &'a dyn CommandRunner) -> DotfilesStep<'a> {
    DotfilesStep {
        runner,
        tools: &repo.tools,
        store: &repo.store,
        clone_location: DEFAULT_CLONE_LOCATION,
        stamp: "run-1",
    }
}

fn plan(script: &str) -> DotfilesPlan {
    DotfilesPlan {
        repo: "example-stranger/dotfiles".to_string(),
        links: [("~/.zshrc".to_string(), "zsh/zshrc".to_string())].into(),
        refused_links: vec![],
        install_script: Some(script.to_string()),
    }
}

fn attempt(repo: &RealRepo, links: &DotfilesPlan) -> (LinkOutcome, ScriptOutcome, bool) {
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
                path: links.install_script.clone().unwrap(),
                ran: false,
            }),
            ..Approvals::default()
        })
        .unwrap();
    let runner = macflayer_repo::process::runner();
    let report = step(repo, &runner)
        .apply_links(links, UserStage::password_is_gone())
        .expect("the clone's commit id is the approved one, so linking is attempted");
    let script =
        step(repo, &runner).run_script(links, UserStage::password_is_gone(), &mut |_, _| {});
    let marker = repo.home.join("Developer/dotfiles/pwned-marker").exists();
    (report.rows[0].outcome.clone(), script, marker)
}

fn refused() -> (LinkOutcome, ScriptOutcome, bool) {
    (
        LinkOutcome::Refused(LinkRefusal::RepoChanged),
        ScriptOutcome::HeldForReview,
        false,
    )
}

fn clone_of(repo: &RealRepo) -> PathBuf {
    repo.home.join("Developer/dotfiles")
}

fn recode(repo: &RealRepo) {
    let clone = clone_of(repo);
    git(&repo.home, &clone, &["add", "--all"]);
    git(
        &repo.home,
        &clone,
        &["commit", "--quiet", "-m", "approved again"],
    );
}

/// The runner drops every `\r` before `tree_entries` splits `ls-tree -z`. `setup.sh` and
/// `setup.sh\r` become one path, and the later id, the other file's blob, is the one kept.
/// The worktree file at `setup.sh` is that other blob, so the check matches and `zsh -f` runs it.
#[test]
fn a_carriage_return_in_a_path_runs_a_different_blob_than_the_approved_script() {
    let repo = real_repo(&[
        ("zsh/zshrc", "approved\n"),
        ("setup.sh", "exit 0\n"),
        ("setup.sh\r", "touch pwned-marker\n"),
    ]);
    let clone = clone_of(&repo);
    assert_eq!(
        git(&repo.home, &clone, &["show", "HEAD:setup.sh"]),
        "exit 0\n",
        "fixture: the approved setup.sh blob does not create a marker"
    );
    assert_eq!(
        git(&repo.home, &clone, &["show", "HEAD:setup.sh\r"]),
        "touch pwned-marker\n",
        "fixture: the other path is the program that creates the marker"
    );
    fs::remove_file(clone.join("setup.sh\r")).unwrap();
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();
    let status = git(&repo.home, &clone, &["status", "--porcelain=v1"]);
    assert!(
        status.contains("setup.sh"),
        "fixture: git itself sees the worktree is not the commit: {status:?}"
    );

    let links = plan("setup.sh");
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "a carriage return hid a different script at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}

/// `.DS_Store` is skipped on the walk and is not in this commit. The approved `zsh/zshrc` is a
/// symlink to `../.DS_Store`, and the install script is `.DS_Store`. Those bytes are never hashed,
/// then the link follows them and `zsh -f` runs them.
#[test]
fn a_skipped_ds_store_is_what_the_link_follows_and_the_script_runs() {
    let mut repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = clone_of(&repo);
    fs::remove_file(clone.join("zsh/zshrc")).unwrap();
    // Relative to `zsh/`, so this is the repo-root `.DS_Store`, which the walk skips.
    std::os::unix::fs::symlink("../.DS_Store", clone.join("zsh/zshrc")).unwrap();
    recode(&repo);
    repo.commit = CommitSha::parse(git(&repo.home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    assert_eq!(
        git(&repo.home, &clone, &["show", "HEAD:zsh/zshrc"]).trim(),
        "../.DS_Store",
        "fixture: the approved zshrc blob is the symlink target, not a script"
    );
    assert!(
        !git(&repo.home, &clone, &["ls-tree", "--name-only", "HEAD"])
            .lines()
            .any(|line| line == ".DS_Store"),
        "fixture: the approved tree has no .DS_Store blob"
    );
    fs::write(clone.join(".DS_Store"), "touch pwned-marker\n").unwrap();
    let status = git(&repo.home, &clone, &["status", "--porcelain=v1"]);
    assert!(
        status.contains(".DS_Store"),
        "fixture: git itself lists the extra file: {status:?}"
    );

    let links = plan(".DS_Store");
    let (outcome, script, marker) = attempt(&repo, &links);
    let followed = fs::read_to_string(repo.home.join(".zshrc")).unwrap_or_default();
    // Changed from one exact pair of outcomes: the fix refuses a `.DS_Store` source and script
    // before the tree check, as `SourceLeavesRepo` and `NotApproved`. What's proven is unchanged:
    // the link is refused, the script doesn't run, and nothing was followed.
    assert!(
        matches!(outcome, LinkOutcome::Refused(_))
            && !matches!(script, ScriptOutcome::Ran { .. })
            && !marker
            && followed.is_empty(),
        "skipping .DS_Store hid its bytes at {}: linked {outcome:?}, script {script:?}, marker {marker}, followed {followed:?}",
        repo.commit
    );
}

/// `ls-tree -z` is one line. Past 64 KiB the runner ends that line, inserts a newline, and
/// starts another, so a clean tree whose listing is longer no longer compares equal to the files.
#[test]
fn a_clean_listing_past_64_kib_is_treated_as_a_different_tree() {
    let mut repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = clone_of(&repo);
    fs::create_dir_all(clone.join("p")).unwrap();
    for n in 0..1400 {
        fs::write(clone.join(format!("p/{n:04}")), "x\n").unwrap();
    }
    recode(&repo);
    repo.commit = CommitSha::parse(git(&repo.home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    let status = git(&repo.home, &clone, &["status", "--porcelain=v1"]);
    assert!(
        status.is_empty(),
        "fixture: the worktree is the commit that was just written: {status:?}"
    );

    let links = plan("setup.sh");
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        (
            LinkOutcome::Linked { backup: None },
            ScriptOutcome::Ran { success: true },
            false
        ),
        "a clean tree past 64 KiB was refused at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}
