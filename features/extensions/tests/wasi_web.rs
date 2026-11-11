//! WASI component extensions in the browser's shape
//! (`features/extensions/web`), run under node: the lab's counter and the
//! component that misbehaves on purpose, built for `wasm32-wasip2` as
//! `tests/wasi.rs` builds them, transpiled by jco, each instance in a
//! worker thread standing in for a Web Worker. The checks are
//! `features/extensions/web/test/web.test.mjs`: the view and its clicks, a
//! view mounted as an inline embed,
//! actions and the host service, a refused call, both limits (a terminated
//! worker, a capped memory), workers, cancellation and both queues'
//! back-pressure.
//!
//! It needs node and jco (`npm ci` in `features/extensions/web`, run here
//! when it is missing): node from the PATH, else from `devenv --profile
//! wasi shell`, which has it (as the components are built there when this
//! toolchain lacks `wasm32-wasip2`). Without them, or without the
//! components, it says so and passes; `ARCHITECT_WASI_WEB_REQUIRED=1`
//! (main CI) makes that a failure.
//!
//! The node run is bounded ([`NODE_LIMIT`]): past it the run is killed and
//! the test fails with what it printed, instead of holding main CI's
//! Tests job to its timeout (#520).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long the node tests may take: about 5 s when they pass.
const NODE_LIMIT: Duration = Duration::from_secs(120);

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

/// `program` (node, npm) from the PATH, else from the `wasi` dev profile;
/// `None` when neither has it.
fn tool(program: &str) -> Option<Command> {
    if has(program) {
        return Some(Command::new(program));
    }
    let mut cmd = Command::new("devenv");
    cmd.args(["--profile", "wasi", "shell", "--", program]);
    let found = Command::new("devenv")
        .args(["--profile", "wasi", "shell", "--", program, "--version"])
        .current_dir(root())
        .output()
        .is_ok_and(|o| o.status.success());
    found.then_some(cmd)
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
        "architect-sandbox-wasi-counter",
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
    let Some(mut node) = tool("node") else {
        return skip("no node on the PATH or in `devenv --profile wasi shell`");
    };
    let web = root().join("features/extensions/web");
    if !web.join("node_modules/.bin/jco").exists() {
        let Some(mut npm) = tool("npm") else {
            return skip("no npm to install jco with");
        };
        let installed = npm
            .args(["ci", "--silent", "--no-audit", "--no-fund"])
            .current_dir(&web)
            .status();
        if !installed.is_ok_and(|s| s.success()) {
            return skip("jco is not installed (npm ci in features/extensions/web)");
        }
    }
    let Some(guests) = guests() else { return };
    node.args(["--test", "--test-reporter=tap", "test/web.test.mjs"])
        .env("ARCHITECT_WASI_GUESTS", &guests)
        .current_dir(&web);
    let out = bounded(node, NODE_LIMIT);
    let tap = String::from_utf8_lossy(&out.stdout);
    let failed: Vec<&str> = tap.lines().filter(|l| l.starts_with("not ok")).collect();
    assert!(
        out.status.success() && failed.is_empty(),
        "{failed:#?}\n{tap}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let passed = tap.lines().filter(|l| l.starts_with("ok ")).count();
    assert!(passed >= 14, "{passed} of the node tests ran:\n{tap}");
    assert!(
        !tap.contains("# SKIP"),
        "the node tests skipped: the components were not where they looked\n{tap}"
    );
}

/// Runs `cmd` to its end, or kills it (and what it started: `devenv`
/// runs node as its child) after `limit` and fails with its output.
fn bounded(mut cmd: Command, limit: Duration) -> Output {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut r: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            buf
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if start.elapsed() >= limit {
            #[cfg(unix)]
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    let Some(status) = status else {
        panic!(
            "the node tests did not end in {limit:?}; killed:\n{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
    };
    Output {
        status,
        stdout,
        stderr,
    }
}
