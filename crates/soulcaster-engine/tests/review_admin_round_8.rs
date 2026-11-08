//! Round 8 of the Admin Stage review. `8538e3b` leaves `gh auth setup-git` alone when
//! `~/.git` exists or a startup file names `XDG_CONFIG_HOME`. These tests fail on this
//! branch for the reason in their assertions.

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

const STARTUP_FILES: [&str; 7] = [
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".bash_profile",
    ".bashrc",
    ".profile",
];

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review8-{name}-{}-{}",
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

fn xdg_not_in_this_process() {
    assert!(
        std::env::var_os("XDG_CONFIG_HOME").is_none(),
        "precondition: this process has no XDG_CONFIG_HOME"
    );
}

fn no_dot_git(home: &Path) {
    assert!(
        fs::symlink_metadata(home.join(".git")).is_err(),
        "precondition: ~/.git does not exist"
    );
}

/// The seven files the scanner reads do not contain the contiguous name.
fn scanned_files_do_not_name_xdg(home: &Path) {
    for file in STARTUP_FILES {
        let Ok(bytes) = fs::read(home.join(file)) else {
            continue;
        };
        assert!(
            !bytes
                .windows(b"XDG_CONFIG_HOME".len())
                .any(|window| window == b"XDG_CONFIG_HOME"),
            "precondition: {file} names XDG_CONFIG_HOME, so the scanner would already skip"
        );
    }
}

fn write_xdg_helper(home: &Path) {
    let config = home.join(".xdg/git/config");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, OTHER_HELPER).unwrap();
    assert!(
        fs::symlink_metadata(&config).unwrap().is_file(),
        "precondition: the XDG config is a regular file"
    );
}

fn assert_setup_did_not_run(calls: &[Command], why: &str) {
    let setup = calls.iter().find(|command| is_setup_git(command));
    let cwd = setup.and_then(|command| command.cwd.as_deref());
    let ceiling = setup.and_then(|command| {
        command
            .env
            .iter()
            .find(|(key, _)| key == "GIT_CEILING_DIRECTORIES")
            .map(|(_, value)| value.as_str())
    });
    assert!(
        setup.is_none(),
        "gh auth setup-git ran (cwd {cwd:?}, GIT_CEILING_DIRECTORIES={ceiling:?}). {why}\n{calls:#?}"
    );
}

