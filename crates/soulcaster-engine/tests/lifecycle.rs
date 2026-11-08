//! Each test names a behaviour from the Item lifecycle. The runner is a fake and `HOME` is a
//! temp directory, so nothing here installs, writes a setting, or uses `sudo` on this Mac.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandRunner, FakeRunner, OutputStream, Reply};
use soulcaster_core::config::Config;
use soulcaster_core::event::{
    ConfigRef, Event, EventKind, EventSink, QueueKind, RunScope, StepPhase,
};
use soulcaster_core::lifecycle::{Assurance, FailureKind, ItemState, SkipReason, Stage};
use soulcaster_core::plan::{DotfilesPlan, Plan, PlanEntry, RunGroup, Why};
use soulcaster_core::recipe::{
    ArchiveKind, BuiltinStep, Completion, ConfirmTrue, Conflict, Effect, Evidence, EvidenceGroup,
    Fact, GitKey, InstallKind, InstallRecipe, MachineValue, MachineValuesKind, MachineValuesRecipe,
    Probe, Recipe, SettingKind, SettingRecipe, Step, Store, StoredType, ValueType,
};
use soulcaster_engine::{
    Approvals, Clock, DotfilesApproval, EngineError, ItemOutcome, LastApplied, MachineRecord,
    RunInput, RunReport, SCHEMA_VERSION, machine_record_path, perform,
};

const SECRET: &str = "s3cret-plover";

/// Hands the command back unwrapped, so a test's runner sees `installer` rather than `sudo`.
/// `tests/admin.rs` covers the real gate.
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

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "soulcaster-engine-{}-{}",
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

struct ScriptedAsks {
    asked: AtomicUsize,
    github: AtomicUsize,
    decline: bool,
}

impl ScriptedAsks {
    fn new() -> Self {
        Self {
            asked: AtomicUsize::new(0),
            github: AtomicUsize::new(0),
            decline: false,
        }
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::Relaxed)
    }
}

impl Asks for ScriptedAsks {
    fn password(&self, request: &PasswordRequest) -> Option<Password> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        assert!(
            !request.why.contains(SECRET),
            "the password request describes the work, not the secret"
        );
        if self.decline {
            return None;
        }
        Some(Password::new(SECRET.to_string()))
    }

    fn show_github_code(&self, _code: &str, _url: &str) {
        self.github.fetch_add(1, Ordering::Relaxed);
    }

    fn github_status(&self, _status: SignInStatus) {}
}

struct ManualClock {
    now: Arc<AtomicU64>,
    waits: Mutex<Vec<Duration>>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(1_000)),
            waits: Mutex::new(Vec::new()),
        }
    }

    fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }

    fn jump(&self, duration: Duration) {
        self.now
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::Relaxed)
    }

    fn wait(&self, duration: Duration) {
        self.waits.lock().unwrap().push(duration);
        self.now
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

struct Harness {
    home: Sandbox,
    runner: FakeRunner,
    asks: ScriptedAsks,
    clock: ManualClock,
    cancel: AtomicBool,
}

impl Harness {
    fn new() -> Self {
        Self {
            home: Sandbox::new(),
            runner: FakeRunner::new(),
            asks: ScriptedAsks::new(),
            clock: ManualClock::new(),
            cancel: AtomicBool::new(false),
        }
    }

    fn script_machine(&self) {
        self.runner.on(
            |command| file_name(command) == "id",
            [Reply::ok("staff admin\n")],
        );
        self.runner.on(
            |command| file_name(command) == "sw_vers",
            [Reply::ok("15.6.1\n")],
        );
    }

    fn run(&self, plan: &mut Plan) -> (RunReport, Vec<Event>) {
        self.run_scoped(plan, RunScope::Full).unwrap()
    }

    fn run_scoped(
        &self,
        plan: &mut Plan,
        scope: RunScope,
    ) -> Result<(RunReport, Vec<Event>), EngineError> {
        // Added last, so a test's own `id` or `sw_vers` rule matches first.
        self.script_machine();
        let mut events = Vec::new();
        let config_name = plan.config_name.clone();
        let asked_before = self.asks.asked();
        let result = perform(RunInput {
            dotfiles: None,
            admin: &OpenGate,
            plan,
            runner: &self.runner,
            home: self.home.path(),
            events: &mut events,
            asks: &self.asks,
            cancel: &self.cancel,
            clock: &self.clock,
            scope,
            config: ConfigRef {
                name: config_name,
                ..ConfigRef::default()
            },
            catalog_version: "test",
            engine_version: "test",
        });
        let blob = serde_json::to_string(&events).unwrap_or_default();
        assert!(!blob.contains(SECRET), "an event carried the password");
        match &result {
            Ok(_) => {
                let path = machine_record_path(self.home.path());
                if path.exists() {
                    let text = fs::read_to_string(&path).unwrap();
                    assert!(
                        !text.contains(SECRET),
                        "the Machine Record carried the password"
                    );
                    let mode = fs::metadata(&path).unwrap().permissions().mode();
                    assert_eq!(mode & 0o777, 0o600);
                }
                assert!(
                    self.asks.asked() - asked_before <= 1,
                    "the password was asked for more than once in one Run"
                );
                assert_discarded(&events);
            }
            Err(error) => assert!(
                !error.to_string().contains(SECRET),
                "an error carried the password"
            ),
        }
        result.map(|report| (report, events))
    }

    fn record(&self) -> MachineRecord {
        MachineRecord::load(self.home.path()).unwrap()
    }
}

fn assert_discarded(events: &[Event]) {
    let held = events
        .iter()
        .position(|event| matches!(event.kind, EventKind::AdminHeld { .. }));
    if let Some(held) = held {
        let discarded = events
            .iter()
            .rposition(|event| matches!(event.kind, EventKind::AdminDiscarded { .. }))
            .expect("the password was still held when the Run ended");
        assert!(discarded > held);
    }
}

fn file_name(command: &Command) -> &str {
    command
        .program
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn args_contain(command: &Command, needle: &str) -> bool {
    command.args.iter().any(|arg| arg.contains(needle))
}

fn is_install(command: &Command, name: &str) -> bool {
    file_name(command) == "brew"
        && command.args.first().map(String::as_str) == Some("install")
        && args_contain(command, name)
}

fn transitions(events: &[Event], id: &str) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ItemState { item, from, to } if item == id => {
                Some(format!("{}->{}", from.name(), to.name()))
            }
            _ => None,
        })
        .collect()
}

fn state<'a>(report: &'a RunReport, id: &str) -> &'a ItemState {
    report
        .states
        .get(id)
        .unwrap_or_else(|| panic!("{id} missing from {:?}", report.states.keys()))
}

fn install_count(runner: &FakeRunner, name: &str) -> usize {
    runner
        .calls()
        .iter()
        .filter(|command| is_install(command, name))
        .count()
}

fn plan(entries: Vec<PlanEntry>) -> Plan {
    Plan {
        config_name: "probe".into(),
        entries,
        skipped: vec![],
        conflicts: vec![],
        notes: vec![],
        findings: vec![],
        taps: BTreeMap::new(),
        dotfiles: None,
    }
}

fn formula(id: &str, requires: &[&str]) -> PlanEntry {
    formula_named(id, id, requires, false)
}

fn tap_formula(id: &str, name: &str) -> PlanEntry {
    formula_named(id, name, &[], true)
}

fn formula_named(id: &str, brew_name: &str, requires: &[&str], third_party: bool) -> PlanEntry {
    PlanEntry {
        id: id.into(),
        name: id.into(),
        stage: Stage::User,
        group: if third_party {
            RunGroup::ThirdParty
        } else {
            RunGroup::Official
        },
        why: Why::Selected,
        recipe: Recipe::Install(InstallRecipe {
            id: id.into(),
            kind: InstallKind::Cli,
            title: id.into(),
            base: false,
            requires: requires.iter().map(|item| (*item).to_string()).collect(),
            min_macos: None,
            conflicts: vec![],
            checks: vec![Probe::BrewFormula {
                name: brew_name.into(),
            }],
            facts: vec![],
            steps: vec![Step::BrewFormula {
                name: brew_name.into(),
                unless: None,
            }],
            notes: vec![],
            bootstrap: None,
        }),
        value: None,
        needs: vec![],
    }
}

fn absent_then_present(harness: &Harness, cellar: &str) {
    let cellar = cellar.to_string();
    harness.runner.on(
        move |command| file_name(command) == "test" && args_contain(command, &cellar),
        [Reply::fails(1, "missing"), Reply::ok("")],
    );
}

fn admin_builtin(id: &str, requires: &[&str]) -> PlanEntry {
    PlanEntry {
        id: id.into(),
        name: id.into(),
        stage: Stage::Admin,
        group: RunGroup::Base,
        why: Why::Selected,
        recipe: Recipe::Install(InstallRecipe {
            id: id.into(),
            kind: InstallKind::Cli,
            title: id.into(),
            base: false,
            requires: requires.iter().map(|item| (*item).to_string()).collect(),
            min_macos: None,
            conflicts: vec![],
            checks: vec![Probe::File {
                path: format!("/tmp/soulcaster-check-{id}"),
            }],
            facts: vec![],
            steps: vec![Step::Builtin {
                name: BuiltinStep::XcodeFirstLaunch,
                admin: true,
                unless: None,
            }],
            notes: vec![],
            bootstrap: None,
        }),
        value: None,
        needs: vec![],
    }
}

fn dock_setting(id: &str, key: &str, effect: Option<Effect>, group: EvidenceGroup) -> PlanEntry {
    PlanEntry {
        id: id.into(),
        name: id.into(),
        stage: Stage::User,
        group: RunGroup::Setting,
        why: Why::Selected,
        recipe: Recipe::Setting(SettingRecipe {
            id: id.into(),
            kind: SettingKind::Setting,
            title: id.into(),
            value_type: ValueType::Boolean,
            values: None,
            pattern: None,
            minimum: None,
            maximum: None,
            unset_means: None,
            requires: vec![],
            stores: vec![Store::Defaults {
                domain: soulcaster_core::recipe::DefaultsDomain::Dock,
                key: key.into(),
                stored_type: StoredType::Bool,
                host_scope: None,
                encode: None,
            }],
            effect,
            evidence: Evidence {
                group,
                confirmed_on: vec![],
            },
            notes: vec![],
        }),
        value: Some(soulcaster_core::config::SettingValue::Bool(true)),
        needs: vec![],
    }
}

#[test]
fn a_passing_check_ends_satisfied_without_acting() {
    let harness = Harness::new();
    let (report, events) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied {
            assurance: Assurance::Proven
        }
    ));
    assert_eq!(
        transitions(&events, "ripgrep"),
        ["pending->checking", "checking->satisfied"]
    );
    assert_eq!(install_count(&harness.runner, "ripgrep"), 0);
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::RunStarted { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::RunFinished { .. }))
    );
    assert_eq!(events[0].seq, 1);
}

#[test]
fn a_failed_check_is_acted_then_verified_and_a_clean_exit_is_not_success() {
    let harness = Harness::new();
    absent_then_present(&harness, "Cellar/ripgrep");
    let (report, events) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied {
            assurance: Assurance::Proven
        }
    ));
    assert_eq!(
        transitions(&events, "ripgrep"),
        [
            "pending->checking",
            "checking->acting",
            "acting->verifying",
            "verifying->satisfied"
        ]
    );
    assert_eq!(install_count(&harness.runner, "ripgrep"), 1);
    assert!(events.iter().any(|event| matches!(
        event.kind,
        EventKind::ItemStep {
            phase: StepPhase::Started,
            ..
        }
    )));
}

#[test]
fn verify_disagrees_with_a_successful_step_so_the_item_fails() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Cellar/ripgrep"),
        [Reply::fails(1, "missing")],
    );
    let (report, _) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert_eq!(
        state(&report, "ripgrep"),
        &ItemState::Failed {
            kind: FailureKind::Verify
        }
    );
    assert!(!harness.record().last_applied.contains_key("ripgrep"));
    assert!(report.states.values().all(|item| item.name() != "differs"));
}

#[test]
fn an_unknowable_check_does_not_act_and_a_fact_does_not_gate() {
    let harness = Harness::new();
    let mut entry = formula("junie", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.checks = vec![Probe::Unknowable {
            reason: "not observable".into(),
        }];
        install.facts = vec![Fact {
            id: "signed-in".into(),
            label: "Signed in".into(),
            probe: Probe::Unknowable {
                reason: "running it may start a server".into(),
            },
        }];
    }
    let (report, events) = harness.run(&mut plan(vec![entry]));
    assert_eq!(state(&report, "junie"), &ItemState::Unknown);
    assert_eq!(install_count(&harness.runner, "junie"), 0);
    let logged = events.iter().any(|event| match &event.kind {
        EventKind::ItemLog { lines, .. } => {
            lines.iter().any(|line| line.contains("can't be checked"))
        }
        _ => false,
    });
    assert!(
        !logged,
        "unknown is not satisfied, so the fact is not a success report"
    );
}

