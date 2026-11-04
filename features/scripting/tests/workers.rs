//! Views and workers: a live script's frames stay within their budget
//! while a worker does the heavy work on another thread; a frame that
//! runs past its budget is stopped; a worker goes when its owner does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList};
use architect_scripting::ErrorKind;
use architect_scripting::luau::{Limits, Runtime};
use architect_scripting::worker::{WorkerGroup, Workers};
use futures::executor::block_on;
use serde_json::{Value, json};

/// Counts the primes below `args.n` the slow way, reporting as it goes.
const PRIMES: &str = r"
local n = args.n
local found = 0
for i = 2, n do
  local prime = true
  local d = 2
  while d * d <= i do
    if i % d == 0 then prime = false break end
    d += 1
  end
  if prime then found += 1 end
  if i % 2000 == 0 then worker.progress(i, n, found .. ' so far') end
end
return found
";

/// Answers each message with its double, until it is cancelled.
const ECHO: &str = r"
worker.on_message(function(m) worker.post(m * 2) end)
return 'serving'
";

/// Spins until it is cancelled.
const FOREVER: &str = "while true do end";

fn workers(group: &WorkerGroup) -> Workers {
    Workers::new()
        .with_source("primes", PRIMES)
        .with_source("echo", ECHO)
        .with_source("forever", FOREVER)
        .with_group(group.clone())
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    let rank = sorted.len().saturating_mul(p).div_ceil(100).max(1);
    sorted
        .get(rank.saturating_sub(1))
        .copied()
        .unwrap_or_default()
}

/// A view that spawns a primes job and reads its progress each frame.
const VIEW: &str = r"
local job = nil
local seen = nil
local function frame()
  if job == nil then job = worker.spawn('primes', { n = args.n }) end
  local p = job:progress()
  if p then seen = p.fraction end
  print(job:status(), seen)
  defer(frame)
end
defer(frame)
";

