//! Review round 7. A test here fails on this commit for the reason in its name.

use std::fs;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::command::{Command, CommandRunner};
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{DotfilesStep, LinkOutcome, ScriptOutcome, UserStage};
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
    let runner = soulcaster_repo::process::runner();
    let report = step(repo, &runner)
        .apply_links(links, UserStage::password_is_gone())
        .expect("the clone's commit id is the approved one, so linking is attempted");
    let script =
        step(repo, &runner).run_script(links, UserStage::password_is_gone(), &mut |_, _| {});
    let marker = repo.home.join("Developer/dotfiles/pwned-marker").exists();
    (report.rows[0].outcome.clone(), script, marker)
}

/// The walk skips a directory named `.DS_Store` before it looks inside, so nothing under it is
/// hashed. The new refusal only looks at the script's own file name, and `safe_parts` still
/// allows a `.DS_Store` parent. The schema allows the path. `zsh -f` then runs those bytes.
#[test]
fn a_script_inside_a_skipped_ds_store_directory_runs() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let clone = repo.home.join("Developer/dotfiles");
    fs::create_dir_all(clone.join(".DS_Store")).unwrap();
    fs::write(clone.join(".DS_Store/setup.sh"), "touch pwned-marker\n").unwrap();
    let listed = git(
        &repo.home,
        &clone,
        &["ls-tree", "-r", "--name-only", "HEAD"],
    );
    assert!(
        !listed.lines().any(|line| line.contains(".DS_Store")),
        "fixture: the approved tree has no .DS_Store path: {listed:?}"
    );
    let status = git(&repo.home, &clone, &["status", "--porcelain=v1"]);
    assert!(
        status.contains(".DS_Store"),
        "fixture: git itself lists the extra directory: {status:?}"
    );

    let links = plan(".DS_Store/setup.sh");
    let (outcome, script, marker) = attempt(&repo, &links);
    assert!(
        !matches!(script, ScriptOutcome::Ran { .. }) && !marker,
        "a skipped .DS_Store directory hid the script at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}