#[test]
fn a_satisfied_item_reports_an_unknowable_fact_without_blocking_on_it() {
    let harness = Harness::new();
    let mut entry = formula("junie", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.facts = vec![Fact {
            id: "signed-in".into(),
            label: "Signed in".into(),
            probe: Probe::Unknowable {
                reason: "agent cli".into(),
            },
        }];
    }
    let (report, events) = harness.run(&mut plan(vec![entry]));
    assert!(matches!(
        state(&report, "junie"),
        ItemState::Satisfied { .. }
    ));
    assert!(events.iter().any(|event| match &event.kind {
        EventKind::ItemLog { item, lines, .. } => {
            item == "junie"
                && lines
                    .iter()
                    .any(|line| line.contains("Signed in: can't be checked"))
        }
        _ => false,
    }));
}

#[test]
fn a_failed_dependency_blocks_only_what_needs_it() {
    let harness = Harness::new();
    harness.runner.on(
        |command| is_install(command, "base-tool"),
        [Reply::fails(1, "tool-exit: boom")],
    );
    absent_then_present(&harness, "Cellar/base-tool");
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Cellar/waiting"),
        [Reply::fails(1, "missing")],
    );
    absent_then_present(&harness, "Cellar/other");
    let (report, _) = harness.run(&mut plan(vec![
        formula("base-tool", &[]),
        formula("waiting", &["base-tool"]),
        formula("other", &[]),
    ]));
    assert_eq!(
        state(&report, "base-tool"),
        &ItemState::Failed {
            kind: FailureKind::ToolExit
        }
    );
    assert_eq!(
        state(&report, "waiting"),
        &ItemState::Blocked {
            by: "base-tool".into()
        }
    );
    assert!(matches!(
        state(&report, "other"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(install_count(&harness.runner, "waiting"), 0);
    assert_eq!(install_count(&harness.runner, "other"), 1);
    assert!(
        harness.clock.waits().is_empty(),
        "a tool exit is not retried"
    );
}

#[test]
fn only_network_errors_retry_and_the_delay_is_the_clocks() {
    let harness = Harness::new();
    harness.runner.on(
        |command| is_install(command, "ripgrep"),
        [
            Reply::fails(1, "operation timed out"),
            Reply::fails(1, "502 bad gateway"),
            Reply::ok(""),
        ],
    );
    absent_then_present(&harness, "Cellar/ripgrep");
    let (report, _) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(
        harness.clock.waits(),
        [Duration::from_secs(30), Duration::from_secs(120)]
    );
    assert_eq!(install_count(&harness.runner, "ripgrep"), 3);
}

#[test]
fn a_person_step_is_queued_and_the_run_does_not_wait() {
    let harness = Harness::new();
    let mut entry = formula("aerospace", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.steps = vec![Step::NeedsYou {
            id: "grant-accessibility".into(),
            text: "Allow AeroSpace under Accessibility.".into(),
            link: Some("x-apple.systempreferences:Privacy_Accessibility".into()),
            completion: Completion::Confirm {
                confirm: ConfirmTrue,
            },
            unless: None,
        }];
    }
    absent_then_present(&harness, "Cellar/aerospace");
    let mut planned = plan(vec![entry]);
    let (report, _) = harness.run(&mut planned);
    assert_eq!(
        state(&report, "aerospace"),
        &ItemState::NeedsYou { cause: None }
    );
    assert_eq!(report.queue.len(), 1);
    assert_eq!(report.queue[0].kind, QueueKind::Person);
    assert_eq!(harness.asks.github.load(Ordering::Relaxed), 0);
    // The next check passes. The open entry still holds the Item.
    let (report, _) = harness.run(&mut planned);
    assert_eq!(
        state(&report, "aerospace"),
        &ItemState::NeedsYou { cause: None }
    );
    assert!(
        harness
            .record()
            .queue
            .iter()
            .any(|entry| { entry.is_open() && entry.step_id == "grant-accessibility" })
    );
    assert_eq!(install_count(&harness.runner, "aerospace"), 0);
}

#[test]
fn a_second_run_on_a_satisfied_machine_only_checks() {
    let harness = Harness::new();
    absent_then_present(&harness, "Cellar/ripgrep");
    let mut plan = plan(vec![formula("ripgrep", &[])]);
    harness.run(&mut plan);
    assert_eq!(install_count(&harness.runner, "ripgrep"), 1);
    let (report, events) = harness.run(&mut plan);
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(install_count(&harness.runner, "ripgrep"), 1);
    assert!(transitions(&events, "ripgrep").contains(&"checking->satisfied".to_string()));
    assert!(
        !transitions(&events, "ripgrep")
            .iter()
            .any(|hop| hop.contains("acting"))
    );
}

#[test]
fn an_interrupted_run_is_recorded_and_the_next_run_rechecks() {
    let harness = Harness::new();
    let path = machine_record_path(harness.home.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        r#"{"schemaVersion":1,"running":{"runId":"old","item":"ripgrep"}}"#,
    )
    .unwrap();
    let (report, events) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied { .. }
    ));
    assert!(events.iter().any(|event| match &event.kind {
        EventKind::RunWarning { text } => text.contains("interrupted"),
        _ => false,
    }));
    let record = harness.record();
    assert!(record.running.is_none());
    assert_eq!(record.interruptions[0].item, "ripgrep");
    assert!(
        harness
            .runner
            .calls()
            .iter()
            .any(|command| file_name(command) == "test")
    );
}

#[test]
fn cancel_between_items_keeps_a_consistent_record_and_discards_the_password() {
    let harness = Harness::new();
    harness.script_machine();
    let flag = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&flag);
    struct Arming<'a> {
        inner: &'a FakeRunner,
        flag: Arc<AtomicBool>,
    }
    impl CommandRunner for Arming<'_> {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<soulcaster_core::command::CommandOutput, soulcaster_core::command::SpawnError>
        {
            if is_install(command, "first") {
                self.flag.store(true, Ordering::Relaxed);
            }
            self.inner.run(command, on_line)
        }
    }
    absent_then_present(&harness, "Cellar/first");
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Cellar/second"),
        [Reply::fails(1, "missing")],
    );
    let mut plan = plan(vec![formula("first", &[]), formula("second", &[])]);
    let mut events = Vec::new();
    let report = perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &Arming {
            inner: &harness.runner,
            flag: seen,
        },
        home: harness.home.path(),
        events: &mut events,
        asks: &harness.asks,
        cancel: flag.as_ref(),
        clock: &harness.clock,
        scope: RunScope::Full,
        config: ConfigRef {
            name: "probe".into(),
            ..ConfigRef::default()
        },
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();
    assert!(report.cancelled);
    assert!(matches!(
        state(&report, "first"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(state(&report, "second"), &ItemState::Pending);
    assert!(harness.record().running.is_none());
    assert!(!harness.record().last_applied.contains_key("second"));
    assert_discarded(&events);
    let blob = serde_json::to_string(&events).unwrap();
    assert!(!blob.contains(SECRET));
}

#[test]
fn the_password_is_discarded_after_sixty_minutes_and_the_rest_waits() {
    let harness = Harness::new();
    harness.script_machine();
    absent_then_present(&harness, "soulcaster-check-earlier");
    absent_then_present(&harness, "soulcaster-check-later");
    struct Jumping<'a> {
        inner: &'a FakeRunner,
        clock: &'a ManualClock,
        jumped: AtomicBool,
    }
    impl CommandRunner for Jumping<'_> {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<soulcaster_core::command::CommandOutput, soulcaster_core::command::SpawnError>
        {
            if file_name(command) == "xcodebuild" && !self.jumped.swap(true, Ordering::Relaxed) {
                self.clock.jump(Duration::from_secs(60 * 60));
            }
            self.inner.run(command, on_line)
        }
    }
    let mut plan = plan(vec![
        admin_builtin("earlier", &[]),
        admin_builtin("later", &[]),
    ]);
    let mut events = Vec::new();
    let report = perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &Jumping {
            inner: &harness.runner,
            clock: &harness.clock,
            jumped: AtomicBool::new(false),
        },
        home: harness.home.path(),
        events: &mut events,
        asks: &harness.asks,
        cancel: &harness.cancel,
        clock: &harness.clock,
        scope: RunScope::Full,
        config: ConfigRef {
            name: "probe".into(),
            ..ConfigRef::default()
        },
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();
    assert!(matches!(
        state(&report, "earlier"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(
        state(&report, "later"),
        &ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin)
        }
    );
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::AdminDiscarded { why } if why.contains("60 minutes")
    )));
    assert!(report.queue.iter().any(|entry| {
        entry.item == "later" && entry.kind == QueueKind::NeedsAdmin && entry.needs_password
    }));
    assert_eq!(
        harness
            .runner
            .calls()
            .iter()
            .filter(|command| file_name(command) == "xcodebuild")
            .count(),
        1
    );
    let blob = serde_json::to_string(&events).unwrap();
    assert!(!blob.contains(SECRET));
}

#[test]
fn a_full_run_does_not_ask_when_nothing_is_in_the_admin_stage() {
    let harness = Harness::new();
    absent_then_present(&harness, "Cellar/ripgrep");
    let (report, events) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert_eq!(harness.asks.asked(), 0);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::AdminHeld { .. }))
    );
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied { .. }
    ));
}

#[test]
fn the_admin_stage_discards_the_password_before_any_user_stage_command() {
    let harness = Harness::new();
    harness.script_machine();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    struct Trace<'a> {
        inner: &'a FakeRunner,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl CommandRunner for Trace<'_> {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<soulcaster_core::command::CommandOutput, soulcaster_core::command::SpawnError>
        {
            self.log.lock().unwrap().push(format!(
                "call {} {}",
                file_name(command),
                command.args.join(" ")
            ));
            self.inner.run(command, on_line)
        }
    }
    struct TraceSink<'a> {
        inner: &'a mut Vec<Event>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl EventSink for TraceSink<'_> {
        fn send(&mut self, event: Event) {
            match &event.kind {
                EventKind::AdminHeld { .. } => self.log.lock().unwrap().push("held".into()),
                EventKind::AdminDiscarded { .. } => {
                    self.log.lock().unwrap().push("discarded".into())
                }
                _ => {}
            }
            self.inner.send(event);
        }
    }
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "soulcaster-check-clt"),
        [Reply::fails(1, "missing"), Reply::ok("")],
    );
    absent_then_present(&harness, "Cellar/ripgrep");
    let mut clt = admin_builtin("clt", &[]);
    if let Recipe::Install(install) = &mut clt.recipe {
        install.steps = vec![Step::Builtin {
            name: BuiltinStep::CommandLineTools,
            admin: true,
            unless: None,
        }];
    }
    let mut plan = plan(vec![clt, formula("ripgrep", &[])]);
    let mut events = Vec::new();
    let log_for_sink = Arc::clone(&log);
    perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &Trace {
            inner: &harness.runner,
            log: Arc::clone(&log),
        },
        home: harness.home.path(),
        events: &mut TraceSink {
            inner: &mut events,
            log: log_for_sink,
        },
        asks: &harness.asks,
        cancel: &harness.cancel,
        clock: &harness.clock,
        scope: RunScope::Full,
        config: ConfigRef::default(),
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();
    let trace = log.lock().unwrap().clone();
    let held = trace.iter().position(|line| line == "held").unwrap();
    let discarded = trace.iter().position(|line| line == "discarded").unwrap();
    assert!(
        trace[held..discarded]
            .iter()
            .any(|line| line.contains("softwareupdate"))
    );
    assert!(
        trace[held..discarded]
            .iter()
            .all(|line| !line.contains("ripgrep"))
    );
    assert!(
        trace[discarded..]
            .iter()
            .any(|line| line.contains("ripgrep"))
    );
}

