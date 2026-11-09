//! Review of Drift's look at the Machine and removal, round 2. The runner is a fake and `HOME` is
//! a temp directory, so nothing here installs, removes or writes anything on this Mac.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use macflayer_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use macflayer_core::catalog::Catalog;
use macflayer_core::command::{Command, CommandRunner, FakeRunner, Reply};
use macflayer_core::config::Config;
use macflayer_core::drift::{Choice, Report, RowKind};
use macflayer_core::event::{ConfigRef, Event, RunScope};
use macflayer_core::lifecycle::Assurance;
use macflayer_core::plan::Plan;
use macflayer_engine::{
    AdminGate, AdminWindow, Budget, Clock, LastApplied, MachineRecord, Removal, RemovalConfirmed,
    RunInput, Validation, check_drift, keep_here, leave_here, perform, remove_from_mac,
};
use serde_json::{Value, json};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "macflayer-review-drift-2-{}-{}",
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

struct Fixed;

impl Clock for Fixed {
    fn now_ms(&self) -> u64 {
        1_000
    }

    fn wait(&self, _: Duration) {}
}

struct Quiet;

impl Asks for Quiet {
    fn password(&self, _: &PasswordRequest) -> Option<Password> {
        None
    }

    fn show_github_code(&self, _: &str, _: &str) {}

    fn github_status(&self, _: SignInStatus) {}
}

struct OpenWindow;

impl AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner
    }
}

struct OpenGate;

impl AdminGate for OpenGate {
    fn validate(&self, _: &dyn CommandRunner, _: &Path, _: &Password) -> Validation {
        Validation::Accepted
    }

    fn open(&self, _: &Password, _: Budget) -> Result<Box<dyn AdminWindow>, String> {
        Ok(Box::new(OpenWindow))
    }

