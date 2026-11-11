//! What each runner costs (#272): the Board's action (`LAB_BOARD_STATE`)
//! and its view's frame (`board`), run linked in and as a process, and
//! the nearest equivalents on the other runners: a Luau action that
//! answers the same JSON, the Board's tab as its TSX page on `QuickJS`,
//! and the lab's WASI component (`apps/sandbox/wasi-counter`: its action and
//! its view's first frame; a `linked::Extension` does not build as a
//! component yet). p50/p99 of each, printed.
//!
//! Not in the default run (it times things):
//! `cargo test -p architect-sandbox-board --test bench -- --ignored --nocapture`.
//! `BENCH_RUNNERS=linked,process` runs only those (`luau`, `quickjs` and
//! `wasi` too by default). The WASI row builds the component for
//! `wasm32-wasip2` (this toolchain's target, else `devenv --profile wasi
//! shell`), or reads it from `ARCHITECT_WASI_GUESTS`; without either it
//! says so in its row.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::arithmetic_side_effects
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::process::{ProcessRunner, Processes, State};
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wasi::WasiRunner;
use architect_extensions::{Origin, Package};
use architect_pages::files::{Origin as PageOrigin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
use architect_sandbox_board::{Board, ID, PAGES};
use architect_settings::Registry;
use futures::executor::block_on;

/// p50 and p99 of `samples`, printed as one row.
fn report(runner: &str, what: &str, mut samples: Vec<Duration>) {
    samples.sort();
    // The sample at percentile `q` (nearest rank).
    let at = |q: usize| samples[(samples.len() - 1) * q / 100];
    println!(
        "| {runner:<8} | {what:<20} | {:>5} | {:>10.1} | {:>10.1} |",
        samples.len(),
        at(50).as_secs_f64() * 1e6,
        at(99).as_secs_f64() * 1e6
    );
}

fn time(n: usize, mut f: impl FnMut()) -> Vec<Duration> {
    for _ in 0..n.min(20) {
        f();
    }
    (0..n)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn wants(runner: &str) -> bool {
    std::env::var("BENCH_RUNNERS").map_or(true, |v| v.split(',').any(|r| r.trim() == runner))
}

fn action(list: &ActionList, id: &str) {
    let out = block_on(list.run_json_async(id, b"")).unwrap();
    assert!(!out.is_empty());
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn what_each_runner_costs() {
    println!("\n| runner   | what                 |     n |   p50 (µs) |   p99 (µs) |");
    println!("|---|---|---:|---:|---:|");
    let tmp = tempfile::tempdir().unwrap();
    if wants("linked") {
        linked(tmp.path());
    }
    if wants("process") {
        process(tmp.path());
    }
    if wants("luau") {
        luau(tmp.path());
    }
    if wants("quickjs") {
        quickjs(tmp.path());
    }
    if wants("wasi") {
        wasi(tmp.path());
    }
}

fn linked(tmp: &Path) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("linked")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    report(
        "linked",
        "action",
        time(5000, || action(&list, "LAB_BOARD_STATE")),
    );
    let root = ext.host().linked_view(ID, "board").unwrap();
    // The view's frame in the app's own document: a fresh VirtualDom of
    // its component, rendered once (the edits the app's document applies).
    report(
        "linked",
        "view frame",
        time(2000, || {
            let mut dom = dioxus::prelude::VirtualDom::new(root);
            let edits = dom.rebuild_to_vec();
            assert!(!edits.edits.is_empty());
        }),
    );
}

fn process(tmp: &Path) {
    let dev = tmp.join("board");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extension.toml"),
        dev.join("extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-sandbox-board"),
        dev.join("architect-sandbox-board"),
    )
    .unwrap();
    let list = ActionList::new();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp);
    let processes = runner.processes();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    let ext = Extensions::new(host, Installs::new(tmp.join("process")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.dev(&dev);
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Dev);
    let start = Instant::now();
    let running = loop {
        if let Some(p) = processes.get(ID) {
            if matches!(p.state(), State::Running { .. }) {
                break p;
            }
        }
        assert!(start.elapsed() < Duration::from_secs(20), "its process");
        std::thread::sleep(Duration::from_millis(10));
    };
    report(
        "process",
        "action",
        time(2000, || action(&list, "LAB_BOARD_STATE")),
    );
    // A view's frame from its process: opened, its first frame sent over
    // the socket and applied to a document here, and closed (a closed
    // view's place is given back: `wire::VIEWS_WIRE`). The first five
    // warm it up, untimed.
    let mut samples = Vec::new();
    for i in 0..250 {
        let t = Instant::now();
        let mut session = running.open("board").unwrap();
        let frame = loop {
            match session.frames.try_recv() {
                Ok(f) => break f,
                Err(futures::channel::mpsc::TryRecvError::Closed) => {
                    panic!("its view ended at {i}: {:?}", running.state())
                }
                Err(_) => {}
            }
            assert!(t.elapsed() < Duration::from_secs(10), "its first frame");
            std::thread::yield_now();
        };
        let (events, _) = futures::channel::mpsc::unbounded();
        let mut doc = RemoteDocument::new(events);
        doc.apply(frame).unwrap();
        if i >= 5 {
            samples.push(t.elapsed());
        }
    }
    report("process", "view frame", samples);
    let _ = ext.end_dev(&dev);
}

const LUAU: &str = r#"
id = "bench.luau"
name = "Bench (Luau)"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "BENCH_LUAU_STATE"
name = "The board, in Luau"
script = "state.luau"
"#;

fn luau(tmp: &Path) {
    let dir = tmp.join("luau");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), LUAU).unwrap();
    std::fs::write(
        dir.join("state.luau"),
        "return { pins = {}, chart = nil, issues = {}, last_invoked = nil, process = 0 }",
    )
    .unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("luau-installs")));
    let _ = ext.dev(&dir);
    assert!(
        ext.host().get("bench.luau").is_some(),
        "{:?}",
        ext.problems()
    );
    report(
        "luau",
        "action",
        time(5000, || action(&list, "BENCH_LUAU_STATE")),
    );
    println!(
        "| luau     | view frame           |     – | (no Luau equivalent of the Board's view) | |"
    );
}

