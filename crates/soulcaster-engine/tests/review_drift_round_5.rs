//! Review of the Run's Dock write, round 5. The runner is a fake and `HOME` is a temp directory,
//! so nothing here reads or changes this Mac, and dockutil is never started.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::event::{ConfigRef, RunScope};
use soulcaster_core::plan::Plan;
use soulcaster_engine::{Clock, RunInput, perform};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-review-drift-5-{}-{}",
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

struct OpenGate;

struct OpenWindow;

impl soulcaster_engine::AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", "/sandbox/askpass")
    }
}

impl soulcaster_engine::AdminGate for OpenGate {
    fn validate(
        &self,
        _runner: &dyn CommandRunner,
        _home: &Path,
        _password: &Password,
    ) -> soulcaster_engine::Validation {
        soulcaster_engine::Validation::Accepted
    }

    fn open(
        &self,
        _password: &Password,
        _budget: soulcaster_engine::Budget,
    ) -> Result<Box<dyn soulcaster_engine::AdminWindow>, String> {
        Ok(Box::new(OpenWindow))
    }

    fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) -> bool {
        true
    }
}

struct NoAsks;

impl Asks for NoAsks {
    fn password(&self, _request: &PasswordRequest) -> Option<Password> {
        Some(Password::new("not-a-real-password".to_string()))
    }

    fn show_github_code(&self, _code: &str, _url: &str) {}

    fn github_status(&self, _status: SignInStatus) {}
}

struct StillClock(AtomicU64);

impl Clock for StillClock {
    fn now_ms(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }

    fn wait(&self, duration: Duration) {
        self.0
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

#[test]
fn changing_the_dock_apps_keeps_the_folders_beside_them() {
    // dockutil 3.1.3's `removeItem("all")` empties persistent-apps, recent-apps and
    // persistent-others. `dock.apps` only ever names apps, and the last commit's `dock_paths` now
    // ignores the folder section when it decides what the Dock holds. So a change to one app also
    // deletes the person's Downloads stack, which no Config can name and no Run puts back.
    let sandbox = Sandbox::new();
    let runner = FakeRunner::new();
    runner.on(
        |c| line(c).ends_with("id -Gn"),
        [Reply::ok("staff admin\n")],
    );
    runner.on(|c| c.program.ends_with("sw_vers"), [Reply::ok("15.6.1\n")]);
    let downloads = format!("{}/Downloads", sandbox.path().display());
    let listing = format!(
        "Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\t/p\tcom.apple.mail\n\
         Downloads\tfile://{downloads}/\tpersistentOthers\t/p\t\n"
    );
    runner.on(
        |c| line(c).contains("dockutil --list"),
        [Reply::ok(listing.as_str())],
    );
    let config = Config::parse(
        json!({
            "schemaVersion": 1,
            "name": "x",
            "settings": {
                "dock.apps": ["/System/Applications/Mail.app", "/Applications/Zed.app"]
            }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let mut plan = Plan::new(&config, Catalog::shipped());
    let mut events = Vec::new();
    let cancel = AtomicBool::new(false);
    perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &runner,
        home: sandbox.path(),
        events: &mut events,
        asks: &NoAsks,
        cancel: &cancel,
        clock: &StillClock(AtomicU64::new(1_000)),
        scope: RunScope::Full,
        config: ConfigRef {
            name: "x".to_string(),
            ..ConfigRef::default()
        },
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();
    let dockutil: Vec<String> = runner
        .calls()
        .iter()
        .filter(|c| c.program.ends_with("dockutil"))
        .map(line)
        .collect();
    let added_zed = dockutil
        .iter()
        .any(|call| call.contains("--add") && call.contains("Zed.app"));
    assert!(added_zed, "the Run never changed the Dock: {dockutil:?}");
    let emptied = dockutil.iter().any(|call| call.contains("--remove all"));
    let put_back = dockutil
        .iter()
        .any(|call| call.contains("--add") && call.contains(&downloads));
    assert!(
        !emptied || put_back,
        "the Downloads folder was removed and never put back: {dockutil:?}"
    );
}
