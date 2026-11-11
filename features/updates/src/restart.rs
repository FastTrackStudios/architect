//! Starting the app again once this process has exited.
//!
//! A watcher (`sh`) waits for a pid to go, then runs the program; it gives
//! up after a bound, so a quit that never happens never restarts the app
//! later. The caller keeps the [`Watcher`] and cancels it when the quit
//! fails.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// How long the watcher waits for the app to exit before it gives up.
pub const MAX_WAIT: Duration = Duration::from_secs(60);

/// A running watcher; [`Watcher::cancel`] stops it before it starts
/// anything.
#[derive(Debug)]
pub struct Watcher(Child);

impl Watcher {
    /// Stop it: nothing is started.
    pub fn cancel(mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }

    /// Leave it running (the app is quitting); it exits on its own.
    pub fn detach(self) {
        drop(self.0);
    }
}

/// Start `program` once process `pid` has exited, if that happens within
/// `max_wait`; otherwise start nothing.
///
/// # Errors
///
/// The watcher cannot be started (no `sh`), or not on this platform.
#[cfg(unix)]
pub fn start_after_exit(program: &Path, pid: u32, max_wait: Duration) -> Result<Watcher, String> {
    // Polls every 0.2 s: the bound in polls.
    let polls = max_wait.as_millis().div_ceil(200).max(1);
    Command::new("sh")
        .arg("-c")
        .arg(
            r#"i=0; while kill -0 "$1" 2>/dev/null; do i=$((i+1)); [ "$i" -gt "$2" ] && exit 0; sleep 0.2; done; exec "$0""#,
        )
        .arg(program)
        .arg(pid.to_string())
        .arg(polls.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Watcher)
        .map_err(|e| format!("start {} again: {e}", program.display()))
}

/// Not on this platform yet.
///
/// # Errors
///
/// Always.
#[cfg(not(unix))]
pub fn start_after_exit(_: &Path, _: u32, _: Duration) -> Result<Watcher, String> {
    Err("restarting into an update is not supported on this platform yet".to_owned())
}

/// Open `url` in the system's browser.
///
/// # Errors
///
/// The opener cannot be started.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("open {url}: {e}"))
}
