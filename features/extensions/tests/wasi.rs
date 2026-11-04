//! WASI component extensions (feature `wasi`), on real components built
//! for `wasm32-wasip2` here: the lab's counter (`apps/lab/wasi-counter`)
//! and a component that misbehaves on purpose
//! (`tests/wasi-hazards`).
//!
//! - the counter loads, its view renders as the wire's edits into a
//!   `RemoteDocument`, a click and its actions change it, and its action
//!   runs an app action through the host service;
//! - a call outside its capabilities is refused (and in its log);
//! - a call that never ends is interrupted at its time limit, and one
//!   that eats memory stops at its memory limit — the host runs on and the
//!   component is started again;
//! - unloading it ends its thread and frees its store;
//! - a component is compiled once: the next start reads the cache.
//!
//! The components are built with `cargo build --target wasm32-wasip2`:
//! by this toolchain when it has the target, else in `devenv --profile
//! wasi shell` (devenv.nix); `ARCHITECT_WASI_GUESTS=<dir>` names a folder
//! they were built into instead. With none of those the tests say so and
//! pass; `ARCHITECT_WASI_REQUIRED=1` (CI) makes that a failure.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::Processes;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wasi::{Limits, WasiRunner};
use architect_extensions::wire::{Edit, EventData, Frame, Pointer, ViewEvent};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const COUNTER: &str = "lab.wasi-counter";
const HAZARDS: &str = "test.hazards";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn has_wasip2(cargo_dir: &Path) -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .args(["--print", "sysroot"])
        .current_dir(cargo_dir)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| {
            Path::new(s.trim())
                .join("lib/rustlib/wasm32-wasip2")
                .is_dir()
        })
}

/// Where the two components were built, once for every test.
fn guests() -> Option<&'static Path> {
    static BUILT: OnceLock<Option<PathBuf>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
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
            let mut cmd = if has_wasip2(&root()) {
                let mut c =
                    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
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
            let exit = cmd.current_dir(root()).status();
            match exit {
                Ok(s) if s.success() => Some(target.join("wasm32-wasip2/debug")),
                other => {
                    let why = format!("the WASI components did not build ({other:?})");
                    assert!(std::env::var("ARCHITECT_WASI_REQUIRED").is_err(), "{why}");
                    eprintln!("skipped: {why}");
                    None
                }
            }
        })
        .as_deref()
}

static THEME_LIST: ActionMeta = ActionMeta {
    id: "THEME_LIST",
    trait_name: "",
    method_name: "THEME_LIST",
    display_name: "List the themes",
    description: "The themes.",
    category: "Theme",
    group: "Theme",
    shortcut: "",
    toggleable: false,
    undo: false,
};

struct App {
    list: ActionList,
    host: Host,
    processes: Processes,
    components: architect_extensions::wasi::Components,
}

/// An app with `THEME_LIST` (three themes) and a WASI runner.
fn app(limits: Limits, cache: &Path) -> App {
    let list = ActionList::new();
    list.register_with(&THEME_LIST, Effect::ReadOnly, |_: &ActionCaller, (): ()| {
        Ok::<_, String>(vec![
            "light".to_owned(),
            "dark".to_owned(),
            "neon".to_owned(),
        ])
    })
    .unwrap();
    let processes = Processes::default();
    let runner = WasiRunner::new()
        .with_limits(limits)
        .with_cache(cache)
        .with_processes(processes.clone());
    let components = runner.components();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_runner(runner);
    App {
        list,
        host,
        processes,
        components,
    }
}

/// A package folder: `manifest` and `component` from the guests' folder.
fn package(tmp: &Path, name: &str, manifest: &str, component: &str) -> PathBuf {
    let dir = tmp.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    std::fs::copy(guests().unwrap().join(component), dir.join(component)).unwrap();
    dir
}

fn counter_manifest() -> String {
    std::fs::read_to_string(root().join("apps/lab/wasi-counter/extension.toml")).unwrap()
}

const COUNTER_WASM: &str = "architect_lab_wasi_counter.wasm";
const HAZARDS_WASM: &str = "architect_extension_wasi_hazards.wasm";

