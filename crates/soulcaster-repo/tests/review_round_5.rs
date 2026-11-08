//! Review round 5. A test here fails on this commit for the reason in its name.

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

fn write_exec(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

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

/// A superproject whose approved tree contains a submodule at `vendor` and ignores
/// `.gitmodules`, so a `.gitmodules` added on disk is not part of the approved tree.
/// The submodule blob of `inside.sh` is `exit 0`.
fn repo_with_vendor() -> (RealRepo, PathBuf) {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let home = repo.home.clone();
    let clone = clone_of(&repo);
    let src = home.join("vendor-src");
    fs::create_dir_all(&src).unwrap();
    git(&home, &src, &["init", "--quiet", "--initial-branch=main"]);
    fs::write(src.join("inside.sh"), "exit 0\n").unwrap();
    git(&home, &src, &["add", "--all"]);
    git(&home, &src, &["commit", "--quiet", "-m", "vendor"]);
    let src_text = src.to_string_lossy().into_owned();
    git(
        &home,
        &clone,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--",
            src_text.as_str(),
            "vendor",
        ],
    );
    fs::write(clone.join(".gitignore"), ".gitmodules\n").unwrap();
    git(
        &home,
        &clone,
        &["rm", "--cached", "--quiet", "--", ".gitmodules"],
    );
    fs::remove_file(clone.join(".gitmodules")).unwrap();
    git(&home, &clone, &["add", "--all"]);
    git(
        &home,
        &clone,
        &["commit", "--quiet", "-m", "approved submodule"],
    );
    let commit = CommitSha::parse(git(&home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    let vendor = clone.join("vendor");
    assert_eq!(
        git(&home, &vendor, &["show", "HEAD:inside.sh"]),
        "exit 0\n",
        "fixture: the approved submodule blob is exit 0"
    );
    assert!(
        !git(&home, &clone, &["ls-tree", "--name-only", "HEAD"])
            .lines()
            .any(|line| line == ".gitmodules"),
        "fixture: the approved tree does not contain .gitmodules"
    );
    (RealRepo { commit, ..repo }, vendor)
}

/// The scratch git dir still reads `.gitattributes` from the worktree.
/// `core.attributesFile=/dev/null` does not turn that off, and `ident` is built in, so no
/// filter command in config is required. `ident` rewrites `$Id: ...$` back to `$Id$` while
/// hashing, and the bytes on disk are what `zsh -f` runs.
#[test]
fn an_ident_attribute_links_a_script_that_is_not_the_approved_blob() {
    let repo = real_repo(&[
        ("zsh/zshrc", "approved\n"),
        ("setup.sh", "true $Id$\n"),
        (".gitattributes", "setup.sh ident\n"),
    ]);
    let clone = clone_of(&repo);
    let payload = "true $Id: ; touch pwned-marker ; true $\n";
    fs::write(clone.join("setup.sh"), payload).unwrap();
    assert_eq!(
        git(&repo.home, &clone, &["show", "HEAD:setup.sh"]),
        "true $Id$\n",
        "fixture: the approved blob is the short script"
    );
    assert_eq!(
        fs::read_to_string(clone.join("setup.sh")).unwrap(),
        payload,
        "fixture: the worktree script is the other program"
    );

    let links = plan("setup.sh");
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "ident hid a different script at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}

/// `status` in the scratch git dir still honors `.gitmodules` from the worktree. The approved
/// tree does not contain that file: `.gitignore` lists it, and the check's second `status`
/// only names the linked path and the script. `ignore = all` then reports a submodule whose
/// `inside.sh` is a different program as clean, and that file is what gets run.
#[test]
fn a_gitignored_gitmodules_hides_a_different_submodule_script() {
    let (repo, vendor) = repo_with_vendor();
    let clone = clone_of(&repo);
    fs::write(
        clone.join(".gitmodules"),
        "[submodule \"vendor\"]\n\tpath = vendor\n\tignore = all\n",
    )
    .unwrap();
    fs::write(vendor.join("inside.sh"), "touch pwned-marker\n").unwrap();
    let shown = git(
        &repo.home,
        &clone,
        &[
            "-c",
            "submodule.vendor.ignore=none",
            "status",
            "--porcelain=v1",
        ],
    );
    assert!(
        shown.contains("vendor"),
        "fixture: without ignore=all the submodule is dirty: {shown:?}"
    );

    let links = plan("vendor/inside.sh");
    let (outcome, script, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome.clone(), script, marker),
        refused(),
        "gitmodules hid a different script at {}: linked {outcome:?}, script {script:?}, marker {marker}",
        repo.commit
    );
}

/// Checking a submodule runs `git status` inside it, with that submodule's git dir, not the
/// scratch one. The trace is `cd vendor; GIT_DIR=.git git status`. A clean filter named by the
/// submodule's `.gitattributes` and defined in its config therefore runs while that status
/// hashes. A same-size edit is what makes this git hash the file; the superproject still shows
/// the submodule as dirty, and the program has already run.
#[test]
fn a_submodule_clean_filter_runs_a_program_while_checking() {
    let repo = real_repo(&[("zsh/zshrc", "approved\n"), ("setup.sh", "exit 0\n")]);
    let home = repo.home.clone();
    let clone = clone_of(&repo);
    let src = home.join("vendor-src");
    fs::create_dir_all(&src).unwrap();
    git(&home, &src, &["init", "--quiet", "--initial-branch=main"]);
    fs::write(src.join("inside.sh"), "exit 0\n").unwrap();
    fs::write(src.join(".gitattributes"), "* filter=x\n").unwrap();
    git(&home, &src, &["add", "--all"]);
    git(&home, &src, &["commit", "--quiet", "-m", "vendor"]);
    let src_text = src.to_string_lossy().into_owned();
    git(
        &home,
        &clone,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--",
            src_text.as_str(),
            "vendor",
        ],
    );
    git(&home, &clone, &["add", "--all"]);
    git(&home, &clone, &["commit", "--quiet", "-m", "with vendor"]);
    let commit = CommitSha::parse(git(&home, &clone, &["rev-parse", "HEAD"]).trim()).unwrap();
    let repo = RealRepo { commit, ..repo };
    let vendor = clone.join("vendor");
    let gitdir = git(&home, &vendor, &["rev-parse", "--absolute-git-dir"]);
    let config = format!("{}/config", gitdir.trim());
    let ran = home.join("filter-ran");
    let script = home.join("sub-filter");
    write_exec(
        &script,
        &format!("#!/bin/sh\ntouch {}\ncat\n", sh_quote(&ran)),
    );
    let script_text = script.to_string_lossy().into_owned();
    git(
        &home,
        &clone,
        &[
            "config",
            "--file",
            config.as_str(),
            "filter.x.clean",
            script_text.as_str(),
        ],
    );
    git(
        &home,
        &clone,
        &[
            "config",
            "--file",
            config.as_str(),
            "filter.x.required",
            "true",
        ],
    );
    // Same length as `exit 0\n`, so this git hashes the worktree instead of trusting the size.
    fs::write(vendor.join("inside.sh"), "exit 1\n").unwrap();
    assert!(
        !ran.exists(),
        "fixture: the filter must not have run before the check"
    );

    let links = plan("setup.sh");
    let (outcome, script_outcome, marker) = attempt(&repo, &links);
    assert_eq!(
        (outcome, script_outcome, marker),
        refused(),
        "fixture: the submodule edit is still visible, so the install script must be held"
    );
    assert!(
        !ran.exists(),
        "submodule clean filter ran while checking at {}: {}",
        repo.commit,
        ran.display()
    );
}