#[test]
fn an_admin_item_that_requires_a_user_stage_tap_does_not_run_that_tap_while_the_password_is_held() {
    let harness = Harness::new();
    harness.script_machine();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    struct Trace<'a> {
        inner: &'a FakeRunner,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl CommandRunner for Trace<'_> {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<soulcaster_core::command::CommandOutput, soulcaster_core::command::SpawnError>
        {
            self.log.lock().unwrap().push(format!(
                "call {} {}",
                file_name(command),
                command.args.join(" ")
            ));
            self.inner.run(command, on_line)
        }
    }
    struct TraceSink<'a> {
        inner: &'a mut Vec<Event>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl EventSink for TraceSink<'_> {
        fn send(&mut self, event: Event) {
            match &event.kind {
                EventKind::AdminHeld { .. } => self.log.lock().unwrap().push("held".into()),
                EventKind::AdminDiscarded { .. } => {
                    self.log.lock().unwrap().push("discarded".into())
                }
                _ => {}
            }
            self.inner.send(event);
        }
    }
    absent_then_present(&harness, "Cellar/tool");
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "soulcaster-check-root"),
        [Reply::fails(1, "missing")],
    );
    // The tap is approved so the missing-approval gate does not fire before the stage rule.
    let mut record = MachineRecord::empty();
    record.approvals.taps.insert("someone/tap".into());
    record.save(harness.home.path()).unwrap();

    let mut plan = plan(vec![
        admin_builtin("root-item", &["tool"]),
        tap_formula("tool", "someone/tap/tool"),
    ]);
    let mut events = Vec::new();
    let report = perform(RunInput {
        dotfiles: None,
        admin: &OpenGate,
        plan: &mut plan,
        runner: &Trace {
            inner: &harness.runner,
            log: Arc::clone(&log),
        },
        home: harness.home.path(),
        events: &mut TraceSink {
            inner: &mut events,
            log: Arc::clone(&log),
        },
        asks: &harness.asks,
        cancel: &harness.cancel,
        clock: &harness.clock,
        scope: RunScope::Full,
        config: ConfigRef::default(),
        catalog_version: "test",
        engine_version: "test",
    })
    .unwrap();

    let tool_at = plan
        .entries
        .iter()
        .position(|entry| entry.id == "tool")
        .unwrap();
    let root_at = plan
        .entries
        .iter()
        .position(|entry| entry.id == "root-item")
        .unwrap();
    assert!(
        tool_at < root_at,
        "sort_for_run places the requirement first, even across stages"
    );
    assert_eq!(plan.entries[tool_at].stage, Stage::User);
    assert_eq!(plan.entries[root_at].stage, Stage::Admin);

    let trace = log.lock().unwrap().clone();
    let held = trace
        .iter()
        .position(|line| line == "held")
        .expect("password held");
    let discarded = trace
        .iter()
        .position(|line| line == "discarded")
        .expect("password discarded");
    assert!(
        trace[held..discarded]
            .iter()
            .all(|line| !line.contains("someone/tap/tool") && !line.contains("xcodebuild")),
        "user-stage or admin work ran while the password was held: {trace:?}"
    );
    assert!(
        trace[discarded..]
            .iter()
            .any(|line| line.contains("someone/tap/tool"))
    );
    assert!(matches!(
        state(&report, "tool"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(
        state(&report, "root-item"),
        &ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin)
        }
    );
    assert_eq!(report.queue[0].kind, QueueKind::NeedsAdmin);
    assert!(report.queue[0].needs_password);
}

#[test]
fn a_scoped_run_does_not_ask_when_it_has_no_admin_work_and_it_reaches_what_the_item_needs() {
    let harness = Harness::new();
    absent_then_present(&harness, "Cellar/app");
    absent_then_present(&harness, "Cellar/tool");
    let mut plan = plan(vec![formula("tool", &[]), formula("app", &["tool"])]);
    let (report, events) = harness
        .run_scoped(
            &mut plan,
            RunScope::Items {
                items: vec!["app".into()],
            },
        )
        .unwrap();
    assert_eq!(harness.asks.asked(), 0);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::AdminHeld { .. }))
    );
    assert!(matches!(
        state(&report, "tool"),
        ItemState::Satisfied { .. }
    ));
    assert!(matches!(state(&report, "app"), ItemState::Satisfied { .. }));
}

#[test]
fn a_missing_tap_approval_queues_the_item_instead_of_installing_it() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Cellar/tool"),
        [Reply::fails(1, "missing")],
    );
    let (report, _) = harness.run(&mut plan(vec![tap_formula("tool", "someone/tap/tool")]));
    assert_eq!(state(&report, "tool"), &ItemState::NeedsYou { cause: None });
    assert_eq!(report.queue[0].kind, QueueKind::Tap);
    assert_eq!(install_count(&harness.runner, "tool"), 0);
}

#[test]
fn a_surprise_admin_need_is_remembered_and_a_third_party_item_is_left_to_the_person() {
    let harness = Harness::new();
    harness.runner.on(
        |command| is_install(command, "ghostty"),
        [Reply::fails(1, "homebrew_no_sudo rejected this cask")],
    );
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Caskroom/ghostty"),
        [Reply::fails(1, "missing")],
    );
    let mut cask = formula("ghostty", &[]);
    if let Recipe::Install(install) = &mut cask.recipe {
        install.steps = vec![Step::BrewCask {
            name: "ghostty".into(),
            admin: false,
            unless: None,
        }];
        install.checks = vec![Probe::BrewCask {
            name: "ghostty".into(),
        }];
    }
    let (report, _) = harness.run(&mut plan(vec![cask]));
    assert_eq!(
        state(&report, "ghostty"),
        &ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin)
        }
    );
    assert!(
        harness
            .record()
            .learned_admin
            .iter()
            .any(|id| id == "ghostty")
    );

    let mut tapped = tap_formula("tool", "someone/tap/tool");
    if let Recipe::Install(install) = &mut tapped.recipe {
        install.steps = vec![Step::BrewCask {
            name: "someone/tap/tool".into(),
            admin: false,
            unless: None,
        }];
        install.checks = vec![Probe::BrewCask {
            name: "someone/tap/tool".into(),
        }];
    }
    let mut record = MachineRecord::empty();
    record.approvals.taps.insert("someone/tap".into());
    record.save(harness.home.path()).unwrap();
    let harness = Harness::new();
    // Reuse is awkward across homes. Drive the third-party case on this fresh home.
    let mut record = MachineRecord::empty();
    record.approvals.taps.insert("someone/tap".into());
    record.save(harness.home.path()).unwrap();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Caskroom/tool"),
        [Reply::fails(1, "missing")],
    );
    harness.runner.on(
        |command| is_install(command, "someone/tap/tool"),
        [Reply::fails(1, "homebrew_no_sudo")],
    );
    let (report, _) = harness.run(&mut plan(vec![tapped]));
    assert_eq!(report.queue[0].kind, QueueKind::NeedsAdmin);
    assert!(!report.queue[0].needs_password);
    assert!(!harness.record().learned_admin.iter().any(|id| id == "tool"));
}

#[test]
fn a_stalled_admin_step_waits_on_a_person_and_a_stalled_user_step_fails() {
    let harness = Harness::new();
    harness
        .runner
        .on(|command| file_name(command) == "xcodebuild", [Reply::Stall]);
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "soulcaster-check-root"),
        [Reply::fails(1, "missing")],
    );
    let (report, _) = harness.run(&mut plan(vec![admin_builtin("root-item", &[])]));
    assert_eq!(
        state(&report, "root-item"),
        &ItemState::NeedsYou {
            cause: Some(FailureKind::Stalled)
        }
    );
    assert_eq!(report.queue[0].kind, QueueKind::NeedsAdmin);

    let harness = Harness::new();
    harness
        .runner
        .on(|command| is_install(command, "ripgrep"), [Reply::Stall]);
    absent_then_present(&harness, "Cellar/ripgrep");
    let (report, _) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert_eq!(
        state(&report, "ripgrep"),
        &ItemState::Failed {
            kind: FailureKind::Stalled
        }
    );
    assert!(report.queue.is_empty());
}

#[test]
fn command_line_tools_falling_back_blocks_what_needs_them_and_other_items_continue() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "CommandLineTools"),
        [Reply::fails(1, "missing")],
    );
    harness.runner.on(
        |command| file_name(command) == "softwareupdate",
        [Reply::fails(1, "not available")],
    );
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Cellar/homebrew"),
        [Reply::fails(1, "missing")],
    );
    absent_then_present(&harness, "Cellar/ripgrep");
    let mut clt = admin_builtin("command-line-tools", &[]);
    if let Recipe::Install(install) = &mut clt.recipe {
        install.checks = vec![Probe::Builtin {
            name: soulcaster_core::recipe::BuiltinProbe::CommandLineTools,
        }];
        install.steps = vec![Step::Builtin {
            name: BuiltinStep::CommandLineTools,
            admin: true,
            unless: None,
        }];
    }
    let (report, _) = harness.run(&mut plan(vec![
        clt,
        formula("homebrew", &["command-line-tools"]),
        formula("ripgrep", &[]),
    ]));
    assert_eq!(
        state(&report, "command-line-tools"),
        &ItemState::NeedsYou { cause: None }
    );
    assert_eq!(
        state(&report, "homebrew"),
        &ItemState::Blocked {
            by: "command-line-tools".into()
        }
    );
    assert!(matches!(
        state(&report, "ripgrep"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(install_count(&harness.runner, "homebrew"), 0);
}

#[test]
fn a_standard_account_without_the_base_is_refused_before_anything_is_installed() {
    let harness = Harness::new();
    harness
        .runner
        .on(|command| file_name(command) == "id", [Reply::ok("staff\n")]);
    harness.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "missing")],
    );
    let error = harness
        .run_scoped(&mut plan(vec![formula("ripgrep", &[])]), RunScope::Full)
        .unwrap_err();
    assert!(matches!(error, EngineError::AdminRequired { .. }));
    assert!(error.to_string().contains("admin"));
    assert_eq!(install_count(&harness.runner, "ripgrep"), 0);
    assert!(!machine_record_path(harness.home.path()).exists());
}

#[test]
fn a_declined_password_does_not_run_the_admin_step() {
    let mut harness = Harness::new();
    harness.asks.decline = true;
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "soulcaster-check-root"),
        [Reply::fails(1, "missing")],
    );
    let (report, events) = harness.run(&mut plan(vec![admin_builtin("root-item", &[])]));
    assert_eq!(
        state(&report, "root-item"),
        &ItemState::NeedsYou {
            cause: Some(FailureKind::NeedsAdmin)
        }
    );
    assert!(
        !harness
            .runner
            .calls()
            .iter()
            .any(|command| file_name(command) == "xcodebuild")
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::AdminHeld { .. }))
    );
}

#[test]
fn settings_are_stored_until_confirmed_and_a_broken_one_is_skipped() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "defaults" && args_contain(command, "autohide"),
        [Reply::ok("0\n"), Reply::ok("1\n")],
    );
    let mut setting = dock_setting(
        "dock.autohide",
        "autohide",
        Some(Effect::RestartDock),
        EvidenceGroup::Documented,
    );
    if let Recipe::Setting(recipe) = &mut setting.recipe {
        recipe.evidence.confirmed_on = vec!["15.6.1".into()];
    }
    let mut broken = dock_setting("menu-bar.extras", "extras", None, EvidenceGroup::Broken);
    broken.recipe = {
        let Recipe::Setting(mut recipe) = broken.recipe else {
            unreachable!()
        };
        recipe.evidence.group = EvidenceGroup::Broken;
        Recipe::Setting(recipe)
    };
    let mut unconfirmed = dock_setting(
        "dock.mineffect",
        "mineffect",
        None,
        EvidenceGroup::Unconfirmed,
    );
    if let Recipe::Setting(recipe) = &mut unconfirmed.recipe {
        recipe.stores[0] = Store::Defaults {
            domain: soulcaster_core::recipe::DefaultsDomain::Dock,
            key: "mineffect".into(),
            stored_type: StoredType::String,
            host_scope: None,
            encode: None,
        };
        recipe.value_type = ValueType::String;
    }
    unconfirmed.value = Some(soulcaster_core::config::SettingValue::String(
        "genie".into(),
    ));
    harness.runner.on(
        |command| file_name(command) == "defaults" && args_contain(command, "mineffect"),
        [Reply::ok("scale\n"), Reply::ok("genie\n")],
    );
    let (report, events) = harness.run(&mut plan(vec![setting, broken, unconfirmed]));
    assert_eq!(
        state(&report, "dock.autohide"),
        &ItemState::Satisfied {
            assurance: Assurance::Proven
        }
    );
    assert_eq!(
        state(&report, "menu-bar.extras"),
        &ItemState::Skipped {
            reason: SkipReason::BrokenHere
        }
    );
    assert_eq!(
        state(&report, "dock.mineffect"),
        &ItemState::Satisfied {
            assurance: Assurance::Stored
        }
    );
    assert!(events.iter().any(|event| {
        match &event.kind {
            EventKind::ItemLog { lines, .. } => lines
                .iter()
                .any(|line| line.contains("effect not confirmed")),
            _ => false,
        }
    }));
    assert_eq!(
        harness
            .record()
            .unavailable
            .get("menu-bar.extras")
            .map(String::as_str),
        Some("broken-here")
    );
    assert!(
        harness
            .runner
            .calls()
            .iter()
            .any(|command| file_name(command) == "killall" && args_contain(command, "Dock"))
    );
    let prior = &harness.record().last_applied["dock.autohide"].prior_value;
    assert_eq!(prior, &Some(serde_json::Value::Bool(false)));
}

#[test]
fn an_unset_setting_is_written_by_a_run() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "defaults" && args_contain(command, "autohide"),
        [Reply::fails(1, "does not exist"), Reply::ok("1\n")],
    );
    let (report, _) = harness.run(&mut plan(vec![dock_setting(
        "dock.autohide",
        "autohide",
        None,
        EvidenceGroup::Documented,
    )]));
    assert!(matches!(
        state(&report, "dock.autohide"),
        ItemState::Satisfied { .. }
    ));
    assert!(harness.runner.calls().iter().any(|command| {
        file_name(command) == "defaults"
            && command.args.first().map(String::as_str) == Some("write")
    }));
}