const HAZARDS_MANIFEST: &str = r#"
id = "test.hazards"
name = "Hazards"
version = "0.1.0"
kind = "wasm"
[wasm]
component = "architect_extension_wasi_hazards.wasm"
[[actions]]
id = "TEST_HAZARDS_SPIN"
name = "Spin"
[[actions]]
id = "TEST_HAZARDS_HOARD"
name = "Hoard"
[[actions]]
id = "TEST_HAZARDS_ECHO"
name = "Echo"
params = '{"type":"object","properties":{"say":{"type":"integer"}}}'
"#;

fn load(app: &App, dir: &Path) {
    let package = Package::read(dir).unwrap();
    app.host.load(&package, Origin::Dev).unwrap();
}

fn run(app: &App, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(
        app.list
            .run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()),
    )
    .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

fn failed(e: ActionError) -> String {
    match e {
        ActionError::Failed { message, .. } => message,
        other => panic!("not a failure: {other:?}"),
    }
}

fn logs(app: &App, id: &str) -> Vec<String> {
    app.host
        .logs()
        .since(id, None, 100)
        .into_iter()
        .map(|l| format!("{} {}", l.source, l.text))
        .collect()
}

/// The next frame, within a few seconds.
fn next(frames: &mut futures::channel::mpsc::UnboundedReceiver<Frame>) -> Frame {
    let started = Instant::now();
    loop {
        if let Ok(frame) = frames.try_recv() {
            return frame;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no frame came");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn text(doc: &RemoteDocument) -> String {
    let d = doc.document();
    let main = d.query_selector("main").unwrap().unwrap();
    d.get_node(main).unwrap().text_content()
}

/// The ids of the elements that listen for clicks, in order.
fn clickable(frame: &Frame) -> Vec<u64> {
    let Frame::Edits { edits } = frame else {
        panic!("{frame:?}");
    };
    let mut out = Vec::new();
    let mut last = None;
    for e in edits {
        match e {
            Edit::SetId { id } | Edit::PushId { id } => last = Some(*id),
            Edit::Listen { name } if name == "click" => out.extend(last),
            _ => {}
        }
    }
    out
}

fn click(element: u64) -> ViewEvent {
    ViewEvent {
        element,
        name: "click".to_owned(),
        bubbles: true,
        data: EventData::Mouse(Pointer::default()),
    }
}

#[test]
fn the_counter_loads_its_view_renders_and_its_actions_run() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    assert_eq!(
        run(&app, "LAB_WASI_COUNTER_ADD", &json!({ "by": 2 })).unwrap(),
        2
    );

    // Its view, as the edits a process's view sends, into the host's
    // document.
    let process = app.processes.get(COUNTER).expect("a local process");
    assert!(process.is_local());
    let mut session = process.open("counter").unwrap();
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut doc = RemoteDocument::new(events);
    let first = next(&mut session.frames);
    let buttons = clickable(&first);
    doc.apply(first).unwrap();
    assert!(text(&doc).contains("A WASI component"), "{}", text(&doc));
    assert!(text(&doc).contains('2'), "{}", text(&doc));
    assert_eq!(buttons.len(), 2, "add and ask");

    // A click goes to its handler; the view changes.
    session.events.unbounded_send(click(buttons[0])).unwrap();
    doc.apply(next(&mut session.frames)).unwrap();
    assert!(text(&doc).contains('3'), "{}", text(&doc));

    // Its button asks the app through the host service, from a handler.
    session.events.unbounded_send(click(buttons[1])).unwrap();
    doc.apply(next(&mut session.frames)).unwrap();
    assert!(
        text(&doc).contains("The app has 3 themes"),
        "{}",
        text(&doc)
    );

    // Its action asks the same and answers what the app said; an action
    // that changes what the view shows sends the view's edits.
    let themes = run(&app, "LAB_WASI_COUNTER_ASK", &Value::Null).unwrap();
    assert_eq!(themes, json!(["light", "dark", "neon"]));
    run(&app, "LAB_WASI_COUNTER_ADD", &json!({ "by": 10 })).unwrap();
    doc.apply(next(&mut session.frames)).unwrap();
    assert!(text(&doc).contains("13"), "{}", text(&doc));
    assert!(
        logs(&app, COUNTER)
            .iter()
            .any(|l| l.contains("The app has 3 themes")),
        "{:?}",
        logs(&app, COUNTER)
    );

    // A reload keeps its count (save-state, then init with it).
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    assert_eq!(
        run(&app, "LAB_WASI_COUNTER_ADD", &json!({ "by": 1 })).unwrap(),
        14
    );
}

#[test]
fn a_call_outside_its_capabilities_is_refused() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    // The same component, its manifest granting it no app action.
    let manifest = counter_manifest().replace(r#"actions = ["THEME_LIST"]"#, "actions = []");
    load(
        &app,
        &package(tmp.path(), "counter", &manifest, COUNTER_WASM),
    );
    let why = failed(run(&app, "LAB_WASI_COUNTER_ASK", &Value::Null).unwrap_err());
    assert!(why.contains("refused"), "{why}");
    assert!(
        logs(&app, COUNTER)
            .iter()
            .any(|l| l.starts_with("host run: refused (actions)")),
        "{:?}",
        logs(&app, COUNTER)
    );
}

#[test]
fn a_call_that_never_ends_is_interrupted_and_the_host_runs_on() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let limits = Limits {
        call: Duration::from_millis(300),
        ..Limits::default()
    };
    let app = app(limits, &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "hazards", HAZARDS_MANIFEST, HAZARDS_WASM),
    );
    let started = Instant::now();
    let why = failed(run(&app, "TEST_HAZARDS_SPIN", &Value::Null).unwrap_err());
    let took = started.elapsed();
    assert!(why.starts_with("interrupted"), "{why}");
    assert!(took < Duration::from_secs(3), "it took {took:?}");
    // Started again, fresh: it answers.
    assert_eq!(
        run(&app, "TEST_HAZARDS_ECHO", &json!({ "say": 1 })).unwrap(),
        json!({ "echo": { "say": 1 } })
    );
    let stats = app.components.get(HAZARDS).unwrap();
    assert_eq!(stats.traps(), 1);
    assert!(stats.last_trap().unwrap().starts_with("interrupted"));
    // Its stdout is in its log.
    assert!(
        logs(&app, HAZARDS)
            .iter()
            .any(|l| l == r#"stdout echo {"say":1}"#),
        "{:?}",
        logs(&app, HAZARDS)
    );
}

