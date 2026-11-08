//! Review of Drift's decisions, round 1. Pure logic over a Config and made-up Machine facts.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{
    Choice, Facts, Report, Row, RowKind, Seen, adopt_onto, held_sentence, report,
};
use soulcaster_core::plan::Plan;

fn config(text: &str) -> Config {
    Config::parse(text.as_bytes()).unwrap()
}

fn look(text: &str, seen: &[(&str, Seen)], last: &[(&str, Value)]) -> Report {
    let catalog = Catalog::shipped();
    let config = config(text);
    let plan = Plan::new(&config, catalog);
    let seen: BTreeMap<String, Seen> = seen
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    let last: BTreeMap<String, Value> = last
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    report(&Facts {
        plan: &plan,
        catalog,
        config: &config,
        seen: &seen,
        last: &last,
        unavailable: &BTreeSet::new(),
        kept: &BTreeSet::new(),
    })
}

fn row(report: &Report, id: &str) -> Row {
    report
        .rows
        .iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("no row for {id} in {:?}", report.rows))
        .clone()
}

fn here(value: Value) -> Seen {
    Seen::Present(value)
}

#[test]
fn a_change_only_the_config_made_offers_no_save_of_this_macs_older_value() {
    // The design's table gives this row Apply or leave it. Saving would put this Mac's old value
    // back over another Machine's change, with nothing else having moved here.
    let report = look(
        r#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":36}}"#,
        &[("keyboard.key-repeat", here(json!(48)))],
        &[("keyboard.key-repeat", json!(48))],
    );
    let row = row(&report, "keyboard.key-repeat");
    assert_eq!(row.kind, RowKind::ChangedInConfig);
    assert_eq!(row.choices(), [Choice::Apply, Choice::NotHere]);
}

#[test]
fn while_the_file_holds_an_unknown_id_a_package_row_says_to_update_and_offers_nothing() {
    let text = r#"{"schemaVersion":1,"name":"x","items":["no-such-thing"]}"#;
    let dropped = look(
        text,
        &[("formula:nobody-covers-this", here(json!(true)))],
        &[("formula:nobody-covers-this", json!(true))],
    );
    let dropped = row(&dropped, "formula:nobody-covers-this");
    assert_eq!(dropped.sentence(), held_sentence("nobody-covers-this"));
    assert_eq!(
        dropped.choices(),
        [],
        "the file might name this through the ID it can't read"
    );

    let extra = look(text, &[("cask:nobody-covers-this", here(json!(true)))], &[]);
    let extra = row(&extra, "cask:nobody-covers-this");
    assert_eq!(extra.sentence(), held_sentence("nobody-covers-this"));
    assert_eq!(extra.choices(), []);
}

#[test]
fn a_reviewed_write_drops_a_package_line_naming_something_always_installed() {
    let catalog = Catalog::shipped();
    let always: BTreeSet<&str> = catalog.base_items().map(|entry| entry.id()).collect();
    let latest = config(r#"{"schemaVersion":1,"name":"x","packages":{"formulae":["gh","jq"]}}"#);
    let out = adopt_onto(&latest, &[], &always).unwrap();
    assert_eq!(out.packages.formulae, ["jq"]);
}

#[test]
fn a_home_folder_value_is_not_offered_to_save() {
    let report = look(
        r#"{"schemaVersion":1,"name":"x","settings":{"git.default-branch":"main"}}"#,
        &[(
            "git.default-branch",
            here(json!("/Users/example-you/secret")),
        )],
        &[("git.default-branch", json!("main"))],
    );
    let row = row(&report, "git.default-branch");
    assert_eq!(row.kind, RowKind::ChangedHere);
    assert!(
        !row.choices().contains(&Choice::Adopt),
        "{:?}",
        row.choices()
    );
}

#[test]
fn something_only_pulled_in_is_not_offered_a_save_the_config_has_no_line_for() {
    // Nothing in the file lists android-sdk, so saving "removed here" can't take out a line.
    let report = look(
        r#"{"schemaVersion":1,"name":"x","items":["android-emulator"]}"#,
        &[
            ("android-emulator", here(json!(true))),
            ("android-sdk", Seen::Absent),
        ],
        &[
            ("android-emulator", json!(true)),
            ("android-sdk", json!(true)),
        ],
    );
    let row = row(&report, "android-sdk");
    assert!(
        !row.choices().contains(&Choice::Adopt),
        "{}: {:?}",
        row.sentence(),
        row.choices()
    );
}
