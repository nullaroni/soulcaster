//! Adopt, with GitHub played by a fake: only what was reviewed changes, and nothing is replaced.

use serde_json::json;
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::Reply;
use soulcaster_core::config::Config;
use soulcaster_core::drift::{Reviewed, value_in};
use soulcaster_repo::adopt::{AdoptOutcome, AdoptRequest, adopt};
use soulcaster_repo::fake::{FakeGitHub, RepoShape};
use soulcaster_repo::save::{CONFIG_REPO_NAME, SaveError, WaitReason};
use soulcaster_repo::tools::Tools;

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

    fn adopt(&self, held: &Config, reviewed: &[Reviewed]) -> Result<AdoptOutcome, SaveError> {
        self.adopt_with(held, reviewed, &|_| true, true)
    }

    fn adopt_with(
        &self,
        held: &Config,
        reviewed: &[Reviewed],
        still_true: &dyn Fn(&Reviewed) -> bool,
        ever_saved: bool,
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
                ever_saved,
            },
        )
    }
}

const BEFORE: &str = r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":2}}"#;

fn six() -> Reviewed {
    Reviewed {
        id: "keyboard.key-repeat".into(),
        key: "keyboard.key-repeat".into(),
        seen: Some(json!(2)),
        write: Some(json!(6)),
    }
}

#[test]
fn a_reviewed_entry_goes_on_top_of_what_else_landed() {
    let world = world();
    let landed = canonical(
        r#"{"schemaVersion":1,"name":"My Mac","items":["ghostty"],"settings":{"keyboard.key-repeat":2}}"#,
    );
    world.on_github(&landed);
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert_eq!(outcome, AdoptOutcome::Saved { public: false });
    let saved = config(&world.gh.file_at_git("add", PATH).unwrap());
    assert_eq!(saved.items, ["ghostty"], "what the other Mac saved stayed");
    assert_eq!(value_in(&saved, "keyboard.key-repeat"), Some(json!(6)));
    assert_eq!(world.pushes(), 1);
}

#[test]
fn the_same_entry_changed_elsewhere_comes_back_and_nothing_is_sent() {
    let world = world();
    world.on_github(&canonical(
        r#"{"schemaVersion":1,"name":"My Mac","settings":{"keyboard.key-repeat":9}}"#,
    ));
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert_eq!(
        outcome,
        AdoptOutcome::ChangedElsewhere {
            ids: vec!["keyboard.key-repeat".into()]
        }
    );
    assert_eq!(world.pushes(), 0);
}

#[test]
fn this_mac_changing_again_stops_the_save() {
    let world = world();
    world.on_github(&canonical(BEFORE));
    let outcome = world
        .adopt_with(&config(BEFORE), &[six()], &|_| false, true)
        .unwrap();
    assert_eq!(
        outcome,
        AdoptOutcome::ChangedAgain {
            ids: vec!["keyboard.key-repeat".into()]
        }
    );
    assert_eq!(world.pushes(), 0);
}

#[test]
fn with_no_network_the_change_waits_on_this_mac() {
    let world = world();
    world.gh.go_offline();
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    let AdoptOutcome::Waiting { why, local } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(why, WaitReason::Offline);
    let kept = config(&std::fs::read_to_string(local).unwrap());
    assert_eq!(value_in(&kept, "keyboard.key-repeat"), Some(json!(6)));
}

#[test]
fn signed_out_the_change_waits_and_nothing_is_lost() {
    let world = world();
    world.gh.sign_out();
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert!(matches!(
        outcome,
        AdoptOutcome::Waiting {
            why: WaitReason::SignIn,
            ..
        }
    ));
}

#[test]
fn a_hand_edited_file_is_tidied_in_its_own_save_first() {
    let world = world();
    world.on_github(BEFORE);
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert_eq!(outcome, AdoptOutcome::Saved { public: false });
    assert_eq!(world.pushes(), 2);
}

#[test]
fn a_file_that_cannot_be_read_stops_everything() {
    for text in [
        r#"{"schemaVersion":1,"name":"My Mac",}"#,
        r#"{"schemaVersion":99,"name":"My Mac"}"#,
        r#"{"schemaVersion":1,"name":"Somebody Else"}"#,
    ] {
        let world = world();
        world.on_github(text);
        let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
        assert_eq!(outcome, AdoptOutcome::Unreadable, "{text}");
        assert_eq!(world.pushes(), 0, "{text}");
    }
}

#[test]
fn a_file_gone_from_github_stops_writes() {
    let world = world();
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert_eq!(
        outcome,
        AdoptOutcome::Missing {
            path: PATH.to_string()
        }
    );
    assert_eq!(world.pushes(), 0);
}

#[test]
fn a_config_never_sent_is_created_from_what_this_mac_holds() {
    let world = world();
    let outcome = world
        .adopt_with(&config(BEFORE), &[six()], &|_| true, false)
        .unwrap();
    assert_eq!(outcome, AdoptOutcome::Saved { public: false });
    let saved = config(&world.gh.file_at_git("add", PATH).unwrap());
    assert_eq!(value_in(&saved, "keyboard.key-repeat"), Some(json!(6)));
}

#[test]
fn github_moving_every_time_stops_after_three_tries_and_keeps_the_change() {
    let world = world();
    let text = canonical(BEFORE);
    world
        .gh
        .add_file("example-you", CONFIG_REPO_NAME, SHA, PATH, text.as_bytes());
    world.gh.set_git_files(
        "checkout",
        vec![(
            PATH,
            r#"{"schemaVersion":1,"name":"My Mac","items":["jq"]}"#,
        )],
    );
    let outcome = world.adopt(&config(BEFORE), &[six()]).unwrap();
    assert!(
        matches!(
            outcome,
            AdoptOutcome::Waiting {
                why: WaitReason::Busy,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(world.pushes(), 0);
}

#[test]
fn a_value_that_must_stay_on_this_mac_is_refused_before_any_send() {
    let world = world();
    world.on_github(&canonical(
        r#"{"schemaVersion":1,"name":"My Mac","settings":{"git.default-branch":"main"}}"#,
    ));
    let reviewed = Reviewed {
        id: "git.default-branch".into(),
        key: "git.default-branch".into(),
        seen: Some(json!("main")),
        write: Some(json!("/Users/example-you/secret")),
    };
    let outcome = world.adopt(&config(BEFORE), &[reviewed]);
    assert!(matches!(outcome, Err(SaveError::Blocked(_))), "{outcome:?}");
    assert_eq!(world.pushes(), 0);
}
