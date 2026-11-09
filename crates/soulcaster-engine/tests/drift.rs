//! Drift's look at the Machine, and what a person decides. The runner is a fake and `HOME` is a
//! temp directory, so nothing here installs, removes or writes anything on this Mac.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Report, Reviewed, RowKind};
use soulcaster_core::lifecycle::Assurance;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{
    Clock, EngineError, LastApplied, MachineRecord, Removal, RemovalConfirmed, check_drift,
    keep_here, leave_here, machine_record_path, record_saved, remove_from_mac, set_waiting_saves,
    survey, waiting_saves,
};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-drift-{}-{}",
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

struct FixedClock;

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        1_000
    }

    fn wait(&self, _duration: Duration) {}
}

fn line(command: &Command) -> String {
    let mut out = vec![command.program.to_string_lossy().into_owned()];
    out.extend(command.args.iter().cloned());
    out.join(" ")
}

fn config(text: &str) -> Config {
    Config::parse(text.as_bytes()).unwrap()
}

/// Everything absent unless a test scripted it first.
fn absent_by_default(runner: &FakeRunner) {
    runner.on(|_| true, [Reply::fails(1, "")]);
}

fn answer(runner: &FakeRunner, needle: &'static str, reply: Reply) {
    runner.on(move |command| line(command).contains(needle), [reply]);
}

fn applied(value: Value) -> LastApplied {
    LastApplied {
        value,
        applied_at: 1,
        run_id: "run".into(),
        catalog_version: "1".into(),
        macos_version: None,
        assurance: Assurance::Stored,
        prior_value: None,
    }
}

fn remember(home: &Path, entries: &[(&str, Value)]) {
    let mut record = MachineRecord::load(home).unwrap();
    for (id, value) in entries {
        record
            .last_applied
            .insert((*id).to_string(), applied(value.clone()));
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

fn only_reads(command: &Command) -> bool {
    let name = command
        .program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let first = command.args.first().map(String::as_str).unwrap_or("");
    match name {
        "test" | "pkgutil" | "sw_vers" | "ls" => true,
        "defaults" => first == "read",
        "plutil" => first == "-extract",
        "git" => first == "config" && command.args.iter().any(|arg| arg == "--get"),
        "dockutil" => first == "--list",
        "brew" => first == "leaves" || first == "list",
        "xcodebuild" => first == "-checkFirstLaunchStatus",
        _ => matches!(first, "--version" | "-version" | "version"),
    }
}

fn everything_config() -> Config {
    let catalog = Catalog::shipped();
    let items: Vec<String> = catalog
        .entries()
        .filter(|entry| entry.listing.is_some())
        .map(|entry| format!("\"{}\"", entry.id()))
        .collect();
    config(&format!(
        r#"{{"schemaVersion":1,"name":"x","items":[{}],"settings":{{"keyboard.key-repeat":2,"dock.apps":["/Applications/Zed.app"]}}}}"#,
        items.join(",")
    ))
}

#[test]
fn a_look_only_reads_and_leaves_no_trace() {
    for all_present in [true, false] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        if !all_present {
            absent_by_default(&runner);
        }
        let config = everything_config();
        check(&runner, sandbox.path(), &config);
        let calls = runner.calls();
        assert!(!calls.is_empty());
        for command in &calls {
            assert!(only_reads(command), "{}", line(command));
            assert!(!line(command).contains("sudo"), "{}", line(command));
            assert!(
                !command.env.iter().any(|(key, _)| key == "SUDO_ASKPASS"),
                "{}",
                line(command)
            );
        }
        assert!(!machine_record_path(sandbox.path()).exists());
    }
}

#[test]
fn a_value_stored_under_another_spelling_is_not_drift() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(
        &runner,
        "read com.apple.dock wvous-bl-corner",
        Reply::ok("2\n"),
    );
    answer(
        &runner,
        "read com.apple.dock wvous-bl-modifier",
        Reply::ok("0\n"),
    );
    absent_by_default(&runner);
    let config = config(
        r#"{"schemaVersion":1,"name":"x","settings":{"hot-corners.bottom-left":"mission-control"}}"#,
    );
    let report = check(&runner, sandbox.path(), &config);
    assert_eq!(kinds(&report), []);
    assert_eq!(report.could_not_check, Vec::<String>::new());
    assert_eq!(report.unavailable, Vec::<String>::new());
}

