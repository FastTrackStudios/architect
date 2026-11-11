//! WASI component extensions (feature `wasi`), on real components built
//! for `wasm32-wasip2` here: the lab's counter (`apps/sandbox/wasi-counter`)
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
//! - a component is compiled once: the next start reads the cache;
//! - its workers: a job in one keeps its view within its frame budget,
//!   is cancelled when it unloads, traps alone past its limits, and its
//!   queues hold back-pressure; the pool's threads start lazily and stop
//!   when idle;
//! - its actions run through the app's own list (`forward_actions_to`).
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
use architect_extensions::remote::REMOTE_BUDGET;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wasi::{Limits, Pool, QUEUE, WasiRunner, WorkerStatus};
use architect_extensions::wire::{Edit, EventData, Frame, Pointer, ViewEvent};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use architect_workspace::ViewMeter;
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
                "architect-sandbox-wasi-counter",
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
            let exit = {
                let _devenv = devenv_lock();
                cmd.current_dir(root()).status()
            };
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
    app_with(WasiRunner::new().with_limits(limits), cache)
}

/// The same, with `runner` (its worker limits, its pool).
fn app_with(runner: WasiRunner, cache: &Path) -> App {
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
    let runner = runner.with_cache(cache).with_processes(processes.clone());
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
    std::fs::read_to_string(root().join("apps/sandbox/wasi-counter/extension.toml")).unwrap()
}

const COUNTER_WASM: &str = "architect_sandbox_wasi_counter.wasm";
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
[[actions]]
id = "TEST_HAZARDS_WORK"
name = "Start a worker"
params = '{"type":"object","properties":{"worker":{"type":"string"},"args":{}}}'
[[actions]]
id = "TEST_HAZARDS_WORKER"
name = "A worker's state"
params = '{"type":"object","properties":{"id":{"type":"integer"}}}'
[[actions]]
id = "TEST_HAZARDS_FLOOD"
name = "Post a worker many messages"
params = '{"type":"object","properties":{"id":{"type":"integer"},"n":{"type":"integer"}}}'
[[actions]]
id = "TEST_HAZARDS_STALL"
name = "Keep busy"
params = '{"type":"object","properties":{"ms":{"type":"integer"}}}'
[[actions]]
id = "TEST_HAZARDS_ASK"
name = "Use the app's interface"
[[actions]]
id = "TEST_HAZARDS_SUBSCRIBE"
name = "Make a hook"
params = '{"type":"object"}'
[[actions]]
id = "TEST_HAZARDS_HEARD"
name = "Hear an event"
params = '{"type":"object"}'
[[actions]]
id = "TEST_HAZARDS_NOW"
name = "The host's clock"
[[actions]]
id = "TEST_HAZARDS_FETCH"
name = "Fetch"
params = '{"type":"object"}'
[[actions]]
id = "TEST_HAZARDS_DOCS"
name = "Make, edit, list and read a document"
params = '{"type":"object"}'
[[views]]
id = "ticker"
title = "Ticker"
[[workers]]
id = "forever"
entry = "forever"
[[workers]]
id = "hoard"
entry = "hoard"
[[workers]]
id = "chatty"
entry = "chatty"
[[workers]]
id = "deaf"
entry = "deaf"
[[workers]]
id = "echo"
entry = "echo"
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

