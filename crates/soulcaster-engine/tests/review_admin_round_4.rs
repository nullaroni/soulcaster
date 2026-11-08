//! Round 4 of the Admin Stage review. These state behaviour `f0fea48` still misses.
//! Each one fails on this branch for the reason in its assertion.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::event::{ConfigRef, Event, RunScope};
use soulcaster_core::lifecycle::{FailureKind, ItemState, Stage};
use soulcaster_core::plan::{Plan, PlanEntry, RunGroup, Why};
use soulcaster_core::recipe::{BuiltinStep, InstallKind, InstallRecipe, Probe, Recipe, Step};
use soulcaster_engine::{
    AdminGate, AdminWindow, Budget, Clock, PeerRule, RunInput, RunReport, SocketGate, Validation,
    perform,
};

const SECRET: &str = "s3cret-plover";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review4-{name}-{}-{}",
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

    fn perform(&self, plan: &mut Plan, admin: &dyn AdminGate) -> (RunReport, Vec<Command>) {
        self.runner.on(
            |command| file_name(command) == "id",
            [Reply::ok("staff admin\n")],
        );
        self.runner.on(
            |command| file_name(command) == "sw_vers",
            [Reply::ok("15.6.1\n")],
        );
        let mut events: Vec<Event> = Vec::new();
        let report = perform(RunInput {
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
        (report, self.runner.calls())
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

fn state<'a>(report: &'a RunReport, id: &str) -> &'a ItemState {
    report
        .states
        .get(id)
        .unwrap_or_else(|| panic!("no state for {id}"))
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
        .join("Library/Application Support/Soulcaster/bootstrap-gh")
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
        PathBuf::from(env!("CARGO_BIN_EXE_soulcaster-askpass")),
        PeerRule::PathOnly,
    )
    .with_sudo(path)
}

fn is_clear(command: &Command) -> bool {
    file_name(command) == "sudo" && args_eq(command, &["-k"])
}

fn is_validate(command: &Command) -> bool {
    file_name(command) == "sudo" && args_eq(command, &["-A", "-v"])
}

fn is_setup_git(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "setup-git")
}

fn is_dialog(command: &Command) -> bool {
    file_name(command) == "xcode-select" && args_contain(command, "--install")
}

/// The hand-off, with `~/.gitconfig` set to `gitconfig`. Command Line Tools are present
/// (an unscripted `test` exits 0), the copy and Homebrew's `gh` report the same account,
/// and nothing in the file is an include or a symlink.
fn hand_off_with_gitconfig(name: &str, gitconfig: &str) -> Vec<Command> {
    let h = Harness::new(name);
    let home = h.home.path().to_path_buf();
    let path = home.join(".gitconfig");
    fs::write(&path, gitconfig).unwrap();
    let meta = fs::symlink_metadata(&path).unwrap();
    assert!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "the config has to be a regular file, so the link check is not what skips setup"
    );
    assert!(
        !gitconfig.to_lowercase().contains("[include"),
        "the config has no include, so the include check is not what skips setup"
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
    let (_report, calls) = h.perform(&mut planned, &QuietGate);
    calls
}

/// `git config --global --replace-all credential.https://github.com.helper` rewrites a
/// helper written on the section line. Git accepts that form: the remainder of the
/// header line is a variable. Checked with git 2.54, which reads it as
/// `credential.https://github.com.helper` and then replaces it.
#[test]
fn a_helper_on_the_section_line_is_still_rewritten() {
    let calls = hand_off_with_gitconfig(
        "same-line-helper",
        "[credential \"https://github.com\"] helper = !/usr/local/bin/other\n",
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. Git reads a helper on the section line as \
         credential.https://github.com.helper, and --replace-all overwrites it: {calls:#?}"
    );
}

/// A `#` or `;` comment is not part of the value. Git 2.54 stores `osxkeychain` here.
/// The sentinel check sees the comment, treats the line as `gh`'s own helper, and
/// `setup-git` replaces `osxkeychain`.
#[test]
fn a_comment_can_make_another_helper_look_like_ghs() {
    let calls = hand_off_with_gitconfig(
        "comment-helper",
        "[credential \"https://github.com\"]\n\thelper = osxkeychain # not gh auth git-credential\n",
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. The value git stores is osxkeychain; the comment is what \
         contains `gh auth git-credential`, and --replace-all overwrites osxkeychain: {calls:#?}"
    );
}

/// A backslash before an ordinary character is dropped. Git 2.54 reads this subsection
/// as `https://github.com` and `--replace-all credential.https://github.com.helper`
/// overwrites the helper. The raw text does not contain the letters `github` in a row.
#[test]
fn an_escaped_github_subsection_is_still_rewritten() {
    let calls = hand_off_with_gitconfig(
        "escaped-subsection",
        "[credential \"https://git\\hub.com\"]\n\thelper = !/usr/local/bin/other\n",
    );
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. Git drops the backslash, so this is \
         credential.https://github.com.helper, and --replace-all overwrites it: {calls:#?}"
    );
}

/// The clear after a step used to fall through the retry loop into `NeedsAdmin`.
/// Skipping the backoff now returns the network error itself. For Command Line Tools
/// that is not the needs-admin queue: Apple's dialog opens while the timestamp a
/// failed `sudo -k` may have left is still valid.
#[test]
fn a_failed_clear_after_a_network_error_opens_apples_dialog() {
    let h = Harness::new("network-after-clear");
    h.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "")],
    );
    // The clear inside validate, then the clear after the check. The clear after the
    // install fails, and so does the one `discard` makes.
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
        |command| file_name(command) == "softwareupdate" && args_contain(command, "-l"),
        [Reply::ok(
            "Software Update found the following:\n\
             * Label: Command Line Tools for Xcode-16.4\n\
             \tTitle: Command Line Tools\n",
        )],
    );
    // The install is `sudo -A -- softwareupdate -i`, so the program is the stand-in.
    h.runner.on(
        |command| args_contain(command, "softwareupdate") && args_contain(command, "-i"),
        [Reply::fails(1, "operation timed out")],
    );
    let gate = stand_in_gate(h.home.path());
    let mut planned = plan(vec![install(
        "command-line-tools",
        vec![Probe::File {
            path: "/tmp/soulcaster-review4-clt-missing".into(),
        }],
        vec![Step::Builtin {
            name: BuiltinStep::CommandLineTools,
            admin: true,
            unless: None,
        }],
    )]);
    let (report, calls) = h.perform(&mut planned, &gate);
    let installs = calls
        .iter()
        .filter(|command| args_contain(command, "softwareupdate") && args_contain(command, "-i"))
        .count();
    assert_eq!(
        installs, 1,
        "the quiet install did not run once, so this did not reach the clear after it: {calls:#?}"
    );
    assert!(
        calls.iter().filter(|command| is_clear(command)).count() >= 3,
        "the clear after the install never ran: {calls:#?}"
    );
    let got = state(&report, "command-line-tools");
    let dialog = calls.iter().any(is_dialog);
    assert!(
        matches!(
            got,
            ItemState::NeedsYou {
                cause: Some(FailureKind::NeedsAdmin)
            }
        ) && !dialog,
        "sudo -k failed after softwareupdate timed out, so this should be the needs-admin \
         queue and Apple's dialog should stay closed. state={got:?} dialog={dialog} \
         queue={:?} calls={calls:#?}",
        report.queue
    );
}
