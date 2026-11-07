//! A [`CommandRunner`](soulcaster_core::command::CommandRunner) that starts a real process.
//! The environment is exactly the command's `env`. Stdin is `/dev/null`. Output is capped, and a
//! stall limit kills the process group.

use std::io::Read;
use std::process::{Child, Command as StdCommand, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use soulcaster_core::command::{
    Command, CommandEnd, CommandOutput, CommandRunner, OutputStream, SpawnError,
};

const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;
const LINE_CAP: usize = 64 * 1024;
/// After the child exits, stop once its pipes have been quiet this long. A grandchild that
/// inherited a pipe must not keep `run` blocked.
const DRAIN: Duration = Duration::from_millis(100);
/// Output already written before the exit is still read, up to this long after it.
const DRAIN_MAX: Duration = Duration::from_secs(5);
/// How often to look at the child once both pipes have closed and it hasn't exited.
const EXIT_POLL: Duration = Duration::from_millis(5);

pub struct ProcessRunner {
    output_limit: usize,
}

impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }
}

impl ProcessRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_output_limit(output_limit: usize) -> Self {
        Self { output_limit }
    }
}

enum PipeMsg {
    Chunk(OutputStream, Vec<u8>),
    Eof(OutputStream),
}

impl CommandRunner for ProcessRunner {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        let mut child = spawn(command)?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::sync_channel(64);
        let stdout_tx = tx.clone();
        let stdout_thread =
            thread::spawn(move || read_pipe(stdout, OutputStream::Stdout, stdout_tx));
        let stderr_thread = thread::spawn(move || read_pipe(stderr, OutputStream::Stderr, tx));

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_line = Vec::new();
        let mut stderr_line = Vec::new();
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut stalled = false;
        let mut exited: Option<ExitStatus> = None;
        let mut exited_at: Option<Instant> = None;
        let mut drain_until: Option<Instant> = None;
        let mut last = Instant::now();

        loop {
            if exited.is_none()
                && let Ok(Some(status)) = child.try_wait()
            {
                exited = Some(status);
                let now = Instant::now();
                exited_at = Some(now);
                drain_until = Some(now + DRAIN);
            }
            let pipes_closed = stdout_done && stderr_done;
            if pipes_closed && exited.is_some() {
                break;
            }
            if drain_until.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            if exited.is_none()
                && let Some(limit) = command.stall_limit
                && last.elapsed() >= limit
            {
                kill_group(pid);
                stalled = true;
                break;
            }

            // A process can close both pipes and carry on. Its stall limit still applies.
            if pipes_closed {
                thread::sleep(EXIT_POLL);
                continue;
            }

            let slice = next_wait(command.stall_limit, last, drain_until);
            match rx.recv_timeout(slice) {
                Ok(PipeMsg::Chunk(stream, bytes)) => {
                    // Any byte is progress, including a line that has not ended yet.
                    last = Instant::now();
                    if let Some(at) = exited_at {
                        drain_until = Some((last + DRAIN).min(at + DRAIN_MAX));
                    }
                    let (line, buf) = match stream {
                        OutputStream::Stdout => (&mut stdout_line, &mut stdout_buf),
                        OutputStream::Stderr => (&mut stderr_line, &mut stderr_buf),
                    };
                    absorb(stream, &bytes, line, buf, self.output_limit, on_line);
                }
                Ok(PipeMsg::Eof(OutputStream::Stdout)) => stdout_done = true,
                Ok(PipeMsg::Eof(OutputStream::Stderr)) => stderr_done = true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    stdout_done = true;
                    stderr_done = true;
                }
            }
        }

        flush_partial(
            OutputStream::Stdout,
            &mut stdout_line,
            &mut stdout_buf,
            self.output_limit,
            on_line,
        );
        flush_partial(
            OutputStream::Stderr,
            &mut stderr_line,
            &mut stderr_buf,
            self.output_limit,
            on_line,
        );

        let status = if stalled {
            let _ = child.wait();
            None
        } else {
            exited
        };
        // A reader blocked in `read` because a grandchild kept the pipe is detached. Joining it
        // would wait for that grandchild.
        if stdout_done && stderr_done {
            join_finished(stdout_thread);
            join_finished(stderr_thread);
        }

        let end = if stalled {
            CommandEnd::Stalled
        } else {
            end_from(status)
        };
        Ok(CommandOutput {
            end,
            stdout: stdout_buf,
            stderr: stderr_buf,
        })
    }
}

fn next_wait(stall: Option<Duration>, last: Instant, drain_until: Option<Instant>) -> Duration {
    let mut slice = Duration::from_millis(50);
    if let Some(limit) = stall {
        slice = slice.min(limit.saturating_sub(last.elapsed()));
    }
    if let Some(deadline) = drain_until {
        slice = slice.min(deadline.saturating_duration_since(Instant::now()));
    }
    if slice.is_zero() {
        Duration::from_millis(1)
    } else {
        slice
    }
}

fn join_finished(handle: JoinHandle<()>) {
    let _ = handle.join();
}

