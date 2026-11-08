//! Review round 2. A test here fails on this commit for the reason in its name.

use std::fs;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::command::{Command, CommandRunner};
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{DotfilesStep, LinkOutcome, LinkRefusal, ScriptOutcome, UserStage};
use soulcaster_repo::fake::FileApprovals;
use soulcaster_repo::tools::Tools;

const URL: &str = "https://github.com/example-stranger/dotfiles.git";
const KEY: &str = "github.com/example-stranger/dotfiles";

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

/// `status` and `ls-files -v` both follow a replace ref, so a different tree at the approved
/// commit looks clean and is linked.
#[test]
fn a_replace_ref_links_a_tree_that_is_not_the_approved_commit() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    let links = DotfilesPlan {
        repo: "example-stranger/dotfiles".to_string(),
        links: [("~/.zshrc".to_string(), "zsh/zshrc".to_string())].into(),
        refused_links: vec![],
        install_script: Some("setup.sh".to_string()),
    };
    fs::write(clone.join("zsh/zshrc"), "EVIL\n").unwrap();
    fs::write(clone.join("setup.sh"), "touch pwned-marker\n").unwrap();
    git(&repo.home, &clone, &["add", "--", "zsh/zshrc", "setup.sh"]);
    git(
        &repo.home,
        &clone,
        &["commit", "--quiet", "-m", "not approved"],
    );
    let evil = git(&repo.home, &clone, &["rev-parse", "HEAD"]);
    git(
        &repo.home,
        &clone,
        &["reset", "--hard", "--quiet", repo.commit.as_str()],
    );
    git(
        &repo.home,
        &clone,
        &["replace", repo.commit.as_str(), evil.trim()],
    );
    git(&repo.home, &clone, &["reset", "--hard", "--quiet"]);

    let head = git(&repo.home, &clone, &["rev-parse", "HEAD^{commit}"]);
    assert_eq!(
        head.trim(),
        repo.commit.as_str(),
        "fixture: the commit id should still be the approved one"
    );
    assert_eq!(
        fs::read_to_string(clone.join("zsh/zshrc")).unwrap(),
        "EVIL\n",
        "fixture: the worktree should be the replacement tree"
    );
    assert!(
        git(&repo.home, &clone, &["status", "--porcelain=v1"]).is_empty(),
        "fixture: status should be clean"
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
    let report = step(&repo, &runner)
        .apply_links(&links, UserStage::password_is_gone())
        .expect("the clone is the approved commit id, so linking is attempted");
    let script =
        step(&repo, &runner).run_script(&links, UserStage::password_is_gone(), &mut |_, _| {});
    let marker = clone.join("pwned-marker");
    assert_eq!(
        (report.rows[0].outcome.clone(), script, marker.exists()),
        (
            LinkOutcome::Refused(LinkRefusal::RepoChanged),
            ScriptOutcome::HeldForReview,
            false
        ),
        "replace ref hid a different tree at {}: linked {:?}, script {:?}, marker {}",
        repo.commit,
        report.rows[0].outcome,
        script,
        marker.exists()
    );
}
