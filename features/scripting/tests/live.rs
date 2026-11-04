//! Live scripts: one Luau state across frames, `defer` as the loop, and
//! the globals an app adds with a `Module`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionList};
use architect_scripting::luau::mlua::{Lua, Value as LuaValue};
use architect_scripting::luau::{Module, Runtime};
use architect_scripting::{ErrorKind, Transactions};
use futures::executor::block_on;
use serde_json::{Value, json};

/// A module counting the calls a script makes to `Counter.bump()`.
struct Counter(Arc<AtomicU32>);

impl Module for Counter {
    fn global(&self) -> &'static str {
        "Counter"
    }

    fn path(&self) -> &'static str {
        "counter"
    }

    fn install(&self, lua: &Lua) -> architect_scripting::luau::mlua::Result<LuaValue> {
        let table = lua.create_table()?;
        let count = Arc::clone(&self.0);
        table.set(
            "bump",
            lua.create_function(move |_, ()| {
                Ok(count.fetch_add(1, Ordering::Relaxed).saturating_add(1))
            })?,
        )?;
        Ok(LuaValue::Table(table))
    }
}

#[test]
fn a_live_script_runs_what_it_defers_one_frame_at_a_time() {
    let count = Arc::new(AtomicU32::new(0));
    let runtime =
        Runtime::new(ActionList::new()).with_module(Arc::new(Counter(Arc::clone(&count))));
    let source = r"
local counter = require('counter')
local frames = 0
local function loop()
  frames += 1
  print('frame', frames, counter.bump())
  if frames < 3 then defer(loop) end
end
defer(loop)
return 'started'
";
    let (report, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    assert_eq!(report.result, Ok(json!("started")));
    let live = live.expect("it started");
    assert!(live.running());
    for n in 1..=3 {
        let frame = block_on(live.frame());
        assert_eq!(frame.result, Ok(Value::Null));
        assert_eq!(frame.log, vec![format!("frame\t{n}\t{n}")]);
    }
    assert!(!live.running(), "it stopped deferring");
    assert_eq!(count.load(Ordering::Relaxed), 3);
    // A frame with nothing due does nothing.
    assert_eq!(block_on(live.frame()).log, Vec::<String>::new());
}

#[test]
fn a_frame_that_fails_stops_the_script_with_its_line() {
    let runtime = Runtime::new(ActionList::new());
    let source = "local n = 0\nlocal function loop()\n  n += 1\n  if n == 2 then error('boom') end\n  defer(loop)\nend\ndefer(loop)\n";
    let (_, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    let live = live.expect("it started");
    assert!(block_on(live.frame()).result.is_ok());
    let failed = block_on(live.frame()).result.expect_err("it raised");
    assert_eq!((failed.kind, failed.line), (ErrorKind::Runtime, Some(4)));
    assert!(!live.running());
}

#[test]
fn a_live_script_is_bounded_by_its_time_not_its_steps_and_a_one_off_run_cannot_defer() {
    let runtime = Runtime::new(ActionList::new()).with_limits(architect_scripting::luau::Limits {
        steps: 5_000,
        ..Default::default()
    });
    // How many steps fit in a frame is the machine's speed: a live
    // script's start and frames of ten times the step limit run; a
    // one-off run of the same stops at its steps.
    let spin = "for _ = 1, 50000 do end";
    let source = format!("local function loop() {spin} defer(loop) end loop()");
    let (started, live) = block_on(runtime.start(&ActionCaller::app(), &source, Value::Null, "S"));
    assert!(started.result.is_ok(), "{:?}", started.result);
    let live = live.expect("it started");
    for _ in 0..3 {
        assert!(block_on(live.frame()).result.is_ok());
    }
    let run = block_on(runtime.run(&ActionCaller::app(), spin, Value::Null, "S"));
    assert_eq!(run.result.map_err(|e| e.kind), Err(ErrorKind::Limit));
    // A start or a frame that never ends stops at its time.
    let (started, live) =
        block_on(runtime.start(&ActionCaller::app(), "while true do end", Value::Null, "S"));
    assert!(live.is_none());
    let stopped = started.result.expect_err("it ran past its time");
    assert!(stopped.message.contains("ran past its time"), "{stopped}");
    let forever = "local function loop() while true do end end defer(loop)";
    let (_, live) = block_on(runtime.start(&ActionCaller::app(), forever, Value::Null, "S"));
    let stopped = block_on(live.expect("it started").frame()).result;
    let stopped = stopped.expect_err("it ran past its frame");
    assert!(stopped.message.contains("ran past its time"), "{stopped}");
    let once = block_on(runtime.run(&ActionCaller::app(), "defer(print)", Value::Null, "S"));
    assert_eq!(once.result.map_err(|e| e.kind), Err(ErrorKind::Runtime));
    let missing = block_on(runtime.run(
        &ActionCaller::app(),
        "return require('io')",
        Value::Null,
        "S",
    ));
    let missing = missing.result.expect_err("no such module");
    assert!(missing.message.contains("no module"), "{missing}");
}

/// Undo steps, by name.
#[derive(Default)]
struct Steps {
    open: Mutex<Option<String>>,
    made: Mutex<Vec<String>>,
}

impl Transactions for Steps {
    fn begin(&self, name: &str) -> bool {
        let mut open = self.open.lock().unwrap();
        if open.is_some() {
            return false;
        }
        *open = Some(name.to_owned());
        true
    }

    fn commit(&self) -> Option<String> {
        let name = self.open.lock().unwrap().take()?;
        self.made.lock().unwrap().push(name.clone());
        Some(name)
    }

    fn abort(&self) -> Option<String> {
        self.open.lock().unwrap().take()
    }
}

#[test]
fn each_frame_is_a_step_of_its_own() {
    let steps = Arc::new(Steps::default());
    let runtime = Runtime::new(ActionList::new()).with_transactions(steps.clone());
    let source = "defer(function() defer(function() end) end)";
    let (report, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "Tool"));
    assert_eq!(report.undo.as_deref(), Some("Tool"));
    let live = live.expect("it started");
    assert_eq!(block_on(live.frame()).undo.as_deref(), Some("Tool"));
    assert_eq!(block_on(live.frame()).undo.as_deref(), Some("Tool"));
    assert_eq!(steps.made.lock().unwrap().len(), 3);
}
