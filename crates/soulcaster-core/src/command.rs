//! The command-runner seam: every external command the engine starts goes through
//! [`CommandRunner`]. Commands are structured, never shell strings.
//!
//! A command's environment is complete: a runner starts the process with exactly `env` and
//! nothing inherited, and stdin at `/dev/null`. That is why a linked `~/.zshenv` or a stray
//! `SUDO_ASKPASS` can't reach a step. There is no field for stdin data, so a password can't travel
//! through here: the Admin Stage answers `sudo -A` over its helper socket instead.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// An absolute path in real use, because `PATH` may be empty.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The whole environment. Nothing is inherited.
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// With no output for this long, the runner kills the process group and reports `Stalled`.
    pub stall_limit: Option<Duration>,
}

impl Command {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            stall_limit: None,
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn stall_limit(mut self, limit: Duration) -> Self {
        self.stall_limit = Some(limit);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// How a process ended. None of these counts as success on its own: only the Item's checks do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandEnd {
    Exited(i32),
    Signaled(i32),
    /// The stall limit passed and the runner killed the process group.
    Stalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub end: CommandEnd,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn is_success(&self) -> bool {
        self.end == CommandEnd::Exited(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// The process never started, as opposed to starting and failing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnError {
    pub program: PathBuf,
    pub message: String,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "couldn't start {}: {}",
            self.program.display(),
            self.message
        )
    }
}

impl std::error::Error for SpawnError {}

/// The one way the engine reaches outside itself. `on_line` gets each output line as it arrives,
/// for the log and the stall limit; the whole output also comes back in the result.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError>;
}

type Rule = (Box<dyn Fn(&Command) -> bool + Send + Sync>, VecDeque<Reply>);

/// A scripted answer from [`FakeRunner`].
#[derive(Clone, Debug)]
pub enum Reply {
    Output {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Stall,
    SpawnFails(String),
}

impl Reply {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self::Output {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn fails(code: i32, stderr: impl Into<String>) -> Self {
        Self::Output {
            code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

/// A recording fake for tests. It starts nothing. It records every command, answers from scripted
/// rules in the order added, and otherwise exits 0 with no output, so a test scripts only what it
/// cares about.
#[derive(Default)]
pub struct FakeRunner {
    calls: Mutex<Vec<Command>>,
    rules: Mutex<Vec<Rule>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers matching commands with `replies` in turn, and repeats the last one after that.
    pub fn on(
        &self,
        matches: impl Fn(&Command) -> bool + Send + Sync + 'static,
        replies: impl IntoIterator<Item = Reply>,
    ) -> &Self {
        self.rules
            .lock()
            .unwrap()
            .push((Box::new(matches), replies.into_iter().collect()));
        self
    }

    /// Every command so far, in order.
    pub fn calls(&self) -> Vec<Command> {
        self.calls.lock().unwrap().clone()
    }

    fn reply_for(&self, command: &Command) -> Reply {
        let mut rules = self.rules.lock().unwrap();
        let Some((_, replies)) = rules.iter_mut().find(|(matches, _)| matches(command)) else {
            return Reply::ok("");
        };
        match replies.len() {
            0 => Reply::ok(""),
            1 => replies[0].clone(),
            _ => replies.pop_front().expect("checked the length"),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        self.calls.lock().unwrap().push(command.clone());
        match self.reply_for(command) {
            Reply::SpawnFails(message) => Err(SpawnError {
                program: command.program.clone(),
                message,
            }),
            Reply::Stall => Ok(CommandOutput {
                end: CommandEnd::Stalled,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            Reply::Output {
                code,
                stdout,
                stderr,
            } => {
                stdout
                    .lines()
                    .for_each(|line| on_line(OutputStream::Stdout, line));
                stderr
                    .lines()
                    .for_each(|line| on_line(OutputStream::Stderr, line));
                Ok(CommandOutput {
                    end: CommandEnd::Exited(code),
                    stdout: stdout.into_bytes(),
                    stderr: stderr.into_bytes(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brew(args: &[&str]) -> Command {
        Command::new("/opt/homebrew/bin/brew")
            .args(args.iter().copied())
            .env("HOMEBREW_NO_SUDO", "1")
            .cwd("/tmp")
    }

    #[test]
    fn the_fake_records_structured_commands_in_order() {
        let runner = FakeRunner::new();
        runner
            .run(&brew(&["install", "ripgrep"]), &mut |_, _| {})
            .unwrap();
        runner
            .run(&brew(&["install", "--cask", "ghostty"]), &mut |_, _| {})
            .unwrap();
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].args, ["install", "--cask", "ghostty"]);
        assert_eq!(
            calls[1].env,
            [("HOMEBREW_NO_SUDO".to_string(), "1".to_string())]
        );
        assert_eq!(calls[1].cwd, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn an_unscripted_command_exits_zero_with_no_output() {
        let out = FakeRunner::new()
            .run(&brew(&["list"]), &mut |_, _| {})
            .unwrap();
        assert!(out.is_success());
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn scripted_replies_run_in_turn_and_the_last_one_repeats() {
        let runner = FakeRunner::new();
        runner.on(
            |c| c.args.contains(&"ripgrep".to_string()),
            [Reply::fails(1, "boom"), Reply::ok("fine")],
        );
        let run = || {
            runner
                .run(&brew(&["install", "ripgrep"]), &mut |_, _| {})
                .unwrap()
        };
        assert_eq!(run().end, CommandEnd::Exited(1));
        assert_eq!(run().stdout_text(), "fine");
        assert_eq!(run().stdout_text(), "fine");
    }

    #[test]
    fn output_arrives_line_by_line_and_whole() {
        let runner = FakeRunner::new();
        runner.on(|_| true, [Reply::ok("one\ntwo\n")]);
        let mut lines = Vec::new();
        let out = runner
            .run(&brew(&["x"]), &mut |stream, line| {
                lines.push((stream, line.to_string()))
            })
            .unwrap();
        assert_eq!(
            lines,
            [
                (OutputStream::Stdout, "one".to_string()),
                (OutputStream::Stdout, "two".to_string())
            ]
        );
        assert_eq!(out.stdout_text(), "one\ntwo\n");
    }

    #[test]
    fn a_stall_and_a_failed_start_are_distinct_results() {
        let runner = FakeRunner::new();
        runner.on(|c| c.args == ["stall"], [Reply::Stall]);
        runner.on(
            |c| c.args == ["missing"],
            [Reply::SpawnFails("no such file".to_string())],
        );
        assert_eq!(
            runner.run(&brew(&["stall"]), &mut |_, _| {}).unwrap().end,
            CommandEnd::Stalled
        );
        assert_eq!(
            runner
                .run(&brew(&["missing"]), &mut |_, _| {})
                .unwrap_err()
                .message,
            "no such file"
        );
    }
}