#[test]
fn a_memory_bomb_hits_its_limit() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let limits = Limits {
        memory: 32 << 20,
        ..Limits::default()
    };
    let app = app(limits, &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "hazards", HAZARDS_MANIFEST, HAZARDS_WASM),
    );
    let why = failed(run(&app, "TEST_HAZARDS_HOARD", &Value::Null).unwrap_err());
    assert!(why.starts_with("out of memory"), "{why}");
    assert!(why.contains("32 MiB"), "{why}");
    assert_eq!(
        run(&app, "TEST_HAZARDS_ECHO", &json!({ "say": 2 })).unwrap(),
        json!({ "echo": { "say": 2 } })
    );
    let stats = app.components.get(HAZARDS).unwrap();
    assert!(stats.memory() < 32 << 20, "{}", stats.memory());
}

#[test]
fn unloading_it_frees_it() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    let stats = app.components.get(COUNTER).unwrap();
    assert!(stats.alive());
    assert!(app.host.unload(COUNTER));
    let deadline = Instant::now() + Duration::from_secs(5);
    while stats.alive() {
        assert!(Instant::now() < deadline, "its store is still held");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.components.get(COUNTER).is_none());
    assert!(app.processes.get(COUNTER).is_none());
    assert!(run(&app, "LAB_WASI_COUNTER_ADD", &Value::Null).is_err());
}

#[test]
fn a_component_is_compiled_once() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let dir = package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM);
    let first = app(Limits::default(), &cache);
    load(&first, &dir);
    assert!(
        logs(&first, COUNTER)
            .iter()
            .any(|l| l.starts_with("wasi compiled")),
        "{:?}",
        logs(&first, COUNTER)
    );
    let second = app(Limits::default(), &cache);
    load(&second, &dir);
    assert!(
        logs(&second, COUNTER)
            .iter()
            .any(|l| l.starts_with("wasi loaded from the compile cache")),
        "{:?}",
        logs(&second, COUNTER)
    );
    let cached: Vec<_> = std::fs::read_dir(&cache).unwrap().collect();
    assert_eq!(cached.len(), 1, "one compiled component");
}
