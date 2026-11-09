//! Review of Drift, round 5. Everything here is the shipped Catalog and in-memory facts, so nothing
//! reads or changes this Mac.

use std::collections::{BTreeMap, BTreeSet};

use macflayer_core::catalog::Catalog;
use macflayer_core::config::Config;
use macflayer_core::drift::{Choice, Facts, Row, RowKind, Seen, adopt_onto, report};
use macflayer_core::plan::Plan;
use serde_json::{Value, json};

fn config(text: &str) -> Config {
    Config::parse(text.as_bytes()).unwrap()
}

fn row(config: &Config, id: &str, seen: &[(&str, Value)], last: &[(&str, Value)]) -> Row {
    let catalog = Catalog::shipped();
    let plan = Plan::new(config, catalog);
    let seen: BTreeMap<String, Seen> = seen
        .iter()
        .map(|(key, value)| ((*key).to_string(), Seen::Present(value.clone())))
        .collect();
    let last: BTreeMap<String, Value> = last
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect();
    let (unavailable, kept) = (BTreeSet::new(), BTreeSet::new());
    report(&Facts {
        plan: &plan,
        catalog,
        config,
        seen: &seen,
        last: &last,
        unavailable: &unavailable,
        kept: &kept,
    })
    .rows
    .into_iter()
    .find(|row| row.id == id)
    .unwrap_or_else(|| panic!("no row for {id}"))
}

/// The Plan once this row's save has landed.
fn plan_after_saving(latest: &Config, row: &Row) -> Plan {
    let catalog = Catalog::shipped();
    let always: BTreeSet<&str> = catalog.base_items().map(|entry| entry.id()).collect();
    let saved = adopt_onto(latest, &[row.reviewed()], &always).unwrap();
    Plan::new(&saved, catalog)
}

fn applies(plan: &Plan, id: &str) -> bool {
    plan.entries.iter().any(|entry| entry.id == id)
}

#[test]
fn saving_something_on_this_mac_never_stops_the_config_applying_what_it_already_lists() {
    // Homebrew keeps rustup keg-only, so rust and rustup sit side by side on a real Mac. The
    // Catalog says they conflict, and the Plan drops both sides "until you pick one". Offering
    // this row saves a Config that no longer installs rust on any Machine, and both then vanish
    // from Drift because a conflict is set aside.
    let latest = config(r#"{"schemaVersion":1,"name":"x","items":["rust"]}"#);
    assert!(
        applies(&Plan::new(&latest, Catalog::shipped()), "rust"),
        "the Machine Type applies rust before the save"
    );
    let extra = row(&latest, "rustup", &[("rustup", json!(true))], &[]);
    assert_eq!(extra.kind, RowKind::Extra);
    if extra.choices().contains(&Choice::Adopt) {
        assert!(
            applies(&plan_after_saving(&latest, &extra), "rust"),
            "the row offers a save that takes rust out of every Run"
        );
    }
}

#[test]
fn saving_a_package_on_this_mac_never_stops_the_config_applying_what_it_already_lists() {
    // The same through `packages`: `brew leaves` names corepack, and the Catalog's yarn conflicts
    // with that package.
    let latest = config(r#"{"schemaVersion":1,"name":"x","items":["yarn"]}"#);
    assert!(applies(&Plan::new(&latest, Catalog::shipped()), "yarn"));
    let extra = row(
        &latest,
        "formula:corepack",
        &[("formula:corepack", json!(true))],
        &[],
    );
    assert_eq!(extra.kind, RowKind::Extra);
    if extra.choices().contains(&Choice::Adopt) {
        assert!(
            applies(&plan_after_saving(&latest, &extra), "yarn"),
            "the row offers a save that takes yarn out of every Run"
        );
    }
}

#[test]
fn putting_an_entry_back_never_stops_the_config_applying_what_it_already_lists() {
    // Another Machine moved the Config from redis to valkey, and this Mac still has redis.
    // "Put it back in your Config" leaves the file listing both, so neither is applied anywhere.
    let latest = config(r#"{"schemaVersion":1,"name":"x","items":["valkey"]}"#);
    assert!(applies(&Plan::new(&latest, Catalog::shipped()), "valkey"));
    let dropped = row(
        &latest,
        "redis",
        &[("redis", json!(true)), ("valkey", json!(true))],
        &[("redis", json!(true)), ("valkey", json!(true))],
    );
    assert_eq!(dropped.kind, RowKind::Dropped);
    if dropped.choices().contains(&Choice::AddBack) {
        assert!(
            applies(&plan_after_saving(&latest, &dropped), "valkey"),
            "the row offers a save that takes valkey out of every Run"
        );
    }
}
