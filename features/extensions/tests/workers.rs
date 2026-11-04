//! An extension's workers: declared in its manifest, spawned by its
//! scripts off their threads, cancelled when it unloads; and `EXT_VIEWS`,
//! how each of its views keeps up.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::views::install_view_actions;
use architect_extensions::{Origin, Package};
use architect_workspace::Meters;
use futures::executor::block_on;
use serde_json::{Value, json};

const MANIFEST: &str = r#"
id = "lab.heavy"
name = "Heavy"
version = "0.1.0"
kind = "luau"
api = "0.1"

[[actions]]
id = "LAB_HEAVY_PRIMES"
name = "Heavy: count primes"
params = '{"type":"object","properties":{"n":{"type":"integer"}}}'
script = "count.luau"

[[actions]]
id = "LAB_HEAVY_WAIT"
name = "Heavy: wait on a job that never ends"
script = "wait.luau"

[[workers]]
id = "primes"
script = "workers/primes.luau"

[[workers]]
id = "forever"
script = "workers/forever.luau"
"#;

const FILES: [(&str, &str); 4] = [
    (
        "count.luau",
        "local w = worker.spawn('primes', { n = args.n }) return w:join()",
    ),
    (
        "wait.luau",
        "local w = worker.spawn('forever') local _, why = w:join() return w:status()",
    ),
    (
        "workers/primes.luau",
        r"
local found = 0
for i = 2, args.n do
  local prime = true
  local d = 2
  while d * d <= i do
    if i % d == 0 then prime = false break end
    d += 1
  end
  if prime then found += 1 end
end
return found
",
    ),
    ("workers/forever.luau", "while true do end"),
];

fn loaded() -> Host {
    let host = Host::new(ActionList::new(), SharedSections::default());
    let package = Package::embedded(MANIFEST, &FILES).unwrap();
    host.load(&package, Origin::Builtin).unwrap();
    host
}

#[test]
fn an_extensions_action_hands_its_work_to_a_declared_worker() {
    let host = loaded();
    let out = block_on(
        host.list()
            .run_json_async("LAB_HEAVY_PRIMES", br#"{"n": 20000}"#),
    )
    .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), json!(2262));
}

#[test]
fn unloading_an_extension_cancels_its_workers() {
    let host = loaded();
    let list = host.list().clone();
    // An action blocked on a worker that never ends, on its own thread.
    let waiting = std::thread::spawn(move || block_on(list.run_json_async("LAB_HEAVY_WAIT", b"")));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiting.is_finished(), "the job runs on");
    let t = Instant::now();
    assert!(host.unload("lab.heavy"));
    let out = waiting.join().unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&out).unwrap(),
        json!("cancelled")
    );
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}

#[test]
fn a_worker_must_be_luau() {
    let host = Host::new(ActionList::new(), SharedSections::default());
    let mut files = FILES;
    files[3] = ("workers/forever.luau", "while true do");
    let package = Package::embedded(MANIFEST, &files).unwrap();
    let e = host.load(&package, Origin::Builtin).unwrap_err();
    assert!(e.to_string().contains("worker forever"), "{e}");
}

#[test]
fn ext_views_says_how_each_view_keeps_up() {
    let list = ActionList::new();
    let meters = Meters::new();
    install_view_actions(&list, meters.clone()).unwrap();
    let m = meters.meter("lab.clicker/counter", Duration::from_millis(33));
    m.record(Duration::from_millis(4));
    m.record(Duration::from_millis(250));
    m.set_busy(true);
    let out: Value = serde_json::from_slice(&list.run_json("EXT_VIEWS", b"").unwrap()).unwrap();
    let view = &out["views"][0];
    assert_eq!(view["view"], "lab.clicker/counter");
    assert_eq!(view["frames"], 2);
    assert_eq!(view["slow"], 1);
    assert_eq!(view["max_ms"], 250.0);
    assert_eq!(view["budget_ms"], 33.0);
    assert_eq!(view["busy"], true);
}
