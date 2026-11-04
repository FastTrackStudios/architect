//! The Luau runtime: one script, run against an Action List as a caller,
//! inside one transaction.
//!
//! What a script sees (its globals, read-only — Luau's sandbox):
//!
//! | global | what it does |
//! |---|---|
//! | `actions.list(filter?)` | the actions the caller may run: `{id, name, description, category, group, effect, enabled, disabled_reason}`, filtered by text |
//! | `actions.describe(id)` | one, with `params` and `result` (JSON Schema as tables); `nil` when there is none |
//! | `actions.run(id, args?)` | runs it with `args` (a table), answering with its result |
//! | `actions.enabled(id)` | whether it can run now, and why not |
//! | `state.query(id, args?)` | runs a read-only action; refuses one that changes the app |
//! | `print(...)`, `log(...)` | a line in the run's log |
//! | `args` | what the run was given (`nil` for nothing) |
//! | `require(name)` | a [`Module`] the app added (`require 'imgui'`); nothing else |
//! | `defer(fn)` | a live script only ([`Runtime::start`]): calls `fn` on the next [`Live::frame`] |
//!
//! An app adds globals of its own with [`Module`]s ([`Runtime::with_module`]):
//! `architect-canvas`'s `ImGui` and `Canvas`, for one, and the app's theme
//! (`theme`, [`crate::theme::ThemeModule`]).
//!
//! A script either runs once ([`Runtime::run`]) or lives
//! ([`Runtime::start`]): its first run sets up, and each function it hands
//! `defer` runs on the next frame its host asks for — `ReaScript`'s
//! `reaper.defer`, the loop of an immediate-mode window. Each frame is a
//! step of its own ([`Live::frame`]); one that fails stops the script.
//!
//! What the script returns is the run's result. An action that fails
//! raises an error a script can catch with `pcall`; one it does not catch
//! stops it, with the line it was on ([`ScriptError`]).
//!
//! Every run is a fresh Luau state: nothing one run sets is seen by the
//! next (a live script's frames share its one state). Each action runs as the run's [`ActionCaller`], so a model's
//! script can do exactly what the model may, action by action.
//!
//! No script runs on its caller's thread (`crate::vm`): a run takes a
//! thread of a small pool, a live script has a thread of its own, and
//! the caller awaits the result — an app's UI thread never waits on a
//! script, however long it runs. Dropping a run's future stops it. The
//! actions a script calls run back on the app's Tokio runtime (the one
//! its run was started from, or [`Runtime::with_runtime`]'s), the script
//! awaiting the answer over a channel ([`crate::vm::Dispatch`]).
//! Heavy work goes to a worker ([`crate::worker`]), a Luau state of its
//! own on the pool, so a live script's frames keep within their budget
//! ([`Limits::frame`]).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionCaller, ActionInfo, ActionList, Effect};
use mlua::{Lua, LuaSerdeExt, MultiValue, Value as LuaValue};
use serde_json::{Value, json};

use crate::vm::{self, Cancel, Dispatch, Guard, Stop, Wake};
use crate::{ErrorKind, ScriptError, Transactions};

/// The Luau binding a [`Module`] is written against: this one, so its
/// version is the runtime's.
pub use mlua;

/// The chunk name a script's errors are reported under (`script:3: …`).
const CHUNK: &str = "script";

/// How far a run may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most memory its Luau state may hold, in bytes.
    pub memory: usize,
    /// The most steps it may take: Luau counts one at every call and every
    /// turn of a loop. What stops `while true do end`. A live script's
    /// count starts again each frame.
    pub steps: u64,
    /// The longest one frame of a live script may run ([`Live::frame`]):
    /// a view that runs past it is stopped, with a message that says to
    /// hand the work to a worker. Its host shows it busy long before
    /// (`architect_workspace::ViewMeter`, a frame's 16 ms).
    pub frame: Duration,
    /// The longest a run may take in all, `None` for no limit: a worker's
    /// CPU budget ([`crate::worker`]).
    pub time: Option<Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory: 64 * 1024 * 1024,
            steps: 10_000_000,
            frame: Duration::from_millis(500),
            time: None,
        }
    }
}

impl Limits {
    /// A worker's: far more steps and memory than a script's, and five
    /// minutes of time.
    #[must_use]
    pub const fn worker() -> Self {
        Self {
            memory: 256 * 1024 * 1024,
            steps: 50_000_000_000,
            frame: Duration::from_millis(500),
            time: Some(Duration::from_secs(300)),
        }
    }
}