#[test]
fn a_changed_value_comes_back_in_the_configs_own_terms() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(
        &runner,
        "read com.apple.dock wvous-bl-corner",
        Reply::ok("5\n"),
    );
    answer(
        &runner,
        "read com.apple.dock wvous-bl-modifier",
        Reply::ok("0\n"),
    );
    absent_by_default(&runner);
    let config = config(
        r#"{"schemaVersion":1,"name":"x","settings":{"hot-corners.bottom-left":"mission-control"}}"#,
    );
    remember(
        sandbox.path(),
        &[("hot-corners.bottom-left", json!("mission-control"))],
    );
    let report = check(&runner, sandbox.path(), &config);
    assert_eq!(
        kinds(&report),
        [("hot-corners.bottom-left".to_string(), RowKind::ChangedHere)]
    );
    assert_eq!(report.rows[0].on_mac, Some(json!("start-screensaver")));
    assert_eq!(report.rows[0].in_config, Some(json!("mission-control")));
}

#[test]
fn a_stalled_check_is_could_not_check_and_never_a_row() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "read NSGlobalDomain KeyRepeat", Reply::Stall);
    absent_by_default(&runner);
    let config = config(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":2}}"#);
    remember(sandbox.path(), &[("keyboard.key-repeat", json!(2))]);
    let report = check(&runner, sandbox.path(), &config);
    assert_eq!(kinds(&report), []);
    assert_eq!(report.could_not_check, ["keyboard.key-repeat"]);
}

#[test]
fn a_look_never_runs_the_git_stub_before_the_developer_tools_exist() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    absent_by_default(&runner);
    let catalog = Catalog::shipped();
    let Some(entry) = catalog.settings().find(|entry| {
        serde_json::to_string(&entry.recipe)
            .unwrap()
            .contains("git-config")
    }) else {
        return;
    };
    let id = entry.id().to_string();
    remember(sandbox.path(), &[(&id, json!("x"))]);
    check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    assert!(
        !runner
            .calls()
            .iter()
            .any(|c| c.program == Path::new("/usr/bin/git"))
    );
}

#[test]
fn a_look_never_runs_apples_stubs_without_the_developer_tools() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::fails(1, ""));
    runner.on(|_| true, [Reply::ok("")]);
    check(&runner, sandbox.path(), &everything_config());
    for command in runner.calls() {
        let name = command.program.to_string_lossy().into_owned();
        assert!(
            name != "/usr/bin/git" && name != "/usr/bin/xcodebuild",
            "{}",
            line(&command)
        );
    }
}

#[test]
fn something_homebrew_has_that_the_config_does_not_list_is_extra_once() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(
        &runner,
        "brew leaves",
        Reply::ok("nobody-covers-this\ndockutil\n../../etc/passwd\nBad Name\n"),
    );
    answer(&runner, "brew list", Reply::ok("some-cask\n"));
    answer(&runner, "Cellar/dockutil", Reply::ok(""));
    absent_by_default(&runner);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    assert_eq!(
        kinds(&report),
        [
            ("cask:some-cask".to_string(), RowKind::Extra),
            ("dockutil".to_string(), RowKind::Extra),
            ("formula:nobody-covers-this".to_string(), RowKind::Extra),
        ],
        "a package the Catalog covers is that Item's row"
    );
}

#[test]
fn homebrew_being_unreachable_is_not_a_failure_and_makes_nothing_up() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "brew leaves", Reply::SpawnFails("no brew".into()));
    absent_by_default(&runner);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    assert_eq!(report, Report::default());
}

#[test]
fn something_recorded_that_the_config_dropped_is_a_dropped_row() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "Cellar/nobody-covers-this", Reply::ok(""));
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    assert_eq!(
        kinds(&report),
        [("formula:nobody-covers-this".to_string(), RowKind::Dropped)]
    );
    assert!(report.rows[0].removable);
}

#[test]
fn keeping_it_here_stops_the_asking() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "brew leaves", Reply::ok("nobody-covers-this\n"));
    answer(&runner, "Cellar/nobody-covers-this", Reply::ok(""));
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    keep_here(sandbox.path(), "formula:nobody-covers-this").unwrap();
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x"}"#),
    );
    assert_eq!(kinds(&report), []);
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        !record
            .last_applied
            .contains_key("formula:nobody-covers-this")
    );
}

