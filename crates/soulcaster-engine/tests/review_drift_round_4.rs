//! Review of Drift and the Run's writes, round 4. The runner is a fake and `HOME` is a temp
//! directory, so nothing here reads or changes this Mac.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Choice, Report, RowKind};
use soulcaster_core::event::{ConfigRef, RunScope};
use soulcaster_core::lifecycle::Assurance;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{Clock, LastApplied, MachineRecord, RunInput, check_drift, perform};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review-drift-4-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
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

fn line(command: &Command) -> String {
    let mut out = vec![command.program.to_string_lossy().into_owned()];
    out.extend(command.args.iter().cloned());
    out.join(" ")
}

fn config(text: &str) -> Config {
    Config::parse(text.as_bytes()).unwrap()
}

fn one_setting(id: &str, value: Value) -> Config {
    config(&json!({"schemaVersion": 1, "name": "x", "settings": {id: value}}).to_string())
}

fn absent_by_default(runner: &FakeRunner) {
    runner.on(|_| true, [Reply::fails(1, "")]);
}

fn answer(runner: &FakeRunner, needle: &'static str, reply: Reply) {
    runner.on(move |command| line(command).contains(needle), [reply]);
}

/// Answers commands whose line ends with `needle`: one key's name can be the start of another's.
fn ends(runner: &FakeRunner, needle: &'static str, reply: Reply) {
    runner.on(move |command| line(command).ends_with(needle), [reply]);
}

fn remember(home: &Path, entries: &[(&str, Value)]) {
    let mut record = MachineRecord::load(home).unwrap();
    for (id, value) in entries {
        record.last_applied.insert(
            (*id).to_string(),
            LastApplied {
                value: value.clone(),
                applied_at: 1,
                run_id: "run".into(),
                catalog_version: "1".into(),
                macos_version: None,
                assurance: Assurance::Stored,
                prior_value: None,
            },
        );
    }
    record.save(home).unwrap();
}

fn check(runner: &FakeRunner, home: &Path, config: &Config) -> Report {
    let catalog = Catalog::shipped();
    let plan = Plan::new(config, catalog);
    check_drift(runner, home, catalog, &plan, config).unwrap()
}

fn state(report: &Report) -> String {
    let rows: Vec<(String, RowKind)> = report
        .rows
        .iter()
        .map(|row| (row.id.clone(), row.kind))
        .collect();
    format!(
        "rows {rows:?}, could not check {:?}",
        report.could_not_check
    )
}

const NOTHING: &str = "rows [], could not check []";

#[test]
fn a_recent_app_the_dock_suggests_is_not_a_change_to_the_dock_contents() {
    // dockutil 3.1.3's `--list` walks persistent-apps, recent-apps and persistent-others. "Show
    // suggested and recent apps" is on unless someone turned it off, so whatever was opened
    // lately comes back as if it were a tile the person put there.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(
        &runner,
        "dockutil --list",
        Reply::ok(
            "Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\t/p\tcom.apple.mail\n\
             Zed\tfile:///Applications/Zed.app/\trecentApps\t/p\tdev.zed.Zed\n",
        ),
    );
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "Cellar/dockutil", Reply::ok(""));
    absent_by_default(&runner);
    let wanted = json!(["/System/Applications/Mail.app"]);
    remember(sandbox.path(), &[("dock.apps", wanted.clone())]);
    let report = check(&runner, sandbox.path(), &one_setting("dock.apps", wanted));
    assert_eq!(
        state(&report),
        NOTHING,
        "offers to save {:?}",
        report.rows.first().map(|row| row.on_mac.clone())
    );
}

#[test]
fn a_hot_corner_that_needs_a_modifier_key_is_not_reported_as_the_same() {
    // The modifier is a companion only in that the Catalog always writes 0. A corner set with
    // Command held stores 1048576, fires only with Command held, and a Run resets it to 0.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(&runner, "wvous-tl-corner", Reply::ok("5\n"));
    ends(&runner, "wvous-tl-modifier", Reply::ok("1048576\n"));
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("hot-corners.top-left", json!("start-screensaver"))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("hot-corners.top-left", json!("start-screensaver")),
    );
    assert_ne!(
        state(&report),
        NOTHING,
        "a corner that needs Command held reads as exactly what the Machine Type says"
    );
}

#[test]
fn a_dock_with_the_same_app_twice_is_not_offered_for_saving() {
    // The schema says `uniqueItems`, and the row checks only the Catalog's pattern and the lint.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(
        &runner,
        "dockutil --list",
        Reply::ok(
            "Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\t/p\tcom.apple.mail\n\
             Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\t/p\tcom.apple.mail\n",
        ),
    );
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "Cellar/dockutil", Reply::ok(""));
    absent_by_default(&runner);
    let wanted = json!(["/System/Applications/Mail.app"]);
    remember(sandbox.path(), &[("dock.apps", wanted.clone())]);
    let report = check(&runner, sandbox.path(), &one_setting("dock.apps", wanted));
    let row = report
        .rows
        .iter()
        .find(|row| row.id == "dock.apps")
        .expect("a row");
    let saved = json!({
        "schemaVersion": 1,
        "name": "x",
        "settings": {"dock.apps": row.on_mac.clone()},
    });
    assert!(
        Config::parse(saved.to_string().as_bytes()).is_err(),
        "the Machine Type has to refuse this for the offer to be wrong"
    );
    assert!(
        !row.choices().contains(&Choice::Adopt),
        "offers a save the Machine Type refuses: {:?}",
        row.choices()
    );
}