/// What a run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// What the script returned (`null` for nothing), or why it stopped.
    pub result: Result<Value, ScriptError>,
    /// What it printed, a line a call.
    pub log: Vec<String>,
    /// The actions it ran and queried, in order — those that failed too.
    pub ran: Vec<String>,
    /// The undo step it made, when it changed something and finished. A
    /// run that fails is taken back, and makes none.
    pub undo: Option<String>,
}

impl Report {
    /// The report as JSON: `{result, log, ran, undo}`, or `{error: {kind,
    /// line, message}, log, ran}` when it failed.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match &self.result {
            Ok(result) => json!({
                "result": result,
                "log": self.log,
                "ran": self.ran,
                "undo": self.undo,
            }),
            Err(error) => json!({
                "error": {
                    "kind": kind_name(error.kind),
                    "line": error.line,
                    "message": error.message,
                },
                "log": self.log,
                "ran": self.ran,
            }),
        }
    }
}

const fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Syntax => "syntax",
        ErrorKind::Runtime => "runtime",
        ErrorKind::Action => "action",
        ErrorKind::Limit => "limit",
    }
}

const fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::ReadOnly => "read_only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
    }
}

/// What a run may reach, past what its caller may: `Err(why)` for an
/// action it may not see or run. An extension's capabilities (the action
/// namespaces it named, the most it may change) are one.
pub type Gate = Arc<dyn Fn(&ActionInfo) -> Result<(), String> + Send + Sync>;

/// What a script reaches as a global, and by `require`, past the
/// runtime's own: `ImGui`, `Canvas`.
///
/// Installed into each fresh state before it is sandboxed, so what it
/// gives is read-only to the script. What it keeps across frames it keeps
/// outside Luau (an `Arc` it shares with its host), never a Luau value:
/// that would keep the state alive past its script.
pub trait Module: Send + Sync {
    /// Its global (`ImGui`).
    fn global(&self) -> &str;

    /// What `require` knows it by (`imgui`).
    fn path(&self) -> &str;

    /// Makes its value in `lua`.
    ///
    /// # Errors
    ///
    /// When Luau does (out of memory).
    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue>;

    /// Whether its fields change while a script runs (`theme.scheme`):
    /// then a script's `Global.field` is read each time it runs, never
    /// resolved once as Luau does for a sandbox's globals.
    fn live_fields(&self) -> bool {
        false
    }

    /// A live script's frame is starting ([`Live::frame`]), before what
    /// it deferred runs: catch up with the host (the theme's snapshot,
    /// `crate::theme::ThemeModule`). An error stops the script.
    ///
    /// # Errors
    ///
    /// What it calls in the script fails.
    fn frame(&self, lua: &Lua) -> mlua::Result<()> {
        let _ = lua;
        Ok(())
    }
}

/// An error a [`Module`]'s function raises: the script stops (or its
/// `pcall` catches it) with `message`, at the line it called from.
#[must_use]
pub fn raise(lua: &Lua, message: impl Into<String>) -> mlua::Error {
    fault(lua, ErrorKind::Runtime, message.into())
}

/// The registry key of a live script's deferred functions.
const DEFERRED: &str = "architect.deferred";
/// The registry key of the modules `require` finds.
const MODULES: &str = "architect.modules";

