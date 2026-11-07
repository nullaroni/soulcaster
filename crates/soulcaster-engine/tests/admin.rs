//! The real gate against a stand-in for `sudo`, with the real helper binary and real sockets. The
//! stand-in does what the design says `sudo` does: it starts `SUDO_ASKPASS` by its path with nothing
//! above stderr, and reads one line. Nothing here runs `/usr/bin/sudo`.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use soulcaster_core::ask::Password;
use soulcaster_core::command::{Command, CommandEnd, CommandRunner};
use soulcaster_engine::{AdminGate, Budget, PeerRule, ProcessRunner, SocketGate, Validation};

const HELPER: &str = env!("CARGO_BIN_EXE_soulcaster-askpass");
const SECRET: &str = "correct horse";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("soulcaster-admin-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// `sudo -A -v` and `sudo -A -- program args`, as a script. `-k` always succeeds.
    fn fake_sudo(&self) -> PathBuf {
        let path = self.0.join("sudo");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
exec 3>&- 4>&- 5>&- 6>&- 7>&- 8>&- 9>&-
if [ "$1" = "-k" ]; then echo forgot >> "{log}"; exit 0; fi
answer=$("$SUDO_ASKPASS" "Password:") || {{ echo "sudo: no password was provided" >&2; exit 1; }}
if [ "$answer" != "{SECRET}" ]; then echo "Sorry, try again." >&2; exit 1; fi
if [ "$1" = "-A" ] && [ "$2" = "-v" ]; then exit 0; fi
shift 2
exec "$@"
"#,
                log = self.0.join("log").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn gate(&self) -> SocketGate {
        SocketGate::new(PathBuf::from(HELPER), PeerRule::PathOnly).with_sudo(self.fake_sudo())
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

fn run(command: &Command) -> soulcaster_core::command::CommandOutput {
    ProcessRunner::new().run(command, &mut |_, _| {}).unwrap()
}

fn askpass_of(command: &Command) -> PathBuf {
    command
        .env
        .iter()
        .find(|(key, _)| key == "SUDO_ASKPASS")
        .map(|(_, value)| PathBuf::from(value))
        .expect("the command has the helper")
}

/// Starts the helper the way `sudo` does: by the symlink's path, with no environment.
fn ask_helper(askpass: &Path) -> (bool, String) {
    let out = run(&Command::new(askpass).args(["Password:"]));
    (out.is_success(), out.stdout_text())
}

#[test]
fn the_right_password_validates_and_the_wrong_one_is_refused_without_a_retry() {
    let sandbox = Sandbox::new("validate");
    let gate = sandbox.gate();
    let runner = ProcessRunner::new();
    assert_eq!(
        gate.validate(&runner, sandbox.path(), &Password::new(SECRET.to_string())),
        Validation::Accepted
    );
    assert_eq!(
        gate.validate(&runner, sandbox.path(), &Password::new("nope".to_string())),
        Validation::Wrong
    );
    // Each try began by clearing any timestamp.
    assert_eq!(
        fs::read_to_string(sandbox.path().join("log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn a_missing_helper_means_no_password_rather_than_a_wrong_one() {
    let sandbox = Sandbox::new("no-helper");
    let gate = SocketGate::new(sandbox.path().join("gone"), PeerRule::PathOnly)
        .with_sudo(sandbox.fake_sudo());
    assert!(matches!(
        gate.validate(
            &ProcessRunner::new(),
            sandbox.path(),
            &Password::new(SECRET.to_string())
        ),
        Validation::Unavailable(_)
    ));
}

#[test]
fn an_elevated_step_runs_through_sudo_and_never_carries_the_password() {
    let sandbox = Sandbox::new("elevated");
    let gate = sandbox.gate();
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::One)
        .unwrap();
    let command = window.elevated(
        Command::new("/bin/echo")
            .args(["ran as asked"])
            .env("HOME", "/nowhere")
            .stall_limit(Duration::from_secs(9)),
    );
    let rendered = format!("{command:?}");
    assert!(!rendered.contains(SECRET), "{rendered}");
    assert_eq!(command.program, sandbox.path().join("sudo"));
    assert_eq!(&command.args[..3], ["-A", "--", "/bin/echo"]);
    assert_eq!(command.stall_limit, Some(Duration::from_secs(9)));
    let out = run(&command);
    assert!(
        out.is_success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout_text(), "ran as asked\n");
}

#[test]
fn the_folder_is_private_and_is_gone_when_the_step_ends() {
    let sandbox = Sandbox::new("folder");
    let gate = sandbox.gate();
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::AsNeeded)
        .unwrap();
    let askpass = askpass_of(&window.may_elevate(Command::new("/bin/true")));
    let folder = askpass.parent().unwrap().to_path_buf();
    let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    assert_eq!(
        fs::read_link(&askpass).unwrap(),
        fs::canonicalize(HELPER).unwrap()
    );
    for entry in fs::read_dir(&folder).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap_or_default();
        assert!(!String::from_utf8_lossy(&bytes).contains(SECRET));
    }
    drop(window);
    assert!(!folder.exists());
}

#[test]
fn a_budget_of_one_answers_once_and_then_nothing_is_listening() {
    let sandbox = Sandbox::new("one");
    let gate = sandbox.gate();
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::One)
        .unwrap();
    let askpass = askpass_of(&window.may_elevate(Command::new("/bin/true")));
    assert_eq!(ask_helper(&askpass), (true, format!("{SECRET}\n")));
    let socket = askpass.parent().unwrap().join("s");
    for _ in 0..100 {
        if !socket.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!socket.exists(), "the socket stayed open after its answer");
    assert_eq!(ask_helper(&askpass), (false, String::new()));
}

#[test]
fn a_brew_cask_can_ask_as_often_as_it_needs() {
    let sandbox = Sandbox::new("many");
    let gate = sandbox.gate();
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::AsNeeded)
        .unwrap();
    let askpass = askpass_of(&window.may_elevate(Command::new("/bin/true")));
    for _ in 0..3 {
        assert_eq!(ask_helper(&askpass), (true, format!("{SECRET}\n")));
    }
}

#[test]
fn anything_but_the_helper_gets_nothing_even_as_the_same_user() {
    let sandbox = Sandbox::new("peer");
    let gate = sandbox.gate();
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::AsNeeded)
        .unwrap();
    let askpass = askpass_of(&window.may_elevate(Command::new("/bin/true")));
    // This test binary is the same user, and it isn't the helper.
    let mut stream = UnixStream::connect(askpass.parent().unwrap().join("s")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut got = Vec::new();
    let _ = stream.read_to_end(&mut got);
    assert!(got.is_empty(), "the socket answered a stranger");
    // A refusal doesn't spend the budget or close the door on the real helper.
    assert_eq!(ask_helper(&askpass), (true, format!("{SECRET}\n")));
}

#[test]
fn a_helper_without_the_required_signature_gets_nothing() {
    let sandbox = Sandbox::new("signed");
    let gate = SocketGate::new(
        PathBuf::from(HELPER),
        PeerRule::Signed {
            identifier: "com.nullworks.macflayer.askpass".to_string(),
            team: "2ZEFAR8TH3".to_string(),
        },
    )
    .with_sudo(sandbox.fake_sudo());
    let window = gate
        .open(&Password::new(SECRET.to_string()), Budget::AsNeeded)
        .unwrap();
    let askpass = askpass_of(&window.may_elevate(Command::new("/bin/true")));
    assert_eq!(ask_helper(&askpass), (false, String::new()));
}

#[test]
fn the_helper_refuses_a_relative_start_and_prints_nothing() {
    let out = run(&Command::new("/bin/sh").args([
        "-c",
        &format!("cd /tmp; exec -a relative/askpass {HELPER} 2>/dev/null || exit 9"),
    ]));
    assert_eq!(out.stdout_text(), "");
    assert_ne!(out.end, CommandEnd::Exited(0));
}

#[test]
fn forget_ends_sudos_own_timestamp() {
    let sandbox = Sandbox::new("forget");
    sandbox.gate().forget(&ProcessRunner::new(), sandbox.path());
    assert_eq!(
        fs::read_to_string(sandbox.path().join("log")).unwrap(),
        "forgot\n"
    );
}
