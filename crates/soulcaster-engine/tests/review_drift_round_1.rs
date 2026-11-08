//! Review of Drift's look at the Machine and removal, round 1. The runner is a fake and `HOME` is
//! a temp directory, so nothing here installs, removes or writes anything on this Mac.

use std::collections::BTreeMap;
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
use soulcaster_core::event::{ConfigRef, Event, RunScope};
use soulcaster_core::lifecycle::Assurance;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{
    AdminGate, AdminWindow, Budget, Clock, LastApplied, MachineRecord, Removal, RemovalConfirmed,
    RunInput, Validation, check_drift, keep_here, leave_here, perform, record_saved,
    remove_from_mac,
};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review-drift-{}-{}",
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

fn uninstalls(runner: &FakeRunner) -> Vec<String> {
    runner
        .calls()
        .iter()
        .filter(|command| command.args.first().is_some_and(|arg| arg == "uninstall"))
        .map(line)
        .collect()
}

const EMPTY: &str = r#"{"schemaVersion":1,"name":"x"}"#;

#[test]
fn neither_side_of_a_conflict_the_file_still_lists_is_dropped_or_uninstalled() {
    // The plan names only one side in `item`; the other side is just as much in the file.
    let mut wrong = Vec::new();
    for (text, other, cellar) in [
        (
            r#"{"schemaVersion":1,"name":"x","items":["redis","valkey"]}"#,
            "valkey",
            "Cellar/valkey",
        ),
        (
            r#"{"schemaVersion":1,"name":"x","items":["deno"],"packages":{"formulae":["dxpy"]}}"#,
            "formula:dxpy",
            "Cellar/dxpy",
        ),
    ] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        answer(&runner, cellar, Reply::ok(""));
        absent_by_default(&runner);
        remember(sandbox.path(), &[(other, json!(true))]);
        let config = config(text);
        let report = check(&runner, sandbox.path(), &config);
        if report.rows.iter().any(|row| row.id == other) {
            wrong.push(format!("{other} is a row: {:?}", kinds(&report)));
        }
        let out = remove(&runner, sandbox.path(), &config, &[other]);
        if out[other] != Removal::Refused || !uninstalls(&runner).is_empty() {
            wrong.push(format!(
                "{other}: {:?}, {:?}",
                out[other],
                uninstalls(&runner)
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn what_a_skipped_entry_needs_is_not_dropped_or_uninstalled() {
    // "One bad setting value skips that entry only and stays in the file." dock.apps still wants
    // dockutil, even though a bad value keeps it out of this Run's plan.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/dockutil", Reply::ok(""));
    absent_by_default(&runner);
    remember(sandbox.path(), &[("dockutil", json!(true))]);
    let config =
        config(r#"{"schemaVersion":1,"name":"x","settings":{"dock.apps":["/nowhere/App.app"]}}"#);
    let report = check(&runner, sandbox.path(), &config);
    assert!(
        !report.rows.iter().any(|row| row.id == "dockutil"),
        "{:?}",
        kinds(&report)
    );
    let out = remove(&runner, sandbox.path(), &config, &["dockutil"]);
    assert_eq!(out["dockutil"], Removal::Refused);
    assert_eq!(uninstalls(&runner), Vec::<String>::new());
}

#[test]
fn a_hostile_key_in_the_record_never_reaches_brew_uninstall() {
    let mut wrong = Vec::new();
    for key in [
        "formula:--force",
        "formula:-f",
        "cask:--zap",
        // The presence check looks only at the last part, so this passes wherever gh is installed.
        "formula:--force/gh",
        "formula:",
        "formula:..",
        "formula:x/../gh",
        // APFS finds Cellar/gh under this spelling, and Homebrew lowercases it before looking.
        "formula:GH",
    ] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        remember(sandbox.path(), &[(key, json!(true))]);
        let out = remove(&runner, sandbox.path(), &config(EMPTY), &[key]);
        if out[key] != Removal::Refused || !uninstalls(&runner).is_empty() {
            wrong.push(format!("{key}: {:?}, {:?}", out[key], uninstalls(&runner)));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn a_recorded_name_is_not_uninstalled_while_the_config_wants_the_same_keg_under_a_tap() {
    // Homebrew's resolve_kegs takes every keg in Cellar/<name> for an untapped name. The Config
    // moved from `nobody-covers-this` to `some/tap/nobody-covers-this`, and both are that rack.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    let config = config(
        r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["some/tap/nobody-covers-this"]}}"#,
    );
    let out = remove(
        &runner,
        sandbox.path(),
        &config,
        &["formula:nobody-covers-this"],
    );
    assert_eq!(out["formula:nobody-covers-this"], Removal::Refused);
    assert_eq!(uninstalls(&runner), Vec::<String>::new());
}

#[test]
fn a_tap_cask_homebrew_lists_by_its_short_name_is_not_extra() {
    // `brew list --cask -1` prints a cask's token, so a tap's cask comes back without its tap.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "Caskroom/aerospace", Reply::ok(""));
    answer(&runner, "brew list", Reply::ok("aerospace\n"));
    answer(&runner, "brew leaves", Reply::ok(""));
    absent_by_default(&runner);
    remember(sandbox.path(), &[("aerospace", json!(true))]);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x","items":["aerospace"]}"#),
    );
    assert_eq!(kinds(&report), []);
}

#[test]
fn a_tool_that_could_not_be_started_is_could_not_check_and_not_removed() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "build.txt", Reply::ok(""));
    answer(
        &runner,
        ".local/bin/kotlin-lsp",
        Reply::SpawnFails("Resource temporarily unavailable (os error 35)".into()),
    );
    absent_by_default(&runner);
    remember(sandbox.path(), &[("kotlin-lsp", json!(true))]);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x","items":["kotlin-lsp"]}"#),
    );
    assert_eq!(kinds(&report), []);
    assert_eq!(report.could_not_check, ["kotlin-lsp"]);
}

