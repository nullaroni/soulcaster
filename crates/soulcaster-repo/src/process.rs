//! A [`CommandRunner`] that starts real processes, for the CLI until the wiring ticket decides
//! whether the engine's runner replaces it. The environment is exactly the command's, stdin is
//! `/dev/null`, output is capped so a hostile server can't fill memory, and a stall or an overrun
//! kills the whole process group.

use std::io::Read;
use std::process::{Command as StdCommand, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use soulcaster_core::command::{
    Command, CommandEnd, CommandOutput, CommandRunner, OutputStream, SpawnError,
};

const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const LONGEST_LINE: usize = 64 * 1024;

#[derive(Default)]
pub struct ProcessRunner;

fn pump(
    mut pipe: impl Read + Send + 'static,
    stream: OutputStream,
    tx: mpsc::Sender<(OutputStream, Vec<u8>)>,
) {
    thread::spawn(move || {
        let mut chunk = [0_u8; 16 * 1024];
        while let Ok(read) = pipe.read(&mut chunk) {
            if read == 0 || tx.send((stream, chunk[..read].to_vec())).is_err() {
                break;
            }
        }
    });
}

/// Hands complete lines to `on_line`. A line longer than anything a person reads is cut.
fn send_lines(
    buffer: &mut Vec<u8>,
    stream: OutputStream,
    on_line: &mut dyn FnMut(OutputStream, &str),
) {
    while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=end).collect();
        on_line(
            stream,
            String::from_utf8_lossy(&line[..end]).trim_end_matches('\r'),
        );
    }
    if buffer.len() > LONGEST_LINE {
        on_line(stream, &String::from_utf8_lossy(buffer));
        buffer.clear();
    }
}

impl CommandRunner for ProcessRunner {
    fn run(
        &self,
        command: &Command,
        on_line: &mut dyn FnMut(OutputStream, &str),
    ) -> Result<CommandOutput, SpawnError> {
        use std::os::unix::process::CommandExt;
        let mut start = StdCommand::new(&command.program);
        start
            .args(&command.args)
            .env_clear()
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(cwd) = &command.cwd {
            start.current_dir(cwd);
        }
        let mut child = start.spawn().map_err(|e| SpawnError {
            program: command.program.clone(),
            message: e.to_string(),
        })?;
        let group = Pid::from_child(&child);
        let (tx, rx) = mpsc::channel();
        if let Some(pipe) = child.stdout.take() {
            pump(pipe, OutputStream::Stdout, tx.clone());
        }
        if let Some(pipe) = child.stderr.take() {
            pump(pipe, OutputStream::Stderr, tx.clone());
        }
        drop(tx);

        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let (mut out_line, mut err_line) = (Vec::new(), Vec::new());
        let mut killed = false;
        let mut last_output = Instant::now();
        let limit = command.stall_limit.unwrap_or(Duration::from_secs(86_400));
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok((stream, chunk)) => {
                    last_output = Instant::now();
                    let (whole, line) = match stream {
                        OutputStream::Stdout => (&mut stdout, &mut out_line),
                        OutputStream::Stderr => (&mut stderr, &mut err_line),
                    };
                    if whole.len() + chunk.len() > OUTPUT_LIMIT && !killed {
                        killed = true;
                        let _ = kill_process_group(group, Signal::KILL);
                    }
                    if !killed {
                        whole.extend_from_slice(&chunk);
                        line.extend_from_slice(&chunk);
                        send_lines(line, stream, on_line);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if !killed && last_output.elapsed() > limit {
                        killed = true;
                        let _ = kill_process_group(group, Signal::KILL);
                    }
                }
            }
        }
        let status = child.wait().map_err(|e| SpawnError {
            program: command.program.clone(),
            message: e.to_string(),
        })?;
        use std::os::unix::process::ExitStatusExt;
        let end = if killed {
            CommandEnd::Stalled
        } else if let Some(code) = status.code() {
            CommandEnd::Exited(code)
        } else {
            CommandEnd::Signaled(status.signal().unwrap_or(0))
        };
        Ok(CommandOutput {
            end,
            stdout,
            stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(command: Command) -> CommandOutput {
        ProcessRunner.run(&command, &mut |_, _| {}).unwrap()
    }

    #[test]
    fn the_environment_is_exactly_what_the_command_says() {
        // SAFETY of the test: `/usr/bin/env` only prints. The variable set here must not reach it.
        let out = run(Command::new("/usr/bin/env").env("ONLY", "this"));
        assert_eq!(out.stdout_text(), "ONLY=this\n");
    }

    #[test]
    fn stdin_is_empty_so_nothing_waits_on_a_person() {
        let out = run(Command::new("/bin/cat"));
        assert!(out.is_success());
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn a_stall_kills_the_whole_group() {
        let out = run(Command::new("/bin/sh")
            .args(["-c", "sleep 60 & wait"])
            .stall_limit(Duration::from_millis(200)));
        assert_eq!(out.end, CommandEnd::Stalled);
    }

    #[test]
    fn a_process_that_never_started_is_a_distinct_result() {
        let error = ProcessRunner
            .run(&Command::new("/nonexistent/program"), &mut |_, _| {})
            .unwrap_err();
        assert!(error.program.ends_with("program"));
    }

    #[test]
    fn output_arrives_line_by_line_and_whole() {
        let mut lines = Vec::new();
        let out = ProcessRunner
            .run(
                &Command::new("/bin/sh").args(["-c", "echo one; echo two"]),
                &mut |_, line| lines.push(line.to_string()),
            )
            .unwrap();
        assert_eq!(lines, ["one", "two"]);
        assert_eq!(out.stdout_text(), "one\ntwo\n");
    }
}
