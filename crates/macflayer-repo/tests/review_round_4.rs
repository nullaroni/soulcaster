//! Review round 4. A test here fails on this commit for the reason in its name.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

fn write_exec(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

/// A single-quoted shell word. Temp paths are absolute and usually have no quotes.
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
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

fn plan() -> DotfilesPlan {
    DotfilesPlan {
        repo: "example-stranger/dotfiles".to_string(),
        links: [("~/.zshrc".to_string(), "zsh/zshrc".to_string())].into(),
        refused_links: vec![],
        install_script: Some("setup.sh".to_string()),
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
                path: "setup.sh".to_string(),
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

/// `.git/info/attributes` names a filter, and the clone's config supplies `filter.hide.clean`.
/// `core.attributesFile=/dev/null` does not turn that off. The clean command's stdout is what
/// `status` compares, so a different `setup.sh` matches the approved blob.
fn install_hiding_filter(repo: &RealRepo) {
    let clone = repo.home.join("Developer/dotfiles");
    let script = clone.join(".git/hide-clean");
    write_exec(&script, "#!/bin/sh\ncat >/dev/null\nprintf 'exit 0\\n'\n");
    fs::create_dir_all(clone.join(".git/info")).unwrap();
    fs::write(clone.join(".git/info/attributes"), "setup.sh filter=hide\n").unwrap();
    git(
        &repo.home,
        &clone,
        &["config", "filter.hide.clean", &script.to_string_lossy()],
    );
    git(
        &repo.home,
        &clone,
        &["config", "filter.hide.required", "true"],
    );
}

/// A clean filter runs while the check hashes. This one copies the bytes through, so the tree
/// still looks dirty, and it touches `marker` as a side effect.
fn install_running_filter(repo: &RealRepo, marker: &Path) {
    let clone = repo.home.join("Developer/dotfiles");
    let script = clone.join(".git/run-clean");
    write_exec(
        &script,
        &format!("#!/bin/sh\ntouch {}\ncat\n", sh_quote(marker)),
    );
    fs::create_dir_all(clone.join(".git/info")).unwrap();
    fs::write(clone.join(".git/info/attributes"), "* filter=run\n").unwrap();
    git(
        &repo.home,
        &clone,
        &["config", "filter.run.clean", &script.to_string_lossy()],
    );
    git(
        &repo.home,
        &clone,
        &["config", "filter.run.required", "true"],
    );
}

/// The check builds a fresh index and then asks `status`. `status` runs `filter.hide.clean` and
/// compares that output to the blob, so the approved commit's id still names this tree while
/// `setup.sh` on disk is something else. That file is what gets run.
#[test]
fn a_clean_filter_links_a_tree_that_is_not_the_approved_commit() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    install_hiding_filter(&repo);
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();

    let head = git(&repo.home, &clone, &["rev-parse", "HEAD^{commit}"]);
    assert_eq!(
        head.trim(),
        repo.commit.as_str(),
        "fixture: the commit id should still be the approved one"
    );
    assert_eq!(
        fs::read_to_string(clone.join("setup.sh")).unwrap(),
        "touch pwned-marker\n",
        "fixture: the worktree script should be the other program"
    );
    // The index left by `commit` still has this file's stat, and `status` then compares the raw
    // bytes, so it reports the edit. The check's fresh index has no stat. Hashing it runs the
    // clean filter, and that output matches the blob.
    let links = plan();
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "clean filter hid a different tree at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}

/// Hashing the worktree runs the clone's clean filter before the check decides. The filter here
/// does not hide the edit, so the link is refused and the install script is held, but the program
/// has already run.
#[test]
fn a_clean_filter_runs_a_program_while_hashing() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    let ran = repo.home.join("filter-ran");
    install_running_filter(&repo, &ran);
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();
    assert!(
        !ran.exists(),
        "fixture: the filter must not have run before the check"
    );

    let links = plan();
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome, script, marker),
        refused(),
        "fixture: the edit is still visible, so the script must be held"
    );
    assert!(
        !ran.exists(),
        "clean filter ran while hashing at {}: {}",
        repo.commit,
        ran.display()
    );
}
