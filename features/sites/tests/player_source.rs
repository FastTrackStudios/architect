//! The page's player with an outside clock attached (#528), under node:
//! `features/sites/test/player-source.test.mjs` runs
//! `assets/player.js` in a small fake DOM, a fake engine as the
//! playhead's source, and checks that the followers (a chart's bars)
//! follow it each frame, that seeks, plays and pauses go to it, that the
//! page's media and bar yield while it is attached, and that the page's
//! player takes over when it detaches.
//!
//! Node from the PATH, else from `devenv --profile wasi shell`, which has
//! it. Without node it says so and passes; `ARCHITECT_WASI_WEB_REQUIRED=1`
//! (main CI's Tests job, where node is there for the WASI browser tests)
//! makes that a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the node run may take: well under a second when it passes.
const NODE_LIMIT: Duration = Duration::from_secs(60);

fn here() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_owned()
}

fn has(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Node from the PATH, else from the `wasi` dev profile.
fn node() -> Option<Command> {
    if has("node") {
        return Some(Command::new("node"));
    }
    let found = Command::new("devenv")
        .args(["--profile", "wasi", "shell", "--", "node", "--version"])
        .current_dir(here().join("../.."))
        .output()
        .is_ok_and(|o| o.status.success());
    found.then(|| {
        let mut cmd = Command::new("devenv");
        cmd.args(["--profile", "wasi", "shell", "--", "node"])
            .current_dir(here().join("../.."));
        cmd
    })
}

#[test]
fn an_attached_source_drives_the_playhead_and_the_page_player_yields() {
    let Some(mut node) = node() else {
        assert!(
            std::env::var("ARCHITECT_WASI_WEB_REQUIRED").is_err(),
            "no node on the PATH or in `devenv --profile wasi shell`"
        );
        eprintln!("skipped: no node");
        return;
    };
    let test = here().join("test/player-source.test.mjs");
    let mut child = node
        .args(["--test", "--test-reporter=tap"])
        .arg(&test)
        .env("ARCHITECT_PLAYER_JS", here().join("assets/player.js"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > NODE_LIMIT {
            let _ = child.kill();
            panic!("the node tests did not end in {NODE_LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().unwrap();
    let tap = String::from_utf8_lossy(&out.stdout);
    let failed: Vec<&str> = tap.lines().filter(|l| l.starts_with("not ok")).collect();
    assert!(
        out.status.success() && failed.is_empty(),
        "{failed:#?}\n{tap}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let passed = tap.lines().filter(|l| l.starts_with("ok ")).count();
    assert!(passed >= 8, "{passed} of the node tests ran:\n{tap}");
}