/// Runs scripts against one Action List.
#[derive(Clone)]
pub struct Runtime {
    list: ActionList,
    transactions: Option<Arc<dyn Transactions>>,
    limits: Limits,
    gate: Option<Gate>,
    modules: Vec<Arc<dyn Module>>,
    /// Where its scripts' action calls run; `None`: the runtime each run
    /// is started from ([`Dispatch::here`]).
    dispatch: Option<Dispatch>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("list", &self.list)
            .field("transactions", &self.transactions.is_some())
            .field("limits", &self.limits)
            .field("gate", &self.gate.is_some())
            .field("dispatch", &self.dispatch)
            .field(
                "modules",
                &self.modules.iter().map(|m| m.path()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Runtime {
    /// Scripts over `list`, with no transactions: what they change stays
    /// as each action left it.
    #[must_use]
    pub fn new(list: ActionList) -> Self {
        Self {
            list,
            transactions: None,
            limits: Limits::default(),
            gate: None,
            modules: Vec::new(),
            dispatch: None,
        }
    }

    /// Its scripts' action calls run on `handle`, whatever runtime a run
    /// is started from. By default they run on the runtime of the thread
    /// that started the run ([`Dispatch::here`]): a script's own thread
    /// has none, and an action that needs one (a timer, a socket) fails
    /// there.
    #[must_use]
    pub fn with_runtime(mut self, handle: tokio::runtime::Handle) -> Self {
        self.dispatch = Some(Dispatch::to(handle));
        self
    }

    /// Its scripts' action calls go where `dispatch` sends them (an
    /// extension host passes the runtime it was loaded on).
    #[must_use]
    pub fn with_dispatch(mut self, dispatch: Dispatch) -> Self {
        if dispatch.has_runtime() {
            self.dispatch = Some(dispatch);
        }
        self
    }

    /// Where a run started now sends its action calls.
    fn dispatch(&self) -> Dispatch {
        self.dispatch.clone().unwrap_or_else(Dispatch::here)
    }

    /// Its scripts reach `module` too.
    #[must_use]
    pub fn with_module(mut self, module: Arc<dyn Module>) -> Self {
        self.modules.push(module);
        self
    }

    /// Its runs see and run only the actions `gate` lets through, on top
    /// of what the caller may.
    #[must_use]
    pub fn with_gate(mut self, gate: Gate) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Each run one step of `transactions`, taken back when it fails.
    #[must_use]
    pub fn with_transactions(mut self, transactions: Arc<dyn Transactions>) -> Self {
        self.transactions = Some(transactions);
        self
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Whether its runs are transactions.
    #[must_use]
    pub const fn has_transactions(&self) -> bool {
        self.transactions.is_some()
    }

    /// The list it runs against.
    #[must_use]
    pub const fn list(&self) -> &ActionList {
        &self.list
    }

    /// Whether `source` is Luau, without running it.
    ///
    /// # Errors
    ///
    /// Where it is not ([`ErrorKind::Syntax`]).
    pub fn check(source: &str) -> Result<(), ScriptError> {
        load(&Lua::new(), source).map(drop)
    }

    /// Runs `source` as `caller` with `args`, as one undo step named
    /// `step` ("Script: Arm selected tracks").
    ///
    /// It runs on a thread of the pool, never the caller's: awaiting it
    /// waits on a channel. Dropping the future stops the script at its
    /// next step.
    pub async fn run(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> Report {
        let (mut runtime, caller, source, step) = (
            self.clone(),
            caller.clone(),
            source.to_owned(),
            step.to_owned(),
        );
        runtime.dispatch = Some(self.dispatch());
        let task = vm::run(move |cancel| async move {
            runtime
                .run_here(&caller, &source, args, &step, cancel)
                .await
        });
        task.await.unwrap_or_else(|| stopped("its thread stopped"))
    }

    /// [`Self::run`], on this thread.
    async fn run_here(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
        cancel: Cancel,
    ) -> Report {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let guard = Arc::new(Guard::new(self.limits.steps, vec![cancel.clone()]));
        guard.deadline(self.limits.time);
        let owner = Owner {
            wake: None,
            cancel: vec![cancel],
        };
        let result = match self.state(caller, &args, &trace, &guard, owner, None) {
            Ok(lua) => {
                let main = load(&lua, source);
                self.step(step, async { execute(&lua, main?).await }).await
            }
            Err(e) => (Err(script_error(&e)), None),
        };
        report(result, &trace)
    }

    /// Starts `source` as a live script, as `caller` with `args`: its
    /// first run is one step named `step`, and what it hands `defer` runs
    /// on each [`Live::frame`] after. `None` when the first run failed.
    ///
    /// The script lives on a thread of its own, its Luau state with it,
    /// until the [`Live`] is dropped: its first run and every frame run
    /// there, the caller awaiting each.
    pub async fn start(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> (Report, Option<Live>) {
        let (ready, started) = futures::channel::oneshot::channel::<Report>();
        let (frames, asked) = std::sync::mpsc::channel::<Ask>();
        let shared = Arc::new(LiveShared::default());
        let (mut runtime, caller, source, name) = (
            self.clone(),
            caller.clone(),
            source.to_owned(),
            step.to_owned(),
        );
        runtime.dispatch = Some(self.dispatch());
        let theirs = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name(format!("luau live: {step}"))
            .spawn(move || {
                futures::executor::block_on(live_thread(
                    runtime, caller, source, args, name, theirs, ready, asked,
                ));
            });
        if let Err(e) = spawned {
            return (stopped(&format!("its thread did not start: {e}")), None);
        }
        let Ok(report) = started.await else {
            return (stopped("its thread stopped"), None);
        };
        let live = report.result.is_ok().then(|| Live {
            frames,
            shared,
            step: step.to_owned(),
        });
        (report, live)
    }

    /// `body`, inside one step named `step` when there are transactions:
    /// committed when it succeeds, taken back when it fails.
    async fn step<T>(
        &self,
        step: &str,
        body: impl Future<Output = Result<T, ScriptError>>,
    ) -> (Result<T, ScriptError>, Option<String>) {
        let opened = self.transactions.as_ref().is_some_and(|t| t.begin(step));
        let result = body.await;
        let undo = match (&self.transactions, opened, result.is_ok()) {
            (Some(t), true, true) => t.commit(),
            (Some(t), true, false) => {
                t.abort();
                None
            }
            _ => None,
        };
        (result, undo)
    }

    /// A fresh, sandboxed Luau state with the script's globals — `defer`
    /// and `wake_after` too when it is `live`.
    fn state(
        &self,
        caller: &ActionCaller,
        args: &Value,
        trace: &Arc<Mutex<Trace>>,
        guard: &Arc<Guard>,
        owner: Owner,
        live: Option<&Arc<LiveShared>>,
    ) -> mlua::Result<Lua> {
        let lua = Lua::new();
        lua.set_memory_limit(self.limits.memory)?;
        // What a module's workers answer to: the run's cancel, a live
        // script's wake.
        lua.set_app_data(owner);
        let globals = lua.globals();
        globals.set("actions", self.actions_table(&lua, caller, trace)?)?;
        globals.set("state", self.state_table(&lua, caller, trace)?)?;
        let print = log_function(&lua, trace)?;
        globals.set("print", print.clone())?;
        globals.set("log", print)?;
        globals.set("args", to_lua(&lua, args)?)?;
        let modules = lua.create_table()?;
        for module in &self.modules {
            let value = module.install(&lua)?;
            globals.set(module.global(), value.clone())?;
            modules.set(module.path(), value)?;
        }
        // Kept in the registry, not captured: a Luau value a Rust closure
        // holds keeps its state alive for ever.
        lua.set_named_registry_value(MODULES, modules)?;
        // Luau resolves `Global.field` once, as a chunk loads, in a
        // sandbox; a module whose fields move is read each time.
        lua.set_compiler(
            mlua::chunk::Compiler::new().set_mutable_globals(
                self.modules
                    .iter()
                    .filter(|m| m.live_fields())
                    .map(|m| m.global().to_owned()),
            ),
        );
        globals.set(
            "require",
            lua.create_function(|lua, path: String| {
                let modules: mlua::Table = lua.named_registry_value(MODULES)?;
                match modules.get(path.as_str())? {
                    LuaValue::Nil => Err(fault(
                        lua,
                        ErrorKind::Runtime,
                        format!("there is no module {path:?} here"),
                    )),
                    found => Ok(found),
                }
            })?,
        )?;
        if let Some(shared) = live {
            lua.set_named_registry_value(DEFERRED, lua.create_table()?)?;
            globals.set(
                "defer",
                lua.create_function(|lua, f: mlua::Function| {
                    let deferred: mlua::Table = lua.named_registry_value(DEFERRED)?;
                    deferred.push(f)
                })?,
            )?;
            let shared = Arc::clone(shared);
            globals.set(
                "wake_after",
                lua.create_function(move |_, ms: f64| {
                    shared.ask_tick(ms);
                    Ok(())
                })?,
            )?;
        }
        // From here the globals above are read-only, and what a script
        // sets lives in its own table.
        lua.sandbox(true)?;
        interrupt(&lua, Arc::clone(guard));
        Ok(lua)
    }

    fn actions_table(
        &self,
        lua: &Lua,
        caller: &ActionCaller,
        trace: &Arc<Mutex<Trace>>,
    ) -> mlua::Result<mlua::Table> {
        let table = lua.create_table()?;
        let (list, who, gate) = (self.list.clone(), caller.clone(), self.gate.clone());
        table.set(
            "list",
            lua.create_function(move |lua, filter: Option<String>| {
                let filter = filter.map(|f| f.to_lowercase());
                let found: Vec<Value> = visible(&list, &who, gate.as_ref())
                    .iter()
                    .filter(|i| filter.as_deref().is_none_or(|f| matches(i, f)))
                    .map(summary)
                    .collect();
                to_lua(lua, &Value::Array(found))
            })?,
        )?;
        let (list, who, gate) = (self.list.clone(), caller.clone(), self.gate.clone());
        table.set(
            "describe",
            lua.create_function(move |lua, id: String| {
                let found = visible(&list, &who, gate.as_ref())
                    .into_iter()
                    .find(|i| i.id == id);
                found.map_or(Ok(LuaValue::Nil), |info| to_lua(lua, &detail(&info)))
            })?,
        )?;
        let (list, who, gate) = (self.list.clone(), caller.clone(), self.gate.clone());
        table.set(
            "enabled",
            lua.create_function(move |_, id: String| {
                let found = visible(&list, &who, gate.as_ref())
                    .into_iter()
                    .find(|i| i.id == id);
                Ok(found.map_or_else(
                    || (false, Some(format!("no action {id}"))),
                    |i| (i.enabled, i.disabled_reason),
                ))
            })?,
        )?;
        table.set("run", self.run_function(lua, caller, trace, false)?)?;
        Ok(table)
    }

    fn state_table(
        &self,
        lua: &Lua,
        caller: &ActionCaller,
        trace: &Arc<Mutex<Trace>>,
    ) -> mlua::Result<mlua::Table> {
        let table = lua.create_table()?;
        table.set("query", self.run_function(lua, caller, trace, true)?)?;
        Ok(table)
    }

    /// `actions.run`, or — `read_only` — `state.query`.
    fn run_function(
        &self,
        lua: &Lua,
        caller: &ActionCaller,
        trace: &Arc<Mutex<Trace>>,
        read_only: bool,
    ) -> mlua::Result<mlua::Function> {
        let (list, who, trace) = (self.list.clone(), caller.clone(), Arc::clone(trace));
        let gate = self.gate.clone();
        let dispatch = self.dispatch();
        lua.create_async_function(move |lua, (id, args): (String, Option<LuaValue>)| {
            // Where the script is, taken now: once the future runs, the
            // script is no longer on that line's frame.
            let line = line_of(&lua);
            let refused = refusal(&list, &id, read_only, gate.as_ref());
            let args = args.map(|a| lua.from_value::<Value>(a));
            let (list, who, trace) = (list.clone(), who.clone(), Arc::clone(&trace));
            let dispatch = dispatch.clone();
            async move {
                if let Some(message) = refused {
                    return Err(fault_at(line, ErrorKind::Action, message));
                }
                let args = match args.transpose() {
                    Ok(args) => args,
                    Err(e) => {
                        let message = format!("{id}: its arguments are not JSON: {e}");
                        return Err(fault_at(line, ErrorKind::Action, message));
                    }
                };
                trace
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .ran
                    .push(id.clone());
                let bytes = args.map(|a| a.to_string().into_bytes()).unwrap_or_default();
                // On the app's runtime, not this thread: an action that
                // needs one (a timer, a socket) has it.
                let call = {
                    let id = id.clone();
                    async move { list.run_json_async_with(&who, &id, &bytes).await }
                };
                let out = dispatch
                    .call(call)
                    .await
                    .map_err(|e| fault_at(line, ErrorKind::Action, format!("{id}: {e}")))?
                    .map_err(|e| fault_at(line, ErrorKind::Action, e.to_string()))?;
                let value: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
                to_lua(&lua, &value)
            }
        })
    }
}

/// What a state's workers answer to (its app data): the cancels that
/// stop its run, and — a live script's — the wake that asks its host for
/// a frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct Owner {
    pub(crate) wake: Option<Wake>,
    pub(crate) cancel: Vec<Cancel>,
}

impl Owner {
    pub(crate) fn gone(&self) -> bool {
        self.cancel.iter().any(Cancel::is_set)
    }
}

/// Stops `lua` at the step its `guard` says to.
pub(crate) fn interrupt(lua: &Lua, guard: Arc<Guard>) {
    lua.set_interrupt(move |lua| match guard.step() {
        None => Ok(mlua::VmState::Continue),
        Some(Stop::Steps(limit)) => Err(fault(
            lua,
            ErrorKind::Limit,
            format!("ran past its limit of {limit} steps (a loop that never ends?)"),
        )),
        Some(Stop::Time(allowed)) => Err(fault(
            lua,
            ErrorKind::Limit,
            format!(
                "ran past its time of {} ms: hand heavy work to a worker (worker.spawn)",
                allowed.as_millis()
            ),
        )),
        Some(Stop::Cancelled) => Err(fault(lua, ErrorKind::Limit, "it was stopped".to_owned())),
    });
}

/// A report of a run that never got to run.
fn stopped(why: &str) -> Report {
    Report {
        result: Err(ScriptError::new(ErrorKind::Runtime, None, why)),
        log: Vec::new(),
        ran: Vec::new(),
        undo: None,
    }
}

/// What a live script's thread and its handle share.
#[derive(Debug, Default)]
struct LiveShared {
    running: AtomicBool,
    /// Stopped ([`Live::stop`], or dropped): what it deferred never runs,
    /// and a frame running now stops at its next step.
    stop: Cancel,
    wake: Wake,
    /// The frame it asked for with no input (`wake_after`): ms + 1, 0
    /// for none.
    tick: AtomicU64,
    /// How long its last frame ran on its thread, nanoseconds.
    took: AtomicU64,
}

impl LiveShared {
    fn ask_tick(&self, ms: f64) {
        // Whole milliseconds, far inside u64.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions
        )]
        let ms = if ms.is_finite() && ms > 0.0 {
            ms.min(3.6e6) as u64
        } else {
            0
        };
        let asked = ms.saturating_add(1);
        // The soonest ask wins.
        let _ = self
            .tick
            .try_update(Ordering::AcqRel, Ordering::Acquire, |now| {
                (now == 0 || asked < now).then_some(asked)
            });
    }
}

/// What a [`Live`] asks its thread.
enum Ask {
    Frame(futures::channel::oneshot::Sender<Report>),
}

/// A live script ([`Runtime::start`]): one Luau state its frames share,
/// on a thread of its own. Dropping it stops the script and ends the
/// thread; its workers are cancelled.
pub struct Live {
    frames: std::sync::mpsc::Sender<Ask>,
    shared: Arc<LiveShared>,
    step: String,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("step", &self.step)
            .field("running", &self.running())
            .finish_non_exhaustive()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.shared.stop.set();
    }
}

impl Live {
    /// Whether it has something to run on the next frame: false once it
    /// stops deferring, or fails, or is stopped.
    #[must_use]
    pub fn running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire) && !self.shared.stop.is_set()
    }

