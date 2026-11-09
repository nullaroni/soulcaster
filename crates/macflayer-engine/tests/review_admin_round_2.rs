//! Round 2 of the Admin Stage review. These state behaviour the fixes still miss.
//! Each one fails on this branch for the reason in its assertion.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use macflayer_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use macflayer_core::catalog::Catalog;
use macflayer_core::command::{Command, CommandRunner, FakeRunner, Reply};
use macflayer_core::config::Config;
use macflayer_core::event::{ConfigRef, Event, RunScope};
use macflayer_core::lifecycle::Stage;
use macflayer_core::plan::{Plan, PlanEntry, RunGroup, Why};
use macflayer_core::recipe::{BuiltinStep, InstallKind, InstallRecipe, Probe, Recipe, Step};
use macflayer_engine::{
    AdminGate, AdminWindow, Budget, Clock, PeerRule, RunInput, SocketGate, Validation, perform,
};

const SECRET: &str = "s3cret-plover";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "macflayer-review2-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ManualClock;

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        1_000
    }

    fn wait(&self, _duration: std::time::Duration) {}
}

struct AcceptsPassword;

impl Asks for AcceptsPassword {
    fn password(&self, request: &PasswordRequest) -> Option<Password> {
        assert!(
            !request.why.contains(SECRET),
            "the password request describes the work, not the secret"
        );
        Some(Password::new(SECRET.to_string()))
    }

    fn show_github_code(&self, _code: &str, _url: &str) {}

    fn github_status(&self, _status: SignInStatus) {}
}

struct MarkerWindow;

impl AdminWindow for MarkerWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner
    }
}

/// Enough of a gate for a hand-off that never asks. `forget` is not the subject here.
struct QuietGate;

impl AdminGate for QuietGate {
    fn validate(
        &self,
        _runner: &dyn CommandRunner,
        _home: &Path,
        _password: &Password,
    ) -> Validation {
        Validation::Accepted
    }

    fn open(&self, _password: &Password, _budget: Budget) -> Result<Box<dyn AdminWindow>, String> {
        Ok(Box::new(MarkerWindow))
    }

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) -> bool {
        true
    }
}

struct Harness {
    home: Sandbox,
    runner: FakeRunner,
    cancel: AtomicBool,
}

impl Harness {
    fn new(name: &str) -> Self {
        Self {
            home: Sandbox::new(name),
            runner: FakeRunner::new(),
            cancel: AtomicBool::new(false),
        }
    }

    fn perform(&self, plan: &mut Plan, admin: &dyn AdminGate) -> Vec<Command> {
        self.runner.on(
            |command| file_name(command) == "id",
            [Reply::ok("staff admin\n")],
        );
        self.runner.on(
            |command| file_name(command) == "sw_vers",
            [Reply::ok("15.6.1\n")],
        );
        let mut events: Vec<Event> = Vec::new();
        perform(RunInput {
            dotfiles: None,
            admin,
            plan,
            runner: &self.runner,
            home: self.home.path(),
            events: &mut events,
            asks: &AcceptsPassword,
            cancel: &self.cancel,
            clock: &ManualClock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "review",
            engine_version: "review",
        })
        .unwrap();
        let blob = serde_json::to_string(&events).unwrap_or_default();
        assert!(!blob.contains(SECRET), "an event carried the password");
        self.runner.calls()
    }
}

