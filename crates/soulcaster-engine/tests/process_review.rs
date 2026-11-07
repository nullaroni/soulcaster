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

#[test]
fn p4_a_command_that_closes_its_pipes_and_hangs_still_hits_its_stall_limit() {
    let started = Instant::now();
    let out = ProcessRunner::new()
        .run(
            &Command::new("/bin/sh")
                .args(["-c", "exec >&- 2>&-; /bin/sleep 30"])
                .stall_limit(Duration::from_millis(300)),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(out.end, CommandEnd::Stalled);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "run() waited {:?} on a process with no open pipes",
        started.elapsed()
    );
}

#[test]
fn p5_output_written_before_the_exit_is_not_lost_to_a_slow_reader() {
    // 20 KB fits in the pipe, so the command has exited before the first line is read, and each
    // chunk takes longer to hand over than the drain's quiet period. No clock decides this one.
    let mut lines = 0;
    let out = ProcessRunner::new()
        .run(
            &Command::new("/usr/bin/perl").args(["-e", "print qq(x\\n) x 10000"]),
            &mut |_, _| {
                std::thread::sleep(Duration::from_micros(100));
                lines += 1;
            },
        )
        .unwrap();
    assert_eq!(out.end, CommandEnd::Exited(0));
    assert_eq!(lines, 10000);
}

#[test]
fn p6_a_slow_reader_is_not_mistaken_for_a_stall() {
    let out = ProcessRunner::new()
        .run(
            &Command::new("/bin/sh")
                .args(["-c", "echo one; /bin/sleep 0.3; echo two"])
                .stall_limit(Duration::from_millis(250)),
            &mut |_, _| std::thread::sleep(Duration::from_millis(300)),
        )
        .unwrap();
    assert_eq!(out.end, CommandEnd::Exited(0));
}

#[test]
fn p7_a_grandchild_that_floods_after_the_exit_cannot_hold_the_command_open() {
    let started = Instant::now();
    let out = ProcessRunner::new()
        .run(
            &Command::new("/bin/sh").args(["-c", "/usr/bin/yes & exit 0"]),
            &mut |_, _| {},
        )
        .unwrap();
    assert_eq!(out.end, CommandEnd::Exited(0));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "run() took {:?}",
        started.elapsed()
    );
}