/// The next frame, looked for every 200 µs: what a frame's time is
/// measured with.
fn next_now(frames: &mut futures::channel::mpsc::UnboundedReceiver<Frame>) -> Frame {
    let started = Instant::now();
    loop {
        if let Ok(frame) = frames.try_recv() {
            return frame;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "no frame came");
        std::thread::sleep(Duration::from_micros(200));
    }
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
    assert_eq!(buttons.len(), 3, "add, ask, and count primes");

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

/// Worker `id`'s state, as the hazards component heard it.
fn worker_state(app: &App, id: &Value) -> Value {
    run(app, "TEST_HAZARDS_WORKER", &json!({ "id": id })).unwrap()
}

/// Waits (10 s at most) until `done` says so of what `poll` gives.
fn wait_for<T: std::fmt::Debug>(
    what: &str,
    mut poll: impl FnMut() -> T,
    done: impl Fn(&T) -> bool,
) -> T {
    let started = Instant::now();
    loop {
        let now = poll();
        if done(&now) {
            return now;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{what}: {now:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn hazards(runner: WasiRunner, tmp: &Path) -> App {
    let app = app_with(runner, &tmp.join("cache"));
    load(
        &app,
        &package(tmp, "hazards", HAZARDS_MANIFEST, HAZARDS_WASM),
    );
    app
}

/// The app's interface (#306), through the world's typed imports: a
/// notification, the clipboard there and back, a picker's, a prompt's
/// and a menu's answers, a badge; refused without `capabilities.ui`.
#[test]
fn a_component_notifies_copies_asks_and_gets_the_answers_back() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ui = architect_extensions::hostui::FakeUi::new()
        .answer(json!("/tmp/picked.csv"))
        .answer(json!("Ada"));
    let app = app(Limits::default(), &tmp.path().join("cache"));
    app.host.set_ui(std::sync::Arc::new(ui.clone()));
    let manifest = HAZARDS_MANIFEST.replace(
        "[wasm]",
        "[capabilities]\nui = [\"notify\", \"clipboard\", \"pickers\", \"prompts\", \"menus\", \"badges\"]\n[wasm]",
    );
    load(
        &app,
        &package(tmp.path(), "hazards", &manifest, HAZARDS_WASM),
    );
    let out = run(&app, "TEST_HAZARDS_ASK", &Value::Null).unwrap();
    assert_eq!(
        out,
        json!({ "clip": "copied by WASI", "path": "/tmp/picked.csv", "name": "Ada", "chosen": "go" })
    );
    assert_eq!(ui.notices()[0].text, "hello from WASI");
    assert_eq!(ui.notices()[0].kind, "success");
    assert_eq!(ui.clipboard().as_deref(), Some("copied by WASI"));
    let badge = app
        .host
        .decorations_of(&architect_extensions::hostcall::Target::file("notes/a.md"));
    assert_eq!(badge[0].decoration.badge.as_deref(), Some("W"));

    // Without `capabilities.ui`: refused, at the first call.
    let bare = tmp.path().join("bare");
    let app = app_with(WasiRunner::new(), &bare.join("cache"));
    let ui = architect_extensions::hostui::FakeUi::new();
    app.host.set_ui(std::sync::Arc::new(ui.clone()));
    load(
        &app,
        &package(&bare, "hazards", HAZARDS_MANIFEST, HAZARDS_WASM),
    );
    let why = failed(run(&app, "TEST_HAZARDS_ASK", &Value::Null).unwrap_err());
    assert!(why.contains("refused") && why.contains("notify"), "{why}");
    assert!(ui.notices().is_empty());
}

/// Timers and events (#308): a view's interval runs on the host's clock
/// (a fake one here, moved by hand), and a hook the component made at run
/// time runs on a document's save.
#[test]
fn a_views_interval_runs_on_the_hosts_clock_and_a_hook_it_made_runs() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let clock = architect_extensions::events::FakeClock::at(1_000_000);
    let app = app(Limits::default(), &tmp.path().join("cache"));
    app.host.set_clock(std::sync::Arc::new(clock.clone()));
    // Its doc.saved hook: hearing a document's events is reading them.
    let manifest =
        HAZARDS_MANIFEST.replace("[wasm]", "[capabilities]\nreads = [\"documents\"]\n[wasm]");
    load(
        &app,
        &package(tmp.path(), "hazards", &manifest, HAZARDS_WASM),
    );
    assert_eq!(
        run(&app, "TEST_HAZARDS_NOW", &Value::Null).unwrap(),
        json!(1_000_000)
    );

    let process = app.processes.get(HAZARDS).expect("a local process");
    let mut session = process.open("ticker").unwrap();
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut doc = RemoteDocument::new(events);
    doc.apply(next(&mut session.frames)).unwrap();
    assert!(text(&doc).contains("ticks: 0"), "{}", text(&doc));
    let stats = app.components.get(HAZARDS).unwrap();
    // Not due on the host's clock: nothing changes.
    stats.tick();
    assert!(
        session.frames.try_recv().is_err(),
        "nothing is due before its second"
    );
    for n in 1..=3 {
        clock.advance(Duration::from_secs(1));
        stats.tick();
        doc.apply(next(&mut session.frames)).unwrap();
        assert!(
            text(&doc).contains(&format!("ticks: {n}")),
            "{}",
            text(&doc)
        );
    }

    // A hook it makes through the world's `subscribe`, run on a save.
    let key = run(
        &app,
        "TEST_HAZARDS_SUBSCRIBE",
        &json!({ "on": "doc.saved", "action": "TEST_HAZARDS_HEARD" }),
    )
    .unwrap();
    assert_eq!(key, json!("doc.saved/TEST_HAZARDS_HEARD"));
    let saved = architect_extensions::events::DocEvent::Saved {
        path: "notes/a.md".into(),
    };
    assert_eq!(app.host.raise_doc(&saved), 1);
    wait_for(
        "the hook's echo",
        || logs(&app, HAZARDS),
        |lines| {
            lines
                .iter()
                .any(|l| l.contains("heard") && l.contains("notes/a.md"))
        },
    );
    // One on an app event it does not ask for is refused.
    let why = failed(
        run(
            &app,
            "TEST_HAZARDS_SUBSCRIBE",
            &json!({ "on": "app:transport.changed", "action": "TEST_HAZARDS_HEARD" }),
        )
        .unwrap_err(),
    );
    assert!(why.contains("transport.changed"), "{why}");
}

#[test]
fn a_worker_job_keeps_the_view_within_its_frame_budget() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    let process = app.processes.get(COUNTER).unwrap();
    let mut session = process.open("counter").unwrap();
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut doc = RemoteDocument::new(events);
    let first = next(&mut session.frames);
    let add = clickable(&first)[0];
    doc.apply(first).unwrap();

    // The job, in a worker: long enough to click through.
    let started = run(&app, "LAB_WASI_COUNTER_PRIMES", &json!({ "n": 2_000_000 })).unwrap();
    assert!(started["worker"].as_u64().is_some(), "{started}");

    // Clicks while it runs, each timed from the event to the frame that
    // shows its count (the job's progress frames in between applied too).
    let meter = ViewMeter::new("lab.wasi-counter/counter", REMOTE_BUDGET);
    let mut seen = std::collections::BTreeSet::new();
    let mut count = 0;
    let began = Instant::now();
    let mut clicks_during_job = 0;
    loop {
        let job = run(&app, "LAB_WASI_COUNTER_JOB", &Value::Null).unwrap();
        if job["status"] != "running" {
            assert_eq!(job["status"], "done", "{job}");
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(60),
            "the job ran on: {job}"
        );
        count += 1;
        let asked = Instant::now();
        session.events.unbounded_send(click(add)).unwrap();
        loop {
            doc.apply(next_now(&mut session.frames)).unwrap();
            let shown = text(&doc);
            if let Some(p) = shown.split("Counting primes: ").nth(1) {
                seen.insert(p.split('%').next().unwrap_or("").to_owned());
            }
            if shown.contains(&format!("component{count}")) {
                break;
            }
        }
        meter.record(asked.elapsed());
        clicks_during_job += 1;
        std::thread::sleep(Duration::from_millis(16));
    }
    let s = meter.summary();
    eprintln!(
        "the WASI counter's view with a primes job in a worker: {} clicks, p50 {:?} p99 {:?} max {:?}, {} past {:?}; progress seen {:?}",
        s.frames, s.p50, s.p99, s.max, s.slow, REMOTE_BUDGET, seen
    );
    assert!(
        clicks_during_job >= 20,
        "the job ended too soon to measure: {clicks_during_job}"
    );
    assert_eq!(s.slow, 0, "{s:?}");
    assert!(seen.len() >= 3, "its progress bar moved: {seen:?}");
    // Its result, into the view.
    let done = wait_for(
        "the result in the view",
        || {
            if let Ok(frame) = session.frames.try_recv() {
                doc.apply(frame).unwrap();
            }
            text(&doc)
        },
        |t| t.contains("primes below 2000000"),
    );
    assert!(done.contains("148933 primes below 2000000"), "{done}");
    let stats = app.components.get(COUNTER).unwrap();
    assert_eq!(stats.traps(), 0);
}

#[test]
fn unloading_it_cancels_its_workers() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = hazards(WasiRunner::new(), tmp.path());
    // A worker that never ends and never looks whether it should.
    let id = run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "forever" })).unwrap();
    let stats = app.components.get(HAZARDS).unwrap();
    wait_for(
        "it runs",
        || stats.workers(),
        |w| w.iter().any(|w| w.status == WorkerStatus::Running),
    );
    assert_eq!(worker_state(&app, &id)["status"], "running");
    assert_eq!(stats.workers_alive(), 1);
    let unloaded = Instant::now();
    assert!(app.host.unload(HAZARDS));
    wait_for(
        "its instance is freed",
        || stats.workers_alive(),
        |n| *n == 0,
    );
    let took = unloaded.elapsed();
    eprintln!("a spinning worker was stopped {took:?} after its owner unloaded");
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn a_worker_past_its_limits_traps_and_its_owner_runs_on() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let runner = WasiRunner::new().with_worker_limits(Limits {
        memory: 32 << 20,
        call: Duration::from_millis(300),
    });
    let app = hazards(runner, tmp.path());
    let stats = app.components.get(HAZARDS).unwrap();

    let spin = run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "forever" })).unwrap();
    let hoard = run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "hoard" })).unwrap();
    // Its owner answers while they run.
    assert_eq!(
        run(&app, "TEST_HAZARDS_ECHO", &json!({ "say": 1 })).unwrap(),
        json!({ "echo": { "say": 1 } })
    );
    let stopped = wait_for(
        "the spinner stops",
        || worker_state(&app, &spin),
        |s| s["status"] != "running",
    );
    assert_eq!(stopped["status"], "failed", "{stopped}");
    assert!(
        stopped["error"]
            .as_str()
            .unwrap()
            .starts_with("interrupted"),
        "{stopped}"
    );
    let hoarded = wait_for(
        "the hoarder stops",
        || worker_state(&app, &hoard),
        |s| s["status"] != "running",
    );
    assert_eq!(hoarded["status"], "failed", "{hoarded}");
    let why = hoarded["error"].as_str().unwrap();
    assert!(
        why.starts_with("out of memory") && why.contains("32 MiB"),
        "{why}"
    );
    // Its owner never trapped, and answers still.
    assert_eq!(stats.traps(), 0);
    assert_eq!(
        run(&app, "TEST_HAZARDS_ECHO", &json!({ "say": 2 })).unwrap(),
        json!({ "echo": { "say": 2 } })
    );
    wait_for(
        "their instances are freed",
        || stats.workers_alive(),
        |n| *n == 0,
    );
    assert!(
        logs(&app, HAZARDS)
            .iter()
            .any(|l| l.starts_with("worker worker forever failed: interrupted")),
        "{:?}",
        logs(&app, HAZARDS)
    );
}

