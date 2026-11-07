//! What `sudo -A` runs. `sudo` closes everything above stderr first and takes stdout for itself,
//! so the only thing that reaches this program is the path it was started by: a symlink in a
//! private folder, beside a socket that exists only while an admin step runs.
//!
//! It prints what the socket answers and nothing else. With no answer it prints nothing and fails,
//! so `sudo` gets no password rather than a wrong one.

#![deny(unsafe_code)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use zeroize::Zeroize;

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

/// It holds the password for a moment, and a core file would keep it.
#[allow(unsafe_code)]
fn no_core_dumps() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid `rlimit` for the length of the call.
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) };
}

fn main() -> ExitCode {
    no_core_dumps();
    let Some(mut bytes) = answer() else {
        return ExitCode::FAILURE;
    };
    let wrote = std::io::stdout()
        .write_all(&bytes)
        .and_then(|()| std::io::stdout().flush());
    bytes.zeroize();
    match wrote {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
