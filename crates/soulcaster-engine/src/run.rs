//! Check every Item, then the Admin Stage, discard the password, then the User Stage.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::command::{Command, CommandEnd, CommandOutput, CommandRunner, OutputStream};
use soulcaster_core::config::SettingValue;
use soulcaster_core::drift::{current_id, set_aside};
use soulcaster_core::event::{
    AskKind, AskStatus, ConfigRef, Event, EventKind, EventSink, LogStream, PlanItem, QueueEntry,
    QueueKind, RunScope, Stamper, StepPhase,
};
use soulcaster_core::lifecycle::{Assurance, FailureKind, ItemState, SkipReason, Stage};
use soulcaster_core::plan::{Plan, PlanEntry, RunGroup, Why};
use soulcaster_core::recipe::{
    BuiltinStep, Completion, Effect, EvidenceGroup, License, Probe, Recipe, SettingRecipe, Step,
    Store, StoredType,
};

use crate::act::{
    self, Beat, codesign_team_ok, failure_kind, is_network, mask_secrets, pkg_signer_has_team,
    restart_command, setting_json, sha_matches, version_below,
};
use crate::admin::{self, AdminGate, Budget, Validation};
use crate::dotfiles::DotfilesHook;
use crate::record::{
    AcceptedLicense, InProgress, Interruption, ItemOutcome, LastApplied, MachineRecord, QueueItem,
    support_dir,
};

const ADMIN_HOLD_MS: u64 = 60 * 60 * 1000;
const LOG_CAP: usize = 10 * 1024 * 1024;
const LOG_BATCH: Duration = Duration::from_millis(100);
static RUN_SEQ: AtomicU64 = AtomicU64::new(1);

/// Asking more would risk locking the account. `sudo` itself allows three.
const PASSWORD_TRIES: u32 = 3;

pub struct RunInput<'a> {
    pub plan: &'a mut Plan,
    pub runner: &'a dyn CommandRunner,
    pub admin: &'a dyn AdminGate,
    /// Applies the Dotfiles Repo inside the Run. Without one, a plan's Dotfiles Repo is left for
    /// later and the Run says so.
    pub dotfiles: Option<&'a dyn DotfilesHook>,
    pub home: &'a Path,
    pub events: &'a mut dyn EventSink,
    pub asks: &'a dyn Asks,
    pub cancel: &'a dyn CancelFlag,
    pub clock: &'a dyn Clock,
    pub scope: RunScope,
    pub config: ConfigRef,
    pub catalog_version: &'a str,
    pub engine_version: &'a str,
}

#[derive(Debug)]
pub struct RunReport {
    pub run_id: String,
    pub states: BTreeMap<String, ItemState>,
    pub queue: Vec<QueueEntry>,
    pub cancelled: bool,
}

#[derive(Debug)]
pub enum EngineError {
    AdminRequired {
        message: String,
    },
    CorruptRecord {
        path: PathBuf,
        detail: String,
    },
    UnsupportedRecord {
        path: PathBuf,
        found: u32,
    },
    RecordIo {
        path: PathBuf,
        detail: String,
    },
    Busy,
    Preflight {
        detail: String,
    },
    IllegalTransition {
        item: String,
        from: String,
        to: String,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AdminRequired { message } => write!(f, "{message}"),
            Self::CorruptRecord { path, detail } => write!(
                f,
                "Machine Record at {} is corrupt and was left unchanged: {detail}",
                path.display()
            ),
            Self::UnsupportedRecord { path, found } => write!(
                f,
                "Machine Record at {} uses schema version {found}, which this Soulcaster doesn't read. It was left unchanged.",
                path.display()
            ),
            Self::RecordIo { path, detail } => write!(
                f,
                "couldn't update the Machine Record at {}: {detail}",
                path.display()
            ),
            Self::Busy => write!(f, "a Run is already in progress on this Machine"),
            Self::Preflight { detail } => write!(f, "couldn't read this Machine: {detail}"),
            Self::IllegalTransition { item, from, to } => {
                write!(f, "Item {item} can't move from {from} to {to}")
            }
        }
    }
}

impl std::error::Error for EngineError {}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    fn wait(&self, duration: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }

    fn wait(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

pub trait CancelFlag: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

impl CancelFlag for AtomicBool {
    fn is_cancelled(&self) -> bool {
        self.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
    Unknown,
    /// Unset, and the Recipe doesn't say what unset means. A Run still writes it.
    Unset,
}

/// Only `gh` itself saying nobody is signed in counts as signed out. Anything that could also be a
/// network problem, or a copy that can't be trusted to answer, can't tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GhSignIn {
    SignedIn,
    SignedOut,
    CantTell,
}

enum CmdResult {
    Done(CommandOutput),
    Failed(FailureKind),
}

pub(crate) struct RunLock(File);

/// A Needs-You entry discovered while checking, opened only after `run.started`.
struct PendingQueue {
    item: String,
    step_id: String,
    kind: QueueKind,
    text: String,
    link: Option<String>,
    needs_password: bool,
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct Engine<'a> {
    admin: &'a dyn AdminGate,
    dotfiles: Option<&'a dyn DotfilesHook>,
    dotfiles_done: bool,
    /// `sudo -k` couldn't be confirmed, so root may still be reachable without asking.
    sudo_may_be_live: bool,
    plan: &'a mut Plan,
    runner: &'a dyn CommandRunner,
    home: &'a Path,
    events: &'a mut dyn EventSink,
    asks: &'a dyn Asks,
    cancel: &'a dyn CancelFlag,
    clock: &'a dyn Clock,
    scope_kind: RunScope,
    scope: BTreeSet<String>,
    config: ConfigRef,
    catalog_version: &'a str,
    engine_version: &'a str,
    stamper: Stamper,
    record: MachineRecord,
    states: BTreeMap<String, ItemState>,
    verdicts: BTreeMap<String, Verdict>,
    /// Check results applied to the stream after `run.started`, so the diagram's hops are legal.
    early: BTreeMap<String, ItemState>,
    early_queues: Vec<PendingQueue>,
    probe_cache: BTreeMap<String, Verdict>,
    /// Item transitions emit only once the stream has started.
    reporting: bool,
    priors: BTreeMap<String, Value>,
    assurances: BTreeMap<String, Assurance>,
    password: Option<Password>,
    held_since: Option<u64>,
    macos: Option<String>,
    admin_account: bool,
    run_id: String,
    cancelled: bool,
    closed: bool,
    interruption_note: Option<String>,
    needs_logout: bool,
    log_bytes: BTreeMap<String, usize>,
    log_pending: Vec<(String, LogStream, String)>,
    log_since: Option<Instant>,
}

pub fn perform(input: RunInput<'_>) -> Result<RunReport, EngineError> {
    let _lock = lock_run(input.home)?;
    let record = MachineRecord::load(input.home)?;
    let mut engine = Engine::new(input, record)?;
    engine.execute()
}

pub(crate) fn lock_run(home: &Path) -> Result<RunLock, EngineError> {
    let dir = support_dir(home);
    fs::create_dir_all(&dir).map_err(|error| EngineError::RecordIo {
        path: dir.clone(),
        detail: error.to_string(),
    })?;
    let path = dir.join("run.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| EngineError::RecordIo {
            path: path.clone(),
            detail: error.to_string(),
        })?;
    match file.try_lock() {
        Ok(()) => Ok(RunLock(file)),
        Err(TryLockError::WouldBlock) => Err(EngineError::Busy),
        Err(TryLockError::Error(error)) => Err(EngineError::RecordIo {
            path,
            detail: error.to_string(),
        }),
    }
}