#[test]
fn back_pressure_holds_both_ways() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = hazards(WasiRunner::new(), tmp.path());
    let stats = app.components.get(HAZARDS).unwrap();

    // Its owner to a worker that never reads: the queue fills, and the
    // owner (a view: it never waits) is told.
    let deaf = run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "deaf" })).unwrap();
    let flood = run(
        &app,
        "TEST_HAZARDS_FLOOD",
        &json!({ "id": deaf, "n": 1000 }),
    )
    .unwrap();
    assert_eq!(flood["accepted"], json!(QUEUE), "{flood}");
    assert!(flood["error"].as_str().unwrap().contains("full"), "{flood}");
    assert!(stats.workers().iter().all(|w| w.to_worker <= QUEUE));

    // A worker that floods its owner while it is busy: the worker waits
    // at a full queue; once its owner hears, all of it comes, in order.
    let chatty = run(
        &app,
        "TEST_HAZARDS_WORK",
        &json!({ "worker": "chatty", "args": { "n": 1000 } }),
    )
    .unwrap();
    let busy = std::thread::spawn({
        let list = app.list.clone();
        move || {
            block_on(list.run_json_async_with(
                &ActionCaller::app(),
                "TEST_HAZARDS_STALL",
                br#"{"ms": 600}"#,
            ))
            .unwrap();
        }
    });
    let mut peak = 0;
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(500) {
        for w in stats.workers() {
            assert!(w.to_owner <= QUEUE, "{w:?}");
            if w.name == "chatty" {
                peak = peak.max(w.to_owner);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    busy.join().unwrap();
    assert_eq!(peak, QUEUE, "the worker waited at a full queue");
    let heard = wait_for(
        "all of it came",
        || worker_state(&app, &chatty),
        |s| s["status"] == "done",
    );
    let messages: Vec<u64> = heard["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_u64().unwrap())
        .collect();
    assert_eq!(messages, (0..1000).collect::<Vec<_>>());
    assert_eq!(heard["result"], json!(1000));
    // Their progress, coalesced: the last.
    assert_eq!(heard["progress"]["done"], json!(1000.0));

    // Messages both ways: a worker answers what it is sent.
    let echo = run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "echo" })).unwrap();
    run(&app, "TEST_HAZARDS_FLOOD", &json!({ "id": echo, "n": 3 })).unwrap();
    let echoed = wait_for(
        "it answered",
        || worker_state(&app, &echo),
        |s| s["messages"].as_array().is_some_and(|m| m.len() == 3),
    );
    assert_eq!(
        echoed["messages"],
        json!([{ "echo": 0 }, { "echo": 1 }, { "echo": 2 }])
    );

    // As many workers as it may, and no more.
    let mut more = Vec::new();
    let refused = loop {
        match run(&app, "TEST_HAZARDS_WORK", &json!({ "worker": "deaf" })) {
            Ok(id) => more.push(id),
            Err(e) => break failed(e),
        }
        assert!(
            more.len() <= architect_extensions::wasi::MAX_WORKERS,
            "no limit"
        );
    };
    assert!(refused.contains("workers running already"), "{refused}");
}