#[test]
fn macos_too_old_and_a_linked_gitconfig_are_skipped() {
    let harness = Harness::new();
    let mut entry = formula("old", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.min_macos = Some("99".into());
    }
    std::os::unix::fs::symlink(
        "/tmp/soulcaster-not-a-config",
        harness.home.path().join(".gitconfig"),
    )
    .unwrap();
    let git = PlanEntry {
        id: "git-identity".into(),
        name: "Git identity".into(),
        stage: Stage::User,
        group: RunGroup::GitValues,
        why: Why::Selected,
        recipe: Recipe::MachineValues(MachineValuesRecipe {
            id: "git-identity".into(),
            kind: MachineValuesKind::MachineValues,
            title: "Git identity".into(),
            requires: vec![],
            values: vec![MachineValue {
                name: "name".into(),
                label: "Your name for Git commits".into(),
                store: Store::GitConfig {
                    key: GitKey::UserName,
                },
            }],
        }),
        value: None,
        needs: vec![],
    };
    let (report, _) = harness.run(&mut plan(vec![entry, git]));
    assert_eq!(
        state(&report, "old"),
        &ItemState::Skipped {
            reason: SkipReason::MacosTooOld
        }
    );
    assert_eq!(
        state(&report, "git-identity"),
        &ItemState::Skipped {
            reason: SkipReason::WritesThroughLink
        }
    );
    assert!(
        !harness
            .runner
            .calls()
            .iter()
            .any(|command| file_name(command) == "git")
    );
    assert_eq!(
        harness.record().unavailable.get("old").map(String::as_str),
        Some("macos-too-old")
    );
}

#[test]
fn git_identity_is_taken_from_git_config_without_asking_and_otherwise_queued() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "git" && args_contain(command, "--get"),
        [Reply::ok("Ada Lovelace\n")],
    );
    let git = git_identity();
    let (report, _) = harness.run(&mut plan(vec![git]));
    assert!(matches!(
        state(&report, "git-identity"),
        ItemState::Satisfied { .. }
    ));
    assert_eq!(
        harness
            .record()
            .local_values
            .get("name")
            .map(String::as_str),
        Some("Ada Lovelace")
    );
    assert_eq!(harness.asks.github.load(Ordering::Relaxed), 0);
    assert!(
        harness
            .runner
            .calls()
            .iter()
            .all(|command| { file_name(command) != "git" || args_contain(command, "--get") })
    );
    assert!(
        harness
            .runner
            .calls()
            .iter()
            .filter(|command| file_name(command) == "git")
            .all(|command| {
                command
                    .env
                    .iter()
                    .any(|(key, value)| key == "GIT_CONFIG_GLOBAL" && value == "/dev/null")
                    && command
                        .env
                        .iter()
                        .any(|(key, value)| key == "GIT_CONFIG_NOSYSTEM" && value == "1")
            })
    );

    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "git",
        [Reply::fails(1, "missing")],
    );
    let (report, _) = harness.run(&mut plan(vec![git_identity()]));
    assert_eq!(
        state(&report, "git-identity"),
        &ItemState::NeedsYou { cause: None }
    );
    assert_eq!(report.queue[0].text, "Your name for Git commits");
    assert_eq!(harness.asks.github.load(Ordering::Relaxed), 0);
}

fn git_identity() -> PlanEntry {
    PlanEntry {
        id: "git-identity".into(),
        name: "Git identity".into(),
        stage: Stage::User,
        group: RunGroup::GitValues,
        why: Why::Selected,
        recipe: Recipe::MachineValues(MachineValuesRecipe {
            id: "git-identity".into(),
            kind: MachineValuesKind::MachineValues,
            title: "Git identity".into(),
            requires: vec![],
            values: vec![MachineValue {
                name: "name".into(),
                label: "Your name for Git commits".into(),
                store: Store::GitConfig {
                    key: GitKey::UserName,
                },
            }],
        }),
        value: None,
        needs: vec![],
    }
}

#[test]
fn a_checksum_mismatch_is_not_retried() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test",
        [Reply::fails(1, "missing")],
    );
    harness.runner.on(
        |command| file_name(command) == "shasum",
        [Reply::ok("ffff  tool.zip\n")],
    );
    let mut entry = formula("kotlin-lsp", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.checks = vec![Probe::File {
            path: "/tmp/soulcaster-kotlin".into(),
        }];
        install.steps = vec![Step::Download {
            tool: "kotlin-lsp".into(),
            version: "1".into(),
            url: "https://example.com/tool.zip".into(),
            sha256: "abc123".into(),
            archive: ArchiveKind::Zip,
            signed_by: "ABCDEFGHIJ".into(),
            signed_file: "tool".into(),
            links: vec![],
            unless: None,
        }];
    }
    let (report, _) = harness.run(&mut plan(vec![entry]));
    assert_eq!(
        state(&report, "kotlin-lsp"),
        &ItemState::Failed {
            kind: FailureKind::Checksum
        }
    );
    assert!(harness.clock.waits().is_empty());
    assert_eq!(
        harness
            .runner
            .calls()
            .iter()
            .filter(|command| file_name(command) == "curl")
            .count(),
        1
    );
}

#[test]
fn a_conflict_already_on_the_machine_is_queued_and_not_installed() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Other.app"),
        [Reply::ok("")],
    );
    let mut entry = formula("tailscale", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.conflicts = vec![Conflict {
            probe: Probe::AppBundle {
                bundle_id: "io.tailscale.ipn.macsys".into(),
                path: "/Applications/Other.app".into(),
            },
            message: "the App Store copy".into(),
        }];
    }
    let (report, _) = harness.run(&mut plan(vec![entry]));
    assert_eq!(
        state(&report, "tailscale"),
        &ItemState::NeedsYou { cause: None }
    );
    assert_eq!(report.queue[0].kind, QueueKind::Conflict);
    assert_eq!(install_count(&harness.runner, "tailscale"), 0);
}

#[test]
fn dotfiles_are_not_applied_by_this_run() {
    let harness = Harness::new();
    let mut plan = plan(vec![formula("ripgrep", &[])]);
    plan.dotfiles = Some(DotfilesPlan {
        repo: "ada/dotfiles".into(),
        links: BTreeMap::from([("~/.gitconfig".into(), "gitconfig".into())]),
        refused_links: vec![],
        install_script: Some("script/setup".into()),
    });
    let (_report, events) = harness.run(&mut plan);
    assert!(events.iter().any(|event| match &event.kind {
        EventKind::RunWarning { text } => text.contains("Dotfiles"),
        _ => false,
    }));
    assert!(
        !harness
            .runner
            .calls()
            .iter()
            .any(|command| args_contain(command, "clone") || args_contain(command, "setup"))
    );
}

