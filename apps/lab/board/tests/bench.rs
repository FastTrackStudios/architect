//! What each runner costs (#272): the Board's action (`LAB_BOARD_STATE`)
//! and its view's frame (`board`), run linked in and as a process, and
//! the nearest equivalents on the scripted runners: a Luau action that
//! answers the same JSON, and the Board's tab as its TSX page on
//! `QuickJS`. p50/p99 of each, printed.
//!
//! Not in the default run (it times things):
//! `cargo test -p architect-lab-board --test bench -- --ignored --nocapture`.
//! `BENCH_RUNNERS=linked,process` runs only those (`luau`, `quickjs` too
//! by default).

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

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::process::{ProcessRunner, State};
use architect_extensions::remote::RemoteDocument;
use architect_lab_board::{Board, ID, PAGES};
use architect_pages::files::{Origin as PageOrigin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
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
        env!("CARGO_BIN_EXE_architect-lab-board"),
        dev.join("architect-lab-board"),
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
    let mut running = loop {
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
    // the socket and applied to a document here. A process serves 64
    // view opens per connection today (closed ones are not given back),
    // so it is started again, untimed, every 50.
    let mut samples = Vec::new();
    for i in 0..250 {
        if i % 50 == 0 && i > 0 {
            let was = running.pid();
            let _ = ext.end_dev(&dev);
            let _ = ext.dev(&dev);
            let t = Instant::now();
            loop {
                if let Some(p) = processes.get(ID) {
                    if matches!(p.state(), State::Running { .. }) && p.pid() != was {
                        running = p;
                        break;
                    }
                }
                assert!(t.elapsed() < Duration::from_secs(20), "its restart");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
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
        if i % 50 >= 5 {
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