#[test]
fn the_pool_starts_its_threads_lazily_and_stops_them_when_idle() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let runner = WasiRunner::new().with_pool(Pool::new(2, Duration::from_millis(300)));
    let pool = runner.pool();
    let app = hazards(runner, tmp.path());
    assert_eq!(pool.threads(), 0, "none before a worker");
    let ids: Vec<Value> = (0..3)
        .map(|_| {
            run(
                &app,
                "TEST_HAZARDS_WORK",
                &json!({ "worker": "chatty", "args": { "n": 3 } }),
            )
            .unwrap()
        })
        .collect();
    assert!(pool.threads() >= 1 && pool.threads() <= 2, "{pool:?}");
    for id in &ids {
        wait_for(
            "it ran",
            || worker_state(&app, id),
            |s| s["status"] == "done",
        );
    }
    wait_for("its threads stop when idle", || pool.threads(), |n| *n == 0);
}

#[test]
fn its_actions_run_through_the_apps_own_list() {
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    // The app's own list (what its control socket serves), beside the
    // extensions' list the host puts actions on.
    let own = ActionList::new();
    app.host.forward_actions_to(&own);
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    let on_own = |id: &str, args: &Value| {
        block_on(own.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
            .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
    };
    assert_eq!(
        on_own("LAB_WASI_COUNTER_ADD", &json!({ "by": 5 })).unwrap(),
        5
    );
    assert_eq!(
        on_own("LAB_WASI_COUNTER_ASK", &Value::Null).unwrap(),
        json!(["light", "dark", "neon"])
    );
    assert!(own.info("LAB_WASI_COUNTER_PRIMES").is_some());
    assert!(app.host.unload(COUNTER));
    assert!(own.info("LAB_WASI_COUNTER_ADD").is_none());
}

/// The engine's name in its compile log line.
const fn engine_name() -> &'static str {
    if architect_extensions::wasi::PULLEY {
        "Pulley, interpreted"
    } else {
        "Cranelift, native"
    }
}

