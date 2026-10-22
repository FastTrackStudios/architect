//! The crash flight recorder: a bounded in-memory log ring, a crash log and
//! a flight log, so a run that dies leaves its last lines on disk.
//!
//! Feature `flight`, native only. Three pieces:
//!
//! - [`RingLayer`]: a `tracing` layer that keeps every event it sees in a
//!   process-wide ring of the last [`CAPACITY`] lines. An app's Logs view
//!   reads it with [`snapshot`].
//! - [`install_panic_recorder`]: a panic hook (chained onto whatever hook
//!   was installed before it) that puts the panic in the ring, appends the
//!   panic, a backtrace and the lines logged just before it to
//!   `<app_dir>/crash.log`, and writes the ring to
//!   `<app_dir>/flight-log.txt`. A panic on a UI thread aborts on iOS, so
//!   the trace has to reach disk inside the hook.
//! - [`restore_previous`]: at the next launch, the previous run's flight
//!   log goes back into the ring under a marker, and the file is moved
//!   aside to `flight-log-previous.txt`.
//!
//! Call [`persist`] from a timer if the system can kill the app without a
//! panic (memory pressure on a phone); the flight log then holds the last
//! lines up to that write.
//!
//! The recorder composes with `architect::host::install_panic_logger` in
//! either order and the panic lands in the ring once: when the other hook
//! logs the panic through `tracing` and a [`RingLayer`] records it, the
//! recorder does not add its own line.
//!
//! ```ignore
//! use tracing_subscriber::layer::SubscriberExt as _;
//! use tracing_subscriber::util::SubscriberInitExt as _;
//!
//! tracing_subscriber::registry()
//!     .with(tracing_subscriber::fmt::layer())
//!     .with(architect_telemetry::flight::RingLayer::new())
//!     .init();
//! architect_telemetry::flight::install_panic_recorder(app_data_dir);
//! architect_telemetry::flight::restore_previous();
//! ```

use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// How many lines the ring keeps; the oldest drop first.
pub const CAPACITY: usize = 600;

/// The crash log's file name inside the app dir.
pub const CRASH_LOG: &str = "crash.log";

/// The flight log's file name inside the app dir.
pub const FLIGHT_LOG: &str = "flight-log.txt";

/// Where [`restore_previous`] moves the flight log it read.
pub const FLIGHT_LOG_PREVIOUS: &str = "flight-log-previous.txt";

/// Past this size `crash.log` starts over: the newest crashes are the ones
/// read.
const CRASH_LOG_KEEP: u64 = 512 * 1024;

/// Lines of the ring copied under each crash in `crash.log`.
const CRASH_TAIL: usize = 40;

/// Ring lines searched for a panic another hook already logged.
const PANIC_LOOKBACK: usize = 8;

/// Lines of the previous flight log [`restore_previous`] puts back.
const RESTORE_TAIL: usize = 200;

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// The app dir [`install_panic_recorder`] was given; set once.
static APP_DIR: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    /// Set by [`RingLayer`] when it records an event while this thread is
    /// panicking, so the recorder's hook knows an earlier hook already put
    /// the panic in the ring.
    static PANIC_RECORDED: Cell<bool> = const { Cell::new(false) };
}

/// Append a line to the ring, dropping the oldest past [`CAPACITY`].
pub fn push(line: String) {
    if let Ok(mut ring) = RING.lock() {
        while ring.len() >= CAPACITY {
            ring.pop_front();
        }
        ring.push_back(line);
    }
}

/// The ring's lines, oldest first.
#[must_use]
pub fn snapshot() -> Vec<String> {
    RING.lock()
        .map(|ring| ring.iter().cloned().collect())
        .unwrap_or_default()
}

/// The last `n` lines of the ring, oldest first.
fn tail(n: usize) -> Vec<String> {
    let lines = snapshot();
    let skip = lines.len().saturating_sub(n);
    lines.into_iter().skip(skip).collect()
}

/// The app dir given to [`install_panic_recorder`], if it ran.
#[must_use]
pub fn app_dir() -> Option<&'static Path> {
    APP_DIR.get().map(PathBuf::as_path)
}

/// `<app_dir>/crash.log`, once [`install_panic_recorder`] ran.
#[must_use]
pub fn crash_log() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join(CRASH_LOG))
}

