//! Independent review of the Admin Stage. These tests state the behaviour the design requires.
//! They fail on the current branch for the reasons named in the PR comment.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::event::{ConfigRef, Event, RunScope};
use soulcaster_core::lifecycle::Stage;
use soulcaster_core::plan::{Plan, PlanEntry, RunGroup, Why};
use soulcaster_core::recipe::{BuiltinStep, InstallKind, InstallRecipe, Probe, Recipe, Step};
use soulcaster_engine::{
    AdminGate, AdminWindow, Budget, Clock, ProcessRunner, RunInput, SocketGate, Validation, perform,
};

const SECRET: &str = "s3cret-plover";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review-{name}-{}-{}",
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

struct CountingGate {
    forgotten: Arc<AtomicUsize>,
}

impl AdminGate for CountingGate {
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

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) {
        self.forgotten.fetch_add(1, Ordering::SeqCst);
    }
}

struct MarkerWindow;

impl AdminWindow for MarkerWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }

    fn may_elevate(&self, inner: Command) -> Command {
        self.elevated(inner)
    }
}

/// Stays "open" until dropped, so a test can see whether a retry still has the helper.
struct HeldWindow {
    open: Arc<AtomicBool>,
}

impl AdminWindow for HeldWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }

    fn may_elevate(&self, inner: Command) -> Command {
        self.elevated(inner)
    }
}

impl Drop for HeldWindow {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

struct HeldGate {
    open: Arc<AtomicBool>,
}

impl AdminGate for HeldGate {
    fn validate(
        &self,
        _runner: &dyn CommandRunner,
        _home: &Path,
        _password: &Password,
    ) -> Validation {
        Validation::Accepted
    }

    fn open(&self, _password: &Password, _budget: Budget) -> Result<Box<dyn AdminWindow>, String> {
        self.open.store(true, Ordering::SeqCst);
        Ok(Box::new(HeldWindow {
            open: Arc::clone(&self.open),
        }))
    }

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) {}
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
        self.perform_on(plan, admin, &ManualClock)
    }

    fn perform_on(
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

fn has_helper(command: &Command) -> bool {
    command.env.iter().any(|(key, _)| key == "SUDO_ASKPASS")
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
            path: format!("/tmp/soulcaster-review-{id}"),
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

fn bootstrap_binary(home: &Path, gh: &PlanEntry) -> PathBuf {
    let bootstrap = match &gh.recipe {
        Recipe::Install(install) => install.bootstrap.clone().unwrap(),
        _ => panic!("gh is an install"),
    };
    home.join("Library/Application Support/Soulcaster/bootstrap-gh")
        .join(&bootstrap.version)
        .join(&bootstrap.binary)
}

fn write_bootstrap(home: &Path, gh: &PlanEntry) -> PathBuf {
    let binary = bootstrap_binary(home, gh);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, "").unwrap();
    binary
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

/// The hand-off step is pending: Homebrew's `gh` is present, the bootstrap copy still is.
fn script_pending_hand_off(runner: &FakeRunner) {
    runner.on(
        |command| file_name(command) == "test" && args_contain(command, "/opt/homebrew/bin/gh"),
        vec![Reply::ok(""); 8],
    );
    runner.on(
        |command| file_name(command) == "test" && args_contain(command, "bootstrap-gh"),
        [Reply::fails(1, ""), Reply::ok("")],
    );
}

fn script_same_account(runner: &FakeRunner) {
    runner.on(
        |command| {
            command.program.starts_with("/opt/homebrew")
                && file_name(command) == "gh"
                && args_contain(command, "user")
        },
        vec![Reply::ok("someone\n"); 4],
    );
    runner.on(
        |command| file_name(command) == "gh" && args_contain(command, "user"),
        vec![Reply::ok("someone\n"); 4],
    );
}

fn is_setup_git(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "setup-git")
}

fn support_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Soulcaster")
}