#[test]
fn the_engine_is_the_platforms() {
    // Pulley on iOS (no JIT) and with the `pulley` feature, which runs
    // every test here on it; Cranelift's native code elsewhere.
    assert_eq!(
        architect_extensions::wasi::PULLEY,
        cfg!(any(target_os = "ios", feature = "pulley"))
    );
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    assert!(
        logs(&app, COUNTER)
            .iter()
            .any(|l| l.starts_with("wasi compiled") && l.contains(engine_name())),
        "{:?}",
        logs(&app, COUNTER)
    );
}

/// An action call's latency, through the app's list into the component and
/// back: `cargo test -p architect-extensions --features wasi --test wasi
/// -- --ignored --nocapture an_action_call` (add `--features pulley` for
/// the interpreter).
#[test]
#[ignore = "a measurement: run it with --ignored --nocapture"]
fn an_action_calls_latency() {
    const CALLS: usize = 2000;
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    let started = Instant::now();
    load(
        &app,
        &package(tmp.path(), "counter", &counter_manifest(), COUNTER_WASM),
    );
    let load_time = started.elapsed();
    for _ in 0..100 {
        run(&app, "LAB_WASI_COUNTER_ADD", &json!({ "by": 1 })).unwrap();
    }
    let mut times: Vec<Duration> = (0..CALLS)
        .map(|_| {
            let t = Instant::now();
            run(&app, "LAB_WASI_COUNTER_ADD", &json!({ "by": 1 })).unwrap();
            t.elapsed()
        })
        .collect();
    times.sort();
    // The `percent`th percentile.
    let at = |percent: usize| times[(times.len() * percent / 100).min(times.len() - 1)];
    let mean = times.iter().sum::<Duration>() / u32::try_from(CALLS).unwrap();
    println!(
        "{}: compile + load {load_time:?}; LAB_WASI_COUNTER_ADD x{CALLS}: \
         p50 {:?}, p90 {:?}, p99 {:?}, mean {mean:?}",
        engine_name(),
        at(50),
        at(90),
        at(99),
    );
}