/// `<app_dir>/flight-log.txt`, once [`install_panic_recorder`] ran.
#[must_use]
pub fn flight_log() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join(FLIGHT_LOG))
}

/// Route panics into the ring, `<app_dir>/crash.log` and
/// `<app_dir>/flight-log.txt`, then run the hook that was installed before.
///
/// Install it once per process, after the subscriber (so a [`RingLayer`]
/// is listening) and before or after `architect::host::install_panic_logger`.
/// The first call wins: a second call installs nothing and returns `false`.
/// `app_dir` is created on the first crash, not here.
pub fn install_panic_recorder(app_dir: impl Into<PathBuf>) -> bool {
    if APP_DIR.set(app_dir.into()).is_err() {
        return false;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        let what = info.to_string();
        // A hook that ran before this one (either side of the chain) may
        // already have logged this panic into a `RingLayer`.
        let flagged = PANIC_RECORDED.with(Cell::take);
        let already_in_ring = flagged && tail(PANIC_LOOKBACK).iter().any(|l| l.contains(&what));
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("<unnamed>");
        if !already_in_ring {
            push(format!("PANIC on {thread}: {what}"));
        }
        record_crash(thread, &what);
        persist();
    }));
    true
}

/// Append one crash to `crash.log`: when, where, the panic, a backtrace and
/// the lines logged just before it.
fn record_crash(thread: &str, what: &str) {
    let Some(path) = crash_log() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > CRASH_LOG_KEEP) {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let when = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let backtrace = std::backtrace::Backtrace::force_capture();
    let before = tail(CRASH_TAIL).join("\n");
    let _ = writeln!(
        file,
        "=== crash at unix {when} on thread {thread} ===\n{what}\n\n{backtrace}\n--- logged before it ---\n{before}\n",
    );
}

/// Write the ring to `flight-log.txt`, so a run that ends without warning
/// still leaves its last lines. Does nothing before
/// [`install_panic_recorder`].
pub fn persist() {
    let Some(path) = flight_log() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, snapshot().join("\n"));
}

/// At launch: put the previous run's last lines back into the ring.
///
/// They go in under a marker, then the flight log moves aside to
/// `flight-log-previous.txt` so the next crash writes fresh. Returns how
/// many lines came back (0 when there was no flight log, or before
/// [`install_panic_recorder`]); ignoring it is fine.
#[allow(clippy::must_use_candidate)]
pub fn restore_previous() -> usize {
    let Some(path) = flight_log() else { return 0 };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(RESTORE_TAIL);
    let restored = lines.len().saturating_sub(skip);
    push("──── the previous run's last lines ────".to_owned());
    for line in lines.into_iter().skip(skip) {
        push(format!("prev| {line}"));
    }
    push("──── this run ────".to_owned());
    let _ = std::fs::rename(&path, path.with_file_name(FLIGHT_LOG_PREVIOUS));
    restored
}

/// The capture layer: every event becomes one ring line,
/// `[seconds since the layer was made] LEVEL target: message fields`.
pub struct RingLayer {
    start: Instant,
}

impl RingLayer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for RingLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Collects an event's message and fields into one line.
struct LineVisitor<'a>(&'a mut String);

impl tracing::field::Visit for LineVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut message = String::new();
        event.record(&mut LineVisitor(&mut message));
        let meta = event.metadata();
        push(format!(
            "[{:9.3}] {:5} {}: {}",
            self.start.elapsed().as_secs_f64(),
            meta.level().as_str(),
            meta.target(),
            message.trim_end(),
        ));
        // Set while panicking (the hooks run then); any later ordinary
        // event clears a flag left by logging during an unwind.
        let panicking = std::thread::panicking();
        PANIC_RECORDED.with(|seen| seen.set(panicking));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_the_newest_capacity_lines() {
        let total = CAPACITY.saturating_add(25);
        for i in 0..total {
            push(format!("ring-test {i}"));
        }
        let lines = snapshot();
        assert_eq!(lines.len(), CAPACITY);
        assert_eq!(lines.first().map(String::as_str), Some("ring-test 25"));
        assert_eq!(
            lines.last().cloned(),
            Some(format!("ring-test {}", total.saturating_sub(1)))
        );
        assert_eq!(tail(3).len(), 3);
    }
}