#[test]
fn sudo_is_forgotten_before_github_sign_in_while_the_password_is_held() {
    let h = Harness::new("signin-timestamp");
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
        [Reply::fails(1, "not logged in")],
    );
    let forgotten = Arc::new(AtomicUsize::new(0));
    let forgotten_at_login = Arc::new(AtomicUsize::new(usize::MAX));
    let forgotten_for_login = Arc::clone(&forgotten);
    let seen = Arc::clone(&forgotten_at_login);
    h.runner.on(
        move |command| {
            let login = file_name(command) == "gh" && args_contain(command, "login");
            if login && seen.load(Ordering::SeqCst) == usize::MAX {
                seen.store(forgotten_for_login.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            login
        },
        [Reply::ok(
            "! First copy your one-time code: ABCD-1234\nOpen this URL: https://evil.example/login\n",
        )],
    );
    let gate = CountingGate {
        forgotten: Arc::clone(&forgotten),
    };
    let mut planned = plan(vec![admin_xcode("xc"), gh]);
    let calls = h.perform(&mut planned, &gate);
    let at_login = forgotten_at_login.load(Ordering::SeqCst);
    assert_ne!(at_login, usize::MAX, "sign-in never ran: {calls:#?}");
    let code_in_event = calls
        .iter()
        .any(|command| command.args.iter().any(|arg| arg.contains("ABCD-1234")));
    assert!(!code_in_event, "the one-time code was placed on a command");
    assert!(
        at_login >= 1,
        "sudo -k did not run before gh auth login; forget count was {at_login}. \
         validate's sudo -v leaves a timestamp, and sign-in can sit there for 15 minutes"
    );
}

#[test]
fn sudo_is_forgotten_between_admin_steps() {
    let h = Harness::new("between-steps");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    let forgotten = Arc::new(AtomicUsize::new(0));
    let at_second = Arc::new(AtomicUsize::new(usize::MAX));
    let forgotten_for_step = Arc::clone(&forgotten);
    let seen = Arc::clone(&at_second);
    let nth = Arc::new(AtomicUsize::new(0));
    h.runner.on(
        move |command| {
            if file_name(command) != "xcodebuild" {
                return false;
            }
            let index = nth.fetch_add(1, Ordering::SeqCst);
            if index == 1 {
                seen.store(forgotten_for_step.load(Ordering::SeqCst), Ordering::SeqCst);
            }
            true
        },
        [Reply::ok(""), Reply::ok("")],
    );
    let gate = CountingGate {
        forgotten: Arc::clone(&forgotten),
    };
    let mut planned = plan(vec![admin_xcode("first"), admin_xcode("second")]);
    let calls = h.perform(&mut planned, &gate);
    let at_second = at_second.load(Ordering::SeqCst);
    assert_ne!(
        at_second,
        usize::MAX,
        "the second admin step never ran: {calls:#?}"
    );
    assert!(
        at_second >= 1,
        "sudo -k did not run after the first admin step; forget count was {at_second}. \
         a successful sudo refreshes the timestamp, and the socket is already closed"
    );
}

#[test]
fn an_admin_window_closes_before_a_network_retry() {
    // Changed from checking at the retry's own command, which needs a window to run at all. What
    // the finding is about is the wait: nothing may be listening while the engine backs off.
    struct WatchingClock {
        open: Arc<AtomicBool>,
        waits: AtomicUsize,
        open_during_wait: AtomicBool,
    }

    impl Clock for WatchingClock {
        fn now_ms(&self) -> u64 {
            1_000
        }

        fn wait(&self, _duration: Duration) {
            self.waits.fetch_add(1, Ordering::SeqCst);
            if self.open.load(Ordering::SeqCst) {
                self.open_during_wait.store(true, Ordering::SeqCst);
            }
        }
    }

    let h = Harness::new("retry-window");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    h.runner.on(
        |command| file_name(command) == "xcodebuild",
        [Reply::fails(1, "operation timed out\n"), Reply::ok("")],
    );
    let open = Arc::new(AtomicBool::new(false));
    let gate = HeldGate {
        open: Arc::clone(&open),
    };
    let clock = WatchingClock {
        open,
        waits: AtomicUsize::new(0),
        open_during_wait: AtomicBool::new(false),
    };
    let mut planned = plan(vec![admin_xcode("xc")]);
    let calls = h.perform_on(&mut planned, &gate, &clock);
    let tries = calls
        .iter()
        .filter(|command| file_name(command) == "xcodebuild")
        .count();
    assert!(
        tries >= 2,
        "the timed-out admin command was not retried: {calls:#?}"
    );
    assert!(
        clock.waits.load(Ordering::SeqCst) >= 1,
        "there was no backoff"
    );
    assert!(
        !clock.open_during_wait.load(Ordering::SeqCst),
        "the admin window was open during the backoff"
    );
}

#[test]
fn a_failed_quiet_route_opens_apples_dialog_once() {
    let h = Harness::new("clt-dialog");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    h.runner.on(
        |command| file_name(command) == "softwareupdate" && args_contain(command, "-l"),
        [Reply::ok("No new software available.\n")],
    );
    h.runner.on(
        |command| file_name(command) == "xcode-select",
        [Reply::fails(1, "operation timed out\n")],
    );
    let mut planned = plan(vec![install(
        "command-line-tools",
        vec![Probe::Builtin {
            name: soulcaster_core::recipe::BuiltinProbe::CommandLineTools,
        }],
        vec![Step::Builtin {
            name: BuiltinStep::CommandLineTools,
            admin: true,
            unless: None,
        }],
    )]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    let dialogs = calls
        .iter()
        .filter(|command| file_name(command) == "xcode-select")
        .count();
    assert_eq!(
        dialogs, 1,
        "xcode-select --install ran {dialogs} times: {calls:#?}"
    );
}

#[test]
fn the_empty_path_directory_is_cleared_before_gh_runs() {
    let h = Harness::new("empty-path");
    let empty = support_dir(h.home.path()).join("empty-path");
    fs::create_dir_all(&empty).unwrap();
    fs::write(empty.join("git"), "#!/bin/sh\n").unwrap();
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
        [Reply::fails(1, "not logged in")],
    );
    let mut planned = plan(vec![admin_xcode("xc"), gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    let login = calls
        .iter()
        .find(|command| file_name(command) == "gh" && args_contain(command, "login"))
        .expect("sign-in never ran");
    let path = login
        .env
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| PathBuf::from(value))
        .expect("sign-in has no PATH");
    let planted = fs::read_dir(&path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert!(
        planted.is_empty(),
        "PATH {} still contains {planted:?}; gh will run a planted git",
        path.display()
    );
}

#[test]
fn the_empty_path_directory_is_not_a_symlink() {
    let h = Harness::new("empty-symlink");
    let support = support_dir(h.home.path());
    let decoy = support.join("decoy-path");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(decoy.join("git"), "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(&decoy, support.join("empty-path")).unwrap();
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
        [Reply::fails(1, "not logged in")],
    );
    let mut planned = plan(vec![admin_xcode("xc"), gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    let login = calls
        .iter()
        .find(|command| file_name(command) == "gh" && args_contain(command, "login"))
        .expect("sign-in never ran");
    let path = PathBuf::from(
        &login
            .env
            .iter()
            .find(|(key, _)| key == "PATH")
            .expect("sign-in has no PATH")
            .1,
    );
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(
        !meta.file_type().is_symlink(),
        "PATH {} is a symlink; gh follows it and can reach git",
        path.display()
    );
}

#[test]
fn an_existing_bootstrap_gh_is_pinned_again_before_it_runs() {
    let h = Harness::new("repin");
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
    let mut planned = plan(vec![gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    let gh_at = calls.iter().position(|command| file_name(command) == "gh");
    assert!(gh_at.is_some(), "bootstrap gh never ran: {calls:#?}");
    let pinned = calls
        .iter()
        .take(gh_at.unwrap())
        .any(|command| file_name(command) == "codesign" || file_name(command) == "shasum");
    assert!(
        pinned,
        "gh ran with no checksum or signature check in front of it: {calls:#?}"
    );
}

#[test]
fn hand_off_keeps_the_copy_when_the_early_account_cannot_be_read() {
    let h = Harness::new("account-unknown");
    let gh = hand_off_entry();
    let copy = write_bootstrap(h.home.path(), &gh);
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
        [Reply::fails(1, "not signed in")],
    );
    script_pending_hand_off(&h.runner);
    let mut planned = plan(vec![gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(
        copy.exists(),
        "the bootstrap copy was deleted without an account match: {calls:#?}"
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "setup-git ran without an account match"
    );
}

#[test]
fn a_failed_setup_git_keeps_the_bootstrap_copy() {
    let h = Harness::new("setup-git-fails");
    let gh = hand_off_entry();
    let copy = write_bootstrap(h.home.path(), &gh);
    script_same_account(&h.runner);
    h.runner
        .on(is_setup_git, [Reply::fails(1, "setup-git failed")]);
    script_pending_hand_off(&h.runner);
    let mut planned = plan(vec![gh]);
    let _calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(
        copy.exists(),
        "a failed gh auth setup-git still deleted the bootstrap copy, so gh-handed-off will not retry it"
    );
}

#[test]
fn a_gitconfig_reached_through_a_parent_symlink_is_left_alone() {
    let h = Harness::new("parent-link");
    let home = h.home.path().to_path_buf();
    let real = home.with_file_name(format!(
        "{}-target",
        home.file_name().unwrap().to_string_lossy()
    ));
    fs::rename(&home, &real).unwrap();
    std::os::unix::fs::symlink(&real, &home).unwrap();
    let _cleanup = Sandbox(real.clone());
    fs::write(real.join(".gitconfig"), "[user]\n\tname = Ada\n").unwrap();
    let gh = hand_off_entry();
    let _copy = write_bootstrap(h.home.path(), &gh);
    script_same_account(&h.runner);
    script_pending_hand_off(&h.runner);
    let mut planned = plan(vec![gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "setup-git wrote through a symlink on the way to ~/.gitconfig: {calls:#?}"
    );
}

#[test]
fn an_include_if_github_helper_is_left_alone() {
    let h = Harness::new("include-if");
    let home = h.home.path();
    let included = home.join("dotfiles-gitconfig");
    fs::write(
        &included,
        "[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other-helper\n",
    )
    .unwrap();
    fs::write(
        home.join(".gitconfig"),
        "[includeIf \"gitdir:~/code/\"]\n\tpath = dotfiles-gitconfig\n",
    )
    .unwrap();
    h.runner.on(
        |command| file_name(command) == "git" && args_contain(command, "--get-regexp"),
        [Reply::ok("")],
    );
    let gh = hand_off_entry();
    let _copy = write_bootstrap(home, &gh);
    script_same_account(&h.runner);
    script_pending_hand_off(&h.runner);
    let mut planned = plan(vec![gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "setup-git ran even though an included gitconfig already has a GitHub helper: {calls:#?}"
    );
}

#[test]
fn setup_git_does_not_run_when_command_line_tools_are_absent() {
    let h = Harness::new("no-stub");
    let gh = hand_off_entry();
    let _copy = write_bootstrap(h.home.path(), &gh);
    h.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "CommandLineTools"),
        [Reply::fails(1, "")],
    );
    script_same_account(&h.runner);
    script_pending_hand_off(&h.runner);
    let mut planned = plan(vec![gh]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(
        !calls.iter().any(|command| file_name(command) == "git"),
        "the hand-off invoked git"
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git still ran without Command Line Tools, so it will hit the git stub: {calls:#?}"
    );
}

#[test]
fn a_brew_download_does_not_carry_the_helper() {
    let h = Harness::new("brew-fetch");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    let mut planned = plan(vec![install(
        "tailscale",
        vec![Probe::BrewCask {
            name: "tailscale".into(),
        }],
        vec![Step::BrewCask {
            name: "tailscale".into(),
            admin: true,
            unless: None,
        }],
    )]);
    let calls = h.perform(
        &mut planned,
        &CountingGate {
            forgotten: Arc::new(AtomicUsize::new(0)),
        },
    );
    let brews: Vec<_> = calls
        .iter()
        .filter(|command| file_name(command) == "brew")
        .collect();
    let fetch_without_helper = brews.iter().any(|command| {
        !has_helper(command)
            && command
                .args
                .iter()
                .any(|arg| arg == "fetch" || arg == "download")
    });
    assert!(
        fetch_without_helper,
        "the cask download is inside the privileged brew and the socket stays up for it: {brews:#?}"
    );
}

fn sudo_script(sandbox: &Sandbox, body: &str) -> PathBuf {
    let path = sandbox.path().join("sudo");
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn gate_with_real_helper(sandbox: &Sandbox, body: &str) -> SocketGate {
    SocketGate::new(
        PathBuf::from(env!("CARGO_BIN_EXE_soulcaster-askpass")),
        soulcaster_engine::PeerRule::PathOnly,
    )
    .with_sudo(sudo_script(sandbox, body))
}

#[test]
fn a_locked_sudo_is_unavailable_rather_than_a_wrong_password() {
    let sandbox = Sandbox::new("locked");
    let gate = gate_with_real_helper(
        &sandbox,
        "#!/bin/sh\nif [ \"$1\" = \"-k\" ]; then exit 0; fi\necho \"sudo: account is locked\" >&2\nexit 1\n",
    );
    let result = gate.validate(
        &ProcessRunner::new(),
        sandbox.path(),
        &Password::new(SECRET.to_string()),
    );
    assert!(
        matches!(result, Validation::Unavailable(_)),
        "a locked account was classified as {result:?}, so the Run will try the password again"
    );
}

#[test]
fn a_missing_askpass_answer_is_unavailable_rather_than_a_wrong_password() {
    let sandbox = Sandbox::new("no-answer");
    let gate = gate_with_real_helper(
        &sandbox,
        "#!/bin/sh\nif [ \"$1\" = \"-k\" ]; then exit 0; fi\necho \"sudo: no password was provided\" >&2\nexit 1\n",
    );
    let result = gate.validate(
        &ProcessRunner::new(),
        sandbox.path(),
        &Password::new(SECRET.to_string()),
    );
    assert!(
        matches!(result, Validation::Unavailable(_)),
        "sudo never received a password and was classified as {result:?}"
    );
}

#[test]
fn the_app_check_requires_the_askpass_identifier() {
    let script = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/check-app.sh"),
    )
    .unwrap();
    assert!(
        script.contains("com.nullworks.macflayer.askpass"),
        "check-app.sh never checks the helper identifier PeerRule::Signed requires"
    );
}