    /// Runs what it deferred, in order, as one step (named as its first
    /// run was): each `defer` they call runs on the frame after. Its
    /// limits start again each frame; a frame past [`Limits::frame`]
    /// stops it. One that fails stops the script.
    ///
    /// The frame runs on the script's thread; this awaits it.
    pub async fn frame(&self) -> Report {
        let (reply, answer) = futures::channel::oneshot::channel();
        if self.frames.send(Ask::Frame(reply)).is_err() {
            return stopped("its thread stopped");
        }
        answer
            .await
            .unwrap_or_else(|_| stopped("its thread stopped"))
    }

    /// Stops it: what it deferred never runs, and a frame running now
    /// stops at its next step.
    pub fn stop(&self) {
        self.shared.stop.set();
        self.shared.running.store(false, Ordering::Release);
    }

    /// How long its last frame ran on its own thread: the script's time
    /// alone, not its host's wait to hear of it.
    #[must_use]
    pub fn last_frame_time(&self) -> Duration {
        Duration::from_nanos(self.shared.took.load(Ordering::Acquire))
    }

    /// Resolves when it asks for a frame with no input to give it: one of
    /// its workers posted news. Its host runs one then.
    pub async fn woken(&self) {
        self.shared.wake.woken().await;
    }

    /// The frame it asked for with no input (`wake_after(ms)`): how soon,
    /// taking the ask. Its host runs one then (an animation's next step).
    #[must_use]
    pub fn wake_after(&self) -> Option<Duration> {
        let asked = self.shared.tick.swap(0, Ordering::AcqRel);
        (asked > 0).then(|| Duration::from_millis(asked.saturating_sub(1)))
    }
}

