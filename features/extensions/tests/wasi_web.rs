//! WASI component extensions in the browser's shape
//! (`features/extensions/web`), run under node: the lab's counter and the
//! component that misbehaves on purpose, built for `wasm32-wasip2` as
//! `tests/wasi.rs` builds them, transpiled by jco, each instance in a
//! worker thread standing in for a Web Worker. The checks are
//! `features/extensions/web/test/web.test.mjs`: the view and its clicks,
//! actions and the host service, a refused call, both limits (a terminated
//! worker, a capped memory), workers, cancellation and both queues'
//! back-pressure.
//!
//! It needs node (in `devenv --profile wasi shell`) and jco
//! (`npm ci` in `features/extensions/web`, run here when it is missing).
//! Without them, or without the components, it says so and passes;
//! `ARCHITECT_WASI_WEB_REQUIRED=1` makes that a failure.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn skip(why: &str) {
    assert!(
        std::env::var("ARCHITECT_WASI_WEB_REQUIRED").is_err(),
        "{why}"
    );
    eprintln!("skipped: {why}");
}

fn has(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The two components, built as `tests/wasi.rs` builds them.
fn guests() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ARCHITECT_WASI_GUESTS") {
        return Some(PathBuf::from(dir));
    }
    let target = root().join("target/wasi-guest");
    let build = [
        "build",
        "-p",
        "architect-lab-wasi-counter",
        "-p",
        "architect-extension-wasi-hazards",
        "--target",
        "wasm32-wasip2",
        "--target-dir",
        target.to_str().unwrap(),
    ];
    let mut cmd = Command::new("devenv");
    cmd.args(["--profile", "wasi", "shell", "--", "cargo"])
        .args(build);
    for var in [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET",
        "RUSTC",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(var);
    }
    let exit = cmd.current_dir(root()).status();
    match exit {
        Ok(s) if s.success() => Some(target.join("wasm32-wasip2/debug")),
        other => {
            skip(&format!("the WASI components did not build ({other:?})"));
            None
        }
    }
}

#[test]
fn components_run_in_the_browsers_shape() {
    if !has("node") {
        return skip("no node on the PATH (devenv --profile wasi shell has it)");
    }
    let web = root().join("features/extensions/web");
    if !web.join("node_modules/.bin/jco").exists() {
        let installed = Command::new("npm")
            .args(["ci", "--silent", "--no-audit", "--no-fund"])
            .current_dir(&web)
            .status();
        if !installed.is_ok_and(|s| s.success()) {
            return skip("jco is not installed (npm ci in features/extensions/web)");
        }
    }
    let Some(guests) = guests() else { return };
    let out = Command::new("node")
        .args(["--test", "--test-reporter=tap", "test/web.test.mjs"])
        .env("ARCHITECT_WASI_GUESTS", &guests)
        .current_dir(&web)
        .output()
        .unwrap();
    let tap = String::from_utf8_lossy(&out.stdout);
    let failed: Vec<&str> = tap.lines().filter(|l| l.starts_with("not ok")).collect();
    assert!(
        out.status.success() && failed.is_empty(),
        "{failed:#?}\n{tap}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let passed = tap.lines().filter(|l| l.starts_with("ok ")).count();
    assert!(passed >= 13, "{passed} of the node tests ran:\n{tap}");
    assert!(
        !tap.contains("# SKIP"),
        "the node tests skipped: the components were not where they looked\n{tap}"
    );
}