    fn forget(&self, _: &dyn CommandRunner, _: &Path) -> bool {
        true
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

fn absent_by_default(runner: &FakeRunner) {
    runner.on(|_| true, [Reply::fails(1, "")]);
}

fn answer(runner: &FakeRunner, needle: &'static str, reply: Reply) {
    runner.on(move |command| line(command).contains(needle), [reply]);
}

fn answers(runner: &FakeRunner, needle: &'static str, replies: Vec<Reply>) {
    runner.on(move |command| line(command).contains(needle), replies);
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

fn kinds(report: &Report) -> Vec<(String, RowKind)> {
    report
        .rows
        .iter()
        .map(|row| (row.id.clone(), row.kind))
        .collect()
}

fn remove(
    runner: &FakeRunner,
    home: &Path,
    config: &Config,
    ids: &[&str],
) -> BTreeMap<String, Removal> {
    let catalog = Catalog::shipped();
    let plan = Plan::new(config, catalog);
    let ids: Vec<String> = ids.iter().map(|id| id.to_string()).collect();
    remove_from_mac(
        runner,
        home,
        catalog,
        &plan,
        &ids,
        RemovalConfirmed::by_the_person(),
    )
    .unwrap()
}

fn run(home: &Path, plan: &mut Plan, scope: RunScope) {
    let everything_there = FakeRunner::new();
    let mut events = Vec::<Event>::new();
    perform(RunInput {
        plan,
        runner: &everything_there,
        admin: &OpenGate,
        dotfiles: None,
        home,
        events: &mut events,
        asks: &Quiet,
        cancel: &AtomicBool::new(false),
        clock: &Fixed,
        scope,
        config: ConfigRef::default(),
        catalog_version: "t",
        engine_version: "t",
    })
    .unwrap();
}

const EMPTY: &str = r#"{"schemaVersion":1,"name":"x"}"#;

#[test]
fn removing_one_formula_does_not_let_homebrew_autoremove_others() {
    // cmd/uninstall.rb ends with `Cleanup.autoremove` unless HOMEBREW_NO_AUTOREMOVE is set, and
    // that takes every formula with installed_on_request false and nothing depending on it. A
    // formula the Config lists, already there as another's dependency, is never installed on
    // request by a Run, because the Run finds it present and does nothing.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answers(
        &runner,
        "Cellar/nobody-covers-this",
        vec![Reply::ok(""), Reply::fails(1, "")],
    );
    answer(&runner, "brew uninstall", Reply::ok(""));
    absent_by_default(&runner);
    let id = "formula:nobody-covers-this";
    remember(sandbox.path(), &[(id, json!(true))]);
    let out = remove(&runner, sandbox.path(), &config(EMPTY), &[id]);
    assert_eq!(out[id], Removal::Removed);
    let calls = runner.calls();
    let uninstall = calls
        .iter()
        .find(|command| command.args.first().is_some_and(|arg| arg == "uninstall"))
        .expect("the uninstall ran");
    assert!(
        uninstall
            .env
            .iter()
            .any(|(key, value)| key == "HOMEBREW_NO_AUTOREMOVE" && value == "1"),
        "this uninstalls whatever else Homebrew thinks is unused: {:?}",
        uninstall.env
    );
}

#[test]
fn a_spawn_error_whose_number_only_begins_with_two_is_could_not_check() {
    // The fix matches "os error 2", which is also the start of 20 through 29.
    let mut wrong = Vec::new();
    for error in [
        "Too many open files in system (os error 23)",
        "Too many open files (os error 24)",
        "Text file busy (os error 26)",
    ] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        answer(&runner, "build.txt", Reply::ok(""));
        answer(
            &runner,
            ".local/bin/kotlin-lsp",
            Reply::SpawnFails(error.to_string()),
        );
        absent_by_default(&runner);
        remember(sandbox.path(), &[("kotlin-lsp", json!(true))]);
        let report = check(
            &runner,
            sandbox.path(),
            &config(r#"{"schemaVersion":1,"name":"x","items":["kotlin-lsp"]}"#),
        );
        if !report.rows.is_empty() || report.could_not_check != ["kotlin-lsp"] {
            wrong.push(format!(
                "{error}: {:?}, could not check {:?}",
                kinds(&report),
                report.could_not_check
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn a_git_config_git_cannot_read_is_could_not_check_and_not_removed() {
    // `git config --get` exits 1 for a key that isn't set. A broken file exits 128 and says nothing
    // about the key, and a hand-edited ~/.gitconfig is an ordinary way to get one.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(
        &runner,
        "config --file",
        Reply::fails(128, "fatal: bad config line 3 in file .gitconfig"),
    );
    absent_by_default(&runner);
    remember(sandbox.path(), &[("git.default-branch", json!("main"))]);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x","settings":{"git.default-branch":"main"}}"#),
    );
    assert_eq!(kinds(&report), []);
    assert_eq!(report.could_not_check, ["git.default-branch"]);
}

#[test]
fn a_setting_whose_stores_disagree_is_could_not_check() {
    // tap-to-click is stored twice. Only the first store is read for what the Mac "says".
    let mut wrong = Vec::new();
    for (first, second) in [
        (Reply::fails(1, "does not exist"), Reply::ok("1\n")),
        (Reply::ok("0\n"), Reply::ok("1\n")),
    ] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        answer(
            &runner,
            "read com.apple.AppleMultitouchTrackpad Clicking",
            first,
        );
        answer(
            &runner,
            "read com.apple.driver.AppleBluetoothMultitouch.trackpad Clicking",
            second,
        );
        absent_by_default(&runner);
        remember(sandbox.path(), &[("trackpad.tap-to-click", json!(true))]);
        let report = check(
            &runner,
            sandbox.path(),
            &config(r#"{"schemaVersion":1,"name":"x","settings":{"trackpad.tap-to-click":true}}"#),
        );
        if !report.rows.is_empty() || report.could_not_check != ["trackpad.tap-to-click"] {
            wrong.push(format!(
                "{:?}, could not check {:?}",
                kinds(&report),
                report.could_not_check
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn a_removal_homebrew_cannot_be_started_for_is_not_offered() {
    // remove_from_mac refuses without the developer tools, so the row's choice can't be carried out.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::fails(1, ""));
    answer(&runner, "Cellar/nobody-covers-this", Reply::ok(""));
    absent_by_default(&runner);
    let id = "formula:nobody-covers-this";
    remember(sandbox.path(), &[(id, json!(true))]);
    let report = check(&runner, sandbox.path(), &config(EMPTY));
    let row = report.rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row.kind, RowKind::Dropped);
    let offered = row.choices().contains(&Choice::Remove);
    let out = remove(&runner, sandbox.path(), &config(EMPTY), &[id]);
    assert!(
        !(offered && out[id] == Removal::Refused),
        "the row offers Remove, and choosing it is refused"
    );
}

#[test]
fn an_apply_ends_a_keep_as_a_full_run_does() {
    // The plan a scoped Run holds is the whole Config, and only `scope_of` narrows what it acts on.
    let sandbox = Sandbox::new();
    let id = "formula:nobody-covers-this";
    remember(sandbox.path(), &[(id, json!(true))]);
    keep_here(sandbox.path(), id).unwrap();
    let listed = config(
        r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":2},"packages":{"formulae":["nobody-covers-this"]}}"#,
    );
    let mut plan = Plan::new(&listed, Catalog::shipped());
    assert!(plan.entries.iter().any(|entry| entry.id == id));
    run(
        sandbox.path(),
        &mut plan,
        RunScope::Only {
            items: vec!["keyboard.key-repeat".into()],
        },
    );
    assert!(
        !MachineRecord::load(sandbox.path())
            .unwrap()
            .kept_here
            .contains(id),
        "the Machine Type lists it again, so this keep is over"
    );
}

#[test]
fn an_apply_ends_leaving_it_alone_once_the_entry_left_the_config() {
    let sandbox = Sandbox::new();
    let id = "keyboard.key-repeat";
    remember(sandbox.path(), &[(id, json!(2))]);
    leave_here(sandbox.path(), id).unwrap();
    // It has left the Config. The person Applies some other row.
    let without = config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.autohide":true}}"#);
    let mut plan = Plan::new(&without, Catalog::shipped());
    run(
        sandbox.path(),
        &mut plan,
        RunScope::Only {
            items: vec!["dock.autohide".into()],
        },
    );
    assert!(
        !MachineRecord::load(sandbox.path())
            .unwrap()
            .unavailable
            .contains_key(id),
        "it left the Machine Type, so the answer is over, and if it comes back it isn't silently skipped"
    );
}

#[test]
fn a_full_run_keeps_the_answer_for_what_a_skipped_entry_needs() {
    // dock.apps stays in the file with a bad value and still needs dockutil. set_aside knows that.
    let sandbox = Sandbox::new();
    leave_here(sandbox.path(), "dockutil").unwrap();
    let skipped =
        config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.apps":["/nowhere/App.app"]}}"#);
    let mut plan = Plan::new(&skipped, Catalog::shipped());
    assert!(plan.skipped.iter().any(|entry| entry.raw == "dock.apps"));
    run(sandbox.path(), &mut plan, RunScope::Full);
    assert!(
        MachineRecord::load(sandbox.path())
            .unwrap()
            .unavailable
            .contains_key("dockutil"),
        "dockutil hasn't left the Machine Type"
    );
}

#[test]
fn an_answer_recorded_before_the_catalog_gained_the_item_still_holds() {
    // Last-applied and the look are read through current_id. A person's answers are not.
    // `formula:jq` was the row's id until a Catalog release added the Item that now covers it.
    let sandbox = Sandbox::new();
    leave_here(sandbox.path(), "formula:jq").unwrap();
    let listed = config(r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["jq"]}}"#);
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/jq", Reply::fails(1, ""));
    absent_by_default(&runner);
    let report = check(&runner, sandbox.path(), &listed);
    assert_eq!(kinds(&report), [], "\"don't ask again\" was asked again");
    assert_eq!(report.unavailable, ["jq"]);
}

#[test]
fn a_full_run_does_not_forget_an_answer_recorded_before_the_catalog_gained_the_item() {
    let sandbox = Sandbox::new();
    leave_here(sandbox.path(), "formula:jq").unwrap();
    let listed = config(r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["jq"]}}"#);
    let mut plan = Plan::new(&listed, Catalog::shipped());
    run(sandbox.path(), &mut plan, RunScope::Full);
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        record.unavailable.contains_key("formula:jq") || record.unavailable.contains_key("jq"),
        "the entry never left the Machine Type: {:?}",
        record.unavailable
    );
}

#[test]
fn keeping_an_entry_stops_tracking_it_under_the_name_it_was_recorded_with() {
    // remove_from_mac finds the record through current_id. keep_here takes the row's id and misses.
    let sandbox = Sandbox::new();
    remember(sandbox.path(), &[("formula:jq", json!(true))]);
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/jq", Reply::ok(""));
    absent_by_default(&runner);
    let report = check(&runner, sandbox.path(), &config(EMPTY));
    let row = report.rows.iter().find(|row| row.id == "jq").unwrap();
    assert_eq!(row.kind, RowKind::Dropped);
    keep_here(sandbox.path(), &row.id).unwrap();
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        !record.last_applied.contains_key("formula:jq"),
        "\"stop tracking it\" left the last-applied record"
    );
}
