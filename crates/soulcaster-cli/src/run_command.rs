//! `soulcaster run`: one Run of a Config on this Mac, on the same engine as the app. The password
//! is read from the terminal with no echo, and only when the plan has admin work. Exit codes are
//! the design's: 0 everything finished, 2 refused before changing anything, 3 finished with queue
//! entries, 4 finished with failures.

use std::cell::UnsafeCell;
use std::io::{BufRead, BufReader, Write};
use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use soulcaster_core::ask::{Asks, Password, PasswordRequest, SignInStatus};
use soulcaster_core::command::CommandRunner;
use soulcaster_core::event::{ConfigRef, Event, EventKind, EventSink, RunScope};
use soulcaster_core::lifecycle::ItemState;
use soulcaster_core::plan::Plan;
use soulcaster_engine::{AdminGate, CancelFlag, Clock, DotfilesHook, RunInput, perform};

pub const FINISHED: u8 = 0;
pub const REFUSED: u8 = 2;
pub const QUEUED: u8 = 3;
pub const FAILED: u8 = 4;

/// Everything a Run touches, so a test can hand in fakes.
pub struct Setup<'a> {
    pub runner: &'a dyn CommandRunner,
    pub admin: &'a dyn AdminGate,
    pub dotfiles: Option<&'a dyn DotfilesHook>,
    pub asks: &'a dyn Asks,
    pub clock: &'a dyn Clock,
    pub cancel: &'a dyn CancelFlag,
    pub home: &'a Path,
    pub json: bool,
}

/// Runs `plan` and prints as it goes. Returns the exit code.
pub fn run_plan(plan: &mut Plan, setup: &Setup<'_>, out: Arc<Mutex<dyn Write + Send>>) -> u8 {
    let mut sink = Printer {
        out: Arc::clone(&out),
        json: setup.json,
        started: false,
    };
    let config = ConfigRef {
        name: plan.config_name.clone(),
        ..ConfigRef::default()
    };
    let result = perform(RunInput {
        plan,
        runner: setup.runner,
        admin: setup.admin,
        dotfiles: setup.dotfiles,
        home: setup.home,
        events: &mut sink,
        asks: setup.asks,
        cancel: setup.cancel,
        clock: setup.clock,
        scope: RunScope::Full,
        config,
        catalog_version: env!("CARGO_PKG_VERSION"),
        engine_version: env!("CARGO_PKG_VERSION"),
    });
    match result {
        Err(error) => {
            // Only a Run that never began can say nothing changed.
            let (words, code) = if sink.started {
                ("The Run stopped partway", FAILED)
            } else {
                ("Refused before changing anything", REFUSED)
            };
            if setup.json {
                eprintln!("{words}: {error}");
            } else if let Ok(mut out) = out.lock() {
                let _ = writeln!(out, "{words}: {error}");
            }
            code
        }
        Ok(report) => {
            // Stopped early, or with an Item still waiting to start, isn't "everything finished".
            let unfinished =
                report.cancelled || report.states.values().any(|state| !state.is_terminal());
            if unfinished
                || report
                    .states
                    .values()
                    .any(|state| matches!(state, ItemState::Failed { .. }))
            {
                FAILED
            } else if !report.queue.is_empty() {
                QUEUED
            } else {
                FINISHED
            }
        }
    }
}

/// Events as text a person reads, or with `--json`, one event per line.
struct Printer {
    out: Arc<Mutex<dyn Write + Send>>,
    json: bool,
    started: bool,
}

