//! Round 3 of the Admin Stage review. These state behaviour `ee93d08` still misses.
//! Each one fails on this branch for the reason in its assertion.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
            "macflayer-review3-{name}-{}-{}",
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

    fn wait(&self, _duration: Duration) {}
}

struct RecordingClock {
    waits: Mutex<Vec<Duration>>,
}

impl RecordingClock {
    fn new() -> Self {
        Self {
            waits: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }
}

impl Clock for RecordingClock {
    fn now_ms(&self) -> u64 {
        1_000
    }

    fn wait(&self, duration: Duration) {
        self.waits.lock().unwrap().push(duration);
    }
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

/// Enough of a gate for a hand-off that never asks.
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
        self.perform_with(plan, admin, &ManualClock)
    }

    fn perform_with(
        &self,
        plan: &mut Plan,
        admin: &dyn AdminGate,
        clock: &dyn Clock,
    ) -> Vec<Command> {
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
            clock,
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
            path: format!("/tmp/macflayer-review3-{id}"),
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

fn is_login(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "login")
}

fn is_setup_git(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "setup-git")
}

/// The opening `sudo -k` lives inside `validate`. A failure comes back as
/// `Unavailable`, and `maybe_ask` does not set `sudo_may_be_live`. Sign-in only
/// looks at that flag, so it still starts while the earlier timestamp may be live.
#[test]
fn a_failed_opening_clear_still_starts_github_sign_in() {
    let h = Harness::new("opening-clear-signin");
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
    h.runner
        .on(is_clear, [Reply::fails(1, "sudo: a password is required")]);
    h.runner.on(is_validate, [Reply::ok("")]);
    let gate = stand_in_gate(h.home.path());
    let mut planned = plan(vec![admin_xcode("xc"), gh]);
    let calls = h.perform(&mut planned, &gate);
    assert!(
        calls.iter().any(is_clear),
        "sudo -k never ran, so this did not reach the failed opening clear: {calls:#?}"
    );
    assert!(
        !calls.iter().any(is_validate),
        "sudo -v ran after the failed opening clear: {calls:#?}"
    );
    assert!(
        !calls.iter().any(is_login),
        "the opening sudo -k failed, so an earlier timestamp may still be valid, but \
         gh auth login started anyway: {calls:#?}"
    );
}

/// After a step, a failed `sudo -k` discards the password and refuses the next admin
/// step. The network backoff is still started before that return, so the engine waits
/// while the timestamp may be live.
#[test]
fn a_failed_clear_after_a_step_still_backs_off() {
    let h = Harness::new("backoff-after-clear");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    // The clear inside validate, then the clear after the check. The clear after the
    // step fails, and so does the one `discard` makes.
    h.runner.on(
        is_clear,
        [
            Reply::ok(""),
            Reply::ok(""),
            Reply::fails(1, "sudo -k failed"),
        ],
    );
    h.runner.on(is_validate, [Reply::ok("")]);
    h.runner.on(
        |command| args_contain(command, "xcodebuild"),
        [Reply::fails(1, "operation timed out")],
    );
    let gate = stand_in_gate(h.home.path());
    let clock = RecordingClock::new();
    let mut planned = plan(vec![admin_xcode("xc")]);
    let calls = h.perform_with(&mut planned, &gate, &clock);
    let xcode = calls
        .iter()
        .filter(|command| args_contain(command, "xcodebuild"))
        .count();
    assert_eq!(
        xcode, 1,
        "the admin step did not run once, so this did not reach the clear after it: {calls:#?}"
    );
    let clears = calls.iter().filter(|command| is_clear(command)).count();
    assert!(
        clears >= 3,
        "the clear after the step never ran: {clears} sudo -k calls, {calls:#?}"
    );
    let waits = clock.waits();
    assert!(
        waits.is_empty(),
        "sudo -k after the step failed, and the engine still waited {waits:?} before stopping"
    );
}

/// `git config --global` writes `~/.config/git/config` when that file exists and
/// `~/.gitconfig` does not. The link and include checks now see it. The other-helper
/// read still opens only `~/.gitconfig`, so a helper that lives in the XDG file is
/// invisible and `gh auth setup-git` rewrites it.
#[test]
fn an_xdg_git_config_with_another_helper_still_gets_setup_git() {
    let h = Harness::new("xdg-helper");
    let home = h.home.path().to_path_buf();
    let xdg = home.join(".config/git");
    fs::create_dir_all(&xdg).unwrap();
    let config = xdg.join("config");
    fs::write(
        &config,
        "[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other-helper\n",
    )
    .unwrap();
    let meta = fs::symlink_metadata(&config).unwrap();
    assert!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "the XDG file has to be a regular file, so the link check is not what skips setup"
    );
    let text = fs::read_to_string(&config).unwrap();
    assert!(
        !text.to_lowercase().contains("[include"),
        "the XDG file has no include, so the include check is not what skips setup"
    );
    assert!(
        !home.join(".gitconfig").exists(),
        "the test needs ~/.gitconfig to be absent, which is when git writes the XDG file"
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
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. ~/.gitconfig is absent and ~/.config/git/config is a regular \
         file that already sets a GitHub helper, which is the file `git config --global` \
         rewrites: {calls:#?}"
    );
}