#[test]
fn a_long_job_in_a_worker_never_delays_a_views_frames() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(workers(&group));
    let budget = Duration::from_millis(16);

    // The same view with nothing running: its frames, as a baseline.
    let (_, idle) = block_on(runtime.start(
        &ActionCaller::app(),
        "local function f() print(1) defer(f) end defer(f)",
        Value::Null,
        "idle",
    ));
    let idle = idle.expect("it started");
    let mut quiet = Vec::new();
    for _ in 0..200 {
        let t = Instant::now();
        assert!(block_on(idle.frame()).result.is_ok());
        quiet.push(t.elapsed());
    }

    let (_, live) =
        block_on(runtime.start(&ActionCaller::app(), VIEW, json!({ "n": 400_000 }), "view"));
    let live = live.expect("it started");
    let mut busy = Vec::new();
    let mut statuses = Vec::new();
    let began = Instant::now();
    // Frames as fast as a host would ask for them, while the job runs.
    while began.elapsed() < Duration::from_secs(60) {
        let t = Instant::now();
        let report = block_on(live.frame());
        busy.push(t.elapsed());
        assert!(report.result.is_ok(), "{:?}", report.result);
        let said = report.log.last().cloned().unwrap_or_default();
        statuses.push(said.clone());
        if said.starts_with("done") {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let job_took = began.elapsed();
    // The same work in the view itself: one frame holds it all.
    let inline = PRIMES
        .replace("worker.progress(i, n, found .. ' so far')", "")
        .replace("return found", "print(found)");
    let (_, naive) = block_on(runtime.start(
        &ActionCaller::app(),
        &format!("local function f() {inline} end defer(f)"),
        json!({ "n": 100_000 }),
        "inline",
    ));
    let t = Instant::now();
    assert!(block_on(naive.expect("it started").frame()).result.is_ok());
    let inline_frame = t.elapsed();
    quiet.sort_unstable();
    busy.sort_unstable();
    let (q50, q99) = (percentile(&quiet, 50), percentile(&quiet, 99));
    let (busy50, busy99) = (percentile(&busy, 50), percentile(&busy, 99));
    eprintln!(
        "frames without a job: {} p50 {q50:?} p99 {q99:?}; with the job ({job_took:?}): {} p50 {busy50:?} p99 {busy99:?} max {:?}; a quarter of the job inline: one frame of {inline_frame:?}",
        quiet.len(),
        busy.len(),
        busy.last().copied().unwrap_or_default(),
    );
    assert!(inline_frame > budget, "inline, the work blows the budget");
    assert!(
        statuses.last().is_some_and(|s| s.starts_with("done")),
        "the job finished: {:?}",
        statuses.last()
    );
    assert!(
        statuses.iter().filter(|s| s.starts_with("running")).count() > 10,
        "the view drew many frames while the job ran"
    );
    assert!(
        busy99 < budget,
        "every frame within its budget while the job ran: p99 {busy99:?}"
    );
}

#[test]
fn a_frame_past_its_budget_is_stopped_and_says_to_use_a_worker() {
    let runtime = Runtime::new(ActionList::new()).with_limits(Limits {
        frame: Duration::from_millis(50),
        steps: u64::MAX,
        ..Limits::default()
    });
    let source = "local function f() local x = 0 while true do x += 1 end end defer(f)";
    let (_, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    let live = live.expect("it started");
    let t = Instant::now();
    let failed = block_on(live.frame()).result.expect_err("stopped");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(failed.kind, ErrorKind::Limit);
    assert!(failed.message.contains("worker"), "{failed}");
    assert!(!live.running());
}

#[test]
fn a_worker_goes_when_its_owner_does() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(workers(&group));
    let source =
        "local a = worker.spawn('forever') local b = worker.spawn('echo') defer(function() end)";
    let (report, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    assert!(report.result.is_ok(), "{:?}", report.result);
    let live = live.expect("it started");
    assert_eq!(group.running(), 2);
    drop(live);
    let t = Instant::now();
    while group.running() > 0 && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(group.running(), 0, "cancelled with their owner");
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}

#[test]
fn a_group_cancels_its_workers_and_a_live_owner_hears_how_they_ended() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(workers(&group));
    let source = r"
local w = worker.spawn('forever')
worker.on_done(w, function(result, err) print('ended', w:status(), err) end)
local function f() defer(f) end
defer(f)
";
    let (_, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    let live = live.expect("it started");
    assert_eq!(group.cancel_all(), 1);
    // Its end wakes the view's host.
    block_on(live.woken());
    let mut heard = Vec::new();
    for _ in 0..50 {
        heard.extend(block_on(live.frame()).log);
        if !heard.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(heard, ["ended\tcancelled\tnil"]);
}

#[test]
fn a_one_off_run_talks_to_its_worker_and_joins_it() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(workers(&group));
    let source = r"
local echo = worker.spawn('echo')
echo:post(21)
local kind, answer = echo:next(5000)
local primes = worker.spawn('primes', { n = 20000 })
local found, err = primes:join()
local p = primes:progress()
echo:cancel()
local _, why = echo:join(5000)
return { kind = kind, answer = answer, found = found, err = err,
         fraction = p.fraction, echo = echo:status(), why = why }
";
    let report = block_on(runtime.run(&ActionCaller::app(), source, Value::Null, "S"));
    let out = report.result.expect("ran");
    assert_eq!(out["kind"], "message");
    assert_eq!(out["answer"], 42);
    assert_eq!(out["found"], 2262);
    assert_eq!(out["err"], Value::Null);
    assert_eq!(out["fraction"], 1.0);
    assert_eq!(out["echo"], "cancelled");
    assert_eq!(group.running(), 0);
}

#[test]
fn a_worker_has_its_own_limits_and_no_app() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(
        workers(&group)
            .with_source("app", "return actions == nil and ImGui == nil")
            .with_limits(Limits {
                steps: 100_000,
                ..Limits::worker()
            }),
    );
    let source = r"
local f = worker.spawn('forever')
local _, err = f:join()
local a = worker.spawn('app')
return { status = f:status(), err = err, alone = a:join() }
";
    let report = block_on(runtime.run(&ActionCaller::app(), source, Value::Null, "S"));
    let out = report.result.expect("ran");
    assert_eq!(out["status"], "failed");
    assert!(
        out["err"].as_str().is_some_and(|e| e.contains("steps")),
        "{out}"
    );
    assert_eq!(out["alone"], true, "a worker reaches no actions, no UI");
    let missing = block_on(runtime.run(
        &ActionCaller::app(),
        "worker.spawn('nope')",
        Value::Null,
        "S",
    ));
    let e = missing.result.expect_err("no such worker");
    assert!(e.message.contains("no worker"), "{e}");
}

/// Notes when a step is taken back.
#[derive(Default)]
struct Aborts(std::sync::atomic::AtomicBool);

impl architect_scripting::Transactions for Aborts {
    fn begin(&self, _: &str) -> bool {
        true
    }

    fn commit(&self) -> Option<String> {
        None
    }

    fn abort(&self) -> Option<String> {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        None
    }
}

#[test]
fn dropping_a_run_stops_its_script() {
    use std::future::Future as _;
    let aborts = std::sync::Arc::new(Aborts::default());
    let runtime = Runtime::new(ActionList::new())
        .with_transactions(aborts.clone())
        .with_limits(Limits {
            steps: u64::MAX,
            ..Limits::default()
        });
    let caller = ActionCaller::app();
    // Polled once, so it starts, then dropped.
    let mut run = Box::pin(runtime.run(&caller, "while true do end", Value::Null, "S"));
    let waker = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    assert!(run.as_mut().poll(&mut cx).is_pending());
    std::thread::sleep(Duration::from_millis(20));
    drop(run);
    // Stopped at its next step, its step taken back.
    let t = Instant::now();
    while !aborts.0.load(std::sync::atomic::Ordering::Relaxed)
        && t.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        aborts.0.load(std::sync::atomic::Ordering::Relaxed),
        "it stopped"
    );
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}
