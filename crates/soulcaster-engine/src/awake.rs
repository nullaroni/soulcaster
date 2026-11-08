//! Holds the Mac awake for a Run, so the person can walk away.

use std::process::{Child, Command, Stdio};

/// Keeps the Mac awake for as long as this lives. `-w` ends it if this process dies first.
pub struct KeepAwake(Option<Child>);

impl KeepAwake {
    pub fn start() -> Self {
        let child = Command::new("/usr/bin/caffeinate")
            .args(["-dimsu", "-w", &std::process::id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        Self(child)
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