impl EventSink for Printer {
    fn send(&mut self, event: Event) {
        self.started |= matches!(event.kind, EventKind::RunStarted { .. });
        let Ok(mut out) = self.out.lock() else {
            return;
        };
        if self.json {
            if let Ok(line) = serde_json::to_string(&event) {
                let _ = writeln!(out, "{line}");
            }
            return;
        }
        let line = match &event.kind {
            EventKind::RunStarted { .. } => Some("Run started.".to_string()),
            EventKind::AdminHeld { .. } => {
                Some("Holding your password for the Admin Stage.".to_string())
            }
            EventKind::AdminDiscarded { .. } => Some("Password discarded.".to_string()),
            EventKind::ItemState { item, to, .. } if to.is_terminal() => {
                Some(format!("{item}: {}", to.name()))
            }
            EventKind::QueueOpened { entry } => Some(format!("Needs you: {}", entry.text)),
            EventKind::RunWarning { text } => Some(format!("Note: {text}")),
            EventKind::RunFinished { counts, queue } => {
                let counts: Vec<String> = counts
                    .iter()
                    .map(|(state, count)| format!("{count} {state}"))
                    .collect();
                Some(format!(
                    "Finished: {}. {} waiting on you.",
                    counts.join(", "),
                    queue.len()
                ))
            }
            _ => None,
        };
        if let Some(line) = line {
            let _ = writeln!(out, "{line}");
        }
    }
}

/// The person at this terminal. The password goes through `/dev/tty` with echo off, never
/// standard input, so it can't come from a pipe or land in a log.
pub struct TerminalAsks;

impl Asks for TerminalAsks {
    fn password(&self, request: &PasswordRequest) -> Option<Password> {
        let prompt = if request.last_was_wrong {
            format!("That didn't work. {}\nPassword: ", request.why)
        } else {
            format!("{}\nPassword: ", request.why)
        };
        read_without_echo(&prompt).map(Password::new)
    }

    fn show_github_code(&self, code: &str, url: &str) {
        eprintln!("Sign in to GitHub: enter {code} at {url}");
        // The engine passes only GitHub's own address here.
        let _ = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .status();
    }

    fn github_status(&self, status: SignInStatus) {
        let words = match status {
            SignInStatus::Waiting => "Waiting for GitHub sign-in.",
            SignInStatus::SignedIn => "Signed in to GitHub.",
            SignInStatus::Failed => "GitHub sign-in didn't finish. It's in the queue.",
        };
        eprintln!("{words}");
    }
}

/// What to put back if a signal ends the process during the prompt. Filled in before `TTY_FD`
/// is set, and only read by the handler once it has been.
struct SavedTerminal(UnsafeCell<MaybeUninit<libc::termios>>);

// SAFETY: written once per prompt, before `TTY_FD` publishes it with `Release`. The handler reads
// it only after seeing that fd with `Acquire`, and nothing writes it again until `TTY_FD` is
// cleared.
unsafe impl Sync for SavedTerminal {}

static SAVED_TERMINAL: SavedTerminal = SavedTerminal(UnsafeCell::new(MaybeUninit::uninit()));
static TTY_FD: AtomicI32 = AtomicI32::new(-1);
const ENDING_SIGNALS: [libc::c_int; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

/// Turns echo back on, then lets the signal do what it would have. `tcsetattr`, `signal` and
/// `raise` are all async-signal-safe.
extern "C" fn restore_echo_and_die(signal: libc::c_int) {
    let fd = TTY_FD.load(Ordering::Acquire);
    if fd >= 0 {
        // SAFETY: a non-negative `fd` means `SAVED_TERMINAL` holds a valid `termios`.
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, (*SAVED_TERMINAL.0.get()).as_ptr()) };
    }
    // SAFETY: resets this signal to its default and raises it again. It's delivered on return.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

fn read_without_echo(prompt: &str) -> Option<String> {
    use std::os::fd::AsRawFd;

    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();
    let mut before = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `fd` is open and `before` has room for a `termios`.
    if unsafe { libc::tcgetattr(fd, before.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `tcgetattr` succeeded, so `before` is initialized.
    let before = unsafe { before.assume_init() };
    // SAFETY: no handler is installed yet and `TTY_FD` is still -1, so nothing reads this.
    unsafe { (*SAVED_TERMINAL.0.get()).write(before) };
    TTY_FD.store(fd, Ordering::Release);

    let handler = restore_echo_and_die as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: installs a handler that only makes async-signal-safe calls, and keeps the old ones.
    let previous = ENDING_SIGNALS.map(|signal| unsafe { libc::signal(signal, handler) });

    let mut quiet = before;
    quiet.c_lflag &= !libc::ECHO;
    let line = (&tty)
        .write_all(prompt.as_bytes())
        .ok()
        // SAFETY: `fd` is open and `quiet` is a valid `termios`.
        .filter(|()| unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) } == 0)
        .and_then(|()| {
            let mut line = String::new();
            BufReader::new(&tty).read_line(&mut line).ok().map(|_| line)
        });

    // SAFETY: `fd` is open and `before` is the terminal's own earlier state.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &before) };
    for (signal, old) in ENDING_SIGNALS.into_iter().zip(previous) {
        // SAFETY: puts back what `signal` returned above.
        unsafe { libc::signal(signal, old) };
    }
    TTY_FD.store(-1, Ordering::Release);
    let _ = (&tty).write_all(b"\n");

    let line = line?;
    let line = line.trim_end_matches(['\n', '\r']).to_string();
    (!line.is_empty()).then_some(line)
}