#[test]
fn a_name_homebrew_could_not_have_given_is_not_a_row() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "brew leaves", Reply::ok("-rf\n--force\n@x\n"));
    answer(&runner, "brew list", Reply::ok("--zap\n"));
    absent_by_default(&runner);
    let report = check(&runner, sandbox.path(), &config(EMPTY));
    assert_eq!(kinds(&report), []);
}

#[test]
fn a_look_never_starts_homebrew_without_the_developer_tools() {
    // brew.sh runs check-git-version at every start. With no developer tools and no brewed git,
    // utils/tool.sh answers with `brew install git`, so `brew leaves` is not a read on that Mac.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::fails(1, ""));
    answer(&runner, "Cellar/git", Reply::fails(1, ""));
    runner.on(|_| true, [Reply::ok("")]);
    check(&runner, sandbox.path(), &config(EMPTY));
    let started: Vec<String> = runner
        .calls()
        .iter()
        .filter(|command| command.program == Path::new("/opt/homebrew/bin/brew"))
        .map(line)
        .collect();
    assert_eq!(started, Vec::<String>::new());
}

#[test]
fn a_keep_ends_once_the_entry_is_back_in_the_config_so_a_later_drop_is_asked_about() {
    // The design: "Not asked again about this keep". The model forgets a keep once the Config
    // lists the entry again, so dropping it after a later Apply is a new question.
    let sandbox = Sandbox::new();
    let id = "formula:nobody-covers-this";
    remember(sandbox.path(), &[(id, json!(true))]);
    keep_here(sandbox.path(), id).unwrap();

    let listed =
        config(r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["nobody-covers-this"]}}"#);
    let mut plan = Plan::new(&listed, Catalog::shipped());
    let everything_there = FakeRunner::new();
    let mut events = Vec::<Event>::new();
    perform(RunInput {
        plan: &mut plan,
        runner: &everything_there,
        admin: &OpenGate,
        dotfiles: None,
        home: sandbox.path(),
        events: &mut events,
        asks: &Quiet,
        cancel: &AtomicBool::new(false),
        clock: &Fixed,
        scope: RunScope::Full,
        config: ConfigRef::default(),
        catalog_version: "t",
        engine_version: "t",
    })
    .unwrap();
    assert!(
        MachineRecord::load(sandbox.path())
            .unwrap()
            .last_applied
            .contains_key(id),
        "the Apply recorded it"
    );

    let runner = FakeRunner::new();
    answer(&runner, "Cellar/nobody-covers-this", Reply::ok(""));
    absent_by_default(&runner);
    let report = check(&runner, sandbox.path(), &config(EMPTY));
    assert_eq!(kinds(&report), [(id.to_string(), RowKind::Dropped)]);
}

#[test]
fn putting_a_left_alone_entry_back_in_the_config_ends_leaving_it_alone() {
    // The design: "It reappears only if the entry leaves the Config." It left, and the person
    // then chose "Put it back in your Config", so it is no longer hidden and skipped here.
    let sandbox = Sandbox::new();
    let id = "keyboard.key-repeat";
    remember(sandbox.path(), &[(id, json!(2))]);
    leave_here(sandbox.path(), id).unwrap();

    let runner = FakeRunner::new();
    answer(&runner, "read NSGlobalDomain KeyRepeat", Reply::ok("2\n"));
    absent_by_default(&runner);
    let report = check(&runner, sandbox.path(), &config(EMPTY));
    let row = report.rows.iter().find(|row| row.id == id).unwrap().clone();
    assert_eq!(row.kind, RowKind::Dropped);
    assert!(row.choices().contains(&Choice::AddBack));
    record_saved(sandbox.path(), &[row.reviewed()], "1", &Fixed).unwrap();

    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":2}}"#),
    );
    assert_eq!(report.unavailable, Vec::<String>::new());
}