#[test]
fn a_shipped_plan_that_already_passes_its_checks_installs_nothing() {
    let harness = Harness::new();
    let config =
        Config::parse(br#"{"schemaVersion":1,"name":"probe","items":["ripgrep","ghostty"]}"#)
            .unwrap();
    let mut plan = Plan::new(&config, Catalog::shipped());
    let (report, _) = harness.run(&mut plan);
    assert_eq!(
        harness
            .runner
            .calls()
            .iter()
            .filter(|command| file_name(command) == "brew"
                && command.args.first().map(String::as_str) == Some("install"))
            .count(),
        0
    );
    for entry in &plan.entries {
        assert!(state(&report, &entry.id).is_terminal(), "{}", entry.id);
    }
}

#[test]
fn the_machine_record_is_schema_versioned_atomic_and_refuses_a_corrupt_file() {
    let home = Sandbox::new();
    let mut record = MachineRecord::empty();
    let commit = soulcaster_core::approval::CommitSha::parse(&"a".repeat(40)).unwrap();
    record.approvals = Approvals {
        taps: ["someone/tap".to_string()].into(),
        packages: ["someone/tap/tool".to_string()].into(),
        dotfiles: Some(DotfilesApproval {
            repo: "github.com/ada/dotfiles".into(),
            commit: commit.clone(),
            links: BTreeMap::from([("~/.zshrc".into(), "zsh/zshrc".into())]),
        }),
        script: None,
    };
    record.clone_path = Some("~/src/dotfiles".into());
    record.local_values.insert("name".into(), "Ada".into());
    record.save(home.path()).unwrap();
    let loaded = MachineRecord::load(home.path()).unwrap();
    assert_eq!(loaded.schema_version, SCHEMA_VERSION);
    assert_eq!(loaded.approvals.dotfiles.as_ref().unwrap().commit, commit);
    assert!(loaded.approvals.packages.contains("someone/tap/tool"));
    assert_eq!(loaded, record);
    assert!(
        fs::read_dir(home.path().join("Library/Application Support/Soulcaster"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .all(|entry| !entry.file_name().to_string_lossy().contains(".tmp"))
    );

    let path = machine_record_path(home.path());
    fs::write(&path, b"{").unwrap();
    let error = MachineRecord::load(home.path()).unwrap_err();
    assert!(error.to_string().contains("corrupt"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), b"{");

    fs::write(&path, br#"{"schemaVersion":99}"#).unwrap();
    match MachineRecord::load(home.path()).unwrap_err() {
        EngineError::UnsupportedRecord { found: 99, .. } => {}
        other => panic!("expected an unsupported-version error, got {other}"),
    }
    assert_eq!(fs::read(&path).unwrap(), br#"{"schemaVersion":99}"#);

    let fresh = Sandbox::new();
    assert_eq!(
        MachineRecord::load(fresh.path()).unwrap(),
        MachineRecord::empty()
    );
}

#[test]
fn a_scoped_run_checks_a_requirement_the_record_already_calls_satisfied() {
    let harness = Harness::new();
    let mut record = MachineRecord::empty();
    record.outcomes.insert(
        "tool".into(),
        ItemOutcome {
            state: "satisfied".into(),
            failure: None,
            blocked_by: None,
            assurance: Some(Assurance::Proven),
            skip: None,
        },
    );
    record.last_applied.insert(
        "tool".into(),
        LastApplied {
            value: serde_json::Value::Bool(true),
            applied_at: 1,
            run_id: "old".into(),
            catalog_version: "test".into(),
            macos_version: None,
            assurance: Assurance::Proven,
            prior_value: None,
        },
    );
    record.save(harness.home.path()).unwrap();
    absent_then_present(&harness, "Cellar/app");
    let mut plan = plan(vec![formula("tool", &[]), formula("app", &["tool"])]);
    let (report, _) = harness
        .run_scoped(
            &mut plan,
            RunScope::Items {
                items: vec!["app".into()],
            },
        )
        .unwrap();
    assert!(
        report.states.contains_key("tool"),
        "the record was treated as proof that tool was installed"
    );
    assert!(matches!(
        state(&report, "tool"),
        ItemState::Satisfied { .. }
    ));
    assert!(matches!(state(&report, "app"), ItemState::Satisfied { .. }));
    assert_eq!(install_count(&harness.runner, "tool"), 0);
    assert!(
        harness.runner.calls().iter().any(|command| {
            file_name(command) == "test" && args_contain(command, "Cellar/tool")
        }),
        "tool was not checked"
    );
}

#[test]
fn xcode_stays_in_the_user_stage_until_its_gate_is_done() {
    let harness = Harness::new();
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "Xcode.app"),
        [Reply::fails(1, "missing")],
    );
    harness.runner.on(
        |command| file_name(command) == "test" && args_contain(command, "soulcaster-check-xcode"),
        [Reply::fails(1, "missing")],
    );
    let mut entry = admin_builtin("xcode", &[]);
    if let Recipe::Install(install) = &mut entry.recipe {
        install.steps = vec![
            Step::NeedsYou {
                id: "install-xcode".into(),
                text: "Install Xcode from the App Store.".into(),
                link: None,
                completion: Completion::Until {
                    until: Probe::File {
                        path: "/Applications/Xcode.app".into(),
                    },
                },
                unless: Some(Probe::File {
                    path: "/Applications/Xcode.app".into(),
                }),
            },
            Step::Builtin {
                name: BuiltinStep::XcodeFirstLaunch,
                admin: true,
                unless: None,
            },
        ];
    }
    let (report, events) = harness.run(&mut plan(vec![entry]));
    assert_eq!(
        state(&report, "xcode"),
        &ItemState::NeedsYou { cause: None }
    );
    assert!(
        !harness
            .runner
            .calls()
            .iter()
            .any(|command| file_name(command) == "xcodebuild")
    );
    let discarded = events
        .iter()
        .position(|event| matches!(event.kind, EventKind::AdminDiscarded { .. }));
    let held = events
        .iter()
        .position(|event| matches!(event.kind, EventKind::AdminHeld { .. }));
    if let (Some(held), Some(discarded)) = (held, discarded) {
        assert!(discarded > held);
    }
}

// Review round 1. Each test asserts the design's behaviour and failed against a78447e.
mod review {
    use super::*;
    use soulcaster_core::config::SettingValue;
    use soulcaster_core::recipe::{GitKey as K, Store as S};

    pub(super) fn git_setting(id: &str, key: K, value: SettingValue) -> PlanEntry {
        let mut e = dock_setting(id, "x", None, EvidenceGroup::Documented);
        if let Recipe::Setting(s) = &mut e.recipe {
            s.stores = vec![S::GitConfig { key }];
        }
        e.value = Some(value);
        e
    }

    pub(super) fn download(id: &str) -> PlanEntry {
        let mut entry = formula(id, &[]);
        if let Recipe::Install(install) = &mut entry.recipe {
            install.checks = vec![Probe::ManagedTool {
                tool: id.into(),
                version: "1".into(),
                file: "tool".into(),
            }];
            install.steps = vec![Step::Download {
                tool: id.into(),
                version: "1".into(),
                url: "https://example.com/tool.zip".into(),
                sha256: "abc123".into(),
                archive: ArchiveKind::Zip,
                signed_by: "2ZEFAR8TH3".into(),
                signed_file: "tool".into(),
                links: vec![],
                unless: None,
            }];
        }
        entry
    }

    #[test]
    fn r01_git_boolean_setting_that_reads_back_true_is_satisfied() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "git" && args_contain(c, "--get"),
            [Reply::ok("true\n")],
        );
        let mut p = plan(vec![git_setting(
            "git.pull-rebase",
            K::PullRebase,
            SettingValue::Bool(true),
        )]);
        let (report, _) = h.run(&mut p);
        assert!(
            matches!(
                state(&report, "git.pull-rebase"),
                ItemState::Satisfied { .. }
            ),
            "got {:?}; git writes: {:?}",
            state(&report, "git.pull-rebase"),
            h.runner
                .calls()
                .iter()
                .filter(|c| file_name(c) == "git" && !args_contain(c, "--get"))
                .map(|c| c.args.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn r02_nothing_runs_the_git_stub_before_command_line_tools_exist() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "CommandLineTools"),
            [Reply::fails(1, "")],
        );
        let config = Config::parse(br#"{"schemaVersion":1,"name":"probe","items":["git-identity"],"settings":{"git.pull-rebase":true}}"#).unwrap();
        let mut p = Plan::new(&config, Catalog::shipped());
        let _ = h.run(&mut p);
        let calls = h.runner.calls();
        let first_git = calls
            .iter()
            .position(|c| c.program == Path::new("/usr/bin/git"));
        assert!(
            first_git.is_none(),
            "/usr/bin/git ran with CLT absent: {:?}",
            calls[first_git.unwrap()].args
        );
    }

    #[test]
    fn r03_dock_setting_is_written_on_a_first_run_where_dockutil_is_not_there_yet() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "dockutil" && args_contain(c, "--list"),
            [
                Reply::SpawnFails("No such file or directory".into()),
                Reply::ok("/Applications/Safari.app\n"),
            ],
        );
        let config = Config::parse(br#"{"schemaVersion":1,"name":"probe","settings":{"dock.apps":["/Applications/Safari.app"]}}"#).unwrap();
        let mut p = Plan::new(&config, Catalog::shipped());
        let (report, _) = h.run(&mut p);
        let wrote = h
            .runner
            .calls()
            .iter()
            .any(|c| file_name(c) == "dockutil" && args_contain(c, "--add"));
        assert!(
            wrote,
            "never wrote; state {:?}",
            state(&report, "dock.apps")
        );
    }

    #[test]
    fn r04_a_5xx_from_curl_is_a_network_failure_and_retries() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner.on(
            |c| file_name(c) == "curl",
            [Reply::fails(
                22,
                "curl: (22) The requested URL returned error: 503\n",
            )],
        );
        let (report, _) = h.run(&mut plan(vec![download("k")]));
        assert_eq!(
            state(&report, "k"),
            &ItemState::Failed {
                kind: FailureKind::Network
            }
        );
        assert_eq!(h.clock.waits().len(), 2);
    }

    #[test]
    fn r05_a_binary_that_merely_mentions_the_team_id_is_refused() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        // Ad-hoc signed by an attacker, with the expected team only in a name they chose.
        h.runner.on(
            |c| file_name(c) == "codesign",
            [Reply::Output {
                code: 0,
                stdout: String::new(),
                stderr: "Identifier=2ZEFAR8TH3\nSignature=adhoc\nTeamIdentifier=not set\n".into(),
            }],
        );
        let (report, _) = h.run(&mut plan(vec![download("k")]));
        assert_eq!(
            state(&report, "k"),
            &ItemState::Failed {
                kind: FailureKind::Signature
            }
        );
    }

    #[test]
    fn r06_signature_is_verified_not_just_displayed() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        h.runner.on(
            |c| file_name(c) == "codesign",
            [Reply::ok("TeamIdentifier=2ZEFAR8TH3\n")],
        );
        let _ = h.run(&mut plan(vec![download("k")]));
        let code = h.runner.calls();
        let c = code.iter().find(|c| file_name(c) == "codesign").unwrap();
        assert!(args_contain(c, "--verify"), "codesign args: {:?}", c.args);
    }

    #[test]
    fn r07_a_refused_download_never_lands_where_its_check_looks() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        h.runner.on(
            |c| file_name(c) == "codesign",
            [Reply::fails(1, "code object is not signed at all\n")],
        );
        let _ = h.run(&mut plan(vec![download("k")]));
        let calls = h.runner.calls();
        let ditto = calls.iter().find(|c| file_name(c) == "ditto").unwrap();
        let dest = ditto.args.last().unwrap();
        assert!(
            !dest.contains("/tools/k/1"),
            "extracted straight into the tools folder: {dest}"
        );
    }

    #[test]
    fn r08_a_formula_is_never_privileged_even_while_the_password_is_held() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let mut e = admin_builtin("mix", &[]);
        if let Recipe::Install(i) = &mut e.recipe {
            i.steps.insert(
                0,
                Step::BrewFormula {
                    name: "openjdk".into(),
                    unless: None,
                },
            );
        }
        let _ = h.run(&mut plan(vec![e]));
        let calls = h.runner.calls();
        let brew = calls.iter().find(|c| is_install(c, "openjdk")).unwrap();
        assert!(
            brew.env
                .iter()
                .any(|(k, v)| k == "HOMEBREW_NO_SUDO" && v == "1"),
            "{:?}",
            brew.env
        );
    }

    #[test]
    fn r09_run_started_is_the_first_event() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "defaults" && c.args.first().map(String::as_str) == Some("read"),
            [
                Reply::fails(
                    1,
                    "The domain/default pair of (com.apple.dock, x) does not exist\n",
                ),
                Reply::ok("1\n"),
            ],
        );
        let (_r, events) = h.run(&mut plan(vec![dock_setting(
            "dock.x",
            "x",
            None,
            EvidenceGroup::Documented,
        )]));
        assert!(
            matches!(events[0].kind, EventKind::RunStarted { .. }),
            "first event: {:?}",
            events[0].kind
        );
    }

    struct SharedSink(Arc<Mutex<Vec<Event>>>);
    impl EventSink for SharedSink {
        fn send(&mut self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }
    struct Live {
        inner: FakeRunner,
        seen: Arc<Mutex<Vec<Event>>>,
        live: AtomicBool,
    }
    impl CommandRunner for Live {
        fn run(
            &self,
            c: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<soulcaster_core::command::CommandOutput, soulcaster_core::command::SpawnError>
        {
            if is_install(c, "ripgrep") {
                on_line(OutputStream::Stderr, "==> Downloading first");
                std::thread::sleep(Duration::from_millis(250));
                on_line(OutputStream::Stderr, "==> Downloading second");
                std::thread::sleep(Duration::from_millis(250));
                let got = self.seen.lock().unwrap().iter().any(|e| matches!(&e.kind, EventKind::ItemLog { lines, .. } if lines.iter().any(|l| l.contains("first"))));
                self.live.store(got, Ordering::Relaxed);
            }
            self.inner.run(c, on_line)
        }
    }

    #[test]
    fn r10_log_lines_reach_the_stream_while_the_command_runs() {
        let h = Harness::new();
        h.script_machine();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Live {
            inner: FakeRunner::new(),
            seen: seen.clone(),
            live: AtomicBool::new(false),
        };
        runner
            .inner
            .on(|c| file_name(c) == "id", [Reply::ok("staff admin\n")]);
        runner
            .inner
            .on(|c| file_name(c) == "sw_vers", [Reply::ok("15.6.1\n")]);
        runner.inner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let mut sink = SharedSink(seen.clone());
        let mut p = plan(vec![formula("ripgrep", &[])]);
        perform(RunInput {
            dotfiles: None,
            admin: &OpenGate,
            plan: &mut p,
            runner: &runner,
            home: h.home.path(),
            events: &mut sink,
            asks: &h.asks,
            cancel: &h.cancel,
            clock: &h.clock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "t",
            engine_version: "t",
        })
        .unwrap();
        assert!(
            runner.live.load(Ordering::Relaxed),
            "no log line was delivered until the command ended"
        );
    }

    #[test]
    fn r11_mixed_streams_keep_their_own_labels() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| is_install(c, "ripgrep"),
            [Reply::Output {
                code: 0,
                stdout: "out-line\n".into(),
                stderr: "err-line\n".into(),
            }],
        );
        let (_r, events) = h.run(&mut plan(vec![formula("ripgrep", &[])]));
        for e in &events {
            if let EventKind::ItemLog { stream, lines, .. } = &e.kind
                && lines.iter().any(|l| l == "err-line")
            {
                assert_eq!(
                    *stream,
                    soulcaster_core::event::LogStream::Stderr,
                    "{lines:?}"
                );
            }
        }
    }

    #[test]
    fn r12_every_external_command_has_a_stall_limit() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let (_r, _e) = h.run(&mut plan(vec![admin_builtin("xc", &[])]));
        let calls = h.runner.calls();
        let xc = calls.iter().find(|c| file_name(c) == "xcodebuild").unwrap();
        assert!(
            xc.stall_limit.is_some(),
            "an admin step with no stall limit can hang the Run forever: {:?}",
            xc.args
        );
    }

    #[test]
    fn r13_a_package_approval_lets_that_package_run() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let mut rec = MachineRecord::empty();
        rec.approvals.packages.insert("ada/tap/maestro".into());
        rec.save(h.home.path()).unwrap();
        let (report, _) = h.run(&mut plan(vec![tap_formula("maestro", "ada/tap/maestro")]));
        assert!(
            matches!(state(&report, "maestro"), ItemState::Satisfied { .. }),
            "{:?}",
            state(&report, "maestro")
        );
    }

    #[test]
    fn r14_a_batched_setting_with_a_failed_requirement_is_blocked() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner
            .on(|c| is_install(c, "dockutil"), [Reply::fails(1, "boom")]);
        h.runner.on(
            |c| file_name(c) == "dockutil",
            [Reply::SpawnFails("No such file or directory".into())],
        );
        let config = Config::parse(br#"{"schemaVersion":1,"name":"probe","settings":{"dock.autohide":true,"dock.apps":["/Applications/Safari.app"]}}"#).unwrap();
        let mut p = Plan::new(&config, Catalog::shipped());
        let (report, _) = h.run(&mut p);
        assert!(
            matches!(state(&report, "dock.apps"), ItemState::Blocked { .. }),
            "dock.apps ended {:?}",
            state(&report, "dock.apps")
        );
    }

    #[test]
    fn r15_a_machine_with_nothing_to_do_as_admin_is_not_asked_for_a_password() {
        let h = Harness::new();
        let (_r, _e) = h.run(&mut plan(vec![formula("ripgrep", &[])]));
        assert_eq!(
            h.asks.asked(),
            0,
            "the map says the password is asked for only when the plan has admin work"
        );
    }

    #[test]
    fn r16_scoped_run_checks_a_requirement_even_if_the_record_says_it_was_fine() {
        let h = Harness::new();
        let mut rec = MachineRecord::empty();
        rec.outcomes.insert(
            "homebrew".into(),
            ItemOutcome {
                state: "satisfied".into(),
                failure: None,
                blocked_by: None,
                assurance: None,
                skip: None,
            },
        );
        rec.save(h.home.path()).unwrap();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        let mut p = plan(vec![
            formula("homebrew", &[]),
            formula("ripgrep", &["homebrew"]),
        ]);
        let (report, _) = h
            .run_scoped(
                &mut p,
                RunScope::Items {
                    items: vec!["ripgrep".into()],
                },
            )
            .unwrap();
        assert!(
            report.states.contains_key("homebrew"),
            "the record, not a check, decided homebrew was installed"
        );
    }

    #[test]
    fn r17_a_key_forced_by_a_profile_is_skipped_as_managed() {
        let h = Harness::new();
        h.runner.on(
            |c| args_contain(c, "Managed Preferences"),
            [Reply::ok("1\n")],
        );
        h.runner
            .on(|c| file_name(c) == "defaults", [Reply::ok("0\n")]);
        let (report, _) = h.run(&mut plan(vec![dock_setting(
            "dock.x",
            "x",
            None,
            EvidenceGroup::Documented,
        )]));
        assert_eq!(
            state(&report, "dock.x"),
            &ItemState::Skipped {
                reason: SkipReason::ManagedByAdmin
            }
        );
    }

    #[test]
    fn r18_dock_contents_are_compared_exactly_not_by_substring() {
        // Extra tiles: a Dock with more apps than the Config is not a match either.
        let h2 = Harness::new();
        h2.runner.on(|c| file_name(c) == "dockutil" && args_contain(c, "--list"),
            [Reply::ok("Safari\tfile:///Applications/Safari.app/\nMail\tfile:///System/Applications/Mail.app/\n")]);
        let config = Config::parse(br#"{"schemaVersion":1,"name":"probe","settings":{"dock.apps":["/Applications/Safari.app"]}}"#).unwrap();
        let mut p = Plan::new(&config, Catalog::shipped());
        let _ = h2.run(&mut p);
        let wrote = h2
            .runner
            .calls()
            .iter()
            .any(|c| file_name(c) == "dockutil" && args_contain(c, "--add"));
        assert!(wrote, "a Dock with an extra tile compared as a match");
    }

    #[test]
    fn r19_kept_logs_exist_for_a_failed_item() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner.on(
            |c| is_install(c, "ripgrep"),
            [Reply::fails(1, "Error: boom\n")],
        );
        let (report, _) = h.run(&mut plan(vec![formula("ripgrep", &[])]));
        let log = h
            .home
            .path()
            .join("Library/Application Support/Soulcaster/runs")
            .join(&report.run_id)
            .join("logs/ripgrep.log");
        assert!(log.exists(), "missing {}", log.display());
    }
}

