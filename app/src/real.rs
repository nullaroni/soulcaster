//! The real engine behind the window. A Run happens on its own thread and its events come back
//! over a channel. When the engine needs the password it waits here, the Run screen shows the
//! field, and the answer goes straight back. Started only with `--real`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use soulcaster_core::approval::ApprovalStore;
use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::catalog::Catalog;
use soulcaster_core::event::{ConfigRef, Event, EventKind, EventSink, RunScope, Stamper};
use soulcaster_core::lint::DEFAULT_CLONE_LOCATION;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{
    Clock, KeepAwake, MachineRecord, ProcessRunner, RecordApprovals, RunInput, SystemClock,
    gate_beside, perform,
};
use soulcaster_repo::in_run::RunDotfiles;
use soulcaster_repo::tools::Tools;

use crate::answers::Answers;
use crate::engine::Engine;

/// What the Run is waiting on from the window.
#[derive(Default)]
struct Waiting {
    password: Mutex<Option<(PasswordRequest, Sender<Option<Password>>)>>,
    github: Mutex<Option<(String, String)>>,
}

struct WindowAsks(Arc<Waiting>);

impl Asks for WindowAsks {
    fn password(&self, request: &PasswordRequest) -> Option<Password> {
        let (answer, answered) = mpsc::channel();
        *self.0.password.lock().ok()? = Some((request.clone(), answer));
        // The window going away drops the sender, which reads as declining.
        let password = answered.recv().ok().flatten();
        if let Ok(mut waiting) = self.0.password.lock() {
            *waiting = None;
        }
        password
    }

    fn show_github_code(&self, code: &str, url: &str) {
        if let Ok(mut github) = self.0.github.lock() {
            *github = Some((code.to_string(), url.to_string()));
        }
        // The engine passes only GitHub's own address here.
        let _ = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .status();
    }

    fn github_status(&self, status: SignInStatus) {
        if status != SignInStatus::Waiting
            && let Ok(mut github) = self.0.github.lock()
        {
            *github = None;
        }
    }
}

struct ChannelSink(Sender<Event>);

impl EventSink for ChannelSink {
    fn send(&mut self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// Clears the flag when the Run's thread ends, however it ends.
struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub struct RealEngine {
    home: PathBuf,
    waiting: Arc<Waiting>,
    running: Arc<AtomicBool>,
    sender: Sender<Event>,
    events: Receiver<Event>,
    plan: Option<Plan>,
}

impl RealEngine {
    pub fn new(home: PathBuf) -> Self {
        let (sender, events) = mpsc::channel();
        Self {
            home,
            waiting: Arc::default(),
            running: Arc::default(),
            sender,
            events,
            plan: None,
        }
    }

    fn start(&mut self, mut plan: Plan, scope: RunScope) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let running = Running(Arc::clone(&self.running));
        let home = self.home.clone();
        let waiting = Arc::clone(&self.waiting);
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _running = running;
            let _awake = KeepAwake::start();
            let Ok(executable) = std::env::current_exe() else {
                finish_with(&sender, "Soulcaster couldn't find its password helper.");
                return;
            };
            let gate = gate_beside(&executable);
            let runner = ProcessRunner::new();
            let repo_runner = soulcaster_repo::process::runner();
            let tools = Tools::for_mac(home.clone());
            let clone_location = MachineRecord::load(&home)
                .ok()
                .and_then(|record| record.clone_path)
                .unwrap_or_else(|| DEFAULT_CLONE_LOCATION.to_string());
            let stamp = SystemClock.now_ms().to_string();
            let dotfiles = RunDotfiles {
                runner: &repo_runner,
                tools: &tools,
                clone_location: &clone_location,
                stamp: &stamp,
            };
            let asks = WindowAsks(waiting);
            let cancel = AtomicBool::new(false);
            let mut sink = ChannelSink(sender.clone());
            let config = ConfigRef {
                name: plan.config_name.clone(),
                ..ConfigRef::default()
            };
            let result = perform(RunInput {
                plan: &mut plan,
                runner: &runner,
                admin: &gate,
                dotfiles: Some(&dotfiles),
                home: &home,
                events: &mut sink,
                asks: &asks,
                cancel: &cancel,
                clock: &SystemClock,
                scope,
                config,
                catalog_version: Catalog::shipped().version(),
                engine_version: env!("CARGO_PKG_VERSION"),
            });
            if let Err(error) = result {
                finish_with(&sender, &format!("The Run stopped: {error}"));
            }
        });
    }

    fn approve_taps(&self, answers: &Answers) {
        if answers.taps.is_empty() {
            return;
        }
        let store = RecordApprovals::new(self.home.clone());
        let Ok(mut approvals) = store.read() else {
            return;
        };
        approvals.taps.extend(answers.taps.iter().cloned());
        let _ = store.write(&approvals);
    }

    /// A person said a queue entry is done. The Run reads that from the record.
    fn finish_queue_entry(&self, id: &str) {
        let Ok(mut record) = MachineRecord::load(&self.home) else {
            return;
        };
        let now = SystemClock.now_ms();
        for entry in record.queue.iter_mut().filter(|entry| entry.id == id) {
            entry.finished_at = Some(now);
            entry.finished_by = Some("person".to_string());
        }
        let _ = record.save(&self.home);
    }
}

