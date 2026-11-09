//! Review of Adopt, round 3, with GitHub played by a fake and a temp home.

use macflayer_core::catalog::Catalog;
use macflayer_core::command::Reply;
use macflayer_core::config::Config;
use macflayer_core::drift::Reviewed;
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

#[test]
fn removing_a_line_that_must_not_be_saved_is_not_refused_for_that_same_line() {
    // A hand edit put a home-folder path in the file, and this Mac no longer has that setting.
    // The merged Config is clean, but the reformat of the old file is checked first and refused.
    let world = world();
    let catalog = Catalog::shipped();
    let hand_edited = r#"{"schemaVersion": 1, "name": "My Mac", "settings": {"git.default-branch": "/Users/example-you/secret"}}"#;
    world.gh.add_file(
        "example-you",
        CONFIG_REPO_NAME,
        SHA,
        PATH,
        hand_edited.as_bytes(),
    );
    world
        .gh
        .set_git_files("checkout", vec![(PATH, hand_edited)]);
    let removal = Reviewed {
        id: "git.default-branch".into(),
        key: "git.default-branch".into(),
        seen: Some(json!("/Users/example-you/secret")),
        write: None,
    };
    let outcome = world.adopt_with(catalog, &config(hand_edited), &[removal], &|_| true);
    assert!(
        matches!(outcome, Ok(AdoptOutcome::Saved { .. })),
        "{outcome:?}"
    );
    assert!(!world.final_text().unwrap().contains("secret"));
}
