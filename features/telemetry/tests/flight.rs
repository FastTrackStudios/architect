//! The flight recorder end to end. Each scenario runs in a child process
//! (this test binary re-run on the `child` test with `FLIGHT_CHILD` set),
//! which installs the recorder, logs, and panics; the parent then reads
//! `crash.log` and `flight-log.txt` back. A last child restores the
//! previous run's flight log into the ring.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::path::Path;
use std::process::{Command, Output};

use architect_telemetry::flight::{self, CRASH_LOG, FLIGHT_LOG, FLIGHT_LOG_PREVIOUS, RingLayer};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

const MODE: &str = "FLIGHT_CHILD";
const DIR: &str = "FLIGHT_DIR";
const BOOM: &str = "flight-test-boom";
const BEFORE: &str = "before the fall";

/// The child side. Without `FLIGHT_CHILD` it does nothing, so the normal
/// test run passes over it.
#[test]
fn child() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let dir = std::env::var(DIR).expect("FLIGHT_DIR");
    tracing_subscriber::registry().with(RingLayer::new()).init();
    match mode.as_str() {
        "alone" => {
            assert!(flight::install_panic_recorder(&dir));
            assert!(
                !flight::install_panic_recorder(&dir),
                "second install is a no-op"
            );
        }
        "logger-first" => {
            architect::host::install_panic_logger();
            assert!(flight::install_panic_recorder(&dir));
        }
        "recorder-first" => {
            assert!(flight::install_panic_recorder(&dir));
            architect::host::install_panic_logger();
        }
        "restore" => {
            assert!(flight::install_panic_recorder(&dir));
            let restored = flight::restore_previous();
            assert!(restored > 0, "the previous flight log came back");
            let ring = flight::snapshot();
            assert!(
                ring.iter()
                    .any(|l| l.contains("the previous run's last lines"))
            );
            assert!(
                ring.iter()
                    .any(|l| l.starts_with("prev| ") && l.contains(BEFORE))
            );
            assert!(
                ring.iter()
                    .any(|l| l.starts_with("prev| ") && l.contains(BOOM))
            );
            assert!(!Path::new(&dir).join(FLIGHT_LOG).exists());
            assert!(Path::new(&dir).join(FLIGHT_LOG_PREVIOUS).exists());
            println!("restored {restored}");
            return;
        }
        other => panic!("unknown mode {other}"),
    }
    tracing::info!(step = 1, "{BEFORE}");
    panic!("{BOOM}");
}

fn run_child(mode: &str, dir: &Path) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["child", "--exact", "--nocapture", "--test-threads=1"])
        .env(MODE, mode)
        .env(DIR, dir)
        .output()
        .unwrap()
}

/// Panic in a child with `mode`; check both files and return the flight log.
fn crash_in(mode: &str, dir: &Path) -> String {
    let out = run_child(mode, dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{mode}: the child panicked");
    assert!(
        stderr.contains(BOOM),
        "{mode}: the panic reached the default hook: {stderr}"
    );

    let crash = std::fs::read_to_string(dir.join(CRASH_LOG)).unwrap();
    assert!(crash.contains("=== crash at unix "), "{mode}: {crash}");
    assert!(crash.contains(BOOM), "{mode}: {crash}");
    assert!(
        crash.contains("--- logged before it ---"),
        "{mode}: {crash}"
    );
    assert!(
        crash.contains(BEFORE),
        "{mode}: the lines before the panic: {crash}"
    );

    let flight = std::fs::read_to_string(dir.join(FLIGHT_LOG)).unwrap();
    assert!(flight.contains(BEFORE), "{mode}: {flight}");
    assert!(
        flight.contains("step=1"),
        "{mode}: fields are kept: {flight}"
    );
    assert_eq!(
        flight.matches(BOOM).count(),
        1,
        "{mode}: the panic is in the ring exactly once: {flight}"
    );
    flight
}

#[test]
fn recorder_alone_writes_crash_and_flight_logs() {
    let dir = tempfile::tempdir().unwrap();
    let flight = crash_in("alone", dir.path());
    assert!(
        flight.contains("PANIC on "),
        "the recorder's own line: {flight}"
    );
}

#[test]
fn composes_with_install_panic_logger_in_either_order() {
    for mode in ["logger-first", "recorder-first"] {
        let dir = tempfile::tempdir().unwrap();
        let flight = crash_in(mode, dir.path());
        assert!(
            flight.contains("panic: "),
            "{mode}: the logger's line: {flight}"
        );
        assert!(
            !flight.contains("PANIC on "),
            "{mode}: no second line from the recorder: {flight}"
        );
    }
}

#[test]
fn restore_previous_brings_the_last_run_back() {
    let dir = tempfile::tempdir().unwrap();
    crash_in("alone", dir.path());
    let out = run_child("restore", dir.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("restored "), "the child ran: {stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");
}
