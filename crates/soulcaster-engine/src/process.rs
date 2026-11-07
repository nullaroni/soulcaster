//! A [`CommandRunner`](soulcaster_core::command::CommandRunner) that starts a real process.
//! The environment is exactly the command's `env`. Stdin is `/dev/null`. Output is capped, and a
//! stall limit kills the process group.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command as StdCommand, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Instant;

use soulcaster_core::command::{
    Command, CommandEnd, CommandOutput, CommandRunner, OutputStream, SpawnError,
};

const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;

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
    Line(OutputStream, String),
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
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut stalled = false;
        let mut last = Instant::now();

        while !stdout_done || !stderr_done {
            let waited = last.elapsed();
            let message = match command.stall_limit {
                Some(limit) if waited >= limit => Err(RecvTimeoutError::Timeout),
                Some(limit) => rx.recv_timeout(limit - waited),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match message {
                Ok(PipeMsg::Line(stream, line)) => {
                    last = Instant::now();
                    push_capped(
                        stream,
                        &line,
                        &mut stdout_buf,
                        &mut stderr_buf,
                        self.output_limit,
                        on_line,
                    );
                }
                Ok(PipeMsg::Eof(OutputStream::Stdout)) => stdout_done = true,
                Ok(PipeMsg::Eof(OutputStream::Stderr)) => stderr_done = true,
                Err(RecvTimeoutError::Timeout) => {
                    kill_group(pid);
                    stalled = true;
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        let status = child.wait();
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();

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
    let Some(pipe) = pipe else {
        let _ = tx.send(PipeMsg::Eof(stream));
        return;
    };
    let mut reader = BufReader::new(pipe);
    loop {
        let mut buf = Vec::new();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                if tx.send(PipeMsg::Line(stream, line)).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(PipeMsg::Eof(stream));
}

fn push_capped(
    stream: OutputStream,
    line: &str,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    limit: usize,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    let buf = match stream {
        OutputStream::Stdout => &mut *stdout,
        OutputStream::Stderr => &mut *stderr,
    };
    if buf.len() >= limit {
        return;
    }
    on_line(stream, line);
    buf.extend_from_slice(line.as_bytes());
    if buf.len() < limit {
        buf.push(b'\n');
    }
    if buf.len() > limit {
        buf.truncate(limit);
    }
}

fn kill_group(pid: u32) {
    // The child leads its own group (`process_group(0)`), so a negative pid also reaches
    // grandchildren. `Child::kill` would leave those running.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn end_from(status: std::io::Result<ExitStatus>) -> CommandEnd {
    match status {
        Ok(status) => match status.code() {
            Some(code) => CommandEnd::Exited(code),
            None => CommandEnd::Signaled(signal_of(&status)),
        },
        Err(_) => CommandEnd::Signaled(0),
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
    fn a_failing_command_reports_its_exit_code() {
        let output = run(Command::new("/usr/bin/false"));
        assert_eq!(output.end, CommandEnd::Exited(1));
    }
}