fn quickjs(tmp: &Path) {
    let app = tmp.join("pages");
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.join("js-installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let manifest = ext.host().get(ID).unwrap().manifest;
    let dir = write_builtin(&app, ID, PAGES).unwrap();
    let page = PageFile::new(dir.join("board.tsx"), PageOrigin::Builtin).unwrap();
    let pages = Pages::new(AppHost::new(list)).with_app_dir(&app).with_js();
    pages.declare(ID, "board", page.clone());
    println!(
        "| quickjs  | action               |     – | (a page calls the Board's action: the linked row) | |"
    );
    // The Board's tab as its page: opened, run on QuickJS, until its
    // first frame with the board's title is applied here.
    report(
        "quickjs",
        "view frame",
        time(30, || {
            let mut session = pages
                .open(&page, ID, "board", Some(gate(&manifest)))
                .unwrap();
            let (events, _) = futures::channel::mpsc::unbounded();
            let mut doc = RemoteDocument::new(events);
            let start = Instant::now();
            loop {
                while let Ok(frame) = session.frames.try_recv() {
                    doc.apply(frame).unwrap();
                }
                let d = doc.document();
                if d.query_selector("#board-title").ok().flatten().is_some() {
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(30), "its first frame");
                std::thread::sleep(Duration::from_micros(200));
            }
        }),
    );
}

/// The workspace's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Whether this toolchain has the `wasm32-wasip2` target.
fn has_wasip2() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| {
            Path::new(s.trim())
                .join("lib/rustlib/wasm32-wasip2")
                .is_dir()
        })
}

