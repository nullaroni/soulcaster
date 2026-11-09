//! Review of the Drift screen (PR #43), round 1. The runner is a fake, the Dotfiles hook only
//! counts, and `HOME` is a temp directory, so nothing here changes anything on this Mac.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use macflayer_core::approval::ApprovalStore;
use macflayer_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use macflayer_core::catalog::Catalog;
use macflayer_core::command::{Command, CommandRunner, FakeRunner};
use macflayer_core::config::Config;
use macflayer_core::event::{ConfigRef, Event, RunScope};
use macflayer_core::plan::{DotfilesPlan, Plan};
use macflayer_engine::{
    AdminGate, AdminWindow, Budget, Clock, DotfilesHook, DotfilesResult, RunInput, Validation,
    perform,
};

struct Fixed;

impl Clock for Fixed {
    fn now_ms(&self) -> u64 {
        1_000
    }

    fn wait(&self, _: Duration) {}
}

struct Quiet;

impl Asks for Quiet {
    fn password(&self, _: &PasswordRequest) -> Option<Password> {
        None
    }

    fn show_github_code(&self, _: &str, _: &str) {}

    fn github_status(&self, _: SignInStatus) {}
}

struct OpenWindow;

impl AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner
    }
}

struct OpenGate;

impl AdminGate for OpenGate {
    fn validate(&self, _: &dyn CommandRunner, _: &Path, _: &Password) -> Validation {
        Validation::Accepted
    }

    fn open(&self, _: &Password, _: Budget) -> Result<Box<dyn AdminWindow>, String> {
        Ok(Box::new(OpenWindow))
    }

    fn forget(&self, _: &dyn CommandRunner, _: &Path) -> bool {
        true
    }
}

#[derive(Default)]
struct CountingDotfiles(AtomicUsize);

impl DotfilesHook for CountingDotfiles {
    fn apply(&self, _: &DotfilesPlan, _: &dyn ApprovalStore) -> DotfilesResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        DotfilesResult {
            warnings: Vec::new(),
        }
    }
}

#[test]
fn an_apply_of_one_setting_does_not_apply_the_dotfiles_repo() {
    let home = std::env::temp_dir().join(format!(
        "macflayer-review-drift-screen-1-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&home).unwrap();
    let config =
        Config::parse(br#"{"schemaVersion":1,"name":"x","settings":{"keyboard.key-repeat":2}}"#)
            .unwrap();
    let mut plan = Plan::new(&config, Catalog::shipped());
    // As `real.rs` hands over for a followed Config that names a Dotfiles Repo.
    plan.dotfiles = Some(DotfilesPlan {
        repo: "example-you/dotfiles".to_string(),
        links: BTreeMap::from([("~/.zshrc".to_string(), "zsh/zshrc".to_string())]),
        refused_links: Vec::new(),
        install_script: Some("install.sh".to_string()),
    });
    let hook = CountingDotfiles::default();
    let runner = FakeRunner::new();
    let mut events = Vec::<Event>::new();
    perform(RunInput {
        plan: &mut plan,
        runner: &runner,
        admin: &OpenGate,
        dotfiles: Some(&hook),
        home: &home,
        events: &mut events,
        asks: &Quiet,
        cancel: &AtomicBool::new(false),
        clock: &Fixed,
        scope: RunScope::Only {
            items: vec!["keyboard.key-repeat".to_string()],
        },
        config: ConfigRef::default(),
        catalog_version: "t",
        engine_version: "t",
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        hook.0.load(Ordering::SeqCst),
        0,
        "the person chose to Apply one setting, and the Run applied the Dotfiles Repo as well"
    );
}
