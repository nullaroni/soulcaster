//! Reviewer probes for the real runner. Only harmless commands: sh, sleep, perl sleeping.
use soulcaster_core::command::{Command, CommandEnd, CommandRunner};
use soulcaster_engine::ProcessRunner;
use std::time::{Duration, Instant};

#[test]
fn p1_a_background_child_holding_stdout_does_not_make_a_finished_command_look_stalled() {
    let started = Instant::now();
    let out = ProcessRunner::new()
        .run(
            &Command::new("/bin/sh")
                .args(["-c", "/bin/sleep 4 & exit 0"])
                .stall_limit(Duration::from_millis(500)),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(
        out.end,
        CommandEnd::Exited(0),
        "after {:?}",
        started.elapsed()
    );
}

#[test]
fn p2_a_stall_returns_even_when_a_daemonized_grandchild_keeps_the_pipe_open() {
    let started = Instant::now();
    let _ = ProcessRunner::new()
        .run(
            &Command::new("/usr/bin/perl")
                .args([
                    "-e",
                    "use POSIX; if (fork) { sleep 20; exit 0 } POSIX::setsid(); sleep 6",
                ])
                .stall_limit(Duration::from_millis(300)),
            &mut |_, _| {},
        )
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "run() blocked for {:?} after the stall",
        started.elapsed()
    );
}

#[test]
fn p3_output_with_no_newline_counts_as_progress_and_is_bounded() {
    let out = ProcessRunner::new()
        .run(
            &Command::new("/bin/sh")
                .args([
                    "-c",
                    "i=0; while [ $i -lt 12 ]; do printf '.'; /bin/sleep 0.1; i=$((i+1)); done",
                ])
                .stall_limit(Duration::from_millis(500)),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(
        out.end,
        CommandEnd::Exited(0),
        "steady progress with no newline was killed as a stall"
    );
}