fn spawn(command: &Command) -> Result<Child, SpawnError> {
    use std::os::unix::process::CommandExt;

    let mut process = StdCommand::new(&command.program);
    process
        .args(&command.args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    for (key, value) in &command.env {
        process.env(key, value);
    }
    process.spawn().map_err(|error| SpawnError {
        program: command.program.clone(),
        message: error.to_string(),
    })
}

fn read_pipe<R: Read + Send>(pipe: Option<R>, stream: OutputStream, tx: SyncSender<PipeMsg>) {
    let Some(mut pipe) = pipe else {
        let _ = tx.send(PipeMsg::Eof(stream));
        return;
    };
    let mut buf = [0u8; 4096];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(PipeMsg::Chunk(stream, buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(PipeMsg::Eof(stream));
}

fn absorb(
    stream: OutputStream,
    bytes: &[u8],
    line: &mut Vec<u8>,
    buf: &mut Vec<u8>,
    limit: usize,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    for &byte in bytes {
        if byte == b'\n' {
            emit_line(stream, line, buf, limit, on_line);
            line.clear();
            continue;
        }
        if byte == b'\r' {
            continue;
        }
        if line.len() >= LINE_CAP {
            emit_line(stream, line, buf, limit, on_line);
            line.clear();
        }
        line.push(byte);
    }
}

fn flush_partial(
    stream: OutputStream,
    line: &mut Vec<u8>,
    buf: &mut Vec<u8>,
    limit: usize,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    if !line.is_empty() {
        emit_line(stream, line, buf, limit, on_line);
        line.clear();
    }
}

fn emit_line(
    stream: OutputStream,
    line: &[u8],
    buf: &mut Vec<u8>,
    limit: usize,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    // The cap is on what's kept in memory. The log still sees every line, and has its own cap.
    let text = String::from_utf8_lossy(line);
    on_line(stream, &text);
    if buf.len() >= limit {
        return;
    }
    let room = limit - buf.len();
    let take = line.len().min(room);
    buf.extend_from_slice(&line[..take]);
    if buf.len() < limit {
        buf.push(b'\n');
    }
    if buf.len() > limit {
        buf.truncate(limit);
    }
}

fn kill_group(pid: u32) {
    // The child leads its own group (`process_group(0)`), so a negative pid also reaches
    // grandchildren that stayed in the group. `Child::kill` would leave those running.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn end_from(status: Option<ExitStatus>) -> CommandEnd {
    match status {
        Some(status) => match status.code() {
            Some(code) => CommandEnd::Exited(code),
            None => CommandEnd::Signaled(signal_of(&status)),
        },
        None => CommandEnd::Signaled(0),
    }
}

fn signal_of(status: &ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status.signal().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulcaster_core::command::{Command, CommandEnd, CommandOutput, CommandRunner};
    use std::time::{Duration, Instant};

    fn run(command: Command) -> CommandOutput {
        ProcessRunner::new()
            .run(&command, &mut |_, _| {})
            .expect("the process starts")
    }

    #[test]
    fn echo_and_true_use_only_the_given_environment() {
        let output = run(Command::new("/bin/echo")
            .args(["hello"])
            .env("HOME", "/tmp/soulcaster-sandbox")
            .env("PATH", "/usr/bin:/bin"));
        assert!(output.is_success());
        assert_eq!(output.stdout_text(), "hello\n");

        let env = run(Command::new("/usr/bin/env")
            .env("HOME", "/tmp/soulcaster-sandbox")
            .env("SOULCASTER_ONLY", "yes"));
        let text = env.stdout_text();
        assert!(text.contains("HOME=/tmp/soulcaster-sandbox"), "{text}");
        assert!(text.contains("SOULCASTER_ONLY=yes"), "{text}");
        assert!(!text.contains("SHELL="), "{text}");
        assert!(
            !text
                .lines()
                .any(|line| line.starts_with("USER=") || line.starts_with("PATH=")),
            "{text}"
        );

        assert!(run(Command::new("/usr/bin/true")).is_success());
    }

    #[test]
    fn stdin_is_dev_null_so_cat_exits_instead_of_waiting() {
        let started = Instant::now();
        let output = run(Command::new("/bin/cat").stall_limit(Duration::from_millis(800)));
        assert!(output.is_success(), "{:?}", output.end);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_stall_limit_kills_the_process_group() {
        let started = Instant::now();
        let output = run(Command::new("/bin/sleep")
            .args(["30"])
            .stall_limit(Duration::from_millis(200)));
        assert_eq!(output.end, CommandEnd::Stalled);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn output_is_capped_and_the_working_directory_is_the_one_given() {
        let output = ProcessRunner::with_output_limit(4)
            .run(
                &Command::new("/bin/echo").args(["hello-there"]),
                &mut |_, _| {},
            )
            .unwrap();
        assert!(output.is_success());
        assert!(output.stdout.len() <= 4, "{}", output.stdout.len());

        let pwd = run(Command::new("/bin/pwd").cwd("/tmp"));
        let text = pwd.stdout_text();
        assert!(
            text.contains("/tmp") || text.contains("/private/tmp"),
            "{text}"
        );
    }

    #[test]
    fn lines_past_the_memory_cap_still_reach_the_log() {
        let mut seen = 0;
        let output = ProcessRunner::with_output_limit(4)
            .run(
                &Command::new("/bin/sh").args(["-c", "echo one; echo two; echo three"]),
                &mut |_, _| seen += 1,
            )
            .unwrap();
        assert_eq!(seen, 3);
        assert!(output.stdout.len() <= 4, "{}", output.stdout.len());
    }

    #[test]
    fn a_failing_command_reports_its_exit_code() {
        let output = run(Command::new("/usr/bin/false"));
        assert_eq!(output.end, CommandEnd::Exited(1));
    }
}
