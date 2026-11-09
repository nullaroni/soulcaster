//! Review of Drift, round 6. `saving_keeps_applying` looks at one row against the Config this Mac
//! holds, but `adopt` saves any number of rows onto the latest file. GitHub is played by a fake,
//! and everything else is the shipped Catalog and in-memory facts, so nothing touches this Mac.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::Reply;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Choice, Facts, Reviewed, Row, Seen, report};
use soulcaster_core::plan::Plan;
use soulcaster_repo::adopt::{AdoptRequest, adopt};
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::save::CONFIG_REPO_NAME;
use soulcaster_repo::tools::Tools;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";
const PATH: &str = "configs/my-mac.json";

const HELD: &str = r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":2}}"#;

struct World {
    _dir: tempfile::TempDir,
    tools: Tools,
    gh: FakeGitHub,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let tools = Tools::fixed(dir.path().to_path_buf());
    let gh = FakeGitHub::new();
    gh.sign_in("example-you", 42);
    gh.on_git("rev-parse", Reply::ok(format!("{PUSHED}\n")));
    gh.add_repo(
        "example-you",
        CONFIG_REPO_NAME,
        RepoShape::private("main", SHA),
    );
    World {
        _dir: dir,
        tools,
        gh,
    }
}

fn config(text: &str) -> Config {
    Config::parse(text.as_bytes()).unwrap()
}

impl World {
    fn on_github(&self, text: &str) {
        self.gh
            .add_file("example-you", CONFIG_REPO_NAME, SHA, PATH, text.as_bytes());
        self.gh.set_git_files("checkout", vec![(PATH, text)]);
    }

    /// Saves these rows, and returns the Config that went to GitHub, if one did.
    fn save(&self, held: &Config, reviewed: &[Reviewed]) -> Option<Config> {
        adopt(
            &self.gh,
            &self.tools,
            Catalog::shipped(),
            &AdoptRequest {
                repo_name: CONFIG_REPO_NAME,
                config_name: "My Mac",
                held,
                reviewed,
                still_true: &|_| true,
                ever_saved: true,
            },
        )
        .unwrap();
        self.gh.file_at_git("add", PATH).map(|text| config(&text))
    }
}

/// The rows Drift shows for what's on this Mac, against the Config this Mac holds.
fn rows(held: &Config, on_mac: &[&str]) -> Vec<Row> {
    let catalog = Catalog::shipped();
    let plan = Plan::new(held, catalog);
    let seen: BTreeMap<String, Seen> = on_mac
        .iter()
        .map(|id| ((*id).to_string(), Seen::Present(json!(true))))
        .collect();
    let (last, unavailable, kept) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new());
    report(&Facts {
        plan: &plan,
        catalog,
        config: held,
        seen: &seen,
        last: &last,
        unavailable: &unavailable,
        kept: &kept,
    })
    .rows
}

fn applies(saved: &Config, id: &str) -> bool {
    Plan::new(saved, Catalog::shipped())
        .entries
        .iter()
        .any(|entry| entry.id == id)
}

#[test]
fn a_save_never_stops_an_entry_another_mac_saved_since_this_mac_last_read() {
    // Another Machine saved rust. This Mac still holds the Config from before that, has rustup
    // installed, and offers to save it. `adopt` puts the line on the latest file, so the Config
    // ends up listing both, and rust stops being applied on every Machine.
    let world = world();
    let latest = config(
        r#"{"schemaVersion":1,"name":"My Mac","items":["rust"],"settings":{"keyboard.key-repeat":2}}"#,
    );
    assert!(
        applies(&latest, "rust"),
        "the Machine Type applies rust before"
    );
    world.on_github(&latest.to_canonical_json());
    let held = config(HELD);
    let found = rows(&held, &["rustup"]);
    let extra = found.iter().find(|row| row.id == "rustup").unwrap();
    assert!(extra.choices().contains(&Choice::Adopt));

    if let Some(saved) = world.save(&held, &[extra.reviewed()]) {
        assert!(
            applies(&saved, "rust"),
            "the save took rust out of every Run: {saved:?}"
        );
    }
}

#[test]
fn saving_two_rows_together_never_saves_both_sides_of_a_conflict() {
    // Homebrew keeps rustup keg-only, so rust and rustup sit side by side on a real Mac, and the
    // Config lists neither. Each row passes on its own, so both offer Save, and `adopt` takes the
    // two together. The Config then lists both and applies neither, on every Machine, and Drift
    // stops showing them because a conflict is set aside.
    let world = world();
    world.on_github(&config(HELD).to_canonical_json());
    let held = config(HELD);
    let found = rows(&held, &["rust", "rustup"]);
    let offered: Vec<Reviewed> = found
        .iter()
        .filter(|row| row.choices().contains(&Choice::Adopt))
        .map(Row::reviewed)
        .collect();
    if offered.len() < 2 {
        return;
    }

    if let Some(saved) = world.save(&held, &offered) {
        let conflicts = Plan::new(&saved, Catalog::shipped()).conflicts;
        assert!(
            conflicts.is_empty(),
            "the save wrote a Machine Type that applies neither side: {conflicts:?}"
        );
    }
}
