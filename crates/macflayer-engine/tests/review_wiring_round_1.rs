//! Review of the wiring Run. Fakes and a temporary home only: nothing here installs, links into a
//! real home, or calls `sudo`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use macflayer_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use macflayer_core::command::{Command, CommandRunner, FakeRunner, Reply};
use macflayer_core::event::{ConfigRef, Event, RunScope};
use macflayer_core::plan::{DotfilesPlan, Plan};
use macflayer_engine::{
    AdminGate, AdminWindow, Budget, Clock, DotfilesHook, DotfilesResult, EngineError,
    MachineRecord, RunInput, Validation, machine_record_path, perform,
};

fn file_name(command: &Command) -> String {
    command
        .program
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

struct Quiet;

impl Asks for Quiet {
    fn password(&self, _: &PasswordRequest) -> Option<Password> {
        None
    }

    fn show_github_code(&self, _: &str, _: &str) {}

    fn github_status(&self, _: SignInStatus) {}
}

struct Fixed;

impl Clock for Fixed {
    fn now_ms(&self) -> u64 {
        1_700_000_000_000
    }

    fn wait(&self, _: Duration) {}
}

struct OpenGate;

struct OpenWindow;

impl AdminWindow for OpenWindow {
    fn elevated(&self, inner: Command) -> Command {
        inner
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner
    }
}

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

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "macflayer-review-wiring-{}-{}",
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

fn dotfiles() -> DotfilesPlan {
    DotfilesPlan {
        repo: "example-you/dotfiles".into(),
        links: [("~/.zshrc".into(), "zsh/zshrc".into())].into(),
        refused_links: vec![],
        install_script: Some("install.sh".into()),
    }
}

fn run(
    home: &Path,
    runner: &FakeRunner,
    plan: &mut Plan,
    hook: &dyn DotfilesHook,
) -> Result<(), EngineError> {
    let mut events = Vec::<Event>::new();
    perform(RunInput {
        plan,
        runner,
        admin: &OpenGate,
        dotfiles: Some(hook),
        home,
        events: &mut events,
        asks: &Quiet,
        cancel: &AtomicBool::new(false),
        clock: &Fixed,
        scope: RunScope::Full,
        config: ConfigRef::default(),
        catalog_version: "t",
        engine_version: "t",
    })
    .map(|_| ())
}

/// Puts the save's temporary name in the way, so the rename cannot land, and puts the directory
/// back afterwards. The original `machine.json` stays where it was.
struct BlockSave {
    home: PathBuf,
    tmp: PathBuf,
}

impl BlockSave {
    fn arm(home: &Path) -> Self {
        let dir = machine_record_path(home)
            .parent()
            .expect("the record lives in a directory")
            .to_path_buf();
        let tmp = dir.join(format!(".machine.json.{}.tmp", std::process::id()));
        fs::create_dir(&tmp).unwrap();
        Self {
            home: home.to_path_buf(),
            tmp,
        }
    }
}

impl Drop for BlockSave {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
        if let Some(dir) = machine_record_path(&self.home).parent() {
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
}

struct Revoke<'a> {
    block: Arc<Mutex<Option<BlockSave>>>,
    home: &'a Path,
}

impl DotfilesHook for Revoke<'_> {
    fn apply(
        &self,
        _: &DotfilesPlan,
        approvals: &dyn macflayer_core::approval::ApprovalStore,
    ) -> DotfilesResult {
        let mut held = approvals.read().unwrap();
        assert!(
            held.taps.remove("revoked/tap"),
            "the step was handed the approval it is revoking"
        );
        held.taps.insert("added/tap".into());
        // `HeldApprovals::write` says this is stored. The script treats that as permission to run.
        approvals.write(&held).unwrap();
        *self.block.lock().unwrap() = Some(BlockSave::arm(self.home));
        DotfilesResult::default()
    }
}

#[test]
fn a_failed_save_keeps_what_the_dotfiles_step_revoked_and_drops_what_it_approved() {
    let home = Sandbox::new();
    let mut record = MachineRecord::empty();
    record.approvals.taps.insert("revoked/tap".into());
    record.save(home.path()).unwrap();

    let runner = FakeRunner::new();
    runner.on(
        |command| file_name(command) == "id",
        [Reply::ok("staff admin\n")],
    );
    runner.on(
        |command| file_name(command) == "sw_vers",
        [Reply::ok("15.6.1\n")],
    );
    let block = Arc::new(Mutex::new(None));
    let hook = Revoke {
        block: Arc::clone(&block),
        home: home.path(),
    };
    let mut plan = Plan {
        config_name: "probe".into(),
        entries: vec![],
        skipped: vec![],
        conflicts: vec![],
        notes: vec![],
        findings: vec![],
        taps: Default::default(),
        dotfiles: Some(dotfiles()),
    };
    let error = run(home.path(), &runner, &mut plan, &hook).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("couldn't update the Machine Record"),
        "{error}"
    );
    let loaded = MachineRecord::load(home.path()).unwrap();
    assert!(
        loaded.approvals.taps.contains("added/tap"),
        "the step's write returned success and the approval is not in the record: {:?}",
        loaded.approvals.taps
    );
    assert!(
        !loaded.approvals.taps.contains("revoked/tap"),
        "the step revoked this approval and the record still grants it: {:?}",
        loaded.approvals.taps
    );
}
