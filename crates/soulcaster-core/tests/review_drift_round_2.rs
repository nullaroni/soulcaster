//! Review of Drift's words, round 2.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Facts, RowKind, Seen, report};
use soulcaster_core::plan::Plan;

#[test]
fn something_only_pulled_in_is_not_called_new_in_your_config() {
    // Round 1's fix gave RemovedHere its own sentence for this. The Config has no line for
    // android-sdk here either, and the row's sentence says it does.
    let catalog = Catalog::shipped();
    let config =
        Config::parse(br#"{"schemaVersion":1,"name":"x","items":["android-emulator"]}"#).unwrap();
    let plan = Plan::new(&config, catalog);
    let seen = BTreeMap::from([
        ("android-emulator".to_string(), Seen::Present(json!(true))),
        ("android-sdk".to_string(), Seen::Absent),
    ]);
    let last = BTreeMap::from([("android-emulator".to_string(), json!(true))]);
    let (unavailable, kept) = (BTreeSet::new(), BTreeSet::new());
    let found = report(&Facts {
        plan: &plan,
        catalog,
        config: &config,
        seen: &seen,
        last: &last,
        unavailable: &unavailable,
        kept: &kept,
    });
    let row = found
        .rows
        .iter()
        .find(|row| row.id == "android-sdk")
        .expect("a row for what's needed and missing");
    assert_eq!(row.kind, RowKind::NewInConfig);
    assert!(row.only_needed);
    assert!(
        !row.sentence().contains("is new in your Machine Type"),
        "{}",
        row.sentence()
    );
}
