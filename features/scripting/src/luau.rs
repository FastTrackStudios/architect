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
//! `architect-canvas`'s `ImGui` and `Canvas`, for one.
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionInfo, ActionList, Effect};
use mlua::{Lua, LuaSerdeExt, MultiValue, Value as LuaValue};
use serde_json::{Value, json};

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
    /// turn of a loop. What stops `while true do end`.
    pub steps: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory: 64 * 1024 * 1024,
            steps: 10_000_000,
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
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("list", &self.list)
            .field("transactions", &self.transactions.is_some())
            .field("limits", &self.limits)
            .field("gate", &self.gate.is_some())
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
        }
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
    pub async fn run(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> Report {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let steps = Arc::new(AtomicU64::new(0));
        let result = match self.state(caller, &args, &trace, &steps, false) {
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
    pub async fn start(
        &self,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> (Report, Option<Live>) {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let steps = Arc::new(AtomicU64::new(0));
        let lua = match self.state(caller, &args, &trace, &steps, true) {
            Ok(lua) => lua,
            Err(e) => return (report((Err(script_error(&e)), None), &trace), None),
        };
        let main = load(&lua, source);
        let result = self.step(step, async { execute(&lua, main?).await }).await;
        let started = result.0.is_ok();
        let report = report(result, &trace);
        let live = started.then(|| Live {
            lua,
            trace,
            steps,
            runtime: self.clone(),
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
    /// too when it is `live`.
    fn state(
        &self,
        caller: &ActionCaller,
        args: &Value,
        trace: &Arc<Mutex<Trace>>,
        steps: &Arc<AtomicU64>,
        live: bool,
    ) -> mlua::Result<Lua> {
        let lua = Lua::new();
        lua.set_memory_limit(self.limits.memory)?;
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
        if live {
            lua.set_named_registry_value(DEFERRED, lua.create_table()?)?;
            globals.set(
                "defer",
                lua.create_function(|lua, f: mlua::Function| {
                    let deferred: mlua::Table = lua.named_registry_value(DEFERRED)?;
                    deferred.push(f)
                })?,
            )?;
        }
        // From here the globals above are read-only, and what a script
        // sets lives in its own table.
        lua.sandbox(true)?;
        let steps = Arc::clone(steps);
        let limit = self.limits.steps;
        lua.set_interrupt(move |lua| {
            if steps.fetch_add(1, Ordering::Relaxed) >= limit {
                return Err(fault(
                    lua,
                    ErrorKind::Limit,
                    format!("ran past its limit of {limit} steps (a loop that never ends?)"),
                ));
            }
            Ok(mlua::VmState::Continue)
        });
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
        lua.create_async_function(move |lua, (id, args): (String, Option<LuaValue>)| {
            // Where the script is, taken now: once the future runs, the
            // script is no longer on that line's frame.
            let line = line_of(&lua);
            let refused = refusal(&list, &id, read_only, gate.as_ref());
            let args = args.map(|a| lua.from_value::<Value>(a));
            let (list, who, trace) = (list.clone(), who.clone(), Arc::clone(&trace));
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
                let out = list
                    .run_json_async_with(&who, &id, &bytes)
                    .await
                    .map_err(|e| fault_at(line, ErrorKind::Action, e.to_string()))?;
                let value: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
                to_lua(&lua, &value)
            }
        })
    }
}

/// A live script ([`Runtime::start`]): one Luau state its frames share.
pub struct Live {
    lua: Lua,
    trace: Arc<Mutex<Trace>>,
    steps: Arc<AtomicU64>,
    runtime: Runtime,
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

impl Live {
    /// Whether it has something to run on the next frame: false once it
    /// stops deferring, or fails, or is stopped.
    #[must_use]
    pub fn running(&self) -> bool {
        self.lua
            .named_registry_value::<mlua::Table>(DEFERRED)
            .is_ok_and(|t| t.raw_len() > 0)
    }

    /// Runs what it deferred, in order, as one step (named as its first
    /// run was): each `defer` they call runs on the frame after. Its
    /// limits start again each frame. One that fails stops the script.
    pub async fn frame(&self) -> Report {
        self.steps.store(0, Ordering::Relaxed);
        let result = self
            .runtime
            .step(&self.step, async {
                let due = self.take().map_err(|e| script_error(&e))?;
                for f in due {
                    f.call_async::<()>(()).await.map_err(|e| script_error(&e))?;
                }
                Ok(Value::Null)
            })
            .await;
        if result.0.is_err() {
            self.stop();
        }
        report(result, &self.trace)
    }

    /// Stops it: what it deferred never runs.
    pub fn stop(&self) {
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

fn load(lua: &Lua, source: &str) -> Result<mlua::Function, ScriptError> {
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
fn to_lua(lua: &Lua, value: &Value) -> mlua::Result<LuaValue> {
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
fn script_error(error: &mlua::Error) -> ScriptError {
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