/// The folder the lab's WASI counter was built into (as
/// `features/extensions/tests/wasi.rs` builds it), or why not.
fn wasi_counter() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("ARCHITECT_WASI_GUESTS") {
        return Ok(PathBuf::from(dir));
    }
    let target = root().join("target/wasi-guest");
    let build = [
        "build",
        "--release",
        "-p",
        "architect-sandbox-wasi-counter",
        "--target",
        "wasm32-wasip2",
        "--target-dir",
        target.to_str().unwrap(),
    ];
    let mut cmd = if has_wasip2() {
        let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
        c.args(build);
        c
    } else {
        let mut c = Command::new("devenv");
        c.args(["--profile", "wasi", "shell", "--", "cargo"])
            .args(build);
        c
    };
    // The outer build's settings are not the component's.
    for var in [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET",
        "RUSTC",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(var);
    }
    let exit = {
        let _devenv = devenv_lock();
        cmd.current_dir(root()).status()
    };
    match exit {
        Ok(s) if s.success() => Ok(target.join("wasm32-wasip2/release")),
        other => Err(format!("the WASI counter did not build ({other:?})")),
    }
}

/// The lab's WASI counter on the `wasi` runner: its action
/// (`LAB_WASI_COUNTER_ADD`, which answers the count as JSON) and its
/// view's first frame, measured as the process row's is.
fn wasi(tmp: &Path) {
    const ID: &str = "lab.wasi-counter";
    const WASM: &str = "architect_sandbox_wasi_counter.wasm";
    let built = match wasi_counter() {
        Ok(dir) => dir,
        Err(why) => {
            println!("| wasi     | –                    |     – | ({why}) | |");
            return;
        }
    };
    let dir = tmp.join("wasi-counter");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        root().join("apps/sandbox/wasi-counter/extension.toml"),
        dir.join("extension.toml"),
    )
    .unwrap();
    std::fs::copy(built.join(WASM), dir.join(WASM)).unwrap();
    let list = ActionList::new();
    let processes = Processes::default();
    let runner = WasiRunner::new()
        .with_cache(tmp.join("wasi-cache"))
        .with_processes(processes.clone());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    host.load(&Package::read(&dir).unwrap(), Origin::Dev)
        .unwrap();
    report(
        "wasi",
        "action",
        time(2000, || action(&list, "LAB_WASI_COUNTER_ADD")),
    );
    let running = processes.get(ID).expect("its local process");
    // Its view's frame: opened, its first frame applied to a document
    // here, and closed. The first five warm it up, untimed.
    let mut samples = Vec::new();
    for i in 0..250 {
        let t = Instant::now();
        let mut session = running.open("counter").unwrap();
        let frame = loop {
            if let Ok(f) = session.frames.try_recv() {
                break f;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "its first frame");
            std::thread::yield_now();
        };
        let (events, _) = futures::channel::mpsc::unbounded();
        let mut doc = RemoteDocument::new(events);
        doc.apply(frame).unwrap();
        if i >= 5 {
            samples.push(t.elapsed());
        }
    }
    report("wasi", "view frame", samples);
}

/// Held while `devenv` runs. Under nextest every test is a process of its
/// own, and each `devenv` run rewrites its profile's bootstrap files: two
/// at once and one reads a half-written `resolve-lock.nix` ("unexpected
/// end of file"), so the components "did not build". One lock file for
/// every test binary that enters the `wasi` profile.
#[allow(
    clippy::incompatible_msrv,
    reason = "a test: it builds with the workspace toolchain, not the MSRV"
)]
fn devenv_lock() -> Option<std::fs::File> {
    let dir = root().join("target");
    std::fs::create_dir_all(&dir).ok()?;
    let file = std::fs::File::create(dir.join(".devenv-wasi.lock")).ok()?;
    file.lock().ok()?;
    Some(file)
}
