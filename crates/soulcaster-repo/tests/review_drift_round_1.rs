//! Review of Adopt, round 1, with GitHub played by a fake and a temp home.

use serde_json::json;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::Reply;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Reviewed, value_in};
use soulcaster_repo::adopt::{AdoptOutcome, AdoptRequest, adopt};
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveError};
use soulcaster_repo::tools::Tools;

const SHA: &str = "3f9a1c2000000000000000000000000000000000";
const PUSHED: &str = "bbbbbbb000000000000000000000000000000000";
const PATH: &str = "configs/my-mac.json";
const BEFORE: &str = r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":2}}"#;

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

fn canonical(text: &str) -> String {
    config(text).to_canonical_json()
}

impl World {
    fn on_github(&self, text: &str) {
        self.gh
            .add_file("example-you", CONFIG_REPO_NAME, SHA, PATH, text.as_bytes());
        self.gh.set_git_files("checkout", vec![(PATH, text)]);
    }

    fn pushes(&self) -> usize {
        self.gh
            .calls()
            .iter()
            .filter(|c| c.program.ends_with("git") && c.args.iter().any(|a| a == "push"))
            .count()
    }

    fn adopt_with(
        &self,
        held: &Config,
        reviewed: &[Reviewed],
        still_true: &dyn Fn(&Reviewed) -> bool,
    ) -> Result<AdoptOutcome, SaveError> {
        adopt(
            &self.gh,
            &self.tools,
            Catalog::shipped(),
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

    fn adopt(&self, held: &Config, reviewed: &[Reviewed]) -> Result<AdoptOutcome, SaveError> {
        self.adopt_with(held, reviewed, &|_| true)
    }
}

fn six() -> Reviewed {
    Reviewed {
        id: "keyboard.key-repeat".into(),
        key: "keyboard.key-repeat".into(),
        seen: Some(json!(2)),
        write: Some(json!(6)),
    }
}

fn ghostty() -> Reviewed {
    Reviewed {
        id: "ghostty".into(),
        key: "ghostty".into(),
        seen: None,
        write: Some(json!(true)),
    }
}

#[test]
fn a_reformat_that_cannot_be_sent_still_keeps_the_reviewed_change_on_this_mac() {
    let world = world();
    world.on_github(BEFORE);
    world.gh.on_git(
        "push",
        Reply::fails(
            128,
            "fatal: unable to access: Could not resolve host: github.com",
        ),
    );
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    let AdoptOutcome::Waiting { local, .. } = outcome else {
        panic!("{outcome:?}");
    };
    let kept = config(&std::fs::read_to_string(local).unwrap());
    assert_eq!(
        value_in(&kept, "keyboard.key-repeat"),
        Some(json!(6)),
        "the change the person chose is what waits on this Mac"
    );
}

#[test]
fn a_value_that_must_stay_on_this_mac_sends_no_reformat_either() {
    let world = world();
    let hand_edited =
        r#"{"schemaVersion":1,"name":"My Mac","settings":{"git.default-branch":"main"}}"#;
    world.on_github(hand_edited);
    let reviewed = Reviewed {
        id: "git.default-branch".into(),
        key: "git.default-branch".into(),
        seen: Some(json!("main")),
        write: Some(json!("/Users/example-you/secret")),
    };
    let outcome = world.adopt(&config(hand_edited), &[reviewed]);
    assert!(matches!(outcome, Err(SaveError::Blocked(_))), "{outcome:?}");
    assert_eq!(world.pushes(), 0, "nothing was going to be saved");
}

#[test]
fn an_entry_already_on_github_writes_nothing_even_in_a_hand_edited_file() {
    let world = world();
    world.on_github(r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":6}}"#);
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert_eq!(outcome, AdoptOutcome::Saved { public: false });
    assert_eq!(world.pushes(), 0, "the next Send writes nothing");
}

#[test]
fn an_entry_changed_elsewhere_does_not_stop_the_entries_nobody_touched() {
    // The design: "Entries the other Machine didn't touch go through; the one both changed is
    // asked again."
    let world = world();
    world.on_github(&canonical(
        r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":9}}"#,
    ));
    let outcome = world.adopt(&config(BEFORE), &[six(), ghostty()]).unwrap();
    assert_eq!(world.pushes(), 1, "{outcome:?}");
    let saved = config(&world.gh.file_at_git("add", PATH).unwrap());
    assert_eq!(saved.items, ["ghostty"]);
    assert_eq!(
        value_in(&saved, "keyboard.key-repeat"),
        Some(json!(9)),
        "the other Machine's value stayed"
    );
}

#[test]
fn an_entry_that_changed_again_here_does_not_stop_the_entries_still_true() {
    let world = world();
    world.on_github(&canonical(BEFORE));
    let outcome = world
        .adopt_with(&config(BEFORE), &[six(), ghostty()], &|entry| {
            entry.id != "keyboard.key-repeat"
        })
        .unwrap();
    assert_eq!(world.pushes(), 1, "{outcome:?}");
    let saved = config(&world.gh.file_at_git("add", PATH).unwrap());
    assert_eq!(saved.items, ["ghostty"]);
    assert_eq!(value_in(&saved, "keyboard.key-repeat"), Some(json!(2)));
}
