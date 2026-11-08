//! Round 7 of the Admin Stage review. `eb06cac` stops reading git's config text.
//! Both tests fail on this branch for the reason in their assertions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::event::{ConfigRef, Event, RunScope};
use soulcaster_core::plan::{Plan, PlanEntry};
use soulcaster_core::recipe::{BuiltinStep, Recipe, Step};
use soulcaster_engine::{AdminGate, AdminWindow, Budget, Clock, RunInput, Validation, perform};

const SECRET: &str = "s3cret-plover";
const OTHER_HELPER: &str =
    "[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other\n";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review7-{name}-{}-{}",
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

    fn perform(&self, plan: &mut Plan) -> Vec<Command> {
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
            admin: &QuietGate,
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

fn is_setup_git(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "setup-git")
}

fn hand_off_calls(h: &Harness) -> Vec<Command> {
    let gh = hand_off_entry();
    let _copy = write_bootstrap(h.home.path(), &gh);
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
    h.perform(&mut planned)
}

fn user_configs_absent(home: &Path) {
    assert!(
        !home.join(".gitconfig").exists(),
        "precondition: ~/.gitconfig is missing"
    );
    assert!(
        !home.join(".config/git/config").exists(),
        "precondition: ~/.config/git/config is missing"
    );
}

/// Puts `XDG_CONFIG_HOME` back when the test ends, including on a panic.
struct EnvVar {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

/// `gh auth setup-git` runs with `cwd` = the home directory and no `GIT_CONFIG_*`.
/// Git therefore reads `$GIT_DIR/config` when that home is a repository.
///
/// Checked with git 2.54.0 (Apple Git-157) in a temp `HOME`, `GIT_CONFIG_NOSYSTEM=1`,
/// not this Machine's config. Before the two commands `ConfigureOurs` runs
/// (`--replace-all credential.https://github.com.helper ''`, then `--add` of a helper),
/// `git credential fill` for `https://github.com` answered `username=other`. After them
/// it answered `username=gh`. `.git/config` was unchanged, and a helper that prints
/// nothing still leaves `other` to answer, so this shadows rather than deletes. The
/// same fill change happened when `~/.git` was a `gitdir:` file and when the helper
/// lived only in `.git/config.worktree`.
#[test]
fn a_repo_in_home_keeps_its_github_helper_from_being_shadowed() {
    let h = Harness::new("repo-home");
    let home = h.home.path().to_path_buf();
    fs::create_dir_all(home.join(".git")).unwrap();
    fs::write(home.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(home.join(".git/config"), OTHER_HELPER).unwrap();
    user_configs_absent(&home);
    assert!(
        fs::symlink_metadata(home.join(".git/config"))
            .unwrap()
            .is_file(),
        "precondition: the repo config is a regular file, so the link check is not what saves it"
    );

    let calls = hand_off_calls(&h);
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. The home is the working directory, it is a git repo, and \
         .git/config sets credential.https://github.com.helper. ~/.gitconfig and \
         ~/.config/git/config are both missing, so the new check allows setup-git. The added \
         helper is tried before the repo one, and the fill answered with it: {calls:#?}"
    );
}

/// With the environment cleared, `setup-git`'s git does not see `XDG_CONFIG_HOME` and
/// writes `~/.gitconfig`. A later git that does have the variable reads that new file
/// after the XDG file, and the blank helper drops the XDG one.
///
/// Checked with git 2.54.0 (Apple Git-157) in a temp `HOME`, `GIT_CONFIG_NOSYSTEM=1`.
/// The XDG file was a regular file with the other helper. After the two `ConfigureOurs`
/// commands, that file was byte for byte the same and `~/.gitconfig` held the blank
/// helper plus the added one. `git credential fill` with `XDG_CONFIG_HOME` set then
/// failed to produce a username (`fatal: could not read Username`) when the added
/// helper printed nothing, where the same fill had answered `username=other` before.
/// `GIT_CONFIG_GLOBAL` does not do this: with that variable set, the fill still
/// answered `username=other` and the other file was unchanged.
#[test]
fn xdg_config_home_is_left_alone() {
    let h = Harness::new("xdg-env");
    let home = h.home.path().to_path_buf();
    let xdg = Sandbox::new("xdg-root");
    fs::create_dir_all(xdg.path().join("git")).unwrap();
    fs::write(xdg.path().join("git/config"), OTHER_HELPER).unwrap();
    let _env = EnvVar::set("XDG_CONFIG_HOME", &xdg.path().to_string_lossy());
    user_configs_absent(&home);
    assert!(
        fs::symlink_metadata(xdg.path().join("git/config"))
            .unwrap()
            .is_file(),
        "precondition: the XDG config is a regular file"
    );

    let calls = hand_off_calls(&h);
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. XDG_CONFIG_HOME/git/config sets \
         credential.https://github.com.helper, and ~/.gitconfig and ~/.config/git/config are \
         both missing. The cleared environment makes setup-git write ~/.gitconfig, whose blank \
         helper drops the XDG one on the next git that has the variable: {calls:#?}"
    );
}
