//! Review of Drift's look at the Machine, round 3. The runner is a fake and `HOME` is a temp
//! directory, so nothing here reads or changes this Mac.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Choice, Report, RowKind};
use soulcaster_core::lifecycle::Assurance;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{LastApplied, MachineRecord, check_drift};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review-drift-3-{}-{}",
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

/// Answers commands whose line ends with `needle`: one key's name can be the start of another's.
fn ends(runner: &FakeRunner, needle: &'static str, reply: Reply) {
    runner.on(move |command| line(command).ends_with(needle), [reply]);
}

fn one_setting(id: &str, value: Value) -> Config {
    config(&json!({"schemaVersion": 1, "name": "x", "settings": {id: value}}).to_string())
}

fn state(report: &Report) -> String {
    format!(
        "rows {:?}, could not check {:?}",
        kinds(report),
        report.could_not_check
    )
}

const NOTHING: &str = "rows [], could not check []";

#[test]
fn a_finder_window_setting_chosen_in_finder_is_the_same_as_the_config() {
    // The Catalog stores this as `file://<home>/Documents/`, and nothing fills in `<home>`, so the
    // real value Finder writes matches neither the comparison nor the decode.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(
        &runner,
        "com.apple.finder NewWindowTarget",
        Reply::ok("PfDo\n"),
    );
    let real = format!("file://{}/Documents/\n", sandbox.path().display());
    ends(
        &runner,
        "com.apple.finder NewWindowTargetPath",
        Reply::ok(real),
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
    assert_eq!(state(&report), NOTHING);
}

#[test]
fn weather_shown_at_a_position_macos_chose_is_the_same_as_the_config() {
    // The position key is 100 for both values, so it can't say anything about shown or hidden.
    // macOS writes its own number there as soon as the item is placed or dragged.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(&runner, "Visible com.apple.WeatherWidget", Reply::ok("1\n"));
    ends(
        &runner,
        "Position com.apple.WeatherWidget",
        Reply::ok("215.5\n"),
    );
    absent_by_default(&runner);
    remember(sandbox.path(), &[("menu-bar.show-weather", json!(true))]);
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("menu-bar.show-weather", json!(true)),
    );
    assert_eq!(state(&report), NOTHING);
}

#[test]
fn a_hot_corner_with_no_modifier_key_stored_is_still_read() {
    // `defaults write com.apple.dock wvous-tl-corner -int 5` alone. The corner says what it does.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(&runner, "wvous-tl-corner", Reply::ok("5\n"));
    ends(
        &runner,
        "wvous-tl-modifier",
        Reply::fails(1, "does not exist"),
    );
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("hot-corners.top-left", json!("mission-control"))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("hot-corners.top-left", json!("mission-control")),
    );
    assert_eq!(
        state(&report),
        r#"rows [("hot-corners.top-left", ChangedHere)], could not check []"#
    );
    assert_eq!(report.rows[0].on_mac, Some(json!("start-screensaver")));
}

#[test]
fn get_info_panes_changed_on_this_mac_are_a_row() {
    // The Run reads this dict and compares it, but the look calls every dict Silent, so any
    // change at all is "could not check", for good.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(
        &runner,
        "FXInfoPanesExpanded",
        Reply::ok("{\n    General = 1;\n    OpenWith = 1;\n    Privileges = 0;\n}\n"),
    );
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("finder.info-panes-expanded", json!(["general"]))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("finder.info-panes-expanded", json!(["general"])),
    );
    assert_eq!(
        state(&report),
        r#"rows [("finder.info-panes-expanded", ChangedHere)], could not check []"#
    );
}