/// A live script's thread: its first run, then a frame for each ask,
/// until its [`Live`] is dropped.
#[allow(clippy::too_many_arguments)]
async fn live_thread(
    runtime: Runtime,
    caller: ActionCaller,
    source: String,
    args: Value,
    step: String,
    shared: Arc<LiveShared>,
    ready: futures::channel::oneshot::Sender<Report>,
    asks: std::sync::mpsc::Receiver<Ask>,
) {
    let trace = Arc::new(Mutex::new(Trace::default()));
    let guard = Arc::new(Guard::new(runtime.limits.steps, vec![shared.stop.clone()]));
    let owner = Owner {
        wake: Some(shared.wake.clone()),
        cancel: vec![shared.stop.clone()],
    };
    let lua = match runtime.state(&caller, &args, &trace, &guard, owner, Some(&shared)) {
        Ok(lua) => lua,
        Err(e) => {
            let _ = ready.send(report((Err(script_error(&e)), None), &trace));
            return;
        }
    };
    let main = load(&lua, &source);
    let result = runtime
        .step(&step, async { execute(&lua, main?).await })
        .await;
    let started = result.0.is_ok();
    let vm = LiveVm {
        lua,
        trace,
        guard,
        runtime,
        step,
        shared,
    };
    vm.note_running();
    if ready.send(report(result, &vm.trace)).is_err() || !started {
        return;
    }
    while let Ok(ask) = asks.recv() {
        match ask {
            Ask::Frame(reply) => {
                let report = vm.frame().await;
                let _ = reply.send(report);
            }
        }
    }
}

