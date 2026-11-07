//! The drive on the real binary (Linux): `architect-desktop serve` with a
//! `drive` mounts it, a `kill -9` leaves no mount (its guard unmounts),
//! a stale mount (guard killed too) is cleared by the next start, and a
//! SIGTERM unmounts cleanly. Skipped where FUSE cannot mount.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn mounted(at: &Path) -> bool {
    let at = at.to_string_lossy().replace(' ', "\\040");
    std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .any(|l| l.split(' ').nth(4) == Some(at.as_str()) && l.contains(" - fuse"))
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out: {what}");
}

fn serve(config: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_architect-desktop"))
        .args(["serve", "--config"])
        .arg(config)
        .env("RUST_LOG", "warn")
        .spawn()
        .unwrap()
}

fn kill(pid: u32, signal: &str) {
    Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .unwrap();
}

/// The children of `pid` (its guard).
fn children(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

#[test]
fn the_drive_never_outlives_its_agent() {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("no /dev/fuse: skipped");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let tree = tmp.path().join("tree/codywright/Projects/Song");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("Song.RPP"), "<REAPER_PROJECT\n>\n").unwrap();
    let mount = tmp.path().join("Architect");
    let config = tmp.path().join("agent.toml");
    std::fs::write(
        &config,
        format!(
            "name = \"drive-test\"\nbind = \"127.0.0.1:0\"\ndata_dir = \"{}\"\nfiles_dir = \"{}\"\nfiles_hold = true\niroh = \"local\"\ndrive = \"{}\"\n",
            tmp.path().join("data").display(),
            tmp.path().join("tree").display(),
            mount.display()
        ),
    )
    .unwrap();

    let mut agent = serve(&config);
    let seen = Instant::now();
    until("the drive mounts", || mounted(&mount));
    if !mounted(&mount) {
        let _ = agent.kill();
        let _ = agent.wait();
        eprintln!("FUSE did not mount here: skipped");
        return;
    }
    until("the project shows", || {
        mount.join("codywright/Projects/Song/Song.RPP").exists()
    });
    eprintln!("mounted and listed in {:?}", seen.elapsed());

    // A crash: the guard unmounts.
    kill(agent.id(), "-KILL");
    let _ = agent.wait();
    until("the guard unmounts after kill -9", || !mounted(&mount));

    // Guard and agent both killed: a stale mount, cleared by the next start.
    let mut agent = serve(&config);
    until("the drive mounts again", || mounted(&mount));
    until("its guard runs", || !children(agent.id()).is_empty());
    for guard in children(agent.id()) {
        kill(guard, "-KILL");
    }
    kill(agent.id(), "-KILL");
    let _ = agent.wait();
    let err = std::fs::read_dir(&mount)
        .err()
        .and_then(|e| e.raw_os_error());
    assert_eq!(
        err,
        Some(107),
        "a stale mount: Transport endpoint is not connected"
    );
    let mut agent = serve(&config);
    until("a new start clears the stale mount and mounts", || {
        std::fs::read_dir(&mount).is_ok() && mounted(&mount)
    });

    // A clean stop.
    kill(agent.id(), "-TERM");
    let _ = agent.wait();
    until("SIGTERM unmounts", || !mounted(&mount));
}