#[test]
fn a_tap_cask_nobody_catalogued_is_not_offered_under_its_short_name() {
    // `brew list --cask -1` prints only the token, and `--full-name` is what gives the tap. Saved
    // as `tapped-thing`, another Mac installs whatever homebrew/cask calls that, or nothing.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "brew leaves", Reply::ok(""));
    answer(
        &runner,
        "list --cask --full-name",
        Reply::ok("someone/tap/tapped-thing\n"),
    );
    answer(&runner, "list --cask -1", Reply::ok("tapped-thing\n"));
    absent_by_default(&runner);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    let offered: Vec<&str> = report
        .rows
        .iter()
        .filter(|row| row.id == "cask:tapped-thing" && row.choices().contains(&Choice::Adopt))
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(offered, Vec::<&str>::new(), "{}", state(&report));
}

#[test]
fn a_git_ignore_file_set_the_usual_way_is_the_same_file_as_the_config() {
    // GitHub's own instructions are `git config --global core.excludesfile ~/.gitignore_global`.
    // The shell expands that `~`, so git stores the full path of the file the Config names.
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.path().join(".gitconfig"),
        "[core]\n\texcludesfile = x\n",
    )
    .unwrap();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    let stored = format!("{}/.gitignore_global\n", sandbox.path().display());
    answer(&runner, "core.excludesfile", Reply::ok(stored));
    absent_by_default(&runner);
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("git.excludes-file", json!("~/.gitignore_global")),
    );
    assert_eq!(state(&report), NOTHING);
}

#[test]
fn a_finder_window_changed_to_another_folder_is_a_row() {
    // Choosing Home in Finder writes `PfHm` and the home folder's URL. Neither is a value the
    // Catalog names, and the two don't "say the same thing", so this can never be a row.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(
        &runner,
        "com.apple.finder NewWindowTarget",
        Reply::ok("PfHm\n"),
    );
    let home = format!("file://{}/\n", sandbox.path().display());
    ends(
        &runner,
        "com.apple.finder NewWindowTargetPath",
        Reply::ok(home),
    );
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("finder.new-window-target", json!("documents"))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("finder.new-window-target", json!("documents")),
    );
    assert_eq!(
        state(&report),
        r#"rows [("finder.new-window-target", ChangedHere)], could not check []"#
    );
}

struct OpenGate;

struct OpenWindow;

impl soulcaster_engine::AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }
}

impl soulcaster_engine::AdminGate for OpenGate {
    fn validate(
        &self,
        _runner: &dyn CommandRunner,
        _home: &Path,
        _password: &Password,
    ) -> soulcaster_engine::Validation {
        soulcaster_engine::Validation::Accepted
    }

    fn open(
        &self,
        _password: &Password,
        _budget: soulcaster_engine::Budget,
    ) -> Result<Box<dyn soulcaster_engine::AdminWindow>, String> {
        Ok(Box::new(OpenWindow))
    }

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) -> bool {
        true
    }
}

struct NoAsks;

impl Asks for NoAsks {
    fn password(&self, _request: &PasswordRequest) -> Option<Password> {
        Some(Password::new("not-a-real-password".to_string()))
    }

    fn show_github_code(&self, _code: &str, _url: &str) {}

    fn github_status(&self, _status: SignInStatus) {}
}

struct StillClock(AtomicU64);

impl Clock for StillClock {
    fn now_ms(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }

    fn wait(&self, duration: Duration) {
        self.0
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

#[test]
fn a_run_adds_a_dock_app_with_a_tilde_in_its_name_by_its_real_path() {
    // `expand_home` replaces every `~`, not only a leading one, and the Run's Dock write now goes
    // through it. The Catalog's pattern allows this name. After `--remove all`, dockutil is asked
    // to add a path that doesn't exist.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    runner.on(
        |c| line(c).ends_with("id -Gn"),
        [Reply::ok("staff admin\n")],
    );
    runner.on(|c| c.program.ends_with("sw_vers"), [Reply::ok("15.6.1\n")]);
    answer(
        &runner,
        "dockutil --list",
        Reply::ok("Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\n"),
    );
    let name = "/Applications/Thing~Beta.app";
    let config = one_setting("dock.apps", json!([name]));
    let mut plan = Plan::new(&config, Catalog::shipped());
    let mut events = Vec::new();
    let cancel = AtomicBool::new(false);
    perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &runner,
        home: sandbox.path(),
        events: &mut events,
        asks: &NoAsks,
        cancel: &cancel,
        clock: &StillClock(AtomicU64::new(1_000)),
        scope: RunScope::Full,
        config: ConfigRef {
            name: "x".to_string(),
            ..ConfigRef::default()
        },
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();
    let added: Vec<String> = runner
        .calls()
        .iter()
        .filter(|c| c.program.ends_with("dockutil") && c.args.iter().any(|a| a == "--add"))
        .map(line)
        .collect();
    assert!(!added.is_empty(), "the Run never added a tile");
    assert!(
        added.iter().all(|call| call.contains(name)),
        "dockutil was told: {added:?}"
    );
}
