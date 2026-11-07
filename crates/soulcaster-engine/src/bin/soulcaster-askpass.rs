//! What `sudo -A` runs. `sudo` closes everything above stderr first and takes stdout for itself,
//! so the only thing that reaches this program is the path it was started by: a symlink in a
//! private folder, beside a socket that exists only while an admin step runs.
//!
//! It prints what the socket answers and nothing else. With no answer it prints nothing and fails,
//! so `sudo` gets no password rather than a wrong one.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

/// Longer than any password, and small enough that a wrong socket can't fill memory.
const MOST: u64 = 4096;

fn answer() -> Option<Vec<u8>> {
    let started_as = PathBuf::from(std::env::args_os().next()?);
    if !started_as.is_absolute() {
        return None;
    }
    let socket = started_as.parent()?.join("s");
    let stream = UnixStream::connect(socket).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    let mut bytes = Vec::new();
    stream.take(MOST).read_to_end(&mut bytes).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

fn main() -> ExitCode {
    let Some(mut bytes) = answer() else {
        return ExitCode::FAILURE;
    };
    let wrote = std::io::stdout()
        .write_all(&bytes)
        .and_then(|()| std::io::stdout().flush());
    bytes.fill(0);
    match wrote {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
