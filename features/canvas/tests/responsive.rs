//! A view that runs past its frame budget never holds up its host or the
//! other views: its frame runs on its script's thread, the host's meter
//! marks it busy while the last good frame stays up, and every other
//! window keeps drawing; a window whose heavy work is in a worker stays
//! within budget and animates, its progress bar moving.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList};
use architect_canvas::Shape;
use architect_canvas::imgui::Input;
use architect_canvas::luau::Session;
use architect_scripting::luau::Runtime;
use architect_scripting::worker::{WorkerGroup, Workers};
use architect_workspace::{FRAME_BUDGET, Meters, ViewMeter};
use futures::executor::block_on;
use serde_json::Value;

/// Each frame spins for `args.ms` before it draws.
const NAUGHTY: &str = r"
local ImGui = require 'imgui'
local ctx = ImGui.CreateContext('naughty')
local frames = 0
local function loop()
  local t = os.clock()
  while os.clock() - t < args.ms / 1000 do end
  frames += 1
  if ImGui.Begin(ctx, 'Naughty') then
    ImGui.Text(ctx, 'frame ' .. frames)
    ImGui.End(ctx)
  end
  defer(loop)
end
loop()
";

/// Draws its frame count, quickly.
const GOOD: &str = r"
local ImGui = require 'imgui'
local ctx = ImGui.CreateContext('good')
local frames = 0
local function loop()
  frames += 1
  if ImGui.Begin(ctx, 'Good') then
    ImGui.Text(ctx, 'frame ' .. frames)
    ImGui.End(ctx)
  end
  defer(loop)
end
loop()
";

fn texts(session: &Session) -> Vec<String> {
    session
        .output()
        .windows
        .iter()
        .flat_map(|w| w.draw.shapes().iter())
        .filter_map(|s| match s {
            Shape::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn start(runtime: &Runtime, source: &str, args: Value) -> Session {
    let (report, session) = block_on(Session::start(
        runtime,
        &ActionCaller::app(),
        source,
        args,
        "S",
    ));
    assert!(report.result.is_ok(), "{:?}", report.result);
    session.expect("it started")
}

#[test]
fn a_naughty_view_goes_busy_and_the_other_panes_keep_updating() {
    let runtime = Runtime::new(ActionList::new());
    let meters = Meters::new();
    let naughty_meter = meters.meter("lab/naughty", FRAME_BUDGET);
    let good_meter = meters.meter("lab/good", FRAME_BUDGET);
    let mut naughty = start(&runtime, NAUGHTY, serde_json::json!({ "ms": 250 }));
    let mut good = start(&runtime, GOOD, Value::Null);
    let last_good_frame = texts(&naughty);

    // The naughty frame, on the host's executor for that pane.
    let told = Arc::new(AtomicBool::new(false));
    let (meter, flag) = (naughty_meter.clone(), Arc::clone(&told));
    let pane = std::thread::spawn(move || {
        let report = block_on(naughty.frame_metered(Input::default(), &meter, || {
            flag.store(true, Ordering::Relaxed);
        }));
        (report, naughty)
    });

    // Meanwhile every other pane keeps drawing, each frame in budget.
    let began = Instant::now();
    let mut drawn = Vec::new();
    let mut saw_busy = false;
    while began.elapsed() < Duration::from_millis(200) {
        let report = block_on(good.frame_metered(Input::default(), &good_meter, || {}));
        assert!(report.result.is_ok());
        drawn.push(texts(&good));
        saw_busy |= naughty_meter.busy();
    }
    let (report, naughty) = pane.join().expect("the pane's task");
    assert!(report.result.is_ok(), "{:?}", report.result);

    assert!(saw_busy, "the naughty view was marked busy as it ran");
    assert!(told.load(Ordering::Relaxed), "its host was told");
    let n = naughty_meter.summary();
    assert_eq!((n.frames, n.slow, n.busy), (1, 1, false));
    assert!(n.max >= Duration::from_millis(250), "{n:?}");
    let g = good_meter.summary();
    eprintln!(
        "while one view spent {:?} on a frame, another drew {} frames: p50 {:?} p99 {:?} max {:?}",
        n.max, g.frames, g.p50, g.p99, g.max
    );
    assert!(g.frames > 50, "the other pane kept drawing: {g:?}");
    assert_eq!(g.slow, 0, "and within its budget: {g:?}");
    assert!(drawn.windows(2).all(|w| w[0] != w[1]), "each frame new");
    assert_ne!(texts(&naughty), last_good_frame, "its frame landed after");
}

/// A window running its heavy work in a worker: a progress bar, and a
/// spinner that moves each frame (`wake_after`).
const HEAVY: &str = r"
local ImGui = require 'imgui'
local ctx = ImGui.CreateContext('heavy')
local job = worker.spawn('spin', { ms = 300 })
local turn = 0
local function loop()
  turn += 1
  if ImGui.Begin(ctx, 'Heavy') then
    local p = job:progress()
    ImGui.ProgressBar(ctx, p and p.fraction or 0)
    ImGui.Text(ctx, ({'|', '/', '-', '\\'})[turn % 4 + 1] .. ' ' .. job:status())
    ImGui.End(ctx)
  end
  wake_after(5)
  defer(loop)
end
loop()
";

/// Spins for `args.ms`, reporting each tenth.
const SPIN: &str = r"
local t = os.clock()
local step = 0
while os.clock() - t < args.ms / 1000 do
  local now = math.floor((os.clock() - t) / (args.ms / 1000) * 10)
  if now > step then step = now worker.progress(step, 10) end
end
worker.progress(10, 10)
return 'done'
";

#[test]
fn a_window_with_a_worker_animates_within_budget_while_the_job_runs() {
    let group = WorkerGroup::new();
    let runtime = Runtime::new(ActionList::new()).with_workers(
        Workers::new()
            .with_source("spin", SPIN)
            .with_group(group.clone()),
    );
    let meter = ViewMeter::new("heavy", FRAME_BUDGET);
    let mut heavy = start(&runtime, HEAVY, Value::Null);
    let mut spinners = Vec::new();
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(10) {
        // What the host does: wait for the script's ask, then a frame.
        let tick = heavy.wake_after().unwrap_or(Duration::from_millis(5));
        std::thread::sleep(tick);
        assert!(
            block_on(heavy.frame_metered(Input::default(), &meter, || {}))
                .result
                .is_ok()
        );
        let said = texts(&heavy);
        let done = said.iter().any(|t| t.ends_with("done"));
        spinners.push(said);
        if done {
            break;
        }
    }
    let s = meter.summary();
    eprintln!(
        "a window with a 300 ms job in a worker: {} frames, p50 {:?} p99 {:?} max {:?}, {} slow",
        s.frames, s.p50, s.p99, s.max, s.slow
    );
    assert!(
        spinners
            .last()
            .is_some_and(|t| t.iter().any(|t| t.ends_with("done")))
    );
    assert!(s.frames > 20, "{s:?}");
    assert_eq!(s.slow, 0, "{s:?}");
    let turning: std::collections::BTreeSet<&String> =
        spinners.iter().filter_map(|t| t.first()).collect();
    assert!(turning.len() >= 4, "the spinner turned: {turning:?}");
    drop(heavy);
    assert_eq!(group.running(), 0);
}