mod round_two {
    use super::review::download;
    use super::*;
    use soulcaster_core::command::{CommandOutput, SpawnError};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct RecordingSink(Arc<Mutex<Vec<Event>>>);

    impl EventSink for RecordingSink {
        fn send(&mut self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    /// Writes a burst during the install, goes quiet, and notes what the stream had by then.
    struct Burst {
        inner: FakeRunner,
        seen: Arc<Mutex<Vec<Event>>>,
        last_line_arrived: AtomicBool,
    }

    impl CommandRunner for Burst {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<CommandOutput, SpawnError> {
            if is_install(command, "ripgrep") {
                on_line(OutputStream::Stderr, "==> one");
                on_line(OutputStream::Stderr, "==> two");
                on_line(OutputStream::Stderr, "==> last before the quiet");
                std::thread::sleep(Duration::from_millis(500));
                let arrived = self.seen.lock().unwrap().iter().any(|event| {
                    matches!(&event.kind, EventKind::ItemLog { lines, .. }
                        if lines.iter().any(|line| line.contains("last before")))
                });
                self.last_line_arrived.store(arrived, Ordering::Relaxed);
            }
            self.inner.run(command, on_line)
        }
    }

    #[test]
    fn the_last_lines_before_a_quiet_stretch_reach_the_stream_without_another_line() {
        let h = Harness::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let runner = Burst {
            inner: FakeRunner::new(),
            seen: seen.clone(),
            last_line_arrived: AtomicBool::new(false),
        };
        runner
            .inner
            .on(|c| file_name(c) == "id", [Reply::ok("staff admin\n")]);
        runner
            .inner
            .on(|c| file_name(c) == "sw_vers", [Reply::ok("15.6.1\n")]);
        runner.inner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let mut sink = RecordingSink(seen);
        let mut p = plan(vec![formula("ripgrep", &[])]);
        perform(RunInput {
            dotfiles: None,
            admin: &OpenGate,
            plan: &mut p,
            runner: &runner,
            home: h.home.path(),
            events: &mut sink,
            asks: &h.asks,
            cancel: &h.cancel,
            clock: &h.clock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "t",
            engine_version: "t",
        })
        .unwrap();
        assert!(
            runner.last_line_arrived.load(Ordering::Relaxed),
            "lines sat in the batch through a quiet stretch"
        );
    }

    /// One install that writes `lines` lines of `width` bytes.
    struct Noisy {
        inner: FakeRunner,
        lines: usize,
        width: usize,
    }

    impl CommandRunner for Noisy {
        fn run(
            &self,
            command: &Command,
            on_line: &mut dyn FnMut(OutputStream, &str),
        ) -> Result<CommandOutput, SpawnError> {
            if is_install(command, "ripgrep") {
                let line = "x".repeat(self.width);
                for _ in 0..self.lines {
                    on_line(OutputStream::Stdout, &line);
                }
            }
            self.inner.run(command, on_line)
        }
    }

    fn noisy_run(
        h: &Harness,
        lines: usize,
        width: usize,
    ) -> (soulcaster_engine::RunReport, Vec<Event>) {
        let runner = Noisy {
            inner: FakeRunner::new(),
            lines,
            width,
        };
        runner
            .inner
            .on(|c| file_name(c) == "id", [Reply::ok("staff admin\n")]);
        runner
            .inner
            .on(|c| file_name(c) == "sw_vers", [Reply::ok("15.6.1\n")]);
        runner.inner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let mut events = Vec::new();
        let mut p = plan(vec![formula("ripgrep", &[])]);
        let report = perform(RunInput {
            dotfiles: None,
            admin: &OpenGate,
            plan: &mut p,
            runner: &runner,
            home: h.home.path(),
            events: &mut events,
            asks: &h.asks,
            cancel: &h.cancel,
            clock: &h.clock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "t",
            engine_version: "t",
        })
        .unwrap();
        (report, events)
    }

    #[test]
    fn the_ten_megabyte_cap_stops_the_log_file_and_the_event_stream() {
        let h = Harness::new();
        // 20 MB of output, against a 10 MB cap.
        let (report, events) = noisy_run(&h, 2_000, 10_000);
        let run_dir = h
            .home
            .path()
            .join("Library/Application Support/Soulcaster/runs")
            .join(&report.run_id);
        let file = std::fs::metadata(run_dir.join("logs/ripgrep.log")).unwrap();
        assert!(file.len() <= 10 * 1024 * 1024 + 10_001, "{}", file.len());
        let logged: usize = events
            .iter()
            .map(|event| match &event.kind {
                EventKind::ItemLog { lines, .. } => lines.iter().map(String::len).sum(),
                _ => 0,
            })
            .sum();
        assert!(logged <= 10 * 1024 * 1024 + 10_001, "{logged}");
    }

    #[test]
    fn the_kept_files_are_private_and_events_ndjson_holds_only_lossless_events() {
        let h = Harness::new();
        let (report, _) = noisy_run(&h, 3, 10);
        let support = h.home.path().join("Library/Application Support/Soulcaster");
        let run_dir = support.join("runs").join(&report.run_id);
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&support), 0o700);
        assert_eq!(mode(&run_dir.join("logs/ripgrep.log")), 0o600);
        assert_eq!(mode(&run_dir.join("events.ndjson")), 0o600);
        let events = std::fs::read_to_string(run_dir.join("events.ndjson")).unwrap();
        assert!(
            events.contains("run.finished") || events.contains("RunFinished"),
            "{events}"
        );
        assert!(
            !events.contains("xxxxxxxxxx"),
            "a log line reached events.ndjson"
        );
    }

    #[test]
    fn a_stale_staging_folder_and_an_existing_tools_folder_are_replaced() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        h.runner.on(|c| file_name(c) == "codesign", [Reply::ok("")]);
        let _ = h.run(&mut plan(vec![download("k")]));
        let calls = h.runner.calls();
        let index_of = |wanted: &dyn Fn(&Command) -> bool| calls.iter().position(wanted);
        let clear_staging =
            index_of(&|c| file_name(c) == "rm" && c.args.iter().any(|a| a.contains("/staging/k/")));
        let extract = index_of(&|c| file_name(c) == "ditto");
        let verify = index_of(&|c| file_name(c) == "codesign");
        let clear_tools =
            index_of(&|c| file_name(c) == "rm" && c.args.iter().any(|a| a.contains("/tools/k/")));
        let swap = index_of(&|c| file_name(c) == "mv");
        assert!(clear_staging.unwrap() < extract.unwrap());
        assert!(extract.unwrap() < verify.unwrap());
        assert!(verify.unwrap() < clear_tools.unwrap());
        assert!(clear_tools.unwrap() < swap.unwrap());
    }

    #[test]
    fn a_refused_signer_fails_as_signature_and_nothing_is_swapped_in() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        h.runner.on(
            |c| file_name(c) == "codesign",
            [Reply::fails(
                3,
                "code failed to satisfy specified code requirement(s)\n",
            )],
        );
        let (report, _) = h.run(&mut plan(vec![download("k")]));
        assert_eq!(
            state(&report, "k"),
            &ItemState::Failed {
                kind: FailureKind::Signature
            }
        );
        let calls = h.runner.calls();
        assert!(!calls.iter().any(|c| file_name(c) == "mv"));
        assert!(
            !calls.iter().any(|c| {
                file_name(c) == "rm" && c.args.iter().any(|a| a.contains("/tools/k/"))
            })
        );
    }
}

mod admin_stage {
    use super::review::download;
    use super::*;
    use soulcaster_core::recipe::Bootstrap;
    use soulcaster_engine::{AdminWindow, Budget, Validation};

    /// A gate with a script of answers, that records what it was asked for and when.
    #[derive(Default)]
    struct WatchedGate {
        answers: Mutex<Vec<Validation>>,
        budgets: Mutex<Vec<Budget>>,
        forgotten: AtomicUsize,
        refuse_open: AtomicBool,
        /// `forget`'s answers in order. Empty means it works.
        forget_results: Mutex<Vec<bool>>,
    }

    impl soulcaster_engine::AdminGate for WatchedGate {
        fn validate(
            &self,
            _runner: &dyn CommandRunner,
            _home: &Path,
            _password: &Password,
        ) -> Validation {
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                Validation::Accepted
            } else {
                answers.remove(0)
            }
        }

        fn open(
            &self,
            _password: &Password,
            budget: Budget,
        ) -> Result<Box<dyn AdminWindow>, String> {
            if self.refuse_open.load(Ordering::Relaxed) {
                return Err("no helper".to_string());
            }
            self.budgets.lock().unwrap().push(budget);
            Ok(Box::new(OpenWindow))
        }