/// Keeps the Mac awake for as long as this lives, then lets it sleep again.
pub struct KeepAwake(Option<std::process::Child>);

impl KeepAwake {
    pub fn start() -> Self {
        let child = std::process::Command::new("/usr/bin/caffeinate")
            .args(["-dimsu", "-w", &std::process::id().to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
        Self(child)
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;

    use soulcaster_core::catalog::Catalog;
    use soulcaster_core::command::{Command, FakeRunner, Reply};
    use soulcaster_core::config::Config;
    use soulcaster_engine::{AdminWindow, Budget, DotfilesHook, DotfilesResult, Validation};

    use super::*;

    struct NoGate;

    struct NoWindow;

    impl AdminWindow for NoWindow {
        fn elevated(&self, inner: Command) -> Command {
            inner
        }

        fn may_elevate(&self, inner: Command) -> Command {
            inner
        }
    }

    impl AdminGate for NoGate {
        fn validate(&self, _: &dyn CommandRunner, _: &Path, _: &Password) -> Validation {
            Validation::Accepted
        }

        fn open(&self, _: &Password, _: Budget) -> Result<Box<dyn AdminWindow>, String> {
            Ok(Box::new(NoWindow))
        }

        fn forget(&self, _: &dyn CommandRunner, _: &Path) -> bool {
            true
        }
    }

    /// Declines everything, so no test waits on a person.
    struct Declines;

    impl Asks for Declines {
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

    fn file_name(command: &Command) -> String {
        command
            .program
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn run(runner: &FakeRunner, json: bool) -> (u8, String) {
        let home = tempfile::tempdir().unwrap();
        let config =
            Config::parse(br#"{"schemaVersion":1,"name":"Test","items":["ripgrep"]}"#).unwrap();
        let mut plan = Plan::new(&config, Catalog::shipped());
        let cancel = AtomicBool::new(false);
        let setup = Setup {
            runner,
            admin: &NoGate,
            dotfiles: None,
            asks: &Declines,
            clock: &Fixed,
            cancel: &cancel,
            home: home.path(),
            json,
        };
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let code = run_plan(&mut plan, &setup, buffer.clone());
        let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        (code, text)
    }

    fn machine(runner: &FakeRunner, admin: bool) {
        let groups = if admin { "staff admin\n" } else { "staff\n" };
        runner.on(|c| file_name(c) == "id", [Reply::ok(groups)]);
        runner.on(|c| file_name(c) == "sw_vers", [Reply::ok("15.6.1\n")]);
    }

    #[test]
    fn a_machine_that_already_has_everything_finishes_with_zero() {
        let runner = FakeRunner::new();
        machine(&runner, true);
        let (code, text) = run(&runner, false);
        assert_eq!(code, FINISHED, "{text}");
        assert!(text.contains("Run started."), "{text}");
        assert!(text.contains("Finished:"), "{text}");
    }

    #[test]
    fn json_is_one_event_per_line() {
        let runner = FakeRunner::new();
        machine(&runner, true);
        let (_, text) = run(&runner, true);
        assert!(!text.is_empty());
        for line in text.lines() {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|error| panic!("{error}: {line}"));
        }
    }

    #[test]
    fn a_standard_account_on_a_bare_mac_is_refused_before_anything_changes() {
        let runner = FakeRunner::new();
        machine(&runner, false);
        runner.on(|c| file_name(c) == "test", [Reply::fails(1, "")]);
        let (code, text) = run(&runner, false);
        assert_eq!(code, REFUSED, "{text}");
        assert!(text.contains("Refused before changing anything"), "{text}");
        assert!(
            !runner
                .calls()
                .iter()
                .any(|c| file_name(c) == "brew" || file_name(c) == "softwareupdate")
        );
    }

    #[test]
    fn a_failed_install_ends_with_four() {
        let runner = FakeRunner::new();
        machine(&runner, true);
        runner.on(
            |c| file_name(c) == "test" && c.args.iter().any(|a| a.contains("Cellar/ripgrep")),
            [Reply::fails(1, "")],
        );
        runner.on(
            |c| file_name(c) == "brew" && c.args.iter().any(|a| a == "ripgrep"),
            [Reply::fails(1, "Error: it broke")],
        );
        let (code, text) = run(&runner, false);
        assert_eq!(code, FAILED, "{text}");
        assert!(text.contains("ripgrep: failed"), "{text}");
    }

    #[test]
    fn a_cancelled_run_is_not_a_clean_finish() {
        let runner = FakeRunner::new();
        machine(&runner, true);
        runner.on(
            |c| file_name(c) == "test" && c.args.iter().any(|a| a.contains("Cellar/ripgrep")),
            [Reply::fails(1, "")],
        );
        let home = tempfile::tempdir().unwrap();
        let config =
            Config::parse(br#"{"schemaVersion":1,"name":"Test","items":["ripgrep"]}"#).unwrap();
        let mut plan = Plan::new(&config, Catalog::shipped());
        let cancel = AtomicBool::new(true);
        let setup = Setup {
            runner: &runner,
            admin: &NoGate,
            dotfiles: None,
            asks: &Declines,
            clock: &Fixed,
            cancel: &cancel,
            home: home.path(),
            json: false,
        };
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let code = run_plan(&mut plan, &setup, buffer.clone());
        let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(
            !runner
                .calls()
                .iter()
                .any(|c| file_name(c) == "brew" && c.args.iter().any(|a| a == "install")),
            "cancel was checked too late and an install ran: {text}"
        );
        assert_ne!(
            code, FINISHED,
            "exit 0 means everything finished, and this Run was cancelled first: {text}"
        );
    }

    /// The Dotfiles step revokes one approval, records another, then the record cannot be saved.
    struct Revokes<'a> {
        home: &'a Path,
    }

    impl DotfilesHook for Revokes<'_> {
        fn apply(
            &self,
            _: &soulcaster_core::plan::DotfilesPlan,
            approvals: &dyn soulcaster_core::approval::ApprovalStore,
        ) -> DotfilesResult {
            let mut held = approvals.read().unwrap_or_default();
            held.taps.insert("added/tap".into());
            approvals.write(&held).unwrap();
            let dir = self.home.join("Library/Application Support/Soulcaster");
            let tmp = dir.join(format!(".machine.json.{}.tmp", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::create_dir(&tmp).unwrap();
            DotfilesResult::default()
        }
    }

    #[test]
    fn a_save_failure_after_the_run_started_is_not_refused_before_anything_changed() {
        let runner = FakeRunner::new();
        machine(&runner, true);
        let home = tempfile::tempdir().unwrap();
        let _cleanup = ClearTmp {
            home: home.path().to_path_buf(),
        };
        let config = Config::parse(
            br#"{"schemaVersion":1,"name":"Test","items":[],"dotfiles":{"repo":"example-you/dotfiles"}}"#,
        )
        .unwrap();
        let mut plan = Plan::new(&config, Catalog::shipped());
        let cancel = AtomicBool::new(false);
        let hook = Revokes { home: home.path() };
        let setup = Setup {
            runner: &runner,
            admin: &NoGate,
            dotfiles: Some(&hook),
            asks: &Declines,
            clock: &Fixed,
            cancel: &cancel,
            home: home.path(),
            json: false,
        };
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let code = run_plan(&mut plan, &setup, buffer.clone());
        let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        assert!(
            text.contains("Run started."),
            "the Run had not started, so a refusal would be right: {text}"
        );
        assert_ne!(
            code, REFUSED,
            "exit 2 is only for a refusal before anything changes: {text}"
        );
        assert!(!text.contains("Refused before changing anything"), "{text}");
    }

    struct ClearTmp {
        home: std::path::PathBuf,
    }

    impl Drop for ClearTmp {
        fn drop(&mut self) {
            let tmp = self.home.join(format!(
                "Library/Application Support/Soulcaster/.machine.json.{}.tmp",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(tmp);
        }
    }

    #[test]
    fn echo_is_restored_when_the_password_read_is_interrupted() {
        if std::env::var_os("SOULCASTER_ECHO_CHILD").is_some() {
            // This process's controlling terminal is the pty the parent created. It is not the
            // terminal this test was launched from. The test harness may have wrapped SIGINT;
            // `soulcaster run` takes the default, which is to die.
            // SAFETY: `set` is initialized by `sigemptyset`. Both calls affect only this child.
            unsafe {
                let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                libc::sigemptyset(set.as_mut_ptr());
                libc::sigaddset(set.as_mut_ptr(), libc::SIGINT);
                libc::pthread_sigmask(libc::SIG_UNBLOCK, set.as_ptr(), std::ptr::null_mut());
                libc::signal(libc::SIGINT, libc::SIG_DFL);
            }
            let _ = super::read_without_echo("Password: ");
            std::process::exit(0);
        }

        let mut primary: libc::c_int = -1;
        let mut replica: libc::c_int = -1;
        // SAFETY: both pointers are live `c_int`s. A null window and name ask for defaults.
        let opened = unsafe {
            libc::openpty(
                &mut primary,
                &mut replica,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
        let primary = unsafe { std::os::fd::OwnedFd::from_raw_fd(primary) };
        let replica = unsafe { std::os::fd::OwnedFd::from_raw_fd(replica) };
        // Keep a replica open in this process. Closing the last one can reset the pty, which would
        // hide echo being left off when the child dies.
        let _replica_kept = replica.try_clone().unwrap();

        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .arg("run_command::tests::echo_is_restored_when_the_password_read_is_interrupted")
            .arg("--exact")
            .env("SOULCASTER_ECHO_CHILD", "1")
            .stdin(std::process::Stdio::from(replica))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: runs in the forked child before exec. `setsid` and `TIOCSCTTY` only affect that
        // child. File descriptor 0 is the pty replica handed in as stdin.
        unsafe {
            child.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = child.spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        struct StopChild(Option<i32>);
        impl Drop for StopChild {
            fn drop(&mut self) {
                if let Some(pid) = self.0.take() {
                    // SAFETY: this is the child spawned above. `SIGKILL` ends it if the test
                    // unwinds while it is still blocked in the password read.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            }
        }
        let mut stop = StopChild(Some(pid));

        let echo_on = |fd: std::os::fd::RawFd| -> bool {
            let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
            // SAFETY: `fd` is the open pty primary and `termios` is a live value of that type.
            let read = unsafe { libc::tcgetattr(fd, &mut termios) };
            assert_eq!(read, 0, "tcgetattr: {}", std::io::Error::last_os_error());
            termios.c_lflag & libc::ECHO != 0
        };
        let primary_fd = primary.as_raw_fd();
        assert!(echo_on(primary_fd), "the pty started with echo already off");
        let started = std::time::Instant::now();
        while echo_on(primary_fd) && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !echo_on(primary_fd),
            "the password read never turned echo off"
        );
        // SAFETY: `pid` is the child this test spawned. `SIGINT` is the interrupt the terminal sends.
        let sent = unsafe { libc::kill(pid, libc::SIGINT) };
        assert_eq!(sent, 0, "kill: {}", std::io::Error::last_os_error());
        let killed = std::time::Instant::now();
        while !echo_on(primary_fd) && killed.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            echo_on(primary_fd),
            "echo stayed off after the password read was interrupted"
        );
        stop.0 = None;
        let _ = child.kill();
        // macOS won't let a process finish exiting while its terminal has unread output, and
        // nothing reads this one. Closing the other end discards it.
        drop(primary);
        let _ = child.wait();
    }

    /// Ctrl-Z during the prompt. This Mac's zsh suspends the job, and `fg` turns echo back on
    /// before the read continues. The password then shows on the terminal.
    #[test]
    fn a_stopped_password_prompt_echoes_the_password_when_continued() {
        if std::env::var_os("SOULCASTER_ECHO_STOP_CHILD").is_some() {
            // Fresh process on the pty, with the signals a terminal actually delivers.
            // SAFETY: `set` is initialized by `sigemptyset`. The mask and dispositions apply only
            // to this child, which is not the terminal this test was launched from.
            unsafe {
                let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
                libc::sigemptyset(set.as_mut_ptr());
                libc::sigaddset(set.as_mut_ptr(), libc::SIGINT);
                libc::sigaddset(set.as_mut_ptr(), libc::SIGTSTP);
                libc::pthread_sigmask(libc::SIG_UNBLOCK, set.as_ptr(), std::ptr::null_mut());
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGTSTP, libc::SIG_DFL);
            }
            let _ = super::read_without_echo("Password: ");
            // After the read returns, on the pty, so a password echoed above was consumed by the
            // prompt and not typed at the shell afterwards.
            if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
                let _ = std::io::Write::write_all(&mut tty, b"PROMPT_DONE\n");
            }
            std::process::exit(0);
        }

        let mut primary: libc::c_int = -1;
        let mut replica: libc::c_int = -1;
        // SAFETY: both pointers are live `c_int`s. A null window and name ask for defaults.
        let opened = unsafe {
            libc::openpty(
                &mut primary,
                &mut replica,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
        let primary = unsafe { std::os::fd::OwnedFd::from_raw_fd(primary) };
        let replica = unsafe { std::os::fd::OwnedFd::from_raw_fd(replica) };
        let _replica_kept = replica.try_clone().unwrap();
        // SAFETY: the primary is open. Non-blocking reads keep the test from waiting on zsh.
        unsafe {
            let flags = libc::fcntl(primary.as_raw_fd(), libc::F_GETFL);
            assert!(flags >= 0, "fcntl: {}", std::io::Error::last_os_error());
            assert_eq!(
                libc::fcntl(primary.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK),
                0,
                "fcntl: {}",
                std::io::Error::last_os_error()
            );
        }

        let exe = std::env::current_exe().unwrap();
        let command = format!(
            "SOULCASTER_ECHO_STOP_CHILD=1 {} {} --exact\n",
            shell_quote(&exe.to_string_lossy()),
            shell_quote(
                "run_command::tests::a_stopped_password_prompt_echoes_the_password_when_continued"
            )
        );
        let mut child = std::process::Command::new("/bin/zsh");
        child
            .args(["-f", "-i"])
            .env("PS1", "ZSH> ")
            .env("TERM", "xterm-256color")
            .stdin(std::process::Stdio::from(replica.try_clone().unwrap()))
            .stdout(std::process::Stdio::from(replica.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(replica));
        // SAFETY: runs in the forked child before exec. `setsid` and `TIOCSCTTY` only affect that
        // child. File descriptor 0 is the pty replica handed in as stdin.
        unsafe {
            child.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = child.spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        struct StopShell(Option<i32>);
        impl Drop for StopShell {
            fn drop(&mut self) {
                if let Some(pid) = self.0.take() {
                    // SAFETY: `pid` leads the session this test spawned. Killing the group ends
                    // zsh and the password prompt if the test unwinds while they are still running.
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
        let mut stop = StopShell(Some(pid));
        let primary_fd = primary.as_raw_fd();
        let mut seen = read_pty(primary_fd, Duration::from_millis(800));
        write_pty(primary_fd, command.as_bytes());
        let prompted = wait_for(primary_fd, &mut seen, b"Password:", Duration::from_secs(8));
        assert!(
            prompted,
            "the password prompt never came up: {}",
            String::from_utf8_lossy(&seen)
        );
        // The prompt is written, then echo goes off, then the read blocks. zsh's own line editor
        // also clears echo, so wait until the job has had the terminal and echo is off again.
        let echo_off_at = std::time::Instant::now();
        let echo_on = |fd: std::os::fd::RawFd| -> bool {
            let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
            // SAFETY: `fd` is the open pty primary and `termios` is a live value of that type.
            let read = unsafe { libc::tcgetattr(fd, &mut termios) };
            assert_eq!(read, 0, "tcgetattr: {}", std::io::Error::last_os_error());
            termios.c_lflag & libc::ECHO != 0
        };
        while echo_on(primary_fd) && echo_off_at.elapsed() < Duration::from_secs(2) {
            seen.extend(read_pty(primary_fd, Duration::from_millis(20)));
        }
        assert!(
            !echo_on(primary_fd),
            "echo was still on after the prompt: {}",
            String::from_utf8_lossy(&seen)
        );
        std::thread::sleep(Duration::from_millis(150));
        // The terminal's suspend character, Ctrl-Z, not a direct kill.
        write_pty(primary_fd, b"\x1a");
        let suspended = wait_for(primary_fd, &mut seen, b"suspended", Duration::from_secs(3));
        assert!(
            suspended,
            "zsh did not suspend the prompt: {}",
            String::from_utf8_lossy(&seen)
        );
        write_pty(primary_fd, b"fg\n");
        let continued = wait_for(primary_fd, &mut seen, b"continued", Duration::from_secs(3));
        assert!(
            continued,
            "zsh did not continue the prompt: {}",
            String::from_utf8_lossy(&seen)
        );
        let password = b"s3cret-password\n";
        write_pty(primary_fd, password);
        let mut after = Vec::new();
        let done = wait_for(
            primary_fd,
            &mut after,
            b"PROMPT_DONE",
            Duration::from_secs(3),
        );
        assert!(
            done,
            "the prompt never finished the read: {}",
            String::from_utf8_lossy(&after)
        );
        assert!(
            !after
                .windows(b"s3cret-password".len())
                .any(|window| window == b"s3cret-password"),
            "the password was echoed after Ctrl-Z and fg. zsh turns echo back on when the job continues, and the prompt never turns it off again: {}",
            String::from_utf8_lossy(&after)
        );
        stop.0 = None;
        // SAFETY: same session as above. It has finished the assertion, so end it.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
        drop(primary);
        let _ = child.wait();
    }

    fn shell_quote(text: &str) -> String {
        format!("'{}'", text.replace('\'', r"'\''"))
    }

    fn write_pty(fd: std::os::fd::RawFd, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            // SAFETY: `fd` is the open pty primary and `rest` is the bytes still to send.
            let wrote = unsafe { libc::write(fd, rest.as_ptr().cast(), rest.len()) };
            assert!(wrote > 0, "write: {}", std::io::Error::last_os_error());
            let wrote = usize::try_from(wrote).unwrap();
            rest = &rest[wrote..];
        }
    }

    fn read_pty(fd: std::os::fd::RawFd, wait: Duration) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0_u8; 2048];
        let deadline = std::time::Instant::now() + wait;
        while std::time::Instant::now() < deadline {
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let left = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .min(200);
            // SAFETY: `poll` describes the open pty primary. A short wait bounds the test.
            let ready = unsafe { libc::poll(&mut poll, 1, i32::try_from(left).unwrap_or(200)) };
            if ready <= 0 {
                continue;
            }
            loop {
                // SAFETY: `buf` is writable storage and `fd` is the open pty primary.
                let read = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if read > 0 {
                    let read = usize::try_from(read).unwrap();
                    out.extend_from_slice(&buf[..read]);
                    continue;
                }
                break;
            }
        }
        out
    }

    fn wait_for(
        fd: std::os::fd::RawFd,
        seen: &mut Vec<u8>,
        needle: &[u8],
        limit: Duration,
    ) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            if seen.windows(needle.len()).any(|window| window == needle) {
                return true;
            }
            let left = limit.saturating_sub(start.elapsed());
            seen.extend(read_pty(fd, left.min(Duration::from_millis(100))));
        }
        seen.windows(needle.len()).any(|window| window == needle)
    }
}