#[test]
fn git_spellings_of_true_are_true() {
    // `git config --get` prints the file's own word, and git treats these all as true.
    let mut wrong = Vec::new();
    for word in ["1\n", "yes\n", "on\n", "TRUE\n"] {
        let sandbox = Sandbox::new();
        fs::write(
            sandbox.path().join(".gitconfig"),
            "[rerere]\n\tenabled = x\n",
        )
        .unwrap();
        let runner = FakeRunner::new();
        answer(&runner, "CommandLineTools", Reply::ok(""));
        answer(&runner, "rerere.enabled", Reply::ok(word));
        absent_by_default(&runner);
        remember(sandbox.path(), &[("git.rerere", json!(true))]);
        let report = check(
            &runner,
            sandbox.path(),
            &one_setting("git.rerere", json!(true)),
        );
        if state(&report) != NOTHING {
            wrong.push(format!("{word:?}: {}", state(&report)));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn a_git_key_set_with_no_value_is_not_removed_from_this_mac() {
    // `[rerere]` then `enabled` alone. Exit 0 with an empty line, and git reads it as true.
    let sandbox = Sandbox::new();
    fs::write(sandbox.path().join(".gitconfig"), "[rerere]\n\tenabled\n").unwrap();
    let runner = FakeRunner::new();
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "rerere.enabled", Reply::ok("\n"));
    absent_by_default(&runner);
    remember(sandbox.path(), &[("git.rerere", json!(true))]);
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("git.rerere", json!(true)),
    );
    assert_eq!(
        report
            .rows
            .iter()
            .filter(|row| row.kind == RowKind::RemovedHere)
            .count(),
        0,
        "{}",
        state(&report)
    );
}

#[test]
fn spotlight_shown_is_not_read_as_spotlight_hidden() {
    // MenuItemHidden is the opposite of "show", and the Catalog's encode says so. The look's bool
    // arm never reads it: MenuItemHidden 0, Spotlight on screen, reads as `show-spotlight: false`.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(&runner, "MenuItemHidden", Reply::ok("0\n"));
    absent_by_default(&runner);
    remember(sandbox.path(), &[("menu-bar.show-spotlight", json!(false))]);
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("menu-bar.show-spotlight", json!(false)),
    );
    assert_eq!(
        state(&report),
        r#"rows [("menu-bar.show-spotlight", ChangedHere)], could not check []"#
    );
    assert_eq!(report.rows[0].on_mac, Some(json!(true)));
}

#[test]
fn a_home_folder_whose_name_has_hidden_in_it_does_not_hide_the_library() {
    // `ls -ldO` prints the owner and the path. Only the flags column says hidden.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    answer(
        &runner,
        "/bin/ls -ldO",
        Reply::ok(
            "drwx------@ 91 hiddenfox  staff  - 2912 Nov  8 08:00 /Users/hiddenfox/Library\n",
        ),
    );
    absent_by_default(&runner);
    remember(
        sandbox.path(),
        &[("finder.show-library-folder", json!(true))],
    );
    let report = check(
        &runner,
        sandbox.path(),
        &one_setting("finder.show-library-folder", json!(true)),
    );
    assert_eq!(state(&report), NOTHING);
}

#[test]
fn a_dock_app_in_the_home_applications_folder_is_the_same_entry() {
    // The Catalog's pattern allows `~/Applications`, and dockutil prints the full path.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    let listing = format!(
        "Notes\tfile://{}/Applications/Notes.app/\tpersistentApps\n",
        sandbox.path().display()
    );
    answer(&runner, "dockutil --list", Reply::ok(listing));
    answer(&runner, "CommandLineTools", Reply::ok(""));
    answer(&runner, "Cellar/dockutil", Reply::ok(""));
    absent_by_default(&runner);
    let wanted = json!(["~/Applications/Notes.app"]);
    remember(sandbox.path(), &[("dock.apps", wanted.clone())]);
    let report = check(&runner, sandbox.path(), &one_setting("dock.apps", wanted));
    assert_eq!(state(&report), NOTHING);
}

#[test]
fn a_dropped_setting_waits_for_an_update_while_the_file_holds_an_unknown_id() {
    // The model holds every dropped row. That unreadable ID may be this setting's new name, and
    // "Put it back" would write the old spelling alongside it.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    ends(&runner, "KeyRepeat", Reply::ok("6\n"));
    absent_by_default(&runner);
    remember(sandbox.path(), &[("keyboard.key-repeat", json!(6))]);
    let report = check(
        &runner,
        sandbox.path(),
        &config(r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat-v2":6}}"#),
    );
    assert_eq!(report.held, ["keyboard.key-repeat-v2"]);
    let row = report
        .rows
        .iter()
        .find(|row| row.id == "keyboard.key-repeat")
        .expect("a row");
    assert_eq!(row.kind, RowKind::Dropped);
    assert_eq!(row.choices(), Vec::<Choice>::new());
    assert!(
        row.sentence().contains("update Soulcaster"),
        "{}",
        row.sentence()
    );
}
