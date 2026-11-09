//! Round 5 of the Admin Stage review. This states behaviour `8654830` still misses.
//! It fails on this branch for the reason in its assertion.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use macflayer_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use macflayer_core::catalog::Catalog;
use macflayer_core::command::{Command, CommandRunner, FakeRunner, Reply};
use macflayer_core::config::Config;
use macflayer_core::event::{ConfigRef, Event, RunScope};
use macflayer_core::plan::{Plan, PlanEntry};
use macflayer_core::recipe::{BuiltinStep, Recipe, Step};
use macflayer_engine::{AdminGate, AdminWindow, Budget, Clock, RunInput, Validation, perform};

const SECRET: &str = "s3cret-plover";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "macflayer-review5-{name}-{}-{}",
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
            dotfiles: None,
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
        .join("Library/Application Support/MacFlayer/bootstrap-gh")
        .join(&bootstrap.version)
        .join(&bootstrap.binary);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(&binary, "").unwrap();
    binary
}

fn is_setup_git(command: &Command) -> bool {
    file_name(command) == "gh" && args_contain(command, "setup-git")
}

/// Git skips a leading UTF-8 BOM, then follows `[include]`. The engine's include check asks
/// whether a line, after trimming and lowercasing, starts with `[include]`, and the byte order
/// mark is not whitespace, so that line does not match. The substring check reads the same text
/// and never sees the included file. `gh auth setup-git` still runs.
///
/// Checked with git 2.54.0 (Apple Git-157), `git config --global` in a temp `HOME`, not this
/// Machine's config. Before, `git credential fill` for `https://github.com` answered with the
/// helper in the included file. After the two commands `gh` 2.103.3 `ConfigureOurs` runs
/// (`--replace-all credential.https://github.com.helper ''`, then `--add` of gh's helper), the
/// included file was unchanged and the fill answered with gh's helper. The blank value clears
/// helpers already gathered from the include.
#[test]
fn a_bom_hides_an_include_and_setup_git_replaces_that_helper() {
    let h = Harness::new("bom-include");
    let home = h.home.path().to_path_buf();
    let included = home.join("other");
    fs::write(
        &included,
        "[credential \"https://github.com\"]\n\thelper = !/usr/local/bin/other\n",
    )
    .unwrap();
    let gitconfig = home.join(".gitconfig");
    // EF BB BF is the UTF-8 byte order mark. The rest is an ordinary include.
    let mut main = vec![0xEF, 0xBB, 0xBF];
    main.extend(b"[include]\n\tpath = ./other\n");
    fs::write(&gitconfig, main).unwrap();
    let text = fs::read_to_string(&gitconfig).unwrap();
    assert!(
        !text
            .lines()
            .any(|line| line.trim_start().to_lowercase().starts_with("[include")),
        "precondition: the include check sees this file, so it is not the hole"
    );
    let folded: String = text.to_lowercase().chars().filter(|c| *c != '\\').collect();
    assert!(
        !(folded.contains("credential") && folded.contains("github")),
        "precondition: the substring check sees a GitHub credential here"
    );
    assert!(
        fs::symlink_metadata(&gitconfig)
            .unwrap()
            .file_type()
            .is_file(),
        "the config has to be a regular file, so the link check is not what skips setup"
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
    let calls = h.perform(&mut planned);
    assert!(
        !calls.iter().any(is_setup_git),
        "gh auth setup-git ran. ~/.gitconfig starts with a UTF-8 BOM and then [include] of a \
         file that sets credential.https://github.com.helper. Git follows the include; the \
         engine's include check and the substring check both miss it, and --replace-all's blank \
         helper clears the included one: {calls:#?}"
    );
}