#[test]
fn leaving_it_alone_shows_it_apart() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "read NSGlobalDomain KeyRepeat", Reply::ok("9\n"));
    absent_by_default(&runner);
    let config = config(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":2}}"#);
    remember(sandbox.path(), &[("keyboard.key-repeat", json!(2))]);
    leave_here(sandbox.path(), "keyboard.key-repeat").unwrap();
    let report = check(&runner, sandbox.path(), &config);
    assert_eq!(kinds(&report), []);
    assert_eq!(report.unavailable, ["keyboard.key-repeat"]);
}

#[test]
fn the_record_moves_only_for_what_github_has() {
    let sandbox = Sandbox::new();
    remember(
        sandbox.path(),
        &[
            ("keyboard.key-repeat", json!(2)),
            ("formula:nobody-covers-this", json!(true)),
        ],
    );
    let saved = [
        Reviewed {
            id: "keyboard.key-repeat".into(),
            key: "keyboard.key-repeat".into(),
            seen: Some(json!(2)),
            write: Some(json!(9)),
        },
        Reviewed {
            id: "formula:nobody-covers-this".into(),
            key: "formula:nobody-covers-this".into(),
            seen: Some(json!(true)),
            write: None,
        },
    ];
    set_waiting_saves(sandbox.path(), &saved).unwrap();
    record_saved(sandbox.path(), &saved[..1], "2", &FixedClock).unwrap();
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert_eq!(record.last_applied["keyboard.key-repeat"].value, json!(9));
    assert!(
        record
            .last_applied
            .contains_key("formula:nobody-covers-this")
    );
    assert_eq!(waiting_saves(sandbox.path()).unwrap(), saved[1..]);
    record_saved(sandbox.path(), &saved[1..], "2", &FixedClock).unwrap();
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        !record
            .last_applied
            .contains_key("formula:nobody-covers-this")
    );
    assert_eq!(waiting_saves(sandbox.path()).unwrap(), []);
}

fn remove(
    runner: &FakeRunner,
    home: &Path,
    config: &Config,
    ids: &[&str],
) -> Result<BTreeMap<String, Removal>, EngineError> {
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
fn removal_uninstalls_with_no_password_and_forgets_the_entry_once_it_is_gone() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    runner.on(
        |command| line(command).contains("Cellar/nobody-covers-this"),
        [Reply::ok(""), Reply::fails(1, "")],
    );
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    let out = remove(
        &runner,
        sandbox.path(),
        &config(EMPTY),
        &["formula:nobody-covers-this"],
    )
    .unwrap();
    assert_eq!(out["formula:nobody-covers-this"], Removal::Removed);
    let done = uninstalls(&runner);
    assert_eq!(
        done,
        ["/opt/homebrew/bin/brew uninstall nobody-covers-this"]
    );
    let command = runner
        .calls()
        .into_iter()
        .find(|c| c.args.first().is_some_and(|a| a == "uninstall"))
        .unwrap();
    assert!(
        command
            .env
            .iter()
            .any(|(k, v)| k == "HOMEBREW_NO_SUDO" && v == "1")
    );
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        !record
            .last_applied
            .contains_key("formula:nobody-covers-this")
    );
}

#[test]
fn a_removal_homebrew_refuses_stays_tracked() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/nobody-covers-this", Reply::ok(""));
    answer(
        &runner,
        "brew uninstall",
        Reply::fails(1, "required by other"),
    );
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    let out = remove(
        &runner,
        sandbox.path(),
        &config(EMPTY),
        &["formula:nobody-covers-this"],
    )
    .unwrap();
    assert_eq!(out["formula:nobody-covers-this"], Removal::Failed);
    let record = MachineRecord::load(sandbox.path()).unwrap();
    assert!(
        record
            .last_applied
            .contains_key("formula:nobody-covers-this")
    );
}

#[test]
fn removal_refuses_everything_the_design_says_it_never_offers() {
    let keeps = r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["nobody-covers-this"]}}"#;
    let unknown = r#"{"schemaVersion":1,"name":"x","items":["no-such-thing"]}"#;
    for (config_text, id, recorded) in [
        (keeps, "formula:nobody-covers-this", true),
        (EMPTY, "formula:nobody-covers-this", false),
        (EMPTY, "gh", true),
        (EMPTY, "homebrew", true),
        (EMPTY, "keyboard.key-repeat", true),
        (EMPTY, "android-emulator", true),
        (EMPTY, "no-such-thing", true),
        (unknown, "formula:nobody-covers-this", true),
    ] {
        let sandbox = Sandbox::new();
        let runner = FakeRunner::new();
        if recorded {
            remember(sandbox.path(), &[(id, json!(true))]);
        }
        let out = remove(&runner, sandbox.path(), &config(config_text), &[id]).unwrap();
        assert_eq!(out[id], Removal::Refused, "{id} with {config_text}");
        assert_eq!(uninstalls(&runner), Vec::<String>::new(), "{id}");
    }
}