        fn forget(&self, _runner: &dyn CommandRunner, _home: &Path) -> bool {
            self.forgotten.fetch_add(1, Ordering::Relaxed);
            let mut results = self.forget_results.lock().unwrap();
            if results.is_empty() {
                true
            } else {
                results.remove(0)
            }
        }
    }

    #[derive(Default)]
    struct AsksInOrder {
        wrong_flags: Mutex<Vec<bool>>,
        codes: Mutex<Vec<(String, String)>>,
        statuses: Mutex<Vec<SignInStatus>>,
    }

    impl Asks for AsksInOrder {
        fn password(&self, request: &PasswordRequest) -> Option<Password> {
            self.wrong_flags
                .lock()
                .unwrap()
                .push(request.last_was_wrong);
            Some(Password::new(SECRET.to_string()))
        }

        fn show_github_code(&self, code: &str, url: &str) {
            self.codes
                .lock()
                .unwrap()
                .push((code.to_string(), url.to_string()));
        }

        fn github_status(&self, status: SignInStatus) {
            self.statuses.lock().unwrap().push(status);
        }
    }

    fn perform_with(
        h: &Harness,
        plan: &mut Plan,
        gate: &WatchedGate,
        asks: &AsksInOrder,
    ) -> (RunReport, Vec<Event>) {
        h.script_machine();
        let mut events = Vec::new();
        let report = perform(RunInput {
            dotfiles: None,
            admin: gate,
            plan,
            runner: &h.runner,
            home: h.home.path(),
            events: &mut events,
            asks,
            cancel: &h.cancel,
            clock: &h.clock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "t",
            engine_version: "t",
        })
        .unwrap();
        (report, events)
    }

    fn admin_item(id: &str) -> PlanEntry {
        let mut entry = formula(id, &[]);
        if let Recipe::Install(install) = &mut entry.recipe {
            install.steps = vec![Step::Builtin {
                name: BuiltinStep::XcodeFirstLaunch,
                admin: true,
                unless: None,
            }];
        }
        entry
    }

    fn base_gh() -> PlanEntry {
        let catalog = Catalog::shipped();
        let config = Config::parse(br#"{"schemaVersion":1,"name":"x"}"#).unwrap();
        let plan = Plan::new(&config, catalog);
        plan.entries
            .iter()
            .find(|entry| entry.id == "gh")
            .unwrap()
            .clone()
    }

    fn bootstrap_of(entry: &PlanEntry) -> Bootstrap {
        match &entry.recipe {
            Recipe::Install(install) => install.bootstrap.clone().unwrap(),
            _ => panic!("gh is an install"),
        }
    }

    fn program_is(command: &Command, name: &str) -> bool {
        file_name(command) == name
    }

    #[test]
    fn a_wrong_password_asks_again_without_starting_the_run() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let gate = WatchedGate::default();
        gate.answers
            .lock()
            .unwrap()
            .extend([Validation::Wrong, Validation::Accepted]);
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("xc")]);
        let (_, events) = perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(*asks.wrong_flags.lock().unwrap(), [false, true]);
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::AdminHeld { .. }))
        );
    }

    #[test]
    fn three_wrong_passwords_stop_asking_and_the_admin_work_goes_to_the_queue() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        let gate = WatchedGate::default();
        gate.answers.lock().unwrap().extend([
            Validation::Wrong,
            Validation::Wrong,
            Validation::Wrong,
        ]);
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("xc")]);
        let (report, events) = perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(asks.wrong_flags.lock().unwrap().len(), 3);
        assert!(gate.budgets.lock().unwrap().is_empty());
        assert!(matches!(state(&report, "xc"), ItemState::NeedsYou { .. }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.kind, EventKind::AdminHeld { .. }))
        );
    }

    #[test]
    fn sudos_timestamp_ends_after_checking_after_each_step_and_at_the_discard() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("xc")]);
        perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(gate.forgotten.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_timestamp_that_wont_end_after_a_step_stops_the_admin_work() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::fails(1, ""), Reply::ok("")],
        );
        let gate = WatchedGate::default();
        // After checking it works, after the first step it doesn't.
        gate.forget_results.lock().unwrap().extend([true, false]);
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("first"), admin_item("second")]);
        let (report, events) = perform_with(&h, &mut p, &gate, &asks);
        assert!(matches!(
            state(&report, "second"),
            ItemState::NeedsYou { .. }
        ));
        assert_eq!(gate.budgets.lock().unwrap().len(), 1);
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::AdminDiscarded { why } if why.contains("couldn't be ended")
        )));
    }

    #[test]
    fn a_run_that_never_held_the_password_has_no_timestamp_to_end() {
        let h = Harness::new();
        absent_then_present(&h, "Cellar/ripgrep");
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![formula("ripgrep", &[])]);
        perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(gate.forgotten.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_admin_step_gets_one_answer_and_an_admin_cask_as_many_as_it_needs() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::fails(1, ""), Reply::ok("")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut cask = formula("cask", &[]);
        if let Recipe::Install(install) = &mut cask.recipe {
            install.steps = vec![Step::BrewCask {
                name: "tailscale-app".to_string(),
                admin: true,
                unless: None,
            }];
        }
        let mut p = plan(vec![admin_item("xc"), cask]);
        perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(
            *gate.budgets.lock().unwrap(),
            [Budget::One, Budget::AsNeeded]
        );
    }

    #[test]
    fn nothing_but_an_admin_step_carries_the_helper() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("xc"), formula("ripgrep", &[])]);
        perform_with(&h, &mut p, &gate, &asks);
        for command in h.runner.calls() {
            let helper = command.env.iter().any(|(key, _)| key == "SUDO_ASKPASS");
            assert_eq!(helper, program_is(&command, "xcodebuild"), "{command:?}");
        }
    }

    #[test]
    fn a_helper_that_wont_open_sends_the_item_to_the_queue() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        let gate = WatchedGate::default();
        gate.refuse_open.store(true, Ordering::Relaxed);
        let asks = AsksInOrder::default();
        let mut p = plan(vec![admin_item("xc")]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(matches!(state(&report, "xc"), ItemState::NeedsYou { .. }));
        assert!(!h.runner.calls().iter().any(|c| program_is(c, "xcodebuild")));
    }

    fn command_line_tools() -> PlanEntry {
        let mut entry = formula("command-line-tools", &[]);
        if let Recipe::Install(install) = &mut entry.recipe {
            install.steps = vec![Step::Builtin {
                name: BuiltinStep::CommandLineTools,
                admin: true,
                unless: None,
            }];
        }
        entry
    }

    #[test]
    fn command_line_tools_install_the_newest_listed_release_then_clear_the_placeholder() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| file_name(c) == "softwareupdate" && args_contain(c, "-l"),
            [Reply::ok(
                "Software Update found the following:\n\
                 * Label: Command Line Tools for Xcode-15.3\n\
                 \tTitle: Command Line Tools\n\
                 * Label: Command Line Tools for Xcode-16.4\n\
                 * Label: Command Line Tools beta 3 for Xcode-26.0\n\
                 * Label: macOS Tahoe 26.1\n",
            )],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![command_line_tools()]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(matches!(
            state(&report, "command-line-tools"),
            ItemState::Satisfied { .. }
        ));
        let calls = h.runner.calls();
        let install = calls
            .iter()
            .find(|c| program_is(c, "softwareupdate") && args_contain(c, "-i"))
            .unwrap();
        assert_eq!(install.args, ["-i", "Command Line Tools for Xcode-16.4"]);
        assert!(install.env.iter().any(|(key, _)| key == "SUDO_ASKPASS"));
        let at = |name: &str| calls.iter().position(|c| program_is(c, name)).unwrap();
        assert!(at("touch") < at("softwareupdate"));
        assert!(calls.iter().rposition(|c| program_is(c, "rm")).unwrap() > at("softwareupdate"));
        assert!(!calls.iter().any(|c| program_is(c, "xcode-select")));
    }

    #[test]
    fn when_the_quiet_route_fails_apples_dialog_opens_and_the_item_waits_in_the_queue() {
        let h = Harness::new();
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        h.runner.on(
            |c| file_name(c) == "softwareupdate",
            [Reply::ok("No new software available.\n")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![command_line_tools()]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(matches!(
            state(&report, "command-line-tools"),
            ItemState::NeedsYou { .. }
        ));
        let calls = h.runner.calls();
        assert!(calls.iter().any(|c| program_is(c, "xcode-select")));
        assert!(
            calls.iter().any(|c| program_is(c, "rm")),
            "placeholder left"
        );
    }

    fn gh_absent(h: &Harness) {
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
    }

    fn sign_in_output() -> Reply {
        Reply::ok(
            "! First copy your one-time code: ABCD-1234\n\
             Open this URL to continue in your web browser: https://evil.example/login\n",
        )
    }

    fn is_gh(command: &Command) -> bool {
        file_name(command) == "gh"
    }

    fn scripted_gh(h: &Harness, statuses: Vec<Reply>, login: Vec<Reply>) {
        h.runner
            .on(|c| is_gh(c) && args_contain(c, "status"), statuses);
        h.runner.on(|c| is_gh(c) && args_contain(c, "login"), login);
    }

    #[test]
    fn a_bare_mac_fetches_the_pinned_gh_and_signs_in_with_nothing_on_path() {
        let h = Harness::new();
        gh_absent(&h);
        let gh = base_gh();
        let pinned = bootstrap_of(&gh);
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok(format!("{}  gh.zip\n", pinned.sha256))],
        );
        h.runner.on(|c| file_name(c) == "codesign", [Reply::ok("")]);
        scripted_gh(
            &h,
            vec![
                Reply::fails(1, "not logged in"),
                Reply::ok("Logged in (keyring)\n"),
            ],
            vec![sign_in_output()],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        perform_with(&h, &mut p, &gate, &asks);

        // The code reaches the person. The address is GitHub's, whatever the tool printed.
        assert_eq!(
            *asks.codes.lock().unwrap(),
            [(
                "ABCD-1234".to_string(),
                "https://github.com/login/device".to_string()
            )]
        );
        assert_eq!(
            *asks.statuses.lock().unwrap(),
            [SignInStatus::Waiting, SignInStatus::SignedIn]
        );
        let calls = h.runner.calls();
        let curl = calls.iter().find(|c| program_is(c, "curl")).unwrap();
        assert!(args_contain(curl, &pinned.url));
        let verify = calls.iter().find(|c| program_is(c, "codesign")).unwrap();
        assert!(args_contain(verify, &pinned.signed_by));
        let login = calls
            .iter()
            .find(|c| is_gh(c) && args_contain(c, "login"))
            .unwrap();
        for flag in ["--web", "--clipboard=false", "--skip-ssh-key"] {
            assert!(login.args.iter().any(|arg| arg == flag), "{flag}");
        }
        let path = &login.env.iter().find(|(key, _)| key == "PATH").unwrap().1;
        assert!(path.ends_with("empty-path"), "{path}");
        assert!(
            login
                .env
                .iter()
                .any(|(k, v)| k == "GH_TELEMETRY" && v == "false")
        );
        // Sign-in comes before the bootstrap copy is used for anything, and never runs git.
        assert!(!calls.iter().any(|c| program_is(c, "git")));
    }

    #[test]
    fn a_failed_sign_in_goes_to_the_queue_and_the_run_carries_on() {
        let h = Harness::new();
        gh_absent(&h);
        let gh = base_gh();
        let pinned = bootstrap_of(&gh);
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok(format!("{}  gh.zip\n", pinned.sha256))],
        );
        h.runner.on(|c| file_name(c) == "codesign", [Reply::ok("")]);
        scripted_gh(
            &h,
            vec![Reply::fails(1, "not logged in")],
            vec![Reply::fails(1, "denied")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert_eq!(
            *asks.statuses.lock().unwrap(),
            [SignInStatus::Waiting, SignInStatus::Failed]
        );
        assert!(
            report
                .queue
                .iter()
                .any(|entry| entry.id.contains("sign-in"))
        );
    }

    #[test]
    fn a_refused_signature_means_no_sign_in_and_no_copy_of_gh_in_place() {
        let h = Harness::new();
        gh_absent(&h);
        let gh = base_gh();
        let pinned = bootstrap_of(&gh);
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok(format!("{}  gh.zip\n", pinned.sha256))],
        );
        h.runner.on(
            |c| file_name(c) == "codesign",
            [Reply::fails(3, "failed to satisfy")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        perform_with(&h, &mut p, &gate, &asks);
        let calls = h.runner.calls();
        assert!(!calls.iter().any(is_gh));
        assert!(!calls.iter().any(|c| program_is(c, "mv")));
        assert!(asks.codes.lock().unwrap().is_empty());
    }

    #[test]
    fn homebrews_gh_already_here_means_no_bootstrap_and_no_sign_in() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "/opt/homebrew/bin/gh"),
            [Reply::ok("")],
        );
        h.runner
            .on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![base_gh()]);
        perform_with(&h, &mut p, &gate, &asks);
        assert!(asks.statuses.lock().unwrap().is_empty());
    }

    fn make_bootstrap_copy(h: &Harness, entry: &PlanEntry) -> PathBuf {
        let pinned = bootstrap_of(entry);
        let folder = h
            .home
            .path()
            .join("Library/Application Support/Soulcaster/bootstrap-gh")
            .join(&pinned.version);
        let binary = folder.join(&pinned.binary);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "").unwrap();
        binary
    }

    /// The hand-off's reads, by which `gh` is asking.
    fn accounts(h: &Harness, early: &str, homebrew: &str) {
        let (early, homebrew) = (early.to_string(), homebrew.to_string());
        h.runner.on(
            |c| c.program.starts_with("/opt/homebrew") && is_gh(c) && args_contain(c, "user"),
            vec![Reply::ok(format!("{homebrew}\n")); 4],
        );
        h.runner.on(
            |c| is_gh(c) && args_contain(c, "user"),
            vec![Reply::ok(format!("{early}\n")); 4],
        );
    }

    fn hand_off_only(h: &Harness) -> PlanEntry {
        // Homebrew's gh is in, the copy is still there, so only the hand-off is left to do.
        let mut gh = base_gh();
        if let Recipe::Install(install) = &mut gh.recipe {
            install.requires.clear();
            install.steps = vec![Step::Builtin {
                name: BuiltinStep::GhHandOff,
                admin: false,
                unless: None,
            }];
        }
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "/opt/homebrew/bin/gh"),
            vec![Reply::ok(""); 8],
        );
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "bootstrap-gh"),
            [Reply::fails(1, ""), Reply::ok("")],
        );
        gh
    }

    #[test]
    fn the_hand_off_leaves_git_to_the_person_and_removes_the_copy() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        let copy = make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone");
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, events) = perform_with(&h, &mut p, &gate, &asks);
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git")),
            "the Run wrote git's credential settings"
        );
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::RunWarning { text } if text.contains("gh auth setup-git")
        )));
        assert!(matches!(state(&report, "gh"), ItemState::Satisfied { .. }));
        assert!(!copy.exists(), "the bootstrap copy is still there");
    }

    #[test]
    fn a_different_account_stops_the_hand_off_and_keeps_the_copy() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        let copy = make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone-else");
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(matches!(state(&report, "gh"), ItemState::Failed { .. }));
        assert!(copy.exists());
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git"))
        );
    }

    #[test]
    fn nobody_signed_in_lets_the_hand_off_finish_and_sign_in_waits() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        let copy = make_bootstrap_copy(&h, &gh);
        h.runner.on(
            |c| is_gh(c) && !c.program.starts_with("/opt/homebrew") && args_contain(c, "status"),
            [Reply::fails(1, "not logged in")],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(!copy.exists());
        assert!(!matches!(state(&report, "gh"), ItemState::Failed { .. }));
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git"))
        );
    }

    /// A real sign-in may still be in that copy. Only `gh` saying nobody is signed in lets it go.
    fn hand_off_keeps_the_copy(reply: Reply, signature: Reply) {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        let copy = make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone");
        h.runner.on(
            |c| is_gh(c) && !c.program.starts_with("/opt/homebrew") && args_contain(c, "status"),
            [reply],
        );
        h.runner.on(|c| file_name(c) == "codesign", [signature]);
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        assert!(copy.exists(), "the copy was deleted");
        assert!(matches!(state(&report, "gh"), ItemState::Failed { .. }));
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git"))
        );
    }

    #[test]
    fn a_stalled_status_isnt_signed_out() {
        hand_off_keeps_the_copy(Reply::Stall, Reply::ok(""));
    }

    #[test]
    fn being_offline_isnt_signed_out() {
        hand_off_keeps_the_copy(
            Reply::fails(1, "error connecting to api.github.com"),
            Reply::ok(""),
        );
    }

    #[test]
    fn a_token_github_refuses_isnt_signed_out() {
        hand_off_keeps_the_copy(
            Reply::fails(
                1,
                "Failed to log in to github.com account someone (keyring)\n- The token in keyring is invalid.",
            ),
            Reply::ok(""),
        );
    }

    #[test]
    fn a_status_that_wont_start_isnt_signed_out() {
        hand_off_keeps_the_copy(Reply::SpawnFails("no such file".to_string()), Reply::ok(""));
    }

    #[test]
    fn a_copy_that_fails_its_pin_isnt_signed_out() {
        hand_off_keeps_the_copy(Reply::ok(""), Reply::fails(3, "failed to satisfy"));
    }

    #[test]
    fn a_waiting_sign_in_finishes_with_homebrews_gh_on_a_later_run() {
        let h = Harness::new();
        let gh = base_gh();
        let mut record = MachineRecord::empty();
        record.queue.push(soulcaster_engine::QueueItem {
            id: "gh:sign-in".to_string(),
            item: "gh".to_string(),
            step_id: "sign-in".to_string(),
            kind: QueueKind::Person,
            text: "Sign in".to_string(),
            link: None,
            opened_at: 1,
            finished_at: None,
            finished_by: None,
            needs_password: false,
        });
        record.save(h.home.path()).unwrap();
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "/opt/homebrew/bin/gh"),
            vec![Reply::ok(""); 4],
        );
        h.runner.on(
            |c| is_gh(c) && args_contain(c, "status"),
            [
                Reply::fails(1, "not logged in"),
                Reply::ok("Logged in (keyring)\n"),
            ],
        );
        h.runner
            .on(|c| is_gh(c) && args_contain(c, "login"), [sign_in_output()]);
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (report, _) = perform_with(&h, &mut p, &gate, &asks);
        let login = h
            .runner
            .calls()
            .into_iter()
            .find(|c| is_gh(c) && args_contain(c, "login"))
            .expect("sign-in ran");
        assert_eq!(login.program, Path::new("/opt/homebrew/bin/gh"));
        assert!(!report.queue.iter().any(|entry| entry.id == "gh:sign-in"));
    }

    #[test]
    fn a_linked_gitconfig_is_left_alone_and_the_report_says_so() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone");
        std::os::unix::fs::symlink("/nowhere", h.home.path().join(".gitconfig")).unwrap();
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        let (_, events) = perform_with(&h, &mut p, &gate, &asks);
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git"))
        );
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::RunWarning { text } if text.contains("gh auth setup-git")
        )));
    }

    #[test]
    fn another_git_helper_for_github_is_left_alone() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone");
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "CommandLineTools"),
            vec![Reply::ok(""); 4],
        );
        fs::write(
            h.home.path().join(".gitconfig"),
            "[credential \"https://github.com\"]
	helper = !/usr/local/bin/other-helper
",
        )
        .unwrap();
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        perform_with(&h, &mut p, &gate, &asks);
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| is_gh(c) && args_contain(c, "setup-git"))
        );
    }

    #[test]
    fn the_stub_never_runs_during_the_hand_off_on_a_bare_mac() {
        let h = Harness::new();
        let gh = hand_off_only(&h);
        make_bootstrap_copy(&h, &gh);
        accounts(&h, "someone", "someone");
        h.runner.on(
            |c| file_name(c) == "test" && args_contain(c, "CommandLineTools"),
            vec![Reply::fails(1, ""); 4],
        );
        let gate = WatchedGate::default();
        let asks = AsksInOrder::default();
        let mut p = plan(vec![gh]);
        perform_with(&h, &mut p, &gate, &asks);
        assert!(!h.runner.calls().iter().any(|c| file_name(c) == "git"));
    }

    #[test]
    fn a_download_item_still_verifies_and_swaps_after_the_refactor() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        h.runner.on(
            |c| file_name(c) == "shasum",
            [Reply::ok("abc123  tool.zip\n")],
        );
        h.runner.on(|c| file_name(c) == "codesign", [Reply::ok("")]);
        let (report, _) = h.run(&mut plan(vec![download("k")]));
        assert!(matches!(state(&report, "k"), ItemState::Satisfied { .. }));
    }
}