#[path = "support/server.rs"]
mod server;

/// The network (#310), through the world's `fetch` and `read-body`: a
/// component fetches from a host in `capabilities.hosts`, reads a
/// streamed body to its end, and is refused another host and a redirect
/// to one.
#[test]
fn a_component_fetches_from_its_host_and_is_refused_another() {
    if guests().is_none() {
        return;
    }
    let server = server::serve();
    let tmp = tempfile::tempdir().unwrap();
    let app = app(Limits::default(), &tmp.path().join("cache"));
    let manifest =
        HAZARDS_MANIFEST.replace("[wasm]", "[capabilities]\nhosts = [\"127.0.0.1\"]\n[wasm]");
    load(
        &app,
        &package(tmp.path(), "hazards", &manifest, HAZARDS_WASM),
    );
    let base = format!("http://127.0.0.1:{}", server.port);
    let out = run(
        &app,
        "TEST_HAZARDS_FETCH",
        &json!({ "url": format!("{base}/hello") }),
    )
    .unwrap();
    assert_eq!(out["status"], 200, "{out}");
    assert_eq!(out["body"], "hello from the server", "{out}");
    let out = run(
        &app,
        "TEST_HAZARDS_FETCH",
        &json!({ "url": format!("{base}/stream"), "stream": true }),
    )
    .unwrap();
    assert_eq!(out["body"].as_str(), Some(server::streamed().as_str()));

    let why = failed(
        run(
            &app,
            "TEST_HAZARDS_FETCH",
            &json!({ "url": format!("http://localhost:{}/hello", server.port) }),
        )
        .unwrap_err(),
    );
    assert!(
        why.contains("refused") && why.contains("localhost"),
        "{why}"
    );
    let why = failed(
        run(
            &app,
            "TEST_HAZARDS_FETCH",
            &json!({ "url": format!("{base}/redirect-out") }),
        )
        .unwrap_err(),
    );
    assert!(
        why.contains("refused") && why.contains("redirected"),
        "{why}"
    );
}