/// A live script's state, on its thread.
struct LiveVm {
    lua: Lua,
    trace: Arc<Mutex<Trace>>,
    guard: Arc<Guard>,
    runtime: Runtime,
    step: String,
    shared: Arc<LiveShared>,
}

impl LiveVm {
    fn note_running(&self) {
        let due = self
            .lua
            .named_registry_value::<mlua::Table>(DEFERRED)
            .is_ok_and(|t| t.raw_len() > 0);
        self.shared
            .running
            .store(due && !self.shared.stop.is_set(), Ordering::Release);
    }

    async fn frame(&self) -> Report {
        if self.shared.stop.is_set() {
            self.clear();
            self.note_running();
            return report((Ok(Value::Null), None), &self.trace);
        }
        let started = std::time::Instant::now();
        self.guard.reset();
        self.guard.deadline(Some(self.runtime.limits.frame));
        let result = self
            .runtime
            .step(&self.step, async {
                for module in &self.runtime.modules {
                    module.frame(&self.lua).map_err(|e| script_error(&e))?;
                }
                let due = self.take().map_err(|e| script_error(&e))?;
                for f in due {
                    f.call_async::<()>(()).await.map_err(|e| script_error(&e))?;
                }
                Ok(Value::Null)
            })
            .await;
        self.guard.deadline(None);
        let took = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.shared.took.store(took, Ordering::Release);
        if result.0.is_err() {
            self.clear();
        }
        self.note_running();
        report(result, &self.trace)
    }

