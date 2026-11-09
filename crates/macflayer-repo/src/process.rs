//! The runner for reading from GitHub and running git: the engine's, with its output treated as
//! data from someone who may be hostile. A server can't fill memory, and an answer that was cut
//! off can't pass for a whole one.

use macflayer_engine::ProcessRunner;

const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;

pub fn runner() -> ProcessRunner {
    ProcessRunner::killing_past(OUTPUT_LIMIT)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use macflayer_core::command::{Command, CommandEnd, CommandOutput, CommandRunner};

    use super::*;

    fn run(command: Command) -> CommandOutput {
        runner().run(&command, &mut |_, _| {}).unwrap()
    }

    #[test]
    fn the_environment_is_exactly_what_the_command_says() {
        // `/usr/bin/env` only prints. A variable set for the test run must not reach it.
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
        let error = runner()
            .run(&Command::new("/nonexistent/program"), &mut |_, _| {})
            .unwrap_err();
        assert!(error.message.contains("No such file"), "{}", error.message);
    }
}
