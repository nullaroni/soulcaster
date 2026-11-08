//! How an admin step reaches the password. Each step gets a private folder holding a socket and a
//! symlink to the signed helper, and `SUDO_ASKPASS` points at the symlink. The socket exists only
//! while that step runs, and answers only the helper, within a budget.
//!
//! This doesn't remove the main limit: while a socket is open, any code running as this user can
//! start `sudo -A` with that path. The design makes the window short and stops the password itself
//! leaking. It never travels in a file, an argument, an environment variable, an event or a log.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use soulcaster_core::ask::Password;
use soulcaster_core::command::{Command, CommandEnd, CommandRunner};

use crate::process::ProcessRunner;

const SUDO: &str = "/usr/bin/sudo";
const ACCEPT_POLL: Duration = Duration::from_millis(10);
/// `sudo -A -v` answers quickly, unless Touch ID is waiting for a finger.
const VALIDATE_STALL: Duration = Duration::from_secs(60);

/// How many answers one window may give.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Budget {
    /// The engine's own `sudo -A` calls. The socket closes after one answer, so `softwareupdate`
    /// runs on with nothing listening.
    One,
    /// A `brew` cask, which resets its timestamp and asks again.
    AsNeeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Validation {
    Accepted,
    /// Ask again, without starting the Run.
    Wrong,
    /// It couldn't be tried at all. The Run goes on without the password.
    Unavailable(String),
    /// `sudo -k` failed, so an earlier timestamp may still answer without a password. Nothing was
    /// checked, and nothing should wait on the person while that may be so.
    StillSignedIn,
}

/// One admin step's access. Dropping it closes the socket and removes the folder.
pub trait AdminWindow {
    /// The command, run through `sudo -A`.
    fn elevated(&self, inner: Command) -> Command;
    /// The command as it is, able to call `sudo -A` itself.
    fn may_elevate(&self, inner: Command) -> Command;
}

/// The Admin Stage's way to `sudo`. [`SocketGate`] is the real one.
pub trait AdminGate: Send + Sync {
    /// Clears any timestamp and tries the password once. `sudo` would otherwise retry three times,
    /// which could lock an account.
    fn validate(&self, runner: &dyn CommandRunner, home: &Path, password: &Password) -> Validation;
    fn open(&self, password: &Password, budget: Budget) -> Result<Box<dyn AdminWindow>, String>;
    /// Ends `sudo`'s own timestamp. Without this, dropping the password wouldn't end root access.
    /// False when that can't be confirmed, and the caller then treats root as still reachable.
    fn forget(&self, runner: &dyn CommandRunner, home: &Path) -> bool;
}

/// Who may have an answer. The path always has to match. The signature is what proves it on a
/// release, and an ad-hoc build has none to check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerRule {
    Signed { identifier: String, team: String },
    PathOnly,
}

pub struct SocketGate {
    helper: PathBuf,
    rule: PeerRule,
    sudo: PathBuf,
    folder_parent: PathBuf,
}

impl SocketGate {
    pub fn new(helper: PathBuf, rule: PeerRule) -> Self {
        Self {
            helper,
            rule,
            sudo: PathBuf::from(SUDO),
            // Per-user on macOS, and short: a socket's path is limited to 104 bytes.
            folder_parent: std::env::temp_dir(),
        }
    }

    /// A stand-in for `/usr/bin/sudo`, so tests never reach the real one.
    pub fn with_sudo(mut self, sudo: PathBuf) -> Self {
        self.sudo = sudo;
        self
    }

    fn plain(&self, home: &Path) -> Command {
        Command::new(&self.sudo)
            .env("HOME", home.to_string_lossy())
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            // Its messages are matched below, so they have to be in one language.
            .env("LC_ALL", "C")
            .stall_limit(VALIDATE_STALL)
    }
}