    /// What it deferred never runs.
    fn clear(&self) {
        if let Ok(fresh) = self.lua.create_table() {
            self.lua.set_named_registry_value(DEFERRED, fresh).ok();
        }
    }

    /// What is due this frame, leaving an empty list for the next.
    fn take(&self) -> mlua::Result<Vec<mlua::Function>> {
        let due: mlua::Table = self.lua.named_registry_value(DEFERRED)?;
        self.lua
            .set_named_registry_value(DEFERRED, self.lua.create_table()?)?;
        due.sequence_values().collect()
    }
}

pub(crate) fn load(lua: &Lua, source: &str) -> Result<mlua::Function, ScriptError> {
    lua.load(source)
        .set_name(format!("={CHUNK}"))
        .into_function()
        .map_err(|e| script_error(&e))
}

async fn execute(lua: &Lua, main: mlua::Function) -> Result<Value, ScriptError> {
    let out: MultiValue = main.call_async(()).await.map_err(|e| script_error(&e))?;
    let first = out.into_iter().next().unwrap_or(LuaValue::Nil);
    from_lua(lua, first).map_err(|message| ScriptError::new(ErrorKind::Runtime, None, message))
}

/// A run's report: its result and undo step, and what it logged and ran
/// since the last.
fn report(
    (result, undo): (Result<Value, ScriptError>, Option<String>),
    trace: &Arc<Mutex<Trace>>,
) -> Report {
    let trace = std::mem::take(&mut *trace.lock().unwrap_or_else(PoisonError::into_inner));
    Report {
        result,
        log: trace.log,
        ran: trace.ran,
        undo,
    }
}

/// What a run printed and ran, gathered as it goes.
#[derive(Debug, Default)]
struct Trace {
    log: Vec<String>,
    ran: Vec<String>,
}

fn log_function(lua: &Lua, trace: &Arc<Mutex<Trace>>) -> mlua::Result<mlua::Function> {
    let trace = Arc::clone(trace);
    lua.create_function(move |_, values: MultiValue| {
        let line: Vec<String> = values
            .iter()
            .map(|v| v.to_string().unwrap_or_else(|_| v.type_name().to_owned()))
            .collect();
        trace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .log
            .push(line.join("\t"));
        Ok(())
    })
}

/// Why a run will not run `id`: its gate keeps it out, or — for
/// `state.query` — it changes the app.
fn refusal(list: &ActionList, id: &str, read_only: bool, gate: Option<&Gate>) -> Option<String> {
    let info = list.info(id)?;
    if let Some(Err(why)) = gate.map(|g| g(&info)) {
        return Some(format!("{id}: {why}"));
    }
    if !read_only {
        return None;
    }
    (info.effect != Effect::ReadOnly).then(|| {
        format!(
            "{id} changes the app ({}): run it with actions.run",
            effect_name(info.effect)
        )
    })
}

/// The actions `who` may run, and the gate lets through.
fn visible(list: &ActionList, who: &ActionCaller, gate: Option<&Gate>) -> Vec<ActionInfo> {
    let mut infos = who
        .principal()
        .map_or_else(|| list.infos(), |p| list.infos_for(p));
    if let Some(gate) = gate {
        infos.retain(|i| gate(i).is_ok());
    }
    infos
}

