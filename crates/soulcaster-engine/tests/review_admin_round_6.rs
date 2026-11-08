//! Round 6 of the Admin Stage review. This states behaviour `83889e0` still misses.
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

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review6-{name}-{}-{}",
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

fn latin1_with(rest: &str) -> Vec<u8> {
    let mut bytes = b"# Jos".to_vec();
    bytes.push(0xE9);
    bytes.extend(rest.as_bytes());
    bytes
}

fn assert_unreadable_as_utf8(path: &Path) {
    let bytes = fs::read(path).unwrap();
    assert!(
        bytes.contains(&0xE9),
        "precondition: the file carries a byte that is not UTF-8"
    );
    assert!(
        fs::read_to_string(path).is_err(),
        "precondition: read_to_string fails, so both checks treat this file as empty"
    );
    assert!(
        fs::symlink_metadata(path).unwrap().file_type().is_file(),
        "the config has to be a regular file, so the link check is not what skips setup"
    );
}

/// Git skips nothing here except a comment. A single non-UTF-8 byte in that comment makes
/// `fs::read_to_string` fail, and both checks then treat `~/.gitconfig` as absent. The bytes
/// still contain `[include]` of a file that sets `credential.https://github.com.helper`.
///
/// Checked with git 2.54.0 (Apple Git-157) in a temp `HOME`, `GIT_CONFIG_NOSYSTEM=1`, not this
/// Machine's config. `git config --file ~/.gitconfig --list --includes` showed the helper coming
/// from `./other`, and `git credential fill` for `https://github.com` answered `username=other`.
/// After the two commands `gh` `ConfigureOurs` runs (`--replace-all
/// credential.https://github.com.helper ''`, then `--add` of a helper), `./other` was unchanged
/// and the fill answered with the added helper. The blank value clears helpers already gathered
/// from the include. The same commands are what gh 2.96.0 and gh 2.103.3 run.
#[test]
fn a_non_utf8_byte_hides_an_include_and_setup_git_replaces_that_helper() {
    let h = Harness::new("latin1-include");
    let home = h.home.path().to_path_buf();
    fs::write(
        home.join("other"),
        "[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other\n",
    )
    .unwrap();
    let gitconfig = home.join(".gitconfig");
    fs::write(&gitconfig, latin1_with("\n[include]\n\tpath = ./other\n")).unwrap();
    assert_unreadable_as_utf8(&gitconfig);
    let bytes = fs::read(&gitconfig).unwrap();
    let lossy = String::from_utf8_lossy(&bytes);
    assert!(
        lossy.to_lowercase().contains("include"),
        "precondition: the include is in the bytes"
    );
    let folded: String = lossy
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\\')
        .collect();
    assert!(
        !(folded.contains("credential") && folded.contains("github")),
        "precondition: this file does not itself name a GitHub credential, so only the include \
         check could see the helper"
    );

    let calls = hand_off_calls(&h);
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. ~/.gitconfig is Latin-1 (a 0xE9 in a comment) and then [include] \
         of a file that sets credential.https://github.com.helper. Git follows the include; \
         read_to_string fails so both checks treat the file as empty, and --replace-all's blank \
         helper clears the included one: {calls:#?}"
    );
}

/// The same unread file, with the helper written directly instead of included. A lossy read
/// contains both `credential` and `github` and does not contain `include`, so fixing only the
/// include scan still runs `setup-git`. `--replace-all` then overwrites this helper.
#[test]
fn a_non_utf8_byte_hides_a_github_helper_and_setup_git_replaces_it() {
    let h = Harness::new("latin1-helper");
    let home = h.home.path().to_path_buf();
    let gitconfig = home.join(".gitconfig");
    fs::write(
        &gitconfig,
        latin1_with("\n[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other\n"),
    )
    .unwrap();
    assert_unreadable_as_utf8(&gitconfig);
    let bytes = fs::read(&gitconfig).unwrap();
    let lossy = String::from_utf8_lossy(&bytes);
    let folded: String = lossy
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\\')
        .collect();
    assert!(
        folded.contains("credential") && folded.contains("github"),
        "precondition: the helper is in the bytes"
    );
    assert!(
        !lossy.to_lowercase().contains("include"),
        "precondition: the include scan cannot be what saves this file"
    );

    let calls = hand_off_calls(&h);
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. ~/.gitconfig is Latin-1 and sets \
         credential.https://github.com.helper to another program. read_to_string fails so the \
         helper check never sees it, and --replace-all overwrites it: {calls:#?}"
    );
}