#[test]
fn a_duplicate_key_in_the_machine_record_is_a_corrupt_file() {
    let harness = Harness::new();
    let path = machine_record_path(harness.home.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, br#"{"schemaVersion":1,"schemaVersion":1}"#).unwrap();
    assert!(MachineRecord::load(harness.home.path()).is_err());
}

mod dotfiles_in_run {
    use super::review::git_setting;
    use super::*;
    use soulcaster_core::approval::{ApprovalStore, CommitSha};
    use soulcaster_core::config::SettingValue;
    use soulcaster_core::plan::DotfilesPlan;
    use soulcaster_engine::{DotfilesHook, DotfilesResult, RecordApprovals};

    /// Notes what the Run had done when it was called, and can link `~/.gitconfig` away.
    struct Watching<'a> {
        runner: &'a FakeRunner,
        home: PathBuf,
        link_gitconfig: bool,
        calls: Mutex<Vec<(usize, Vec<String>)>>,
    }

    impl DotfilesHook for Watching<'_> {
        fn apply(&self, _plan: &DotfilesPlan, approvals: &dyn ApprovalStore) -> DotfilesResult {
            let installed: Vec<String> = self
                .runner
                .calls()
                .iter()
                .filter(|c| file_name(c) == "brew" && args_contain(c, "install"))
                .filter_map(|c| c.args.last().cloned())
                .collect();
            self.calls
                .lock()
                .unwrap()
                .push((self.runner.calls().len(), installed));
            if self.link_gitconfig {
                std::os::unix::fs::symlink("/nowhere/gitconfig", self.home.join(".gitconfig"))
                    .unwrap();
            }
            let mut held = approvals.read().unwrap();
            held.taps.insert("written/by-the-step".into());
            approvals.write(&held).unwrap();
            DotfilesResult {
                warnings: vec!["~/.zshrc wasn't linked: a test said so".into()],
            }
        }
    }

    fn dotfiles_plan() -> DotfilesPlan {
        DotfilesPlan {
            repo: "example-you/dotfiles".into(),
            links: BTreeMap::from([("~/.zshrc".into(), "zsh/zshrc".into())]),
            refused_links: vec![],
            install_script: None,
        }
    }

    fn run_with(h: &Harness, p: &mut Plan, hook: &dyn DotfilesHook) -> (RunReport, Vec<Event>) {
        h.script_machine();
        let mut events = Vec::new();
        let report = perform(RunInput {
            dotfiles: Some(hook),
            admin: &OpenGate,
            plan: p,
            runner: &h.runner,
            home: h.home.path(),
            events: &mut events,
            asks: &h.asks,
            cancel: &h.cancel,
            clock: &h.clock,
            scope: RunScope::Full,
            config: ConfigRef::default(),
            catalog_version: "t",
            engine_version: "t",
        })
        .unwrap();
        (report, events)
    }

    fn grouped(mut entry: PlanEntry, group: RunGroup) -> PlanEntry {
        entry.group = group;
        entry
    }

    #[test]
    fn the_dotfiles_go_after_third_party_items_and_before_git_settings_once() {
        let h = Harness::new();
        absent_then_present(&h, "Cellar/");
        let hook = Watching {
            runner: &h.runner,
            home: h.home.path().to_path_buf(),
            link_gitconfig: false,
            calls: Mutex::new(vec![]),
        };
        let mut p = plan(vec![
            grouped(formula("third", &[]), RunGroup::ThirdParty),
            grouped(
                git_setting(
                    "git.name",
                    GitKey::UserName,
                    SettingValue::String("Ada".into()),
                ),
                RunGroup::GitValues,
            ),
        ]);
        p.dotfiles = Some(dotfiles_plan());
        let (_, events) = run_with(&h, &mut p, &hook);
        let calls = hook.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "called {} times", calls.len());
        assert_eq!(calls[0].1, ["third"], "the third-party Item wasn't first");
        let git_after = h.runner.calls()[calls[0].0..]
            .iter()
            .any(|c| file_name(c) == "git");
        assert!(git_after, "git settings didn't come after the dotfiles");
        assert!(events.iter().any(|e| matches!(
            &e.kind,
            EventKind::RunWarning { text } if text.contains("wasn't linked")
        )));
        assert!(!events.iter().any(|e| matches!(
            &e.kind,
            EventKind::RunWarning { text } if text.contains("not applied by this Run")
        )));
    }

    #[test]
    fn the_dotfiles_never_run_while_the_password_is_held() {
        let h = Harness::new();
        h.runner.on(
            |c| file_name(c) == "test",
            [Reply::fails(1, ""), Reply::ok("")],
        );
        let hook = Watching {
            runner: &h.runner,
            home: h.home.path().to_path_buf(),
            link_gitconfig: false,
            calls: Mutex::new(vec![]),
        };
        let mut admin = formula("adm", &[]);
        if let Recipe::Install(install) = &mut admin.recipe {
            install.steps = vec![Step::Builtin {
                name: BuiltinStep::XcodeFirstLaunch,
                admin: true,
                unless: None,
            }];
        }
        let mut p = plan(vec![admin]);
        p.dotfiles = Some(dotfiles_plan());
        let (_, events) = run_with(&h, &mut p, &hook);
        assert_eq!(hook.calls.lock().unwrap().len(), 1);
        let discarded = events
            .iter()
            .position(|e| matches!(e.kind, EventKind::AdminDiscarded { .. }))
            .expect("the password was held and discarded");
        let warned = events
            .iter()
            .position(|e| matches!(&e.kind, EventKind::RunWarning { text } if text.contains("wasn't linked")))
            .unwrap();
        assert!(
            warned > discarded,
            "the dotfiles ran while the password was held"
        );
    }

    #[test]
    fn a_gitconfig_linked_by_the_dotfiles_isnt_written_through() {
        let h = Harness::new();
        let hook = Watching {
            runner: &h.runner,
            home: h.home.path().to_path_buf(),
            link_gitconfig: true,
            calls: Mutex::new(vec![]),
        };
        let mut p = plan(vec![grouped(
            git_setting(
                "git.name",
                GitKey::UserName,
                SettingValue::String("Ada".into()),
            ),
            RunGroup::GitValues,
        )]);
        p.dotfiles = Some(dotfiles_plan());
        let (report, _) = run_with(&h, &mut p, &hook);
        assert_eq!(
            state(&report, "git.name"),
            &ItemState::Skipped {
                reason: SkipReason::WritesThroughLink
            }
        );
        // The opening check reads git, before any link exists. Nothing writes the value.
        assert!(
            !h.runner
                .calls()
                .iter()
                .any(|c| file_name(c) == "git" && args_contain(c, "Ada"))
        );
    }

    #[test]
    fn what_the_dotfiles_step_approves_is_saved_with_the_record() {
        let h = Harness::new();
        let hook = Watching {
            runner: &h.runner,
            home: h.home.path().to_path_buf(),
            link_gitconfig: false,
            calls: Mutex::new(vec![]),
        };
        let mut p = plan(vec![]);
        p.dotfiles = Some(dotfiles_plan());
        run_with(&h, &mut p, &hook);
        let record = MachineRecord::load(h.home.path()).unwrap();
        assert!(record.approvals.taps.contains("written/by-the-step"));
    }

    #[test]
    fn without_a_hook_the_run_says_the_dotfiles_wait() {
        let h = Harness::new();
        let mut p = plan(vec![]);
        p.dotfiles = Some(dotfiles_plan());
        let (_, events) = h.run(&mut p);
        assert!(events.iter().any(|e| matches!(
            &e.kind,
            EventKind::RunWarning { text } if text.contains("not applied by this Run")
        )));
    }

    #[test]
    fn approvals_in_the_record_read_back_and_never_land_during_a_run() {
        let h = Harness::new();
        let store = RecordApprovals::new(h.home.path().to_path_buf());
        assert_eq!(store.read().unwrap(), Approvals::default());
        let mut approvals = Approvals::default();
        approvals.packages.insert("owner/tap/name".into());
        approvals.dotfiles = Some(DotfilesApproval {
            repo: "github.com/example-you/dotfiles".into(),
            commit: CommitSha::parse(&"b".repeat(40)).unwrap(),
            links: BTreeMap::new(),
        });
        store.write(&approvals).unwrap();
        assert_eq!(store.read().unwrap(), approvals);
        // A Run holds its lock. A write then would be overwritten by the Run's own save.
        let lock_path = h
            .home
            .path()
            .join("Library/Application Support/Soulcaster/run.lock");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        held.try_lock().unwrap();
        assert!(store.write(&Approvals::default()).is_err());
        drop(held);
        assert_eq!(store.read().unwrap(), approvals);
    }
}