impl AdminGate for SocketGate {
    fn validate(&self, runner: &dyn CommandRunner, home: &Path, password: &Password) -> Validation {
        // Without this, an earlier timestamp would answer `-v` and nothing would be checked.
        if !self.forget(runner, home) {
            return Validation::StillSignedIn;
        }
        let window = match self.open(password, Budget::One) {
            Ok(window) => window,
            Err(why) => return Validation::Unavailable(why),
        };
        let check = window.may_elevate(self.plain(home).args(["-A", "-v"]));
        let ran = runner.run(&check, &mut |_, _| {});
        drop(window);
        match ran {
            Err(error) => Validation::Unavailable(error.to_string()),
            Ok(output) if output.is_success() => Validation::Accepted,
            Ok(output) if output.end == CommandEnd::Stalled => {
                Validation::Unavailable("sudo didn't answer in time".to_string())
            }
            Ok(output) => {
                // Only `sudo` saying the password was wrong counts. Asking again after anything
                // else, a locked account above all, tells the person the wrong thing and can make
                // it worse.
                let said = String::from_utf8_lossy(&output.stderr).to_lowercase();
                if said.contains("locked") {
                    Validation::Unavailable("this account is locked".to_string())
                } else if said.contains("sorry, try again") || said.contains("incorrect password") {
                    Validation::Wrong
                } else if said.contains("not in the sudoers") || said.contains("not allowed") {
                    Validation::Unavailable("this account can't use sudo".to_string())
                } else {
                    Validation::Unavailable("sudo didn't take the password".to_string())
                }
            }
        }
    }

    fn open(&self, password: &Password, budget: Budget) -> Result<Box<dyn AdminWindow>, String> {
        disable_core_dumps();
        let helper = fs::canonicalize(&self.helper)
            .map_err(|error| format!("the password helper isn't there: {error}"))?;
        let folder = private_folder(&self.folder_parent)
            .map_err(|error| format!("couldn't make a private folder: {error}"))?;
        let opened = open_in(&folder, &helper).map_err(|error| error.to_string());
        let (askpass, listener) = match opened {
            Ok(opened) => opened,
            Err(why) => {
                let _ = fs::remove_dir_all(&folder);
                return Err(format!("couldn't open the password helper: {why}"));
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let server = Server {
            listener,
            socket: folder.join("s"),
            secret: Secret::copy_of(password),
            budget,
            helper,
            rule: self.rule.clone(),
            stop: Arc::clone(&stop),
        };
        Ok(Box::new(SocketWindow {
            folder,
            askpass,
            sudo: self.sudo.clone(),
            stop,
            serving: Some(thread::spawn(move || server.serve())),
        }))
    }

    fn forget(&self, runner: &dyn CommandRunner, home: &Path) -> bool {
        runner
            .run(&self.plain(home).args(["-k"]), &mut |_, _| {})
            .is_ok_and(|output| output.is_success())
    }
}

fn open_in(folder: &Path, helper: &Path) -> std::io::Result<(PathBuf, UnixListener)> {
    let askpass = folder.join("askpass");
    symlink(helper, &askpass)?;
    let listener = UnixListener::bind(folder.join("s"))?;
    listener.set_nonblocking(true)?;
    Ok((askpass, listener))
}

/// A new folder only this user can enter. `mkdir` fails if the name is taken, so nobody can have
/// put one there first.
fn private_folder(parent: &Path) -> std::io::Result<PathBuf> {
    for _ in 0..8 {
        let mut noise = [0_u8; 8];
        fs::File::open("/dev/urandom").and_then(|mut file| {
            use std::io::Read;
            file.read_exact(&mut noise)
        })?;
        let name: String = noise.iter().map(|byte| format!("{byte:02x}")).collect();
        let folder = parent.join(format!("sc-{name}"));
        match fs::DirBuilder::new().mode(0o700).create(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("no free name"))
}

struct SocketWindow {
    folder: PathBuf,
    askpass: PathBuf,
    sudo: PathBuf,
    stop: Arc<AtomicBool>,
    serving: Option<JoinHandle<()>>,
}

impl AdminWindow for SocketWindow {
    fn elevated(&self, inner: Command) -> Command {
        let mut args = vec![
            "-A".to_string(),
            "--".to_string(),
            inner.program.to_string_lossy().into_owned(),
        ];
        args.extend(inner.args);
        let mut command = Command::new(&self.sudo).args(args);
        command.env = inner.env;
        command.cwd = inner.cwd;
        command.stall_limit = inner.stall_limit;
        self.may_elevate(command)
    }

    fn may_elevate(&self, inner: Command) -> Command {
        inner.env("SUDO_ASKPASS", self.askpass.to_string_lossy())
    }
}

impl Drop for SocketWindow {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(serving) = self.serving.take() {
            let _ = serving.join();
        }
        let _ = fs::remove_dir_all(&self.folder);
    }
}

struct Server {
    listener: UnixListener,
    socket: PathBuf,
    secret: Secret,
    budget: Budget,
    helper: PathBuf,
    rule: PeerRule,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn serve(self) {
        while !self.stop.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.answer(stream) && self.budget == Budget::One {
                        break;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(_) => break,
            }
        }
        // The step may run on for an hour. Nothing should be listening for it.
        let _ = fs::remove_file(&self.socket);
    }

    fn answer(&self, mut stream: UnixStream) -> bool {
        if !self.peer_allowed(&stream) {
            return false;
        }
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        stream.write_all(self.secret.line()).is_ok()
    }

    fn peer_allowed(&self, stream: &UnixStream) -> bool {
        let Some((uid, pid)) = peer_of(stream) else {
            return false;
        };
        // SAFETY: `getuid` takes nothing and can't fail.
        if uid != unsafe { libc::getuid() } {
            return false;
        }
        let Some(path) = path_of(pid) else {
            return false;
        };
        if path != self.helper {
            return false;
        }
        match &self.rule {
            PeerRule::PathOnly => true,
            PeerRule::Signed { identifier, team } => signed_as(&path, identifier, team),
        }
    }
}

fn signed_as(path: &Path, identifier: &str, team: &str) -> bool {
    let requirement = format!(
        "identifier \"{identifier}\" and anchor apple generic and certificate leaf[subject.OU] = \"{team}\""
    );
    let command = Command::new("/usr/bin/codesign")
        .args([
            "--verify".to_string(),
            "--strict".to_string(),
            format!("-R={requirement}"),
            path.to_string_lossy().into_owned(),
        ])
        .stall_limit(Duration::from_secs(10));
    ProcessRunner::new()
        .run(&command, &mut |_, _| {})
        .is_ok_and(|output| output.is_success())
}

fn peer_of(stream: &UnixStream) -> Option<(libc::uid_t, libc::pid_t)> {
    let fd = stream.as_raw_fd();
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: `fd` stays open for the call, and the pointers are to live locals of the right type.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return None;
    }
    let mut pid: libc::pid_t = 0;
    let mut size = size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: the same, and `size` says how much `pid` can hold.
    let read = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &mut size,
        )
    };
    (read == 0 && pid > 0).then_some((uid, pid))
}