/// Home is a bare repository. There is no `~/.git`, which is all the new check looks for.
/// Git still uses `$HOME/config` when the working directory is that home.
///
/// Checked with git 2.54.0 (Apple Git-157) in a temp `HOME`, `GIT_CONFIG_NOSYSTEM=1`,
/// not this Machine's config. `git init --bare` left no `.git`. `config` held
/// `helper = !…/other`. Before the two `ConfigureOurs` commands, `git credential fill`
/// for `https://github.com` answered `username=other` and the log showed only `other`.
/// `git -C / config --list` with `GIT_CEILING_DIRECTORIES=/` showed nothing from that
/// repo. After `--replace-all credential.https://github.com.helper ''` and `--add` of a
/// helper that prints `username=gh`, run from `/` with that ceiling, `config` was
/// unchanged and the fill answered `username=gh`. The log showed only `gh`.
#[test]
fn a_bare_repo_at_home_keeps_its_github_helper_from_being_shadowed() {
    let h = Harness::new("bare-home");
    let home = h.home.path().to_path_buf();
    fs::write(home.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(home.join("config"), OTHER_HELPER).unwrap();
    fs::create_dir_all(home.join("objects")).unwrap();
    fs::create_dir_all(home.join("refs")).unwrap();
    user_configs_absent(&home);
    no_dot_git(&home);
    xdg_not_in_this_process();
    scanned_files_do_not_name_xdg(&home);
    assert!(
        fs::symlink_metadata(home.join("config")).unwrap().is_file(),
        "precondition: the bare repo config is a regular file"
    );

    let calls = hand_off_calls(&h);
    assert_setup_did_not_run(
        &calls,
        "HOME is a bare repo: no ~/.git, and config sets \
         credential.https://github.com.helper. The fill answered username=other before \
         the two ConfigureOurs commands and username=gh after. The ceiling kept the write \
         from reading the repo; the blank helper in the new ~/.gitconfig is what answered.",
    );
}

/// `~/.zshenv` sources another file. That file exports `XDG_CONFIG_HOME`. The startup
/// file never contains those bytes, so the scanner does not see it. zsh does.
///
/// Checked with zsh 5.9 and git 2.54.0 (Apple Git-157) in a temp `HOME`,
/// `GIT_CONFIG_NOSYSTEM=1`. `zsh -c` printed the XDG path. The XDG file held the other
/// helper. Before the two commands the fill answered `username=other`. After them,
/// with the added helper printing nothing, the same fill failed with
/// `fatal: could not read Username`. The XDG file was unchanged. `~/.gitconfig` held
/// the blank helper and the added one.
#[test]
fn a_sourced_xdg_config_home_is_left_alone() {
    let h = Harness::new("xdg-source");
    let home = h.home.path().to_path_buf();
    fs::create_dir_all(home.join("shell")).unwrap();
    fs::write(home.join(".zshenv"), "source \"$HOME/shell/env\"\n").unwrap();
    fs::write(
        home.join("shell/env"),
        "export XDG_CONFIG_HOME=\"$HOME/.xdg\"\n",
    )
    .unwrap();
    write_xdg_helper(&home);
    user_configs_absent(&home);
    no_dot_git(&home);
    xdg_not_in_this_process();
    scanned_files_do_not_name_xdg(&home);

    let calls = hand_off_calls(&h);
    assert_setup_did_not_run(
        &calls,
        "~/.zshenv sources ~/shell/env, which exports XDG_CONFIG_HOME. The scanned file \
         does not contain that name. ~/.xdg/git/config sets \
         credential.https://github.com.helper. The blank helper written to ~/.gitconfig \
         dropped it: the fill went from username=other to could not read Username.",
    );
}

/// `~/.zshenv` only points `ZDOTDIR` at another directory. A login shell then reads
/// `$ZDOTDIR/.zprofile`, not `~/.zprofile`. The name is in the file zsh reads.
///
/// Checked with zsh 5.9: `zsh -lc` and `zsh -lic`, with `HOME` set to a temp directory,
/// both printed the XDG path. `~/.zshenv` does not contain the name. The fill change is
/// the one in `a_sourced_xdg_config_home_is_left_alone`: once the variable is set, the
/// blank helper drops that file's GitHub helper.
#[test]
fn a_zdotdir_login_shell_sets_xdg_config_home() {
    let h = Harness::new("xdg-zdotdir");
    let home = h.home.path().to_path_buf();
    fs::create_dir_all(home.join(".config/zsh")).unwrap();
    fs::write(
        home.join(".zshenv"),
        "export ZDOTDIR=\"$HOME/.config/zsh\"\n",
    )
    .unwrap();
    fs::write(
        home.join(".config/zsh/.zprofile"),
        "export XDG_CONFIG_HOME=\"$HOME/.xdg\"\n",
    )
    .unwrap();
    write_xdg_helper(&home);
    user_configs_absent(&home);
    no_dot_git(&home);
    xdg_not_in_this_process();
    scanned_files_do_not_name_xdg(&home);
    assert!(
        !home.join(".zprofile").exists(),
        "precondition: ~/.zprofile is absent, so the scanner's zprofile read misses"
    );

    let calls = hand_off_calls(&h);
    assert_setup_did_not_run(
        &calls,
        "~/.zshenv sets ZDOTDIR, and $ZDOTDIR/.zprofile exports XDG_CONFIG_HOME. A login \
         zsh prints that path. The scanner reads ~/.zprofile, which is not there, and \
         ~/.zshenv does not contain the name. The blank helper then drops the XDG file's \
         GitHub helper.",
    );
}

/// Bash, as a login shell, reads `~/.bash_login` when `~/.bash_profile` is absent.
/// That file is not one of the seven the scanner opens.
///
/// Checked with `/bin/bash -lc` in a temp `HOME`: it printed the XDG path.
/// `/bin/bash --noprofile --norc -lc` printed nothing, so the value came from the
/// login file. The fill change is the same one the source test describes.
#[test]
fn a_bash_login_sets_xdg_config_home() {
    let h = Harness::new("xdg-bash-login");
    let home = h.home.path().to_path_buf();
    fs::write(
        home.join(".bash_login"),
        "export XDG_CONFIG_HOME=\"$HOME/.xdg\"\n",
    )
    .unwrap();
    write_xdg_helper(&home);
    user_configs_absent(&home);
    no_dot_git(&home);
    xdg_not_in_this_process();
    scanned_files_do_not_name_xdg(&home);
    for file in STARTUP_FILES {
        assert!(!home.join(file).exists(), "precondition: {file} is absent");
    }

    let calls = hand_off_calls(&h);
    assert_setup_did_not_run(
        &calls,
        "~/.bash_login exports XDG_CONFIG_HOME, and bash -lc reads that file when \
         ~/.bash_profile is absent. None of the seven scanned files exist. \
         ~/.xdg/git/config sets credential.https://github.com.helper, and the blank \
         helper dropped it.",
    );
}