fn file_name(command: &Command) -> &str {
    command
        .program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn args_contain(command: &Command, needle: &str) -> bool {
    command.args.iter().any(|arg| arg.contains(needle))
}

fn args_eq(command: &Command, expected: &[&str]) -> bool {
    command.args.len() == expected.len()
        && command
            .args
            .iter()
            .zip(expected)
            .all(|(got, want)| got == want)
}

fn plan(entries: Vec<PlanEntry>) -> Plan {
    Plan {
        config_name: "review".into(),
        entries,
        skipped: vec![],
        conflicts: vec![],
        notes: vec![],
        findings: vec![],
        taps: std::collections::BTreeMap::new(),
        dotfiles: None,
    }
}

fn install(id: &str, checks: Vec<Probe>, steps: Vec<Step>) -> PlanEntry {
    PlanEntry {
        id: id.into(),
        name: id.into(),
        stage: Stage::User,
        group: RunGroup::Base,
        why: Why::Selected,
        recipe: Recipe::Install(InstallRecipe {
            id: id.into(),
            kind: InstallKind::Cli,
            title: id.into(),
            base: false,
            requires: vec![],
            min_macos: None,
            conflicts: vec![],
            checks,
            facts: vec![],
            steps,
            notes: vec![],
            bootstrap: None,
        }),
        value: None,
        needs: vec![],
    }
}

fn admin_xcode(id: &str) -> PlanEntry {
    install(
        id,
        vec![Probe::File {
            path: format!("/tmp/macflayer-review2-{id}"),
        }],
        vec![Step::Builtin {
            name: BuiltinStep::XcodeFirstLaunch,
            admin: true,
            unless: None,
        }],
    )
}

fn shipped_gh() -> PlanEntry {
    let catalog = Catalog::shipped();
    let config = Config::parse(br#"{"schemaVersion":1,"name":"review"}"#).unwrap();
    Plan::new(&config, catalog)
        .entries
        .into_iter()
        .find(|entry| entry.id == "gh")
        .unwrap()
}

fn hand_off_entry() -> PlanEntry {
    let mut gh = shipped_gh();
    if let Recipe::Install(install) = &mut gh.recipe {
        install.requires.clear();
        install.steps = vec![Step::Builtin {
            name: BuiltinStep::GhHandOff,
            admin: false,
            unless: None,
        }];
    }
    gh
}

fn write_bootstrap(home: &Path, gh: &PlanEntry) -> PathBuf {
    let bootstrap = match &gh.recipe {
        Recipe::Install(install) => install.bootstrap.clone().unwrap(),
        _ => panic!("gh is an install"),
    };
    let binary = home
        .join("Library/Application Support/MacFlayer/bootstrap-gh")
        .join(&bootstrap.version)
        .join(&bootstrap.binary);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, "").unwrap();
    binary
}

/// A stand-in path. The fake runner answers it, so `/usr/bin/sudo` is never started.
fn stand_in_gate(home: &Path) -> SocketGate {
    let path = home.join("sudo");
    fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    SocketGate::new(
        PathBuf::from(env!("CARGO_BIN_EXE_macflayer-askpass")),
        PeerRule::PathOnly,
    )
    .with_sudo(path)
}

fn is_sudo(command: &Command) -> bool {
    file_name(command) == "sudo"
}

fn is_clear(command: &Command) -> bool {
    is_sudo(command) && args_eq(command, &["-k"])
}

fn is_validate(command: &Command) -> bool {
    is_sudo(command) && args_eq(command, &["-A", "-v"])
}

/// `sudo -k` is what forces `sudo -v` to check the password. Its result is discarded, so a
/// failure leaves the old timestamp and `-v` succeeds without the helper ever being asked.
#[test]
fn a_failed_sudo_clear_still_accepts_the_password() {
    let h = Harness::new("stale-timestamp");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    h.runner
        .on(is_clear, [Reply::fails(1, "sudo: a password is required")]);
    h.runner.on(is_validate, [Reply::ok("")]);
    let gate = stand_in_gate(h.home.path());
    let mut planned = plan(vec![admin_xcode("xc")]);
    let calls = h.perform(&mut planned, &gate);
    // Changed from requiring `sudo -v` to run: the fix refuses at the failed clear, before `-v`,
    // so a stale timestamp is never taken as a check. The finding's assertion below is unchanged.
    assert!(
        calls.iter().any(is_clear),
        "sudo -k never ran, so this did not reach the failed clear: {calls:#?}"
    );
    assert!(
        !calls.iter().any(is_validate),
        "sudo -v ran after a failed clear: {calls:#?}"
    );
    assert!(
        !calls
            .iter()
            .any(|command| args_contain(command, "xcodebuild")),
        "sudo -k failed and the old timestamp was left in place, but sudo -v was treated as \
         the password having been checked and admin work ran: {calls:#?}"
    );
}

/// The clear after a successful check is also discarded. Sign-in can then sit for 15 minutes
/// with the timestamp `sudo -v` just refreshed.
#[test]
fn a_failed_forget_still_starts_github_sign_in() {
    let h = Harness::new("forget-failed");
    let gh = shipped_gh();
    let version = match &gh.recipe {
        Recipe::Install(install) => install.bootstrap.as_ref().unwrap().version.clone(),
        _ => panic!("gh is an install"),
    };
    h.runner.on(
        move |command| file_name(command) == "test" && args_contain(command, &version),
        [Reply::ok("")],
    );
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    h.runner.on(
        |command| file_name(command) == "gh" && args_contain(command, "status"),
        [Reply::fails(1, "You are not logged into any GitHub hosts.")],
    );
    // The clear inside validate has to succeed, or `-v` is not a real check. The next `-k`
    // is `forget`, and it fails.
    h.runner
        .on(is_clear, [Reply::ok(""), Reply::fails(1, "sudo -k failed")]);
    h.runner.on(is_validate, [Reply::ok("")]);
    let gate = stand_in_gate(h.home.path());
    let mut planned = plan(vec![admin_xcode("xc"), gh]);
    let calls = h.perform(&mut planned, &gate);
    let checked = calls.iter().position(is_validate);
    assert!(
        checked.is_some(),
        "the password was never checked: {calls:#?}"
    );
    let checked = checked.unwrap();
    assert!(
        calls.iter().skip(checked + 1).any(is_clear),
        "forget never ran after the check: {calls:#?}"
    );
    assert!(
        !calls
            .iter()
            .any(|command| file_name(command) == "gh" && args_contain(command, "login")),
        "sudo -k after the successful check failed, and gh auth login still started. \
         The timestamp from sudo -v is still valid for that wait: {calls:#?}"
    );
}

/// `git config --global` writes `~/.config/git/config` when that file exists and `~/.gitconfig`
/// does not. The hand-off only looks at `~/.gitconfig`.
#[test]
fn setup_git_writes_through_the_xdg_git_config() {
    let h = Harness::new("xdg-gitconfig");
    let home = h.home.path().to_path_buf();
    let repo = home.join("dotfiles");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("gitconfig"),
        "[includeIf \"gitdir:~/code/\"]\n\tpath = ~/dotfiles/more.gitconfig\n\
         [credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other-helper\n",
    )
    .unwrap();
    let xdg = home.join(".config/git");
    fs::create_dir_all(&xdg).unwrap();
    std::os::unix::fs::symlink(repo.join("gitconfig"), xdg.join("config")).unwrap();
    assert!(
        !home.join(".gitconfig").exists(),
        "the test needs ~/.gitconfig to be absent, which is when git selects the XDG file"
    );

    let gh = hand_off_entry();
    let _copy = write_bootstrap(&home, &gh);
    h.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "/opt/homebrew/bin/gh"),
        vec![Reply::ok(""); 8],
    );
    h.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "bootstrap-gh"),
        [Reply::fails(1, ""), Reply::ok("")],
    );
    h.runner.on(
        |command| {
            command.program.starts_with("/opt/homebrew")
                && file_name(command) == "gh"
                && args_contain(command, "user")
        },
        vec![Reply::ok("someone\n"); 4],
    );
    h.runner.on(
        |command| file_name(command) == "gh" && args_contain(command, "user"),
        vec![Reply::ok("someone\n"); 4],
    );
    h.runner.on(
        |command| file_name(command) == "gh" && args_contain(command, "status"),
        [Reply::ok(
            "Logged in to github.com account someone (keyring)\n",
        )],
    );
    let mut planned = plan(vec![gh]);
    let calls = h.perform(&mut planned, &QuietGate);
    assert!(
        !calls
            .iter()
            .any(|command| file_name(command) == "gh" && args_contain(command, "setup-git")),
        "gh auth setup-git ran. ~/.gitconfig is absent and ~/.config/git/config is a symlink \
         into the Dotfiles checkout, which is the file `git config --global` writes: {calls:#?}"
    );
}
