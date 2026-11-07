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
    RunInput, RunReport, SCHEMA_VERSION, TapApproval, machine_record_path, perform,
};

const SECRET: &str = "s3cret-plover";

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
fn a_full_run_asks_even_when_nothing_needs_admin_and_discards_before_user_work() {
    let harness = Harness::new();
    absent_then_present(&harness, "Cellar/ripgrep");
    let (_report, events) = harness.run(&mut plan(vec![formula("ripgrep", &[])]));
    assert_eq!(harness.asks.asked(), 1);
    let discarded = events
        .iter()
        .position(|event| match &event.kind {
            EventKind::AdminDiscarded { why } => why.contains("nothing"),
            _ => false,
        })
        .expect("a full Run with no admin work still discards the password");
    let acting = events
        .iter()
        .position(|event| match &event.kind {
            EventKind::ItemState { item, to, .. } => item == "ripgrep" && to.is_working(),
            _ => false,
        })
        .expect("the formula was acted on");
    assert!(acting > discarded);
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
    record
        .approvals
        .taps
        .insert("someone/tap".into(), TapApproval { approved_at: 1 });
    record.save(harness.home.path()).unwrap();

    let mut plan = plan(vec![
        admin_builtin("root-item", &["tool"]),
        tap_formula("tool", "someone/tap/tool"),
    ]);
    let mut events = Vec::new();
    let report = perform(RunInput {
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
    record
        .approvals
        .taps
        .insert("someone/tap".into(), TapApproval { approved_at: 1 });
    record.save(harness.home.path()).unwrap();
    let harness = Harness::new();
    // Reuse is awkward across homes. Drive the third-party case on this fresh home.
    let mut record = MachineRecord::empty();
    record
        .approvals
        .taps
        .insert("someone/tap".into(), TapApproval { approved_at: 1 });
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
    record.approvals = Approvals {
        taps: BTreeMap::from([("someone/tap".into(), TapApproval { approved_at: 5 })]),
        packages: BTreeMap::from([("someone/tap/tool".into(), TapApproval { approved_at: 6 })]),
        dotfiles: Some(DotfilesApproval {
            repo: "ada/dotfiles".into(),
            commit: "abc123".into(),
            script: true,
        }),
    };
    record.clone_path = Some("~/src/dotfiles".into());
    record.local_values.insert("name".into(), "Ada".into());
    record.save(home.path()).unwrap();
    let loaded = MachineRecord::load(home.path()).unwrap();
    assert_eq!(loaded.schema_version, SCHEMA_VERSION);
    assert_eq!(loaded.approvals.dotfiles.as_ref().unwrap().commit, "abc123");
    assert_eq!(loaded.approvals.packages["someone/tap/tool"].approved_at, 6);
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
fn a_finished_requirement_is_not_pulled_into_a_scoped_run() {
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
    assert!(!report.states.contains_key("tool"));
    assert!(matches!(state(&report, "app"), ItemState::Satisfied { .. }));
    assert_eq!(install_count(&harness.runner, "tool"), 0);
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