fn path_of(pid: libc::pid_t) -> Option<PathBuf> {
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buffer` is as long as the size passed.
    let length =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if length <= 0 {
        return None;
    }
    buffer.truncate(length as usize);
    let text = String::from_utf8(buffer).ok()?;
    fs::canonicalize(text).ok()
}

/// The server thread's own copy. It stays out of swap where the system allows, and is zeroed when
/// the window closes.
struct Secret(Box<[u8]>);

impl Secret {
    fn copy_of(password: &Password) -> Self {
        let mut bytes = Vec::with_capacity(password.expose().len() + 1);
        bytes.extend_from_slice(password.expose().as_bytes());
        bytes.push(b'\n');
        let bytes = bytes.into_boxed_slice();
        lock_in_memory(&bytes);
        Self(bytes)
    }

    fn line(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            // SAFETY: a valid, exclusive reference. Volatile so the write isn't optimized away.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        unlock_memory(&self.0);
    }
}

/// Best effort: a refusal leaves the password in ordinary memory, which is still zeroed.
pub(crate) fn lock_in_memory(bytes: &[u8]) {
    if !bytes.is_empty() {
        // SAFETY: the range is a live slice. `mlock` only changes how the pages are kept.
        unsafe { libc::mlock(bytes.as_ptr().cast(), bytes.len()) };
    }
}

pub(crate) fn unlock_memory(bytes: &[u8]) {
    if !bytes.is_empty() {
        // SAFETY: the range is a live slice. `munlock` only changes how the pages are kept.
        unsafe { libc::munlock(bytes.as_ptr().cast(), bytes.len()) };
    }
}

/// A core file would hold the password. This lasts for the process, which is what's wanted.
pub(crate) fn disable_core_dumps() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid `rlimit` for the length of the call.
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) };
}