fn matches(info: &ActionInfo, filter: &str) -> bool {
    [
        &info.id,
        &info.display_name,
        &info.description,
        &info.category,
        &info.group,
    ]
    .iter()
    .any(|field| field.to_lowercase().contains(filter))
}

/// One action, as `actions.list` gives it.
pub(crate) fn summary(info: &ActionInfo) -> Value {
    json!({
        "id": info.id,
        "name": info.display_name,
        "description": info.description,
        "category": info.category,
        "group": info.group,
        "effect": effect_name(info.effect),
        "enabled": info.enabled,
        "disabled_reason": info.disabled_reason,
    })
}

/// One action with what it takes and gives, as `actions.describe` gives it.
pub(crate) fn detail(info: &ActionInfo) -> Value {
    let schema = |s: &Option<String>| {
        s.as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
    };
    let mut out = summary(info);
    if let Value::Object(map) = &mut out {
        map.insert("params".into(), schema(&info.params_schema).into());
        map.insert("result".into(), schema(&info.result_schema).into());
    }
    out
}

/// JSON into Luau: `null` is `nil`, an array a sequence.
pub(crate) fn to_lua(lua: &Lua, value: &Value) -> mlua::Result<LuaValue> {
    let options = mlua::serde::SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);
    lua.to_value_with(value, options)
}

fn from_lua(lua: &Lua, value: LuaValue) -> Result<Value, String> {
    lua.from_value::<Value>(value)
        .map_err(|e| format!("what it returned is not JSON: {e}"))
}

/// A failure a script's globals raise, with where the script was.
#[derive(Debug)]
struct Fault(ScriptError);

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Fault {}

fn fault_at(line: Option<u32>, kind: ErrorKind, message: String) -> mlua::Error {
    mlua::Error::external(Fault(ScriptError::new(kind, line, message)))
}

fn fault(lua: &Lua, kind: ErrorKind, message: String) -> mlua::Error {
    fault_at(line_of(lua), kind, message)
}

/// The script's line now: the innermost frame that is the script's (a
/// global's own frame is not).
fn line_of(lua: &Lua) -> Option<u32> {
    (0..8).find_map(|level| {
        lua.inspect_stack(level, |debug| {
            let ours = debug.source().short_src.as_deref() == Some(CHUNK);
            ours.then(|| debug.current_line()).flatten()
        })
        .flatten()
        .and_then(|line| u32::try_from(line).ok())
    })
}

/// What stopped a script, with its line, from what Luau raised.
pub(crate) fn script_error(error: &mlua::Error) -> ScriptError {
    if let Some(Fault(found)) = fault_in(error) {
        return found.clone();
    }
    match error {
        mlua::Error::SyntaxError { message, .. } => located(ErrorKind::Syntax, message),
        mlua::Error::RuntimeError(message) => located(ErrorKind::Runtime, message),
        mlua::Error::MemoryError(_) => {
            ScriptError::new(ErrorKind::Limit, None, "ran out of memory")
        }
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
            script_error(cause)
        }
        other => located(ErrorKind::Runtime, &other.to_string()),
    }
}

fn fault_in(error: &mlua::Error) -> Option<&Fault> {
    match error {
        mlua::Error::ExternalError(inner) => inner.downcast_ref::<Fault>(),
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => {
            fault_in(cause)
        }
        _ => None,
    }
}

/// `script:3: attempt to …` as line 3 and its message; the traceback
/// mlua adds after the first line is left out.
fn located(kind: ErrorKind, message: &str) -> ScriptError {
    let first = message.lines().next().unwrap_or_default();
    let mut rest = first;
    // A nested error repeats the prefix (`script:3: script:5: …`): the
    // innermost line is where it happened.
    let mut line = None;
    while let Some((at, tail)) = rest
        .strip_prefix(CHUNK)
        .and_then(|r| r.strip_prefix(':'))
        .and_then(|r| r.split_once(':'))
        .and_then(|(n, tail)| n.parse::<u32>().ok().map(|n| (n, tail)))
    {
        line = Some(at);
        rest = tail.trim_start();
    }
    ScriptError::new(kind, line, rest)
}

#[cfg(test)]
mod tests {
    use super::located;
    use crate::ErrorKind;

    #[test]
    fn a_message_gives_up_its_line() {
        let e = located(
            ErrorKind::Runtime,
            "script:3: attempt to add\nstack traceback:\n…",
        );
        assert_eq!(e.line, Some(3));
        assert_eq!(e.message, "attempt to add");
        let e = located(ErrorKind::Runtime, "boom");
        assert_eq!((e.line, e.message.as_str()), (None, "boom"));
    }
}
