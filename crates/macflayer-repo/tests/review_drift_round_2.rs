//! Review of Adopt, round 2, with GitHub played by a fake and a temp home.

use macflayer_core::catalog::Catalog;
use macflayer_core::command::Reply;
use macflayer_core::config::Config;
use macflayer_core::drift::Reviewed;
use macflayer_core::plan::Plan;
use macflayer_repo::adopt::{AdoptOutcome, AdoptRequest, adopt};
use macflayer_repo::fake::{FakeGitHub, RepoShape};
use macflayer_repo::save::{CONFIG_REPO_NAME, SaveError};
use macflayer_repo::tools::Tools;
use serde_json::json;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";
const PATH: &str = "configs/my-mac.json";

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
        let text = config(text).to_canonical_json();
        self.gh
            .add_file("example-you", CONFIG_REPO_NAME, SHA, PATH, text.as_bytes());
        self.gh.set_git_files("checkout", vec![(PATH, &text)]);
    }

    fn adopt_with(
        &self,
        catalog: &Catalog,
        held: &Config,
        reviewed: &[Reviewed],
        still_true: &dyn Fn(&Reviewed) -> bool,
    ) -> Result<AdoptOutcome, SaveError> {
        adopt(
            &self.gh,
            &self.tools,
            catalog,
            &AdoptRequest {
                repo_name: CONFIG_REPO_NAME,
                config_name: "My Mac",
                held,
                reviewed,
                still_true,
                ever_saved: true,
            },
        )
    }

    fn final_text(&self) -> Option<String> {
        self.gh.file_at_git("add", PATH)
    }
}

/// Whether the Config on GitHub at the end still wants this entry.
fn still_wanted(world: &World, start: &str, catalog: &Catalog, id: &str) -> bool {
    let text = world
        .final_text()
        .unwrap_or_else(|| config(start).to_canonical_json());
    let plan = Plan::new(&config(&text), catalog);
    plan.entries.iter().any(|entry| entry.id == id)
}

#[test]
fn a_removal_is_not_saved_while_the_file_still_lists_the_entry_under_its_new_name() {
    // The row was made while the file said `formula:jq`, which the Item `jq` covers. Then an
    // explicit Tidy moved the line. The old spelling is gone, which reads as "already saved".
    let world = world();
    let catalog = Catalog::shipped();
    let held = r#"{"schemaVersion":1,"name":"My Mac","packages":{"formulae":["jq"]}}"#;
    let now = r#"{"schemaVersion":1,"name":"My Mac","items":["jq"]}"#;
    world.on_github(now);
    let removal = Reviewed {
        id: "jq".into(),
        key: "formula:jq".into(),
        seen: Some(json!(true)),
        write: None,
    };
    let outcome = world
        .adopt_with(catalog, &config(held), &[removal], &|_| true)
        .unwrap();
    assert!(
        !(outcome == (AdoptOutcome::Saved { public: false })
            && still_wanted(&world, now, catalog, "jq")),
        "Saved, so last-applied is forgotten, and GitHub still lists it"
    );
}

#[test]
fn a_removal_takes_out_every_spelling_of_the_entry() {
    // One entry listed as the Item and as the package it covers. A row is made per entry, and its
    // write names only the first spelling found.
    let world = world();
    let catalog = Catalog::shipped();
    let now =
        r#"{"schemaVersion":1,"name":"My Mac","items":["jq"],"packages":{"formulae":["jq"]}}"#;
    world.on_github(now);
    let removal = Reviewed {
        id: "jq".into(),
        key: "jq".into(),
        seen: Some(json!(true)),
        write: None,
    };
    let outcome = world
        .adopt_with(catalog, &config(now), &[removal], &|_| true)
        .unwrap();
    assert!(
        !(outcome == (AdoptOutcome::Saved { public: false })
            && still_wanted(&world, now, catalog, "jq")),
        "Saved, and the file still lists it: {:?}",
        world.final_text()
    );
}

#[test]
fn an_entry_that_moved_both_ways_comes_back_once() {
    // The model checks "changed again" first and returns the entry for that one reason.
    let world = world();
    let held = r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":2}}"#;
    let now = r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":9}}"#;
    world.on_github(now);
    let reviewed = Reviewed {
        id: "keyboard.key-repeat".into(),
        key: "keyboard.key-repeat".into(),
        seen: Some(json!(2)),
        write: Some(json!(6)),
    };
    let outcome = world
        .adopt_with(Catalog::shipped(), &config(held), &[reviewed], &|_| false)
        .unwrap();
    assert_eq!(
        outcome,
        AdoptOutcome::ChangedAgain {
            ids: vec!["keyboard.key-repeat".into()]
        }
    );
}