/// The document imports (#309) from a real component: it makes a
/// document, edits it as one step, lists and reads it, on the app's
/// documents; refused without the capabilities.
#[test]
fn a_component_makes_edits_lists_and_reads_a_document() {
    use architect_extensions::documents::CrdtDocuments;
    if guests().is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let docs = CrdtDocuments::new().with_space("notes");
    let app = app(Limits::default(), &tmp.path().join("cache"));
    app.host.set_documents(std::sync::Arc::new(docs.clone()));
    let manifest = HAZARDS_MANIFEST.replace(
        "[wasm]",
        "[capabilities]\nreads = [\"documents\"]\nwrites = [\"documents\"]\n[wasm]",
    );
    load(
        &app,
        &package(tmp.path(), "hazards", &manifest, HAZARDS_WASM),
    );
    let out = run(
        &app,
        "TEST_HAZARDS_DOCS",
        &json!({ "path": "notes/wasi.md" }),
    )
    .unwrap();
    assert_eq!(out["made"]["path"], "notes/wasi.md", "{out}");
    assert_eq!(out["text"], "one two", "{out}");
    assert_eq!(out["read"], "one two", "{out}");
    assert_eq!(out["listed"][0]["path"], "notes/wasi.md", "{out}");
    assert_eq!(docs.text("notes/wasi.md").as_deref(), Some("one two"));
    assert_eq!(docs.steps("notes/wasi.md"), ["WASI edit"]);

    let bare = tmp.path().join("bare");
    let app = app_with(WasiRunner::new(), &bare.join("cache"));
    app.host.set_documents(std::sync::Arc::new(
        CrdtDocuments::new().with_space("notes"),
    ));
    load(
        &app,
        &package(&bare, "hazards", HAZARDS_MANIFEST, HAZARDS_WASM),
    );
    let why = failed(run(&app, "TEST_HAZARDS_DOCS", &json!({})).unwrap_err());
    assert!(
        why.contains("refused") && why.contains("documents"),
        "{why}"
    );
}

/// Held while `devenv` runs. Under nextest every test is a process of its
/// own, and each `devenv` run rewrites its profile's bootstrap files: two
/// at once and one reads a half-written `resolve-lock.nix` ("unexpected
/// end of file"), so the components "did not build". One lock file for
/// every test binary that enters the `wasi` profile.
fn devenv_lock() -> Option<std::fs::File> {
    let dir = root().join("target");
    std::fs::create_dir_all(&dir).ok()?;
    let file = std::fs::File::create(dir.join(".devenv-wasi.lock")).ok()?;
    file.lock().ok()?;
    Some(file)
}