#[test]
fn removal_refuses_what_a_check_cannot_find_here() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/nobody-covers-this", Reply::fails(1, ""));
    remember(
        sandbox.path(),
        &[("formula:nobody-covers-this", json!(true))],
    );
    let out = remove(
        &runner,
        sandbox.path(),
        &config(EMPTY),
        &["formula:nobody-covers-this"],
    )
    .unwrap();
    assert_eq!(out["formula:nobody-covers-this"], Removal::Refused);
    assert_eq!(uninstalls(&runner), Vec::<String>::new());
}

#[test]
fn a_decision_does_not_land_in_the_middle_of_a_run() {
    let sandbox = Sandbox::new();
    let support = machine_record_path(sandbox.path())
        .parent()
        .unwrap()
        .to_path_buf();
    fs::create_dir_all(&support).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(support.join("run.lock"))
        .unwrap();
    held.try_lock().unwrap();
    assert!(matches!(
        keep_here(sandbox.path(), "formula:nobody-covers-this"),
        Err(EngineError::Busy)
    ));
    assert!(matches!(
        leave_here(sandbox.path(), "formula:nobody-covers-this"),
        Err(EngineError::Busy)
    ));
    assert!(matches!(
        remove(
            &FakeRunner::new(),
            sandbox.path(),
            &config(EMPTY),
            &["formula:nobody-covers-this"]
        ),
        Err(EngineError::Busy)
    ));
}

fn setting_store(id: &str, index: usize) -> soulcaster_core::recipe::Store {
    let Some(soulcaster_core::recipe::Recipe::Setting(setting)) = Catalog::shipped()
        .entry(id)
        .map(|entry| entry.recipe.clone())
    else {
        panic!("{id} is a setting");
    };
    setting.stores[index].clone()
}

#[test]
fn a_run_writes_a_show_setting_stored_as_hidden_the_right_way_round() {
    let store = setting_store("menu-bar.show-spotlight", 0);
    let home = Path::new("/Users/someone");
    let write = soulcaster_engine::defaults_write_for_test(
        home,
        &store,
        &soulcaster_core::config::SettingValue::Bool(true),
    );
    assert_eq!(
        write.args,
        [
            "write",
            "com.apple.Spotlight",
            "MenuItemHidden",
            "-bool",
            "false"
        ]
    );
}

#[test]
fn a_run_fills_in_the_home_folder_a_setting_stores() {
    let store = setting_store("finder.new-window-target", 1);
    let home = Path::new("/Users/someone");
    let write = soulcaster_engine::defaults_write_for_test(
        home,
        &store,
        &soulcaster_core::config::SettingValue::String("documents".into()),
    );
    assert_eq!(
        write.args.last().unwrap(),
        "file:///Users/someone/Documents/"
    );
}

#[test]
fn a_survey_finds_what_is_here_and_only_the_settings_someone_changed() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(&runner, "Cellar/ripgrep", Reply::ok(""));
    answer(&runner, "read com.apple.dock autohide", Reply::ok("1\n"));
    answer(&runner, "read com.apple.dock tilesize", Reply::ok("48\n"));
    answer(
        &runner,
        "read com.apple.dock show-recents",
        Reply::ok("1\n"),
    );
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "brew leaves", Reply::ok("ripgrep\nhtop\n"));
    answer(&runner, "brew list", Reply::ok("some-cask\n"));
    absent_by_default(&runner);

    let found = survey(&runner, sandbox.path(), Catalog::shipped());

    assert_eq!(found.items, ["ripgrep"]);
    assert_eq!(found.packages, ["htop", "cask:some-cask"]);
    assert!(found.settings.contains_key("dock.autohide"));
    assert!(
        !found.settings.contains_key("dock.show-recents"),
        "show-recents is on by default, so this is nothing the person chose: {:?}",
        found.settings
    );
    assert!(runner.calls().iter().all(only_reads), "it only looks");
}

#[test]
fn a_survey_of_a_bare_mac_finds_nothing() {
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    absent_by_default(&runner);
    let found = survey(&runner, sandbox.path(), Catalog::shipped());
    assert_eq!(found, soulcaster_engine::Found::default());
}