/// So the Run screen ends rather than waiting for a `run.finished` that won't come.
fn finish_with(sender: &Sender<Event>, words: &str) {
    let mut stamper = Stamper::after(u64::MAX / 2);
    let now = SystemClock.now_ms();
    let _ = sender.send(stamper.stamp(
        now,
        EventKind::RunWarning {
            text: words.to_string(),
        },
    ));
    let _ = sender.send(stamper.stamp(
        now,
        EventKind::RunFinished {
            counts: Default::default(),
            queue: Vec::new(),
        },
    ));
}

impl Engine for RealEngine {
    fn is_real(&self) -> bool {
        true
    }

    fn start_run(&mut self, plan: &Plan, answers: &Answers) {
        self.approve_taps(answers);
        self.plan = Some(plan.clone());
        self.start(plan.clone(), RunScope::Full);
    }

    fn start_scoped(&mut self, item: &str, waiting: &[String], queue_id: Option<&str>) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if let Some(id) = queue_id {
            self.finish_queue_entry(id);
        }
        let mut items = vec![item.to_string()];
        items.extend(waiting.iter().cloned());
        self.start(plan, RunScope::Items { items });
    }

    fn submit_password(&mut self, password: Password) {
        if let Ok(mut waiting) = self.waiting.password.lock()
            && let Some((_, answer)) = waiting.take()
        {
            let _ = answer.send(Some(password));
        }
    }

    fn decline_password(&mut self) {
        if let Ok(mut waiting) = self.waiting.password.lock()
            && let Some((_, answer)) = waiting.take()
        {
            let _ = answer.send(None);
        }
    }

    fn password_request(&self) -> Option<PasswordRequest> {
        let waiting = self.waiting.password.lock().ok()?;
        waiting.as_ref().map(|(request, _)| request.clone())
    }

    fn github_code(&self) -> Option<(String, String)> {
        self.waiting.github.lock().ok()?.clone()
    }

    /// Sign-in happens inside the Run, which shows the code when it has one.
    fn start_github_sign_in(&mut self) -> String {
        String::new()
    }

    fn poll(&mut self, _now: f64) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Events arrive whenever the thread sends them, so the window checks while a Run is on.
    fn next_due(&self, _now: f64) -> Option<f64> {
        self.running.load(Ordering::SeqCst).then_some(0.1)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_password_the_window_gives_reaches_the_waiting_run_and_the_question_ends() {
        let waiting = Arc::new(Waiting::default());
        let asks = WindowAsks(Arc::clone(&waiting));
        let mut engine = RealEngine::new(PathBuf::from("/nonexistent"));
        engine.waiting = Arc::clone(&waiting);
        let run = thread::spawn(move || {
            asks.password(&PasswordRequest {
                why: "for a test".to_string(),
                last_was_wrong: true,
            })
            .map(|password| password.expose().to_string())
        });
        let started = Instant::now();
        while engine.password_request().is_none() {
            assert!(started.elapsed() < Duration::from_secs(5), "never asked");
            thread::sleep(Duration::from_millis(5));
        }
        assert!(engine.password_request().unwrap().last_was_wrong);
        engine.submit_password(Password::new("typed".to_string()));
        assert_eq!(run.join().unwrap().as_deref(), Some("typed"));
        assert!(engine.password_request().is_none());
    }

    #[test]
    fn declining_or_closing_the_window_answers_no() {
        let waiting = Arc::new(Waiting::default());
        let asks = WindowAsks(Arc::clone(&waiting));
        let mut engine = RealEngine::new(PathBuf::from("/nonexistent"));
        engine.waiting = Arc::clone(&waiting);
        let run = thread::spawn(move || {
            asks.password(&PasswordRequest {
                why: "for a test".to_string(),
                last_was_wrong: false,
            })
            .is_none()
        });
        while engine.password_request().is_none() {
            thread::sleep(Duration::from_millis(5));
        }
        engine.decline_password();
        assert!(run.join().unwrap());
    }

    #[test]
    fn a_github_code_shows_until_sign_in_ends() {
        let waiting = Arc::new(Waiting::default());
        let asks = WindowAsks(Arc::clone(&waiting));
        let mut engine = RealEngine::new(PathBuf::from("/nonexistent"));
        engine.waiting = Arc::clone(&waiting);
        *waiting.github.lock().unwrap() = Some(("ABCD-1234".into(), "url".into()));
        assert_eq!(engine.github_code().unwrap().0, "ABCD-1234");
        asks.github_status(SignInStatus::Waiting);
        assert!(engine.github_code().is_some());
        asks.github_status(SignInStatus::SignedIn);
        assert!(engine.github_code().is_none());
        engine.decline_password();
    }

    #[test]
    fn a_run_that_cannot_start_still_ends_the_run_screen() {
        let (sender, events) = mpsc::channel();
        finish_with(&sender, "stopped");
        let kinds: Vec<EventKind> = events.try_iter().map(|event| event.kind).collect();
        assert!(matches!(kinds[0], EventKind::RunWarning { .. }));
        assert!(matches!(kinds[1], EventKind::RunFinished { .. }));
    }
}