impl<'a> Engine<'a> {
    fn new(input: RunInput<'a>, record: MachineRecord) -> Result<Self, EngineError> {
        let run_id = format!(
            "run-{}-{}",
            input.clock.now_ms(),
            RUN_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let scope = scope_of(&input.scope, input.plan);
        Ok(Self {
            plan: input.plan,
            runner: input.runner,
            admin: input.admin,
            dotfiles: input.dotfiles,
            dotfiles_done: false,
            sudo_may_be_live: false,
            home: input.home,
            events: input.events,
            asks: input.asks,
            cancel: input.cancel,
            clock: input.clock,
            scope_kind: input.scope,
            scope,
            config: input.config,
            catalog_version: input.catalog_version,
            engine_version: input.engine_version,
            stamper: Stamper::new(),
            record,
            states: BTreeMap::new(),
            verdicts: BTreeMap::new(),
            early: BTreeMap::new(),
            early_queues: Vec::new(),
            probe_cache: BTreeMap::new(),
            reporting: false,
            priors: BTreeMap::new(),
            assurances: BTreeMap::new(),
            password: None,
            held_since: None,
            macos: None,
            admin_account: false,
            run_id,
            cancelled: false,
            closed: false,
            interruption_note: None,
            needs_logout: false,
            log_bytes: BTreeMap::new(),
            log_pending: Vec::new(),
            log_since: None,
        })
    }

    /// A person's answers last only as long as what they were about. Every Run holds the whole
    /// Config, whatever it acts on, so any Run can tell.
    fn settle_answers(&mut self) {
        let catalog = Catalog::shipped();
        let listed: BTreeSet<String> = self.plan.entries.iter().map(|e| e.id.clone()).collect();
        let aside = set_aside(self.plan, catalog);
        // "This keep" ends once the Config lists it again.
        self.record
            .kept_here
            .retain(|id| !listed.contains(&current_id(catalog, id)));
        // "Leave this Mac as it is" ends once the entry has left the Config.
        let marked = reason_label(SkipReason::MarkedByPerson);
        self.record.unavailable.retain(|id, reason| {
            let id = current_id(catalog, id);
            reason != marked || listed.contains(&id) || aside.contains(&id)
        });
    }

    fn execute(&mut self) -> Result<RunReport, EngineError> {
        self.settle_answers();
        self.prepare_run_dir();
        self.preflight()?;
        self.reconcile()?;
        self.precheck()?;
        self.assign_stages()?;
        self.emit_started();
        if let Some(note) = self.interruption_note.clone() {
            self.emit(EventKind::RunWarning { text: note });
        }
        self.publish_checked()?;
        self.note_touch_id()?;
        self.note_dotfiles();
        self.maybe_ask();
        self.early_github()?;
        self.run_stage(Stage::Admin)?;
        self.discard("the Admin Stage finished");
        self.run_stage(Stage::User)?;
        if !self.cancelled {
            self.apply_dotfiles()?;
        }
        self.finish()
    }

    fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    fn emit(&mut self, kind: EventKind) {
        if let Ok(text) = serde_json::to_string(&kind) {
            self.debug_no_secret(&text);
        }
        let event = self.stamper.stamp(self.now(), kind);
        self.append_event(&event);
        self.events.send(event);
    }

    fn debug_no_secret(&self, text: &str) {
        if let Some(password) = &self.password {
            let secret = password.expose();
            if secret.len() >= 4 {
                debug_assert!(
                    !text.contains(secret),
                    "a password reached an event or the Machine Record"
                );
            }
        }
    }

    fn preflight(&mut self) -> Result<(), EngineError> {
        let groups = self.read_command(&act::id_command(self.home))?;
        self.admin_account = groups.split_whitespace().any(|group| group == "admin");
        let version = self.read_command(&act::sw_vers(self.home))?;
        let version = version.trim();
        if !version.is_empty() {
            self.macos = Some(version.to_string());
        }
        if !self.admin_account
            && (!self.base_present("command-line-tools") || !self.base_present("homebrew"))
        {
            return Err(EngineError::AdminRequired {
                message: "This account isn't an admin, and Command Line Tools or Homebrew aren't installed. An admin has to set those up once.".into(),
            });
        }
        Ok(())
    }

    fn base_present(&mut self, name: &str) -> bool {
        let probe = match name {
            "homebrew" => Probe::Builtin {
                name: soulcaster_core::recipe::BuiltinProbe::Homebrew,
            },
            _ => Probe::Builtin {
                name: soulcaster_core::recipe::BuiltinProbe::CommandLineTools,
            },
        };
        matches!(self.eval_probe(&probe), Ok(Verdict::Pass))
    }

    fn reconcile(&mut self) -> Result<(), EngineError> {
        let Some(running) = self.record.running.take() else {
            return Ok(());
        };
        self.interruption_note = Some(format!(
            "{} was interrupted and will be checked again",
            running.item
        ));
        self.record.interruptions.push(Interruption {
            item: running.item,
            run_id: running.run_id,
            at: self.now(),
        });
        self.record.save(self.home)
    }

    fn precheck(&mut self) -> Result<(), EngineError> {
        let ids: Vec<String> = self
            .plan
            .entries
            .iter()
            .filter(|entry| self.scope.contains(&entry.id))
            .map(|entry| entry.id.clone())
            .collect();
        for id in ids {
            self.precheck_one(&id)?;
        }
        Ok(())
    }

    fn precheck_one(&mut self, id: &str) -> Result<(), EngineError> {
        self.states.insert(id.to_string(), ItemState::Pending);
        let Some(entry) = self.entry(id).cloned() else {
            return Ok(());
        };
        if let Some(reason) = self.skip_reason(&entry) {
            self.early
                .insert(id.to_string(), ItemState::Skipped { reason });
            return Ok(());
        }
        if self.forced_by_profile(&entry)? {
            self.early.insert(
                id.to_string(),
                ItemState::Skipped {
                    reason: SkipReason::ManagedByAdmin,
                },
            );
            return Ok(());
        }
        if self.conflict_hit(&entry)? {
            self.defer_queue(
                id,
                "conflict",
                QueueKind::Conflict,
                "This conflicts with something already on this Machine.",
                None,
                false,
            );
            self.early
                .insert(id.to_string(), ItemState::NeedsYou { cause: None });
            return Ok(());
        }
        let verdict = self.judge(&entry)?;
        self.verdicts.insert(id.to_string(), verdict);
        if verdict != Verdict::Pass && self.missing_tap(&entry) {
            let tap = entry.recipe.taps().join(", ");
            self.defer_queue(
                id,
                "tap",
                QueueKind::Tap,
                &format!("This loads the third-party tap {tap}, which needs its own approval."),
                None,
                false,
            );
            self.early
                .insert(id.to_string(), ItemState::NeedsYou { cause: None });
            return Ok(());
        }
        if verdict == Verdict::Pass && self.person_gate_open(id) {
            self.early
                .insert(id.to_string(), ItemState::NeedsYou { cause: None });
            return Ok(());
        }
        if verdict == Verdict::Pass {
            let assurance = self.assurance_for(&entry);
            self.assurances.insert(id.to_string(), assurance);
            self.early
                .insert(id.to_string(), ItemState::Satisfied { assurance });
            return Ok(());
        }
        if verdict == Verdict::Unknown {
            self.early.insert(id.to_string(), ItemState::Unknown);
        }
        Ok(())
    }

    fn defer_queue(
        &mut self,
        item: &str,
        step_id: &str,
        kind: QueueKind,
        text: &str,
        link: Option<String>,
        needs_password: bool,
    ) {
        self.early_queues.push(PendingQueue {
            item: item.to_string(),
            step_id: step_id.to_string(),
            kind,
            text: text.to_string(),
            link,
            needs_password,
        });
    }

    fn skip_reason(&self, entry: &PlanEntry) -> Option<SkipReason> {
        // "Leave this Mac as it is": a Run doesn't quietly undo that answer.
        let marked = reason_label(SkipReason::MarkedByPerson);
        if self
            .record
            .unavailable
            .iter()
            .any(|(id, reason)| reason == marked && current_id(Catalog::shipped(), id) == entry.id)
        {
            return Some(SkipReason::MarkedByPerson);
        }
        if let Recipe::Setting(setting) = &entry.recipe
            && setting.is_broken()
        {
            return Some(SkipReason::BrokenHere);
        }
        if let Recipe::Install(install) = &entry.recipe
            && let (Some(minimum), Some(machine)) = (&install.min_macos, &self.macos)
            && version_below(machine, minimum)
        {
            return Some(SkipReason::MacosTooOld);
        }
        if self.writes_through_link(entry) {
            return Some(SkipReason::WritesThroughLink);
        }
        None
    }

    fn writes_through_link(&self, entry: &PlanEntry) -> bool {
        let git = matches!(
            &entry.recipe,
            Recipe::Setting(setting)
                if setting
                    .stores
                    .iter()
                    .any(|store| matches!(store, Store::GitConfig { .. }))
        ) || matches!(entry.recipe, Recipe::MachineValues(_));
        if !git {
            return false;
        }
        // A linked gitconfig would run as soon as git starts. The read happens in precheck via
        // the runner; a synchronous metadata check stays inside the sandbox home.
        link_on_the_way(self.home, Path::new(".gitconfig"))
    }

    fn conflict_hit(&mut self, entry: &PlanEntry) -> Result<bool, EngineError> {
        let Recipe::Install(install) = &entry.recipe else {
            return Ok(false);
        };
        let probes: Vec<Probe> = install
            .conflicts
            .iter()
            .map(|conflict| conflict.probe.clone())
            .collect();
        for probe in probes {
            if self.eval_probe(&probe)? == Verdict::Pass {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn missing_tap(&self, entry: &PlanEntry) -> bool {
        entry.recipe.steps().iter().any(|step| {
            let Some(name) = step.brew_name() else {
                return false;
            };
            let Some(tap) = soulcaster_core::recipe::tap_of(name) else {
                return false;
            };
            let tap_ok = self.record.approvals.taps.contains(&tap);
            let package_ok = self.record.approvals.packages.contains(name);
            !tap_ok && !package_ok
        })
    }

    /// A profile that forces the key. The read is the managed-preferences file, not the user's defaults.
    fn forced_by_profile(&mut self, entry: &PlanEntry) -> Result<bool, EngineError> {
        let Recipe::Setting(setting) = &entry.recipe else {
            return Ok(false);
        };
        let stores = setting.stores.clone();
        for store in &stores {
            let Store::Defaults { domain, key, .. } = store else {
                continue;
            };
            for command in act::managed_preference_reads(self.home, domain.as_str(), key) {
                if let CmdResult::Done(output) = self.run_command(&command, "", false)?
                    && output.is_success()
                    && !output.stdout_text().trim().is_empty()
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn command_line_tools_present(&mut self) -> Result<bool, EngineError> {
        let probe = Probe::Builtin {
            name: soulcaster_core::recipe::BuiltinProbe::CommandLineTools,
        };
        Ok(self.eval_probe(&probe)? == Verdict::Pass)
    }

    fn person_gate_open(&self, item: &str) -> bool {
        self.record.open_queue_for(item).any(|entry| {
            matches!(
                entry.kind,
                QueueKind::Person | QueueKind::License | QueueKind::Tap | QueueKind::Conflict
            )
        })
    }

    fn judge(&mut self, entry: &PlanEntry) -> Result<Verdict, EngineError> {
        match &entry.recipe {
            Recipe::Install(install) => {
                let checks = install.checks.clone();
                self.judge_probes(&checks)
            }
            Recipe::Setting(setting) => {
                let value = entry.value.clone();
                self.judge_setting(entry.id.as_str(), setting, value.as_ref())
            }
            Recipe::MachineValues(recipe) => self.judge_machine_values(recipe),
        }
    }

    /// Git config, then the Machine Record. The GitHub account is the sign-in ticket.
    fn judge_machine_values(
        &mut self,
        recipe: &soulcaster_core::recipe::MachineValuesRecipe,
    ) -> Result<Verdict, EngineError> {
        let values = recipe.values.clone();
        let mut ready = true;
        for value in &values {
            if let Some(found) = self.lookup_git(&value.store)? {
                self.record
                    .local_values
                    .entry(value.name.clone())
                    .or_insert(found);
                continue;
            }
            if self.record.local_values.contains_key(&value.name) {
                ready = false;
                continue;
            }
            ready = false;
        }
        Ok(if ready { Verdict::Pass } else { Verdict::Fail })
    }

    fn judge_probes(&mut self, probes: &[Probe]) -> Result<Verdict, EngineError> {
        let mut unknown = false;
        for probe in probes {
            match self.eval_probe(probe)? {
                Verdict::Fail | Verdict::Unset => return Ok(Verdict::Fail),
                Verdict::Unknown => unknown = true,
                Verdict::Pass => {}
            }
        }
        Ok(if unknown {
            Verdict::Unknown
        } else {
            Verdict::Pass
        })
    }

    fn eval_probe(&mut self, probe: &Probe) -> Result<Verdict, EngineError> {
        let key = serde_json::to_string(probe).unwrap_or_default();
        if let Some(verdict) = self.probe_cache.get(&key).copied() {
            return Ok(verdict);
        }
        let verdict = self.eval_probe_now(probe)?;
        self.probe_cache.insert(key, verdict);
        Ok(verdict)
    }

    fn eval_probe_now(&mut self, probe: &Probe) -> Result<Verdict, EngineError> {
        if matches!(
            probe,
            Probe::Unknowable { .. }
                | Probe::Builtin {
                    name: soulcaster_core::recipe::BuiltinProbe::GithubSignedIn
                }
        ) {
            return Ok(Verdict::Unknown);
        }
        let commands = act::probe_commands(self.home, probe);
        if commands.is_empty() {
            return Ok(Verdict::Unknown);
        }
        for command in &commands {
            match self.run_command(command, "", false)? {
                CmdResult::Failed(FailureKind::Stalled)
                | CmdResult::Failed(FailureKind::Network) => {
                    return Ok(Verdict::Unknown);
                }
                CmdResult::Failed(_) => return Ok(Verdict::Fail),
                CmdResult::Done(output) if !output.is_success() => return Ok(Verdict::Fail),
                CmdResult::Done(_) => {}
            }
        }
        Ok(Verdict::Pass)
    }

    fn judge_setting(
        &mut self,
        id: &str,
        setting: &SettingRecipe,
        desired: Option<&SettingValue>,
    ) -> Result<Verdict, EngineError> {
        let Some(desired) = desired else {
            return Ok(Verdict::Unknown);
        };
        let mut saw_unset = false;
        for store in &setting.stores {
            match self.read_store(id, store, desired)? {
                ReadBack::Differ => return Ok(Verdict::Fail),
                ReadBack::Unknown => return Ok(Verdict::Unknown),
                ReadBack::Unset => saw_unset = true,
                ReadBack::Match => {}
            }
        }
        if saw_unset {
            if setting
                .unset_means
                .as_ref()
                .is_some_and(|unset| unset == &setting_json(desired))
            {
                return Ok(Verdict::Pass);
            }
            if setting.unset_means.is_none() {
                return Ok(Verdict::Unset);
            }
            return Ok(Verdict::Fail);
        }
        self.assurances
            .insert(id.to_string(), assurance_of(setting, self.macos.as_deref()));
        Ok(Verdict::Pass)
    }

    fn read_store(
        &mut self,
        id: &str,
        store: &Store,
        desired: &SettingValue,
    ) -> Result<ReadBack, EngineError> {
        let store = &act::with_home(self.home, store);
        match store {
            Store::Defaults { .. } => {
                let Some(command) = act::defaults_read(self.home, store) else {
                    return Ok(ReadBack::Unknown);
                };
                match self.run_command(&command, id, false)? {
                    CmdResult::Done(output) if output.is_success() => {
                        let prior = parse_defaults(store, &output.stdout_text());
                        if let Some(value) = &prior {
                            self.priors.entry(id.to_string()).or_insert(value.clone());
                        }
                        Ok(compare_defaults(store, desired, &output.stdout_text()))
                    }
                    CmdResult::Done(_) => Ok(ReadBack::Unset),
                    CmdResult::Failed(FailureKind::Stalled) => Ok(ReadBack::Unknown),
                    CmdResult::Failed(_) => Ok(ReadBack::Unset),
                }
            }
            Store::GitConfig { key } => {
                // `/usr/bin/git` is Apple's stub until Command Line Tools exist. The value is
                // read at act time, once that requirement has passed.
                let Some(text) = self.read_git_config(key)? else {
                    return Ok(ReadBack::Unset);
                };
                self.priors
                    .entry(id.to_string())
                    .or_insert(Value::String(text.clone()));
                Ok(if text == rendered_git(desired) {
                    ReadBack::Match
                } else {
                    ReadBack::Differ
                })
            }
            Store::Builtin { name } => match name {
                soulcaster_core::recipe::BuiltinStore::LibraryVisible => {
                    self.read_library(id, desired)
                }
                soulcaster_core::recipe::BuiltinStore::DockTiles => self.read_dock(id, desired),
                soulcaster_core::recipe::BuiltinStore::SidebarFavorites => Ok(ReadBack::Unknown),
            },
        }
    }

    fn read_library(&mut self, id: &str, desired: &SettingValue) -> Result<ReadBack, EngineError> {
        match self.run_command(&act::library_read(self.home), id, false)? {
            CmdResult::Done(output) if output.is_success() => {
                let hidden = library_hidden(&output.stdout_text());
                let visible = !hidden;
                let wanted = matches!(desired, SettingValue::Bool(true));
                Ok(if visible == wanted {
                    ReadBack::Match
                } else {
                    ReadBack::Differ
                })
            }
            _ => Ok(ReadBack::Unknown),
        }
    }

    fn read_dock(&mut self, id: &str, desired: &SettingValue) -> Result<ReadBack, EngineError> {
        let SettingValue::List(wanted) = desired else {
            return Ok(ReadBack::Differ);
        };
        match self.run_command(&act::dock_list(self.home), id, false)? {
            CmdResult::Done(output) if output.is_success() => {
                let got = dock_paths(&output.stdout_text());
                let same = got.len() == wanted.len()
                    && got.iter().zip(wanted.iter()).all(|(got, want)| {
                        dock_path(got)
                            == dock_path(&act::expand_home(self.home, want).to_string_lossy())
                    });
                Ok(if same {
                    ReadBack::Match
                } else {
                    ReadBack::Differ
                })
            }
            // dockutil is not on the Machine yet. That is unset, so the Run writes it after
            // the requirement installs the tool. A stall still can't be told from a real value.
            CmdResult::Failed(FailureKind::Stalled) | CmdResult::Failed(FailureKind::Network) => {
                Ok(ReadBack::Unknown)
            }
            _ => Ok(ReadBack::Unset),
        }
    }

    fn assurance_for(&self, entry: &PlanEntry) -> Assurance {
        match &entry.recipe {
            Recipe::Setting(setting) => self
                .assurances
                .get(&entry.id)
                .copied()
                .unwrap_or_else(|| assurance_of(setting, self.macos.as_deref())),
            _ => Assurance::Proven,
        }
    }

    fn assign_stages(&mut self) -> Result<(), EngineError> {
        let ids: Vec<String> = self
            .plan
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        let mut stages = Vec::new();
        for id in ids {
            let Some(entry) = self.entry(&id).cloned() else {
                continue;
            };
            stages.push((id, self.stage_for(&entry)?));
        }
        self.plan.assign_stages(|entry| {
            stages
                .iter()
                .find(|(id, _)| id == &entry.id)
                .map(|(_, stage)| *stage)
                .unwrap_or(entry.stage)
        });
        Ok(())
    }

    fn stage_for(&mut self, entry: &PlanEntry) -> Result<Stage, EngineError> {
        let needs_act = matches!(
            self.verdicts.get(&entry.id),
            Some(Verdict::Fail | Verdict::Unset)
        );
        if !needs_act || !self.scope.contains(&entry.id) {
            return Ok(Stage::User);
        }
        if !entry.recipe.taps().is_empty() || !self.admin_account {
            return Ok(Stage::User);
        }
        if self.record.learned_admin.iter().any(|id| id == &entry.id) {
            return Ok(Stage::Admin);
        }
        let probes: Vec<Probe> = entry
            .recipe
            .steps()
            .iter()
            .filter_map(|step| step.unless().cloned())
            .collect();
        let mut passing = Vec::new();
        for probe in &probes {
            if self.eval_probe(probe)? == Verdict::Pass {
                passing.push(probe.clone());
            }
        }
        Ok(entry
            .recipe
            .stage(|probe| passing.iter().any(|known| known == probe)))
    }

    fn emit_started(&mut self) {
        let plan = self.plan_items();
        self.emit(EventKind::RunStarted {
            run_id: self.run_id.clone(),
            scope: self.scope_kind.clone(),
            config: self.config.clone(),
            catalog_version: self.catalog_version.to_string(),
            engine_version: self.engine_version.to_string(),
            macos_version: self.macos.clone(),
            plan,
        });
    }

    fn plan_items(&self) -> Vec<PlanItem> {
        self.plan
            .summary()
            .into_iter()
            .filter(|item| self.scope.contains(&item.id))
            .collect()
    }

    /// Walks each checked Item through the legal hops. `pending` cannot jump to `satisfied`.
    fn publish_checked(&mut self) -> Result<(), EngineError> {
        self.reporting = true;
        let mut ids: Vec<String> = self
            .plan
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        for id in self.early.keys() {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        for id in ids {
            let Some(state) = self.early.get(&id).cloned() else {
                continue;
            };
            self.flush_queues(&id)?;
            self.reach(&id, state)?;
        }
        if matches!(self.scope_kind, RunScope::Full) {
            let skipped: Vec<_> = self
                .plan
                .skipped
                .iter()
                .map(|entry| (entry.raw.clone(), entry.reason))
                .collect();
            for (raw, reason) in skipped {
                self.states.entry(raw.clone()).or_insert(ItemState::Pending);
                if self.states.get(&raw) == Some(&ItemState::Pending) {
                    self.move_item(&raw, ItemState::Skipped { reason })?;
                }
            }
        }
        Ok(())
    }

    fn flush_queues(&mut self, item: &str) -> Result<(), EngineError> {
        let queues: Vec<_> = self
            .early_queues
            .iter()
            .filter(|entry| entry.item == item)
            .map(|entry| PendingQueue {
                item: entry.item.clone(),
                step_id: entry.step_id.clone(),
                kind: entry.kind,
                text: entry.text.clone(),
                link: entry.link.clone(),
                needs_password: entry.needs_password,
            })
            .collect();
        for entry in queues {
            self.open_queue(
                &entry.item,
                &entry.step_id,
                entry.kind,
                &entry.text,
                entry.link,
                entry.needs_password,
            )?;
        }
        Ok(())
    }

    fn reach(&mut self, id: &str, to: ItemState) -> Result<(), EngineError> {
        if matches!(to, ItemState::Skipped { .. }) {
            return self.move_item(id, to);
        }
        self.move_item(id, ItemState::Checking)?;
        if matches!(to, ItemState::Satisfied { .. }) {
            self.close_retry_queue(id);
            self.note_satisfied_extras(id);
        }
        self.move_item(id, to)
    }

    fn note_satisfied_extras(&mut self, id: &str) {
        let unconfirmed = self.entry(id).is_some_and(|entry| {
            matches!(
                &entry.recipe,
                Recipe::Setting(setting) if setting.evidence.group == EvidenceGroup::Unconfirmed
            )
        });
        let mut lines = Vec::new();
        if unconfirmed {
            lines.push("effect not confirmed".to_string());
        }
        if let Some(entry) = self.entry(id)
            && let Recipe::Install(install) = &entry.recipe
        {
            for fact in &install.facts {
                if matches!(fact.probe, Probe::Unknowable { .. }) {
                    lines.push(format!("{}: can't be checked", fact.label));
                }
            }
        }
        if lines.is_empty() {
            return;
        }
        self.emit(EventKind::ItemLog {
            item: id.to_string(),
            stream: LogStream::Engine,
            lines,
        });
    }

    fn note_touch_id(&mut self) -> Result<(), EngineError> {
        for path in ["/etc/pam.d/sudo_local", "/etc/pam.d/sudo"] {
            let command = act::base_command(self.home, "/bin/cat").args([path]);
            let CmdResult::Done(output) = self.run_command(&command, "", false)? else {
                continue;
            };
            for line in output.stdout_text().lines() {
                let trimmed = line.trim();
                if trimmed.contains("pam_tid.so") && !trimmed.starts_with('#') {
                    self.emit(EventKind::RunWarning {
                        text: "Touch ID is on for sudo. Stay nearby for the first part of the Run."
                            .into(),
                    });
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Once per Run, and never while the password is held.
    fn apply_dotfiles(&mut self) -> Result<(), EngineError> {
        // An Apply changes what was chosen. The Dotfiles Repo isn't a row it can choose.
        if self.dotfiles_done
            || self.password.is_some()
            || matches!(self.scope_kind, RunScope::Only { .. })
        {
            return Ok(());
        }
        self.dotfiles_done = true;
        let (Some(hook), Some(plan)) = (self.dotfiles, self.plan.dotfiles.clone()) else {
            return Ok(());
        };
        let held = crate::approvals::HeldApprovals {
            record: std::sync::Mutex::new(self.record.clone()),
            home: self.home.to_path_buf(),
        };
        let result = hook.apply(&plan, &held);
        if let Ok(record) = held.record.into_inner() {
            self.record = record;
        }
        self.record.save(self.home)?;
        for text in result.warnings {
            self.emit(EventKind::RunWarning { text });
        }
        Ok(())
    }

    fn note_dotfiles(&mut self) {
        if self.plan.dotfiles.is_some() && self.dotfiles.is_none() {
            self.emit(EventKind::RunWarning {
                text: "The Dotfiles Repo is not applied by this Run.".into(),
            });
        }
    }

    fn maybe_ask(&mut self) {
        // The map: the password is asked for only when the plan has admin work after the check.
        if !self.admin_account || !self.pending_admin() {
            self.emit(EventKind::AskState {
                ask: AskKind::Password,
                state: AskStatus::Skipped,
            });
            return;
        }
        self.emit(EventKind::AskState {
            ask: AskKind::Password,
            state: AskStatus::Waiting,
        });
        admin::disable_core_dumps();
        let mut last_was_wrong = false;
        for _ in 0..PASSWORD_TRIES {
            let Some(password) = self.asks.password(&PasswordRequest {
                why: "Soulcaster needs your password for the Admin Stage. It is discarded before anything else runs.".into(),
                last_was_wrong,
            }) else {
                break;
            };
            match self.admin.validate(self.runner, self.home, &password) {
                Validation::Accepted => {
                    // Checking it left a timestamp. Nothing should have root while the person
                    // signs in to GitHub, or between steps. If that can't be ended, the password
                    // isn't held at all.
                    if !self.admin.forget(self.runner, self.home) {
                        self.sudo_may_be_live = true;
                        self.emit(EventKind::RunWarning {
                            text:
                                "sudo's sign-in couldn't be ended, so the admin work waits for you."
                                    .into(),
                        });
                        break;
                    }
                    admin::lock_in_memory(password.expose().as_bytes());
                    self.password = Some(password);
                    self.held_since = Some(self.now());
                    self.emit(EventKind::AskState {
                        ask: AskKind::Password,
                        state: AskStatus::Done,
                    });
                    self.emit(EventKind::AdminHeld {
                        why: "the Admin Stage is starting".into(),
                    });
                    return;
                }
                Validation::Wrong => last_was_wrong = true,
                Validation::StillSignedIn => {
                    self.sudo_may_be_live = true;
                    self.emit(EventKind::RunWarning {
                        text: "sudo's earlier sign-in couldn't be cleared, so the admin work waits for you.".into(),
                    });
                    break;
                }
                Validation::Unavailable(why) => {
                    self.emit(EventKind::RunWarning {
                        text: format!("The password couldn't be checked: {why}"),
                    });
                    break;
                }
            }
        }
        self.emit(EventKind::AskState {
            ask: AskKind::Password,
            state: AskStatus::Skipped,
        });
    }

    /// The bootstrap `gh`, and GitHub sign-in, while the person is still here. If Homebrew's `gh`
    /// is on the Machine, or there's no base `gh` Item in this Run, none of this runs, except to
    /// finish a sign-in that's still waiting in the queue.
    fn early_github(&mut self) -> Result<(), EngineError> {
        let wanted = self.entry("gh").and_then(|entry| match &entry.recipe {
            Recipe::Install(install) if self.scope.contains(&entry.id) => install.bootstrap.clone(),
            _ => None,
        });
        let Some(bootstrap) = wanted else {
            return Ok(());
        };
        let brew_gh = PathBuf::from(act::brew_gh_path());
        let home_probe =
            act::base_command(self.home, "/bin/test").args(["-x", act::brew_gh_path()]);
        if matches!(
            self.run_command(&home_probe, "", false)?,
            CmdResult::Done(ref output) if output.is_success()
        ) {
            if self.queue_is_open(&queue_id("gh", "sign-in")) {
                if self.gh_signed_in(&brew_gh)? {
                    self.close_queue_id(&queue_id("gh", "sign-in"));
                } else {
                    self.sign_in_to_github(&brew_gh)?;
                }
            }
            return Ok(());
        }
        if self.states.get("gh") != Some(&ItemState::Pending) {
            return Ok(());
        }
        let (_, binary) = act::bootstrap_paths(self.home, &bootstrap);
        let present = act::base_command(self.home, "/bin/test")
            .args(["-x".to_string(), binary.to_string_lossy().into_owned()]);
        let present = matches!(
            self.run_command(&present, "", false)?,
            CmdResult::Done(ref output) if output.is_success()
        );
        if !present || !self.gh_may_run(&binary)? {
            // A copy that no longer passes is replaced, never run.
            self.remove_bootstrap()?;
            let beats = act::bootstrap_beats(self.home, &bootstrap);
            if self.run_beats("", &beats)?.is_some() {
                self.emit(EventKind::RunWarning {
                    text: "The early GitHub tool couldn't be fetched, so sign-in waits until Homebrew's is in.".into(),
                });
                return Ok(());
            }
        }
        if self.gh_signed_in(&binary)? {
            self.close_queue_id(&queue_id("gh", "sign-in"));
            return Ok(());
        }
        self.sign_in_to_github(&binary)
    }

    /// Everything that has to hold before any `gh` starts. Its `PATH` folder really is empty, so
    /// it can't find `git`. The early copy runs while the password is held on the strength of its
    /// pin, so it's checked again every time. Homebrew's is the Machine's own.
    fn gh_may_run(&mut self, program: &Path) -> Result<bool, EngineError> {
        if !self.clear_empty_path() {
            return Ok(false);
        }
        if program == Path::new(act::brew_gh_path()) {
            return Ok(true);
        }
        let Some(bootstrap) = self.entry("gh").and_then(|entry| match &entry.recipe {
            Recipe::Install(install) => install.bootstrap.clone(),
            _ => None,
        }) else {
            return Ok(false);
        };
        let beats = act::bootstrap_check_beats(self.home, &bootstrap);
        Ok(self.run_beats("", &beats)?.is_none())
    }

    fn clear_empty_path(&self) -> bool {
        let support = support_dir(self.home);
        let empty = support.join("empty-path");
        let Ok(inside) = support.strip_prefix(self.home) else {
            return false;
        };
        if link_on_the_way(self.home, inside) {
            return false;
        }
        if let Ok(meta) = fs::symlink_metadata(&empty)
            && !meta.is_dir()
        {
            // A link is removed, not followed.
            if fs::remove_file(&empty).is_err() {
                return false;
            }
        }
        if fs::create_dir_all(&empty).is_err()
            || fs::set_permissions(&empty, fs::Permissions::from_mode(0o700)).is_err()
        {
            return false;
        }
        let Ok(entries) = fs::read_dir(&empty) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let removed = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path),
                _ => fs::remove_file(&path),
            };
            if removed.is_err() {
                return false;
            }
        }
        true
    }

    fn queue_is_open(&self, id: &str) -> bool {
        self.record.open_queue_for("gh").any(|entry| entry.id == id)
    }

    fn gh_signed_in(&mut self, program: &Path) -> Result<bool, EngineError> {
        Ok(self.gh_sign_in_state(program)? == GhSignIn::SignedIn)
    }

    fn gh_sign_in_state(&mut self, program: &Path) -> Result<GhSignIn, EngineError> {
        if !self.gh_may_run(program)? {
            return Ok(GhSignIn::CantTell);
        }
        let status =
            act::quiet_gh(self.home, program).args(["auth", "status", "--hostname", "github.com"]);
        // One try, read directly: the words on a failure are what tell these apart.
        let Ok(output) = self.run_once(&status, "", false) else {
            return Ok(GhSignIn::CantTell);
        };
        let said = format!(
            "{}{}",
            output.stdout_text(),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.is_success() {
            if said.contains("hosts.yml") {
                self.emit(EventKind::RunWarning {
                    text: "Keychain refused the GitHub sign-in, so it's stored in a plain file."
                        .into(),
                });
            }
            return Ok(GhSignIn::SignedIn);
        }
        Ok(
            if output.end == CommandEnd::Exited(1) && said.to_lowercase().contains("not logged in")
            {
                GhSignIn::SignedOut
            } else {
                GhSignIn::CantTell
            },
        )
    }

    fn sign_in_to_github(&mut self, program: &Path) -> Result<(), EngineError> {
        // Not a long wait with root possibly reachable. A later Run asks again.
        if self.sudo_may_be_live {
            self.asks.github_status(SignInStatus::Failed);
            return self.open_queue(
                "gh",
                "sign-in",
                QueueKind::Person,
                "Sign in to GitHub. Run again when you're ready.",
                None,
                false,
            );
        }
        self.emit(EventKind::AskState {
            ask: AskKind::Github,
            state: AskStatus::Waiting,
        });
        self.asks.github_status(SignInStatus::Waiting);
        if !self.gh_may_run(program)? {
            self.asks.github_status(SignInStatus::Failed);
            self.emit(EventKind::AskState {
                ask: AskKind::Github,
                state: AskStatus::Skipped,
            });
            return self.open_queue(
                "gh",
                "sign-in",
                QueueKind::Person,
                "Sign in to GitHub. Run again when you're ready.",
                None,
                false,
            );
        }
        let asks = self.asks;
        let mut shown = false;
        // What it prints stays out of the log and the events. The code is the person's to use.
        let ran = self
            .runner
            .run(&act::gh_sign_in(self.home, program), &mut |_, line| {
                if !shown && let Some(code) = act::gh_code_in(line) {
                    shown = true;
                    asks.show_github_code(&code, act::GITHUB_DEVICE_URL);
                }
            });
        let finished = ran.is_ok_and(|output| output.is_success());
        if finished && self.gh_signed_in(program)? {
            self.asks.github_status(SignInStatus::SignedIn);
            self.close_queue_id(&queue_id("gh", "sign-in"));
            self.emit(EventKind::AskState {
                ask: AskKind::Github,
                state: AskStatus::Done,
            });
            return Ok(());
        }
        self.asks.github_status(SignInStatus::Failed);
        self.emit(EventKind::AskState {
            ask: AskKind::Github,
            state: AskStatus::Skipped,
        });
        self.open_queue(
            "gh",
            "sign-in",
            QueueKind::Person,
            "Sign in to GitHub. Run again when you're ready.",
            None,
            false,
        )
    }

    /// The `command-line-tools` step: the quiet route, then Apple's dialog in the failure path.
    fn install_command_line_tools(&mut self) -> Result<Option<FailureKind>, EngineError> {
        let on = act::clt_placeholder_on(self.home);
        let _ = self.run_command(&on, "command-line-tools", false)?;
        let result = self.quiet_command_line_tools();
        let off = act::clt_placeholder_off(self.home);
        let _ = self.run_command(&off, "command-line-tools", false)?;
        result
    }

    fn quiet_command_line_tools(&mut self) -> Result<Option<FailureKind>, EngineError> {
        let list = act::clt_list(self.home);
        let output = match self.run_command(&list, "command-line-tools", true)? {
            CmdResult::Done(output) => output,
            CmdResult::Failed(kind) => return Ok(Some(kind)),
        };
        let Some(label) = act::newest_clt_label(&output.stdout_text()) else {
            return Ok(Some(FailureKind::NotFound));
        };
        let install = Beat::Admin(act::clt_install(self.home, &label));
        self.run_beats("command-line-tools", &[install])
    }

    /// The last step of the `gh` Item. Anything short of finishing leaves the bootstrap copy, so
    /// the `gh-handed-off` check brings this back.
    fn hand_off_gh(&mut self) -> Result<Option<FailureKind>, EngineError> {
        let brew_gh = PathBuf::from(act::brew_gh_path());
        let bootstrap = self.entry("gh").and_then(|entry| match &entry.recipe {
            Recipe::Install(install) => install.bootstrap.clone(),
            _ => None,
        });
        let early = bootstrap
            .map(|bootstrap| act::bootstrap_paths(self.home, &bootstrap).1)
            .filter(|binary| binary.exists());
        if let Some(early) = &early {
            match self.gh_sign_in_state(early)? {
                GhSignIn::SignedIn => {}
                // Nobody signed in, so there's nothing to carry over. Sign-in waits in the queue,
                // and a later Run finishes it with Homebrew's `gh`.
                GhSignIn::SignedOut => {
                    self.remove_bootstrap()?;
                    return Ok(None);
                }
                // Offline, stalled, or a copy that fails its pin. A real sign-in may be in there.
                GhSignIn::CantTell => return Ok(Some(FailureKind::Verify)),
            }
            let Some(account) = self.gh_account(early)? else {
                return Ok(Some(FailureKind::Verify));
            };
            if self.gh_account(&brew_gh)?.as_deref() != Some(account.as_str()) {
                return Ok(Some(FailureKind::Verify));
            }
        }
        if let Some(kind) = self.set_up_git_sign_in()? {
            return Ok(Some(kind));
        }
        self.remove_bootstrap()?;
        Ok(None)
    }

    fn gh_account(&mut self, program: &Path) -> Result<Option<String>, EngineError> {
        if !self.gh_may_run(program)? {
            return Ok(None);
        }
        let who = act::quiet_gh(self.home, program).args(["api", "user", "--jq", ".login"]);
        Ok(match self.run_command(&who, "", false)? {
            CmdResult::Done(output) if output.is_success() => {
                Some(output.stdout_text().trim().to_string()).filter(|login| !login.is_empty())
            }
            _ => None,
        })
    }

    /// Git's own GitHub sign-in is left to the person. Running `gh auth setup-git` for them
    /// writes credential settings that git combines with whatever else it reads: other config
    /// files, a repo at home, `XDG_CONFIG_HOME` from a shell file. Six review rounds each found
    /// another arrangement where it took over a helper the person relied on. So the Run never
    /// writes those settings, and says how to do it.
    fn set_up_git_sign_in(&mut self) -> Result<Option<FailureKind>, EngineError> {
        let brew_gh = PathBuf::from(act::brew_gh_path());
        if self.gh_account(&brew_gh)?.is_some() {
            self.emit(EventKind::RunWarning {
                text: "To let git use your GitHub sign-in, run `gh auth setup-git` in Terminal."
                    .into(),
            });
        }
        Ok(None)
    }

    fn pending_admin(&self) -> bool {
        self.plan.entries.iter().any(|entry| {
            self.scope.contains(&entry.id)
                && entry.stage == Stage::Admin
                && self.states.get(&entry.id) == Some(&ItemState::Pending)
        })
    }

    fn discard(&mut self, why: &str) {
        if let Some(password) = self.password.take() {
            // Dropping it zeroes and frees it, so the range has to be released first.
            admin::unlock_memory(password.expose().as_bytes());
            drop(password);
            if !self.admin.forget(self.runner, self.home) {
                self.sudo_may_be_live = true;
                self.emit(EventKind::RunWarning {
                    text: "sudo's sign-in couldn't be ended. It runs out by itself within a few minutes."
                        .into(),
                });
            }
            self.held_since = None;
            self.emit(EventKind::AdminDiscarded {
                why: why.to_string(),
            });
        }
    }

    fn hold_expired(&self) -> bool {
        self.held_since
            .is_some_and(|start| self.now().saturating_sub(start) >= ADMIN_HOLD_MS)
    }

    fn run_stage(&mut self, stage: Stage) -> Result<(), EngineError> {
        // Checks from the first pass must not answer verify. The Machine may have changed.
        self.probe_cache.clear();
        let ids: Vec<String> = self
            .plan
            .entries
            .iter()
            .filter(|entry| self.scope.contains(&entry.id) && entry.stage == stage)
            .map(|entry| entry.id.clone())
            .collect();
        let mut index = 0;
        while index < ids.len() {
            if self.hold_expired() {
                self.discard("the password was held for 60 minutes");
            }
            if self.cancel.is_cancelled() {
                self.cancelled = true;
                break;
            }
            let id = ids[index].clone();
            if stage == Stage::User
                && self
                    .entry(&id)
                    .is_some_and(|entry| entry.group == RunGroup::GitValues)
            {
                // Links go after third-party Items and before git settings.
                self.apply_dotfiles()?;
                // A link may now carry `~/.gitconfig` into the repo, and git settings must not
                // write through it.
                if self.states.get(&id) == Some(&ItemState::Pending)
                    && let Some(entry) = self.entry(&id).cloned()
                    && self.writes_through_link(&entry)
                {
                    self.move_item(
                        &id,
                        ItemState::Skipped {
                            reason: SkipReason::WritesThroughLink,
                        },
                    )?;
                }
            }
            if self.states.get(&id) != Some(&ItemState::Pending) {
                index += 1;
                continue;
            }
            if self.consider_dependency(&id, stage)? {
                index += 1;
                continue;
            }
            if self.same_restart_group(&id) {
                let batch = self.collect_batch(&ids[index..]);
                let count = batch.len();
                let mut runnable = Vec::new();
                for member in &batch {
                    if self.states.get(member) != Some(&ItemState::Pending) {
                        continue;
                    }
                    if self.consider_dependency(member, stage)? {
                        continue;
                    }
                    runnable.push(member.clone());
                }
                if !runnable.is_empty() {
                    self.run_setting_batch(&runnable)?;
                }
                index += count;
            } else {
                self.run_one(&id)?;
                index += 1;
            }
        }
        Ok(())
    }

    /// `true` means this Item is finished for this stage and must not be acted on.
    ///
    /// `sort_for_run` places an entry after whatever it requires, even when that requirement is in
    /// the other stage. An Admin Stage Item can therefore sit after a third-party tap. Running the
    /// tap to unblock it would execute User Stage code while the password is held, so the Admin
    /// Item waits in the Needs-You Queue instead.
    fn consider_dependency(&mut self, id: &str, stage: Stage) -> Result<bool, EngineError> {
        let Some(entry) = self.entry(id).cloned() else {
            return Ok(false);
        };
        for requirement in entry.recipe.requires() {
            if self.requirement_ok(requirement) {
                continue;
            }
            let requirement_stage = self.entry(requirement).map(|entry| entry.stage);
            let waiting_on_user = stage == Stage::Admin && requirement_stage == Some(Stage::User);
            if waiting_on_user {
                self.defer_admin_for_user_work(id)?;
                return Ok(true);
            }
            self.move_item(
                id,
                ItemState::Blocked {
                    by: requirement.clone(),
                },
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    fn defer_admin_for_user_work(&mut self, id: &str) -> Result<(), EngineError> {
        self.open_queue(
            id,
            "needs-admin",
            QueueKind::NeedsAdmin,
            "This waits until the password is discarded, because it depends on a User Stage Item.",
            None,
            true,
        )?;
        self.become_needs_you(id, Some(FailureKind::NeedsAdmin))
    }

    fn become_needs_you(
        &mut self,
        id: &str,
        cause: Option<FailureKind>,
    ) -> Result<(), EngineError> {
        let current = self.states.get(id).cloned().unwrap_or(ItemState::Pending);
        if matches!(current, ItemState::Pending) {
            self.move_item(id, ItemState::Checking)?;
        }
        self.move_item(id, ItemState::NeedsYou { cause })
    }

    fn requirement_ok(&self, id: &str) -> bool {
        match self.states.get(id) {
            Some(ItemState::Satisfied { .. } | ItemState::Unknown) => true,
            Some(ItemState::Pending) => false,
            Some(_) => false,
            None => {
                self.record.outcomes.get(id).is_some_and(|outcome| {
                    outcome.state == "satisfied" || outcome.state == "unknown"
                }) || self.record.last_applied.contains_key(id)
            }
        }
    }

    fn same_restart_group(&self, id: &str) -> bool {
        self.entry(id).is_some_and(|entry| {
            matches!(
                &entry.recipe,
                Recipe::Setting(setting)
                    if matches!(
                        setting.effect,
                        Some(
                            Effect::RestartDock
                                | Effect::RestartFinder
                                | Effect::RestartSystemuiserver
                                | Effect::RestartControlcenter
                        )
                    )
            )
        })
    }

    fn collect_batch(&self, ids: &[String]) -> Vec<String> {
        let Some(first) = ids.first() else {
            return Vec::new();
        };
        let effect = self.entry(first).and_then(|entry| match &entry.recipe {
            Recipe::Setting(setting) => setting.effect,
            _ => None,
        });
        let mut batch = vec![first.clone()];
        for id in ids.iter().skip(1) {
            if self.states.get(id) != Some(&ItemState::Pending) {
                break;
            }
            let same = self.entry(id).is_some_and(|entry| match &entry.recipe {
                Recipe::Setting(setting) => setting.effect == effect,
                _ => false,
            });
            if !same {
                break;
            }
            batch.push(id.clone());
        }
        batch
    }

    fn run_setting_batch(&mut self, ids: &[String]) -> Result<(), EngineError> {
        let mut effect = Effect::None;
        for id in ids {
            self.begin_act(id)?;
            effect = self
                .entry(id)
                .and_then(|entry| match &entry.recipe {
                    Recipe::Setting(setting) => setting.effect,
                    _ => None,
                })
                .unwrap_or(Effect::None);
            self.write_setting(id)?;
        }
        if let Some(command) = restart_command(self.home, effect) {
            let _ = self.run_command(
                &command,
                ids.first().map(String::as_str).unwrap_or(""),
                true,
            )?;
        }
        if effect == Effect::LogOut {
            self.needs_logout = true;
        }
        for id in ids {
            self.move_item(id, ItemState::Verifying)?;
            self.verify_setting(id)?;
        }
        Ok(())
    }

    fn run_one(&mut self, id: &str) -> Result<(), EngineError> {
        let Some(entry) = self.entry(id).cloned() else {
            return Ok(());
        };
        if !self.admin_account && needs_admin_rights(&entry) {
            self.open_queue(
                id,
                "needs-admin",
                QueueKind::NeedsAdmin,
                "This needs an admin account, because the destination isn't writable.",
                None,
                true,
            )?;
            return self.become_needs_you(id, Some(FailureKind::NeedsAdmin));
        }
        self.begin_act(id)?;
        match &entry.recipe {
            Recipe::Install(_) => self.run_install(id)?,
            Recipe::Setting(_) => {
                self.write_setting(id)?;
                if let Some(effect) = self.entry(id).and_then(|entry| match &entry.recipe {
                    Recipe::Setting(setting) => setting.effect,
                    _ => None,
                }) {
                    if effect == Effect::LogOut {
                        self.needs_logout = true;
                    }
                    if let Some(command) = restart_command(self.home, effect) {
                        let _ = self.run_command(&command, id, true)?;
                    }
                }
                self.move_item(id, ItemState::Verifying)?;
                self.verify_setting(id)?;
            }
            Recipe::MachineValues(_) => self.run_machine_values(id)?,
        }
        Ok(())
    }

    fn run_install(&mut self, id: &str) -> Result<(), EngineError> {
        let steps = self
            .entry(id)
            .map(|entry| entry.recipe.steps().to_vec())
            .unwrap_or_default();
        for (index, step) in steps.iter().enumerate() {
            if self.cancel.is_cancelled() {
                self.cancelled = true;
                self.move_item(
                    id,
                    ItemState::Failed {
                        kind: FailureKind::Interrupted,
                    },
                )?;
                return Ok(());
            }
            if let Some(unless) = step.unless()
                && self.eval_probe(unless)? == Verdict::Pass
            {
                continue;
            }
            if let Some(stop) = self.person_step(id, index, step)? {
                if stop {
                    return Ok(());
                }
                continue;
            }
            if step.is_admin() && self.password.is_none() {
                return self.queue_admin(id, step);
            }
            self.emit(EventKind::ItemStep {
                item: id.to_string(),
                index,
                step_type: step.type_name().to_string(),
                phase: StepPhase::Started,
            });
            let beats = act::step_beats(self.home, step);
            let failed = match step {
                Step::Builtin {
                    name: BuiltinStep::CommandLineTools,
                    ..
                } => self.install_command_line_tools()?,
                _ => self.run_beats(id, &beats)?,
            };
            let failed = match (failed, step) {
                (
                    None,
                    Step::Builtin {
                        name: BuiltinStep::GhHandOff,
                        ..
                    },
                ) => self.hand_off_gh()?,
                (failed, _) => failed,
            };
            if let Some(kind) = failed {
                self.emit(EventKind::ItemStep {
                    item: id.to_string(),
                    index,
                    step_type: step.type_name().to_string(),
                    phase: StepPhase::Finished,
                });
                return self.finish_step_failure(id, step, kind);
            }
            self.emit(EventKind::ItemStep {
                item: id.to_string(),
                index,
                step_type: step.type_name().to_string(),
                phase: StepPhase::Finished,
            });
        }
        self.move_item(id, ItemState::Verifying)?;
        self.verify_install(id)
    }

    fn person_step(
        &mut self,
        id: &str,
        index: usize,
        step: &Step,
    ) -> Result<Option<bool>, EngineError> {
        match step {
            Step::NeedsYou {
                id: step_id,
                text,
                link,
                completion,
                ..
            } => {
                if self.gate_done(id, step_id, completion)? {
                    self.close_queue_id(&queue_id(id, step_id));
                    return Ok(Some(false));
                }
                self.open_queue(id, step_id, QueueKind::Person, text, link.clone(), false)?;
                self.emit_step(id, index, step, StepPhase::Finished);
                self.become_needs_you(id, None)?;
                Ok(Some(true))
            }
            Step::AcceptLicense {
                id: step_id,
                license,
                text,
                link,
                admin,
                ..
            } => {
                if self.license_done(id, step_id, *license) {
                    if *admin && self.password.is_none() {
                        self.queue_admin(id, step)?;
                        return Ok(Some(true));
                    }
                    let command = match license {
                        License::Xcode => act::xcode_license(self.home),
                        License::AndroidSdk => act::android_licenses(self.home),
                    };
                    let beat = if *admin {
                        Beat::Admin(command)
                    } else {
                        Beat::Run(command)
                    };
                    self.emit_step(id, index, step, StepPhase::Started);
                    if let Some(kind) = self.run_beats(id, &[beat])? {
                        self.emit_step(id, index, step, StepPhase::Finished);
                        self.finish_step_failure(id, step, kind)?;
                        return Ok(Some(true));
                    }
                    let name = license_name(*license);
                    if !self
                        .record
                        .accepted
                        .iter()
                        .any(|accepted| accepted.license == name)
                    {
                        self.record.accepted.push(AcceptedLicense {
                            license: name.into(),
                            at: self.now(),
                        });
                    }
                    self.close_queue_id(&queue_id(id, step_id));
                    self.emit_step(id, index, step, StepPhase::Finished);
                    return Ok(Some(false));
                }
                self.open_queue(id, step_id, QueueKind::License, text, link.clone(), *admin)?;
                self.emit_step(id, index, step, StepPhase::Finished);
                self.become_needs_you(id, None)?;
                Ok(Some(true))
            }
            _ => Ok(None),
        }
    }

    fn gate_done(
        &mut self,
        item: &str,
        step_id: &str,
        completion: &Completion,
    ) -> Result<bool, EngineError> {
        if self.queue_finished(item, step_id) {
            return Ok(true);
        }
        if let Completion::Until { until } = completion {
            return Ok(self.eval_probe(until)? == Verdict::Pass);
        }
        Ok(false)
    }

    fn license_done(&self, item: &str, step_id: &str, license: License) -> bool {
        self.queue_finished(item, step_id)
            || self
                .record
                .accepted
                .iter()
                .any(|accepted| accepted.license == license_name(license))
    }

    fn queue_finished(&self, item: &str, step_id: &str) -> bool {
        self.record.queue.iter().any(|entry| {
            entry.item == item && entry.step_id == step_id && entry.finished_at.is_some()
        })
    }

    fn queue_admin(&mut self, id: &str, _step: &Step) -> Result<(), EngineError> {
        let third_party = self
            .entry(id)
            .is_some_and(|entry| !entry.recipe.taps().is_empty());
        if third_party {
            let name = self
                .entry(id)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| id.into());
            self.open_queue(
                id,
                "install-it-yourself",
                QueueKind::NeedsAdmin,
                &format!("Install {name} yourself. Third-party code never gets the password."),
                None,
                false,
            )?;
        } else {
            if !self.record.learned_admin.iter().any(|known| known == id) {
                self.record.learned_admin.push(id.to_string());
            }
            self.open_queue(
                id,
                "needs-admin",
                QueueKind::NeedsAdmin,
                "This needs your password, which has already been discarded.",
                None,
                true,
            )?;
        }
        self.become_needs_you(id, Some(FailureKind::NeedsAdmin))
    }

    fn finish_step_failure(
        &mut self,
        id: &str,
        step: &Step,
        kind: FailureKind,
    ) -> Result<(), EngineError> {
        if kind == FailureKind::NeedsAdmin || (kind == FailureKind::Stalled && step.is_admin()) {
            if kind == FailureKind::Stalled {
                self.open_queue(
                    id,
                    "stalled",
                    QueueKind::NeedsAdmin,
                    "This admin step stalled. It may be waiting on a person.",
                    None,
                    true,
                )?;
            } else {
                self.queue_admin(id, step)?;
                return Ok(());
            }
            return self.become_needs_you(id, Some(kind));
        }
        if matches!(
            step,
            Step::Builtin {
                name: BuiltinStep::CommandLineTools,
                ..
            }
        ) && kind != FailureKind::Interrupted
        {
            // Once, outside the retries. It's a window on the person's screen.
            let _ = self.runner.run(&act::clt_dialog(self.home), &mut |_, _| {});
            self.open_queue(
                id,
                "command-line-tools",
                QueueKind::Person,
                "Install Command Line Tools. Soulcaster will continue once they're on this Machine.",
                None,
                true,
            )?;
            return self.become_needs_you(id, None);
        }
        self.move_item(id, ItemState::Failed { kind })?;
        Ok(())
    }

    fn run_beats(&mut self, id: &str, beats: &[Beat]) -> Result<Option<FailureKind>, EngineError> {
        let mut previous: Option<(Command, CommandOutput)> = None;
        for beat in beats {
            match beat {
                Beat::Admin(command) | Beat::AdminBrew(command) => {
                    let elevate_itself = matches!(beat, Beat::AdminBrew(_));
                    match self.run_admin(command, elevate_itself, id)? {
                        CmdResult::Failed(kind) => return Ok(Some(kind)),
                        CmdResult::Done(output) => previous = Some((command.clone(), output)),
                    }
                }
                Beat::Run(command) => match self.run_command(command, id, true)? {
                    CmdResult::Failed(kind) => {
                        if is_signature_program(command)
                            && !matches!(
                                kind,
                                FailureKind::Network | FailureKind::NotFound | FailureKind::Stalled
                            )
                        {
                            return Ok(Some(FailureKind::Signature));
                        }
                        return Ok(Some(kind));
                    }
                    CmdResult::Done(output) => previous = Some((command.clone(), output)),
                },
                Beat::Sha256(expected) => {
                    let stdout = previous
                        .as_ref()
                        .map(|(_, output)| output.stdout_text())
                        .unwrap_or_default();
                    if !sha_matches(&stdout, expected) {
                        return Ok(Some(FailureKind::Checksum));
                    }
                }
                Beat::Team(team) => {
                    let Some((command, output)) = &previous else {
                        return Ok(Some(FailureKind::Signature));
                    };
                    let stdout = output.stdout_text();
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let ok = if command.program.ends_with("pkgutil") {
                        pkg_signer_has_team(&stdout, team)
                    } else {
                        codesign_team_ok(&stdout, &stderr, team)
                    };
                    if !ok {
                        return Ok(Some(FailureKind::Signature));
                    }
                }
            }
        }
        Ok(None)
    }

    fn verify_install(&mut self, id: &str) -> Result<(), EngineError> {
        let checks = self
            .entry(id)
            .and_then(|entry| match &entry.recipe {
                Recipe::Install(install) => Some(install.checks.clone()),
                _ => None,
            })
            .unwrap_or_default();
        match self.judge_probes(&checks)? {
            Verdict::Pass => {
                let assurance = Assurance::Proven;
                self.assurances.insert(id.to_string(), assurance);
                self.close_retry_queue(id);
                self.note_satisfied_extras(id);
                self.move_item(id, ItemState::Satisfied { assurance })?;
            }
            Verdict::Unknown => self.move_item(id, ItemState::Unknown)?,
            Verdict::Fail | Verdict::Unset => {
                self.move_item(
                    id,
                    ItemState::Failed {
                        kind: FailureKind::Verify,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn write_setting(&mut self, id: &str) -> Result<(), EngineError> {
        let Some(entry) = self.entry(id).cloned() else {
            return Ok(());
        };
        let Recipe::Setting(setting) = &entry.recipe else {
            return Ok(());
        };
        let Some(value) = entry.value.clone() else {
            return Ok(());
        };
        for store in &setting.stores.clone() {
            self.write_store(id, store, &value)?;
        }
        Ok(())
    }

    fn write_store(
        &mut self,
        id: &str,
        store: &Store,
        value: &SettingValue,
    ) -> Result<(), EngineError> {
        match store {
            Store::Defaults { .. } => {
                if let Some(command) = act::defaults_write(self.home, store, value) {
                    let _ = self.run_command(&command, id, true)?;
                }
            }
            Store::GitConfig { key } => {
                if !self.command_line_tools_present()? {
                    return Ok(());
                }
                if !self.priors.contains_key(id)
                    && let Some(current) = self.read_git_config(key)?
                {
                    self.priors.insert(id.to_string(), Value::String(current));
                }
                let text = rendered_git(value);
                let mut command = act::git_command(self.home);
                command.args = vec![
                    "config".into(),
                    "--file".into(),
                    act::git_file(self.home).to_string_lossy().into_owned(),
                    git_key(key).into(),
                    text,
                ];
                let _ = self.run_command(&command, id, true)?;
            }
            Store::Builtin { name } => match name {
                soulcaster_core::recipe::BuiltinStore::LibraryVisible => {
                    let visible = matches!(value, SettingValue::Bool(true));
                    let _ = self.run_command(&act::library_write(self.home, visible), id, true)?;
                }
                soulcaster_core::recipe::BuiltinStore::DockTiles => {
                    if let SettingValue::List(apps) = value {
                        // Without knowing what's there, the Dock is left as it is.
                        let CmdResult::Done(listing) =
                            self.run_command(&act::dock_list(self.home), id, false)?
                        else {
                            return Ok(());
                        };
                        if !listing.is_success() {
                            return Ok(());
                        }
                        let tiles = dock_app_tiles(&listing.stdout_text());
                        for command in act::dock_write(self.home, &tiles, apps) {
                            let _ = self.run_command(&command, id, true)?;
                        }
                    }
                }
                soulcaster_core::recipe::BuiltinStore::SidebarFavorites => {}
            },
        }
        Ok(())
    }

    fn verify_setting(&mut self, id: &str) -> Result<(), EngineError> {
        let Some(entry) = self.entry(id).cloned() else {
            return Ok(());
        };
        let Recipe::Setting(setting) = entry.recipe.clone() else {
            return Ok(());
        };
        match self.judge_setting(id, &setting, entry.value.as_ref())? {
            Verdict::Pass => {
                let assurance = assurance_of(&setting, self.macos.as_deref());
                self.assurances.insert(id.to_string(), assurance);
                self.note_satisfied_extras(id);
                self.move_item(id, ItemState::Satisfied { assurance })?;
            }
            // A read that still can't answer is `unknown`. A value that is still unset after the
            // write did not land, so verify failed.
            Verdict::Unknown => self.move_item(id, ItemState::Unknown)?,
            Verdict::Fail | Verdict::Unset => {
                self.move_item(
                    id,
                    ItemState::Failed {
                        kind: FailureKind::Verify,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn run_machine_values(&mut self, id: &str) -> Result<(), EngineError> {
        let values = self
            .entry(id)
            .and_then(|entry| match &entry.recipe {
                Recipe::MachineValues(recipe) => Some(recipe.values.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut resolved = Vec::new();
        for value in &values {
            if let Some(found) = self.lookup_machine_value(&value.name, &value.store)? {
                self.record
                    .local_values
                    .entry(value.name.clone())
                    .or_insert_with(|| found.clone());
                resolved.push((value.store.clone(), found));
            } else {
                self.open_queue(
                    id,
                    &value.name,
                    QueueKind::Person,
                    &value.label,
                    None,
                    false,
                )?;
            }
        }
        if resolved.len() != values.len() {
            self.become_needs_you(id, None)?;
            return Ok(());
        }
        for (store, text) in &resolved {
            self.write_store(id, store, &SettingValue::String(text.clone()))?;
        }
        self.move_item(id, ItemState::Verifying)?;
        let mut ok = true;
        for value in &values {
            let Some(found) = self.lookup_git(&value.store)? else {
                ok = false;
                break;
            };
            let wanted = self
                .record
                .local_values
                .get(&value.name)
                .cloned()
                .unwrap_or_default();
            if found != wanted {
                ok = false;
                break;
            }
        }
        if ok {
            self.move_item(
                id,
                ItemState::Satisfied {
                    assurance: Assurance::Proven,
                },
            )?;
        } else {
            self.move_item(
                id,
                ItemState::Failed {
                    kind: FailureKind::Verify,
                },
            )?;
        }
        Ok(())
    }

    fn lookup_machine_value(
        &mut self,
        name: &str,
        store: &Store,
    ) -> Result<Option<String>, EngineError> {
        if let Some(found) = self.lookup_git(store)? {
            return Ok(Some(found));
        }
        Ok(self.record.local_values.get(name).cloned())
    }

    fn lookup_git(&mut self, store: &Store) -> Result<Option<String>, EngineError> {
        let Store::GitConfig { key } = store else {
            return Ok(None);
        };
        self.read_git_config(key)
    }

    /// `None` when Command Line Tools are not there yet, or the key is unset. Neither runs the stub.
    fn read_git_config(
        &mut self,
        key: &soulcaster_core::recipe::GitKey,
    ) -> Result<Option<String>, EngineError> {
        if !self.command_line_tools_present()? {
            return Ok(None);
        }
        let mut command = act::git_command(self.home);
        command.args = vec![
            "config".into(),
            "--file".into(),
            act::git_file(self.home).to_string_lossy().into_owned(),
            "--get".into(),
            git_key(key).into(),
        ];
        match self.run_command(&command, "", false)? {
            CmdResult::Done(output) if output.is_success() => {
                let text = output.stdout_text().trim().to_string();
                if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                }
            }
            _ => Ok(None),
        }
    }

    fn remove_bootstrap(&self) -> Result<(), EngineError> {
        let dir = support_dir(self.home).join("bootstrap-gh");
        if dir.starts_with(support_dir(self.home)) && dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| EngineError::RecordIo {
                path: dir,
                detail: error.to_string(),
            })?;
        }
        Ok(())
    }

    fn begin_act(&mut self, id: &str) -> Result<(), EngineError> {
        self.move_item(id, ItemState::Checking)?;
        self.move_item(id, ItemState::Acting)?;
        self.mark_running(id)
    }

    fn mark_running(&mut self, id: &str) -> Result<(), EngineError> {
        self.record.running = Some(InProgress {
            run_id: self.run_id.clone(),
            item: id.to_string(),
        });
        self.record.save(self.home)
    }

    fn move_item(&mut self, id: &str, to: ItemState) -> Result<(), EngineError> {
        let from = self.states.get(id).cloned().unwrap_or(ItemState::Pending);
        if !from.can_move_to(&to) {
            return Err(EngineError::IllegalTransition {
                item: id.to_string(),
                from: from.name().to_string(),
                to: to.name().to_string(),
            });
        }
        let terminal = to.is_terminal();
        if terminal {
            self.note_outcome(id, &to);
            self.record.running = None;
        }
        self.states.insert(id.to_string(), to.clone());
        if self.reporting {
            self.emit(EventKind::ItemState {
                item: id.to_string(),
                from,
                to,
            });
        }
        if terminal {
            if let Ok(text) = serde_json::to_string(&self.record) {
                self.debug_no_secret(&text);
            }
            self.record.save(self.home)?;
        }
        Ok(())
    }

    fn note_outcome(&mut self, id: &str, state: &ItemState) {
        let outcome = ItemOutcome {
            state: state.name().to_string(),
            failure: match state {
                ItemState::Failed { kind } => Some(*kind),
                ItemState::NeedsYou { cause } => *cause,
                _ => None,
            },
            blocked_by: match state {
                ItemState::Blocked { by } => Some(by.clone()),
                _ => None,
            },
            assurance: match state {
                ItemState::Satisfied { assurance } => Some(*assurance),
                _ => None,
            },
            skip: match state {
                ItemState::Skipped { reason } => Some(*reason),
                _ => None,
            },
        };
        if let ItemState::Skipped { reason } = state
            && reason.is_unavailable_here()
        {
            self.record
                .unavailable
                .insert(id.to_string(), reason_label(*reason).to_string());
        } else if self.record.unavailable.get(id).map(String::as_str)
            != Some(reason_label(SkipReason::MarkedByPerson))
        {
            // The Machine changed, so what it couldn't do no longer hides the entry from Drift.
            self.record.unavailable.remove(id);
        }
        if let ItemState::Satisfied { assurance } = state {
            self.remember_applied(id, *assurance);
        }
        self.record.outcomes.insert(id.to_string(), outcome);
    }

    fn remember_applied(&mut self, id: &str, assurance: Assurance) {
        let value = self
            .entry(id)
            .and_then(|entry| entry.value.as_ref().map(setting_json))
            .unwrap_or(Value::Bool(true));
        let prior = self
            .record
            .last_applied
            .get(id)
            .and_then(|applied| applied.prior_value.clone())
            .or_else(|| self.priors.get(id).cloned());
        if let Some(entry) = self.entry(id)
            && let Why::PulledIn { by } = &entry.why
        {
            self.record.pulled_in.insert(id.to_string(), by.clone());
        }
        self.record.last_applied.insert(
            id.to_string(),
            LastApplied {
                value,
                applied_at: self.now(),
                run_id: self.run_id.clone(),
                catalog_version: self.catalog_version.to_string(),
                macos_version: self.macos.clone(),
                assurance,
                prior_value: prior,
            },
        );
    }

    fn open_queue(
        &mut self,
        item: &str,
        step_id: &str,
        kind: QueueKind,
        text: &str,
        link: Option<String>,
        needs_password: bool,
    ) -> Result<(), EngineError> {
        let id = queue_id(item, step_id);
        if self
            .record
            .queue
            .iter()
            .any(|entry| entry.id == id && entry.is_open())
        {
            return Ok(());
        }
        let entry = QueueItem {
            id: id.clone(),
            item: item.to_string(),
            step_id: step_id.to_string(),
            kind,
            text: text.to_string(),
            link: link.clone(),
            opened_at: self.now(),
            finished_at: None,
            finished_by: None,
            needs_password,
        };
        self.record.queue.push(entry);
        self.emit(EventKind::QueueOpened {
            entry: QueueEntry {
                id,
                item: item.to_string(),
                kind,
                text: text.to_string(),
                detail: None,
                link,
                needs_password,
            },
        });
        Ok(())
    }

    fn close_retry_queue(&mut self, item: &str) {
        let ids: Vec<String> = self
            .record
            .queue
            .iter()
            .filter(|entry| {
                entry.item == item && entry.is_open() && entry.kind == QueueKind::NeedsAdmin
            })
            .map(|entry| entry.id.clone())
            .collect();
        for id in ids {
            self.close_queue_id(&id);
        }
    }

    fn close_queue_id(&mut self, id: &str) {
        let now = self.now();
        let mut closed = false;
        for entry in &mut self.record.queue {
            if entry.id == id && entry.is_open() {
                entry.finished_at = Some(now);
                entry.finished_by = Some("run".to_string());
                closed = true;
            }
        }
        if closed {
            self.emit(EventKind::QueueClosed { id: id.to_string() });
        }
    }

    fn run_command(
        &mut self,
        command: &Command,
        item: &str,
        log: bool,
    ) -> Result<CmdResult, EngineError> {
        let mut attempt = 0;
        loop {
            let ran = self.run_once(command, item, log);
            match outcome_of(ran) {
                Ok(output) => return Ok(CmdResult::Done(output)),
                Err(FailureKind::Network) if attempt < 2 => {
                    self.backoff(attempt);
                    attempt += 1;
                }
                Err(kind) => return Ok(CmdResult::Failed(kind)),
            }
        }
    }

    /// One admin step. Only it gets the helper, and only while it runs: the window closes and
    /// `sudo`'s timestamp ends after every try, so nothing answers during a backoff or between
    /// steps.
    fn run_admin(
        &mut self,
        command: &Command,
        elevate_itself: bool,
        id: &str,
    ) -> Result<CmdResult, EngineError> {
        let budget = if elevate_itself {
            Budget::AsNeeded
        } else {
            Budget::One
        };
        let mut attempt = 0;
        loop {
            if self.hold_expired() {
                self.discard("the password was held for 60 minutes");
            }
            let Some(password) = &self.password else {
                return Ok(CmdResult::Failed(FailureKind::NeedsAdmin));
            };
            let window = match self.admin.open(password, budget) {
                Ok(window) => window,
                Err(why) => {
                    self.emit(EventKind::RunWarning {
                        text: format!("An admin step couldn't start: {why}"),
                    });
                    return Ok(CmdResult::Failed(FailureKind::NeedsAdmin));
                }
            };
            let real = if elevate_itself {
                window.may_elevate(command.clone())
            } else {
                window.elevated(command.clone())
            };
            let ran = self.run_once(&real, id, true);
            drop(window);
            if !self.admin.forget(self.runner, self.home) {
                // Root may still be reachable, so no more admin work this Run.
                self.sudo_may_be_live = true;
                self.discard("sudo's sign-in couldn't be ended");
            }
            match outcome_of(ran) {
                Ok(output) => return Ok(CmdResult::Done(output)),
                // It would have been retried, but the password went with the failed clear. It's
                // admin work for later, not a failure of the step, and not a reason for a dialog.
                Err(FailureKind::Network) if self.password.is_none() => {
                    return Ok(CmdResult::Failed(FailureKind::NeedsAdmin));
                }
                // With the password gone there's no retry, so there's nothing to wait for.
                Err(FailureKind::Network) if attempt < 2 && self.password.is_some() => {
                    self.backoff(attempt);
                    attempt += 1;
                }
                Err(kind) => return Ok(CmdResult::Failed(kind)),
            }
        }
    }

    fn run_once(
        &mut self,
        command: &Command,
        item: &str,
        log: bool,
    ) -> Result<CommandOutput, soulcaster_core::command::SpawnError> {
        let runner = self.runner;
        if !log || item.is_empty() || !self.reporting {
            return runner.run(command, &mut |_, _| {});
        }
        // The command runs on its own thread so a batch goes out on time even if the command
        // then goes quiet for twenty minutes. Only this thread touches the Engine.
        let (lines, received) = mpsc::channel();
        thread::scope(|scope| {
            let worker = scope.spawn(move || {
                runner.run(command, &mut |stream, line| {
                    let _ = lines.send((stream, line.to_string()));
                })
            });
            loop {
                let wait = self
                    .log_since
                    .map_or(LOG_BATCH, |since| LOG_BATCH.saturating_sub(since.elapsed()));
                match received.recv_timeout(wait) {
                    Ok((stream, line)) => self.note_output(item, stream, &line),
                    Err(RecvTimeoutError::Timeout) => self.flush_output(),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            self.flush_output();
            worker
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    fn note_output(&mut self, item: &str, stream: OutputStream, line: &str) {
        if !self.reporting {
            return;
        }
        let masked = mask_secrets(line);
        // Past the cap the line goes nowhere: not the log file, and not the event stream.
        if !self.append_item_log(item, &masked) {
            return;
        }
        let stream = match stream {
            OutputStream::Stdout => LogStream::Stdout,
            OutputStream::Stderr => LogStream::Stderr,
        };
        if self.log_since.is_none() {
            self.log_since = Some(Instant::now());
        }
        self.log_pending.push((item.to_string(), stream, masked));
        if self
            .log_since
            .is_some_and(|start| start.elapsed() >= LOG_BATCH)
        {
            self.flush_output();
        }
    }

    fn flush_output(&mut self) {
        if self.log_pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.log_pending);
        self.log_since = None;
        let mut batch: Vec<(String, LogStream, Vec<String>)> = Vec::new();
        for (item, stream, line) in pending {
            if let Some(last) = batch.last_mut()
                && last.0 == item
                && last.1 == stream
            {
                last.2.push(line);
            } else {
                batch.push((item, stream, vec![line]));
            }
        }
        for (item, stream, lines) in batch {
            self.emit(EventKind::ItemLog {
                item,
                stream,
                lines,
            });
        }
    }

    fn backoff(&self, attempt: u32) {
        let delay = if attempt == 0 {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(120)
        };
        self.clock.wait(delay);
    }

    fn read_command(&mut self, command: &Command) -> Result<String, EngineError> {
        match self.run_command(command, "", false)? {
            CmdResult::Done(output) if output.is_success() => Ok(output.stdout_text()),
            CmdResult::Done(output) => Err(EngineError::Preflight {
                detail: format!("{} exited {:?}", command.program.display(), output.end),
            }),
            CmdResult::Failed(kind) => Err(EngineError::Preflight {
                detail: format!("{} failed ({})", command.program.display(), kind_name(kind)),
            }),
        }
    }

    fn entry(&self, id: &str) -> Option<&PlanEntry> {
        self.plan.entries.iter().find(|entry| entry.id == id)
    }

    fn emit_step(&mut self, id: &str, index: usize, step: &Step, phase: StepPhase) {
        self.emit(EventKind::ItemStep {
            item: id.to_string(),
            index,
            step_type: step.type_name().to_string(),
            phase,
        });
    }

    fn finish(&mut self) -> Result<RunReport, EngineError> {
        self.discard("the Run finished");
        if self.needs_logout {
            self.emit(EventKind::RunWarning {
                text: "Log out to finish applying these settings.".into(),
            });
        }
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for state in self.states.values() {
            *counts.entry(state.name().to_string()).or_insert(0) += 1;
        }
        let queue = self.event_queue();
        self.emit(EventKind::RunFinished {
            counts,
            queue: queue.clone(),
        });
        self.record.running = None;
        self.record.save(self.home)?;
        self.write_run_files()?;
        self.prune_runs();
        self.closed = true;
        Ok(RunReport {
            run_id: self.run_id.clone(),
            states: self.states.clone(),
            queue,
            cancelled: self.cancelled,
        })
    }

    fn event_queue(&self) -> Vec<QueueEntry> {
        self.record
            .queue
            .iter()
            .filter(|entry| entry.is_open())
            .map(|entry| QueueEntry {
                id: entry.id.clone(),
                item: entry.item.clone(),
                kind: entry.kind,
                text: entry.text.clone(),
                detail: None,
                link: entry.link.clone(),
                needs_password: entry.needs_password,
            })
            .collect()
    }

    fn prepare_run_dir(&self) {
        let dir = support_dir(self.home).join("runs").join(&self.run_id);
        let _ = fs::create_dir_all(dir.join("logs"));
        // The design keeps these in a folder only the user can read, from the first event on.
        let _ = fs::set_permissions(support_dir(self.home), fs::Permissions::from_mode(0o700));
    }

    fn append_event(&self, event: &Event) {
        // `item.log` is lossy by design and has its own file. This one holds the lossless events.
        if matches!(event.kind, EventKind::ItemLog { .. }) {
            return;
        }
        let path = support_dir(self.home)
            .join("runs")
            .join(&self.run_id)
            .join("events.ndjson");
        let Ok(mut file) = private_append(&path) else {
            return;
        };
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(file, "{line}");
        }
    }

    /// False once this Item's log is full.
    fn append_item_log(&mut self, item: &str, line: &str) -> bool {
        let used = self.log_bytes.entry(item.to_string()).or_insert(0);
        if *used >= LOG_CAP {
            return false;
        }
        *used += line.len() + 1;
        let path = support_dir(self.home)
            .join("runs")
            .join(&self.run_id)
            .join("logs")
            .join(log_file_name(item));
        if let Ok(mut file) = private_append(&path) {
            let _ = writeln!(file, "{line}");
        }
        true
    }

    fn write_run_files(&self) -> Result<(), EngineError> {
        let dir = support_dir(self.home).join("runs").join(&self.run_id);
        fs::create_dir_all(&dir).map_err(|error| EngineError::RecordIo {
            path: dir.clone(),
            detail: error.to_string(),
        })?;
        let summary = serde_json::json!({
            "runId": self.run_id,
            "states": self.states,
        });
        fs::write(
            dir.join("run.json"),
            serde_json::to_vec_pretty(&summary).unwrap_or_default(),
        )
        .map_err(|error| EngineError::RecordIo {
            path: dir.join("run.json"),
            detail: error.to_string(),
        })?;
        Ok(())
    }

    fn prune_runs(&self) {
        let dir = support_dir(self.home).join("runs");
        let Ok(read) = fs::read_dir(&dir) else {
            return;
        };
        let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = read
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect();
        if dirs.len() <= 20 {
            return;
        }
        dirs.sort_by_key(|(modified, _)| *modified);
        let extra = dirs.len() - 20;
        for (_, path) in dirs.into_iter().take(extra) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

impl Drop for Engine<'_> {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.discard("the Run ended");
        if self.record.running.take().is_some() {
            let _ = self.record.save(self.home);
        }
    }
}

fn scope_of(scope: &RunScope, plan: &Plan) -> BTreeSet<String> {
    let ids: BTreeSet<String> = plan.entries.iter().map(|entry| entry.id.clone()).collect();
    match scope {
        RunScope::Full => ids,
        // An Apply changes what was chosen and what that needs. What depends on it is left alone,
        // because each of those is its own row.
        RunScope::Only { items } => {
            let mut out: BTreeSet<String> = items
                .iter()
                .filter(|id| ids.contains(*id))
                .cloned()
                .collect();
            let seeds: Vec<String> = out.iter().cloned().collect();
            for id in seeds {
                add_requirements(plan, &id, &mut out);
            }
            out.retain(|id| ids.contains(id));
            out
        }
        RunScope::Items { items } => {
            let mut out = BTreeSet::new();
            let mut todo = items.clone();
            while let Some(id) = todo.pop() {
                if !ids.contains(&id) || !out.insert(id.clone()) {
                    continue;
                }
                for entry in &plan.entries {
                    if entry
                        .recipe
                        .requires()
                        .iter()
                        .any(|requirement| requirement == &id)
                    {
                        todo.push(entry.id.clone());
                    }
                }
            }
            let seeds: Vec<String> = out.iter().cloned().collect();
            for id in seeds {
                add_requirements(plan, &id, &mut out);
            }
            out.retain(|id| ids.contains(id));
            out
        }
    }
}

fn add_requirements(plan: &Plan, id: &str, out: &mut BTreeSet<String>) {
    let Some(entry) = plan.entries.iter().find(|entry| entry.id == id) else {
        return;
    };
    for requirement in entry.recipe.requires() {
        if out.contains(requirement) {
            continue;
        }
        // Installed or not is a check. The Machine Record does not answer it.
        if out.insert(requirement.clone()) {
            add_requirements(plan, requirement, out);
        }
    }
}

fn needs_admin_rights(entry: &PlanEntry) -> bool {
    entry.recipe.has_admin_step()
        || entry
            .recipe
            .steps()
            .iter()
            .any(|step| matches!(step, Step::BrewCask { .. }))
}

fn assurance_of(setting: &SettingRecipe, macos: Option<&str>) -> Assurance {
    match macos {
        Some(version)
            if setting
                .evidence
                .confirmed_on
                .iter()
                .any(|got| got == version) =>
        {
            Assurance::Proven
        }
        _ => Assurance::Stored,
    }
}

pub(crate) enum ReadBack {
    Match,
    Differ,
    Unset,
    Unknown,
}

pub(crate) fn compare_defaults(store: &Store, desired: &SettingValue, stdout: &str) -> ReadBack {
    let Store::Defaults {
        stored_type,
        encode,
        ..
    } = store
    else {
        return ReadBack::Unknown;
    };
    let text = stdout.trim();
    match stored_type {
        StoredType::Bool => {
            let got = matches!(text, "1" | "true");
            let wanted = act::stored_bool(desired, encode);
            if (text == "0" || text == "false" || text == "1" || text == "true") && got == wanted {
                ReadBack::Match
            } else if text == "0" || text == "false" || text == "1" || text == "true" {
                ReadBack::Differ
            } else {
                ReadBack::Unknown
            }
        }
        StoredType::Int | StoredType::Float => {
            if text == act::stored_number(desired, encode) {
                ReadBack::Match
            } else {
                ReadBack::Differ
            }
        }
        StoredType::String => {
            let wanted = match desired {
                SettingValue::String(value) => encoded_lookup(value, encode),
                _ => return ReadBack::Differ,
            };
            if text == wanted {
                ReadBack::Match
            } else {
                ReadBack::Differ
            }
        }
        StoredType::StringArray => {
            let SettingValue::List(list) = desired else {
                return ReadBack::Differ;
            };
            let got = parse_array(text);
            let wanted: Vec<String> = list
                .iter()
                .map(|item| encoded_lookup(item, encode))
                .collect();
            if got == wanted {
                ReadBack::Match
            } else {
                ReadBack::Differ
            }
        }
        StoredType::Dict => {
            let SettingValue::List(list) = desired else {
                return ReadBack::Differ;
            };
            let Some(encode) = encode else {
                return ReadBack::Unknown;
            };
            let ok = encode.iter().all(|(name, encoded)| {
                let key = encoded.as_str().unwrap_or("");
                let on = list.iter().any(|item| item == name);
                let present =
                    text.contains(&format!("{key} = 1")) || text.contains(&format!("{key} = true"));
                present == on
            });
            if ok {
                ReadBack::Match
            } else {
                ReadBack::Differ
            }
        }
    }
}

fn encoded_lookup(text: &str, encode: &Option<BTreeMap<String, Value>>) -> String {
    encode
        .as_ref()
        .and_then(|map| map.get(text))
        .and_then(Value::as_str)
        .unwrap_or(text)
        .to_string()
}

pub(crate) fn parse_array(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            line.trim()
                .trim_matches(|ch| ch == '(' || ch == ')' || ch == ',' || ch == '"')
                .trim()
        })
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_defaults(store: &Store, stdout: &str) -> Option<Value> {
    let Store::Defaults { stored_type, .. } = store else {
        return None;
    };
    let text = stdout.trim();
    match stored_type {
        StoredType::Bool if text == "1" || text == "true" => Some(Value::Bool(true)),
        StoredType::Bool if text == "0" || text == "false" => Some(Value::Bool(false)),
        StoredType::Int | StoredType::Float => text
            .parse::<i64>()
            .ok()
            .map(|number| Value::Number(number.into())),
        StoredType::String => Some(Value::String(text.to_string())),
        _ => None,
    }
}

pub(crate) fn rendered_git(value: &SettingValue) -> String {
    match value {
        SettingValue::String(text) => text.clone(),
        SettingValue::Bool(value) => value.to_string(),
        SettingValue::Number(value) => value.to_string(),
        SettingValue::List(list) => list.join("\n"),
    }
}

/// Whether `relative`, from `home`, passes through a symlink, `home` itself included. Above `home`
/// is the system's business: `/var` is a link on every Mac.
pub(crate) fn link_on_the_way(home: &Path, relative: &Path) -> bool {
    let is_link =
        |path: &Path| fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    if is_link(home) {
        return true;
    }
    let mut walked = home.to_path_buf();
    relative.components().any(|part| {
        walked.push(part);
        is_link(&walked)
    })
}

/// Whether a git config file says anything about signing in to GitHub. Git's syntax has too many
/// spellings for a reader to tell one helper from another (a key on the section line, a comment,
/// an escaped subsection), so any mention leaves the file alone. `gh`'s own helper being there
/// already means there's nothing to set up.
fn outcome_of(
    ran: Result<CommandOutput, soulcaster_core::command::SpawnError>,
) -> Result<CommandOutput, FailureKind> {
    match ran {
        Err(error) if is_network(&error.message) => Err(FailureKind::Network),
        Err(_) => Err(FailureKind::ToolExit),
        Ok(output) if output.is_success() => Ok(output),
        Ok(output) => {
            let text = format!(
                "{}{}",
                output.stdout_text(),
                String::from_utf8_lossy(&output.stderr)
            );
            Err(failure_kind(&text, output.end == CommandEnd::Stalled))
        }
    }
}

/// Whether `ls -ldO` lists `hidden` among a folder's flags. The flags column comes right before
/// the size, so a name with "hidden" in it elsewhere on the line doesn't count.
pub(crate) fn library_hidden(listing: &str) -> bool {
    let fields: Vec<&str> = listing.split_whitespace().collect();
    let size = fields
        .iter()
        .skip(2)
        .position(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|at| at + 2);
    size.and_then(|at| fields.get(at.checked_sub(1)?))
        .is_some_and(|flags| flags.split(',').any(|flag| flag == "hidden"))
}

pub(crate) fn dock_path(path: &str) -> String {
    let path = path.trim();
    let path = path.strip_prefix("file://").unwrap_or(path);
    percent_decode(path.trim_end_matches('/'))
}

/// `dockutil --list` prints a URL, so `Google Chrome.app` comes back as `Google%20Chrome.app`.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        if let Some(byte) = hex {
            out.push(byte);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A package's id holds slashes (`owner/tap/name`). One Item is one file in `logs/`.
fn log_file_name(item: &str) -> String {
    let safe: String = item
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            other => other,
        })
        .collect();
    format!("{safe}.log")
}

fn private_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

pub(crate) fn dock_paths(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let path = fields.nth(1).unwrap_or(line).trim();
            // `--list` also prints recent apps and folders, which aren't tiles a person placed.
            let tile = fields
                .next()
                .is_none_or(|section| section == "persistentApps");
            (tile && !path.is_empty()).then(|| dock_path(path))
        })
        .collect()
}

/// What to name each app tile by when removing it: the bundle ID, or the label for a tile with
/// none. A name dockutil would read as an option, or as `all`, is left alone.
fn dock_app_tiles(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let is_app = fields
                .get(2)
                .is_none_or(|section| *section == "persistentApps");
            let has_path = fields.get(1).is_some_and(|path| !path.trim().is_empty());
            let id = fields
                .get(4)
                .map(|bundle| bundle.trim())
                .filter(|bundle| !bundle.is_empty())
                .or(fields.first().map(|label| label.trim()))?;
            (is_app
                && has_path
                && !id.is_empty()
                && !id.starts_with('-')
                && !id.eq_ignore_ascii_case("all"))
            .then(|| id.to_string())
        })
        .collect()
}

pub(crate) fn git_key(key: &soulcaster_core::recipe::GitKey) -> &'static str {
    match key {
        soulcaster_core::recipe::GitKey::UserName => "user.name",
        soulcaster_core::recipe::GitKey::UserEmail => "user.email",
        soulcaster_core::recipe::GitKey::InitDefaultBranch => "init.defaultBranch",
        soulcaster_core::recipe::GitKey::CoreExcludesFile => "core.excludesfile",
        soulcaster_core::recipe::GitKey::RerereEnabled => "rerere.enabled",
        soulcaster_core::recipe::GitKey::PushAutoSetupRemote => "push.autoSetupRemote",
        soulcaster_core::recipe::GitKey::PullRebase => "pull.rebase",
    }
}

fn license_name(license: License) -> &'static str {
    match license {
        License::AndroidSdk => "android-sdk",
        License::Xcode => "xcode",
    }
}

fn queue_id(item: &str, step: &str) -> String {
    format!("{item}:{step}")
}

fn is_signature_program(command: &Command) -> bool {
    let name = command.program.to_string_lossy();
    name.ends_with("codesign") || name.ends_with("pkgutil")
}

fn kind_name(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Network => "network",
        FailureKind::NotFound => "not-found",
        FailureKind::Checksum => "checksum",
        FailureKind::Signature => "signature",
        FailureKind::ToolExit => "tool-exit",
        FailureKind::Verify => "verify",
        FailureKind::Stalled => "stalled",
        FailureKind::NeedsAdmin => "needs-admin",
        FailureKind::Interrupted => "interrupted",
    }
}

pub(crate) fn reason_label(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::UnknownId => "unknown-id",
        SkipReason::Retired => "retired",
        SkipReason::RenameCycle => "rename-cycle",
        SkipReason::RenameCollision => "rename-collision",
        SkipReason::WrongKind => "wrong-kind",
        SkipReason::BadValue => "bad-value",
        SkipReason::Lint => "lint",
        SkipReason::MacosTooOld => "macos-too-old",
        SkipReason::ManagedByAdmin => "managed-by-admin",
        SkipReason::BrokenHere => "broken-here",
        SkipReason::WritesThroughLink => "writes-through-link",
        SkipReason::MarkedByPerson => "marked-by-person",
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn only_app_tiles_are_removed_and_never_by_a_name_dockutil_would_misread() {
        assert_eq!(
            dock_app_tiles(
                "Mail\tfile:///System/Applications/Mail.app/\tpersistentApps\t/p\tcom.apple.mail\n\
                 Downloads\tfile:///Users/x/Downloads/\tpersistentOthers\t/p\t\n\
                 Zed\tfile:///Applications/Zed.app/\trecentApps\t/p\tdev.zed.Zed\n\
                 all\tfile:///Applications/all.app/\tpersistentApps\t/p\t\n\
                 --zap\tfile:///Applications/z.app/\tpersistentApps\t/p\t\n\
                 Thing\tfile:///Applications/Thing.app/\tpersistentApps\t/p\t\n"
            ),
            ["com.apple.mail", "Thing"]
        );
    }

    #[test]
    fn dock_paths_are_compared_as_paths_not_as_urls() {
        assert_eq!(
            dock_paths(
                "Google Chrome\tfile:///Applications/Google%20Chrome.app/\tpersistentApps\n"
            ),
            ["/Applications/Google Chrome.app"]
        );
        assert_eq!(
            dock_path("/Applications/Google Chrome.app"),
            "/Applications/Google Chrome.app"
        );
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn a_package_item_gets_one_log_file_inside_logs() {
        assert_eq!(log_file_name("ripgrep"), "ripgrep.log");
        assert_eq!(log_file_name("owner/tap/name"), "owner_tap_name.log");
        assert_eq!(log_file_name("../../x"), ".._.._x.log");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn a_second_run_lock_on_the_same_home_is_busy() {
        let path = std::env::temp_dir().join(format!("soulcaster-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        let held = lock_run(&path).unwrap();
        match lock_run(&path) {
            Err(EngineError::Busy) => {}
            Err(error) => panic!("expected a busy Machine, got {error}"),
            Ok(_) => panic!("a second lock was granted"),
        }
        drop(held);
        let _ = fs::remove_dir_all(&path);
    }
}
