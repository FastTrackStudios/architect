//! The host (feature `host`): puts loaded extensions' actions on the
//! app's Action List and their settings sections on its settings page,
//! and takes them off again — on unload, or as a diff on a reload.
//!
//! Each kind runs through a [`Runner`]: [`DataRunner`] (no code: presets
//! only) and [`LuauRunner`] (scripts, on `architect-scripting`'s runtime)
//! are built in; WASM components and processes over vox plug in as
//! runners of their own ([`Host::with_runner`]). What a reload does, the
//! same for every kind (`docs/content/architecture/extensions.md`, "Hot
//! reload"):
//!
//! 1. the old instance's state is asked for ([`Instance::save_state`]);
//! 2. the new one is started with it, and everything it adds is built —
//!    if that fails, the last good one stays loaded and nothing changes;
//! 3. its actions go on as a diff: one still there keeps its id (so its
//!    bindings and its place on the ribbon), one gone is taken off, a new
//!    one added; its settings sections are replaced, their values kept;
//! 4. an [`Event`] says so, with how long it took, or why it failed.
//!
//! With the app's theme store ([`Host::with_themes`]), an extension's
//! `[[themes]]` files and `[tokens]` go into it as it loads, as a diff
//! on a reload, and out as it unloads ([`theme_contribution`]). A theme
//! file that does not read is a theme problem (`THEME_PROBLEMS`), never a
//! failed load.
//!
//! What an extension adds to the interface ([`Contributions`]: its
//! openers, tabs, status items, menu entries, hooks, slots…) goes into
//! the host's registry ([`Host::contributions`]) as it loads, as a diff
//! on a reload, and out with its runtime state as it unloads. Its hooks
//! run on the events the host hears ([`Host::emit`], [`Host::watch`]):
//! `startup` once as it first loads, the app's `changed:<topic>` and the
//! names they map to (`workspace.changed`, `theme.changed`, …).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, CallerAsyncHandler, Effect,
    EventKind, Invocation, InvocationSource,
};
use architect_scripting::Transactions;
use architect_scripting::luau::{Gate, Limits, Runtime};
use architect_scripting::worker::{WorkerGroup, Workers};
use architect_settings::{Owner, Registry, Section};
use architect_theme::{Contribution, Problem, Theme, ThemeStore};
use architect_workspace::{KindError, ViewKinds};
use semver::Version;
use serde_json::Value;

use crate::catalog::Origin;
use crate::contrib::{Contributions, HookDecl};
use crate::manifest::{ActionDecl, Kind, Manifest, Reach};
use crate::package::{Package, PackageError};

/// Where a host puts extensions' settings sections: the app's settings
/// registry, directly ([`SharedSections`]) or through the UI's
/// (`ui` feature: `use_extension_sections`).
pub trait SectionSink: Send + Sync {
    /// Puts `owner`'s sections in place of the ones it had.
    ///
    /// # Errors
    ///
    /// A section's id or a setting's key belongs to someone else.
    fn put(&self, owner: &str, sections: Vec<Section>) -> Result<(), String>;

    /// Takes every section of `owner` away.
    fn withdraw(&self, owner: &str);
}

/// A settings registry shared with the host.
#[derive(Clone, Debug, Default)]
pub struct SharedSections(pub Arc<Mutex<Registry>>);

impl SharedSections {
    #[must_use]
    pub fn new(registry: Registry) -> Self {
        Self(Arc::new(Mutex::new(registry)))
    }

    /// The registry now.
    #[must_use]
    pub fn registry(&self) -> Registry {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SectionSink for SharedSections {
    fn put(&self, owner: &str, sections: Vec<Section>) -> Result<(), String> {
        let mut registry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *registry = with_sections(&registry, owner, sections)?;
        drop(registry);
        Ok(())
    }

    fn withdraw(&self, owner: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove_owner(&Owner::Extension(owner.to_owned()));
    }
}

/// `registry` with `owner`'s sections replaced by `sections`, or why not:
/// all of them or none.
///
/// # Errors
///
/// A section's id or a setting's key belongs to someone else.
pub fn with_sections(
    registry: &Registry,
    owner: &str,
    sections: Vec<Section>,
) -> Result<Registry, String> {
    let mut next = registry.clone();
    next.remove_owner(&Owner::Extension(owner.to_owned()));
    for section in sections {
        next.register(section).map_err(|e| format!("{e:?}"))?;
    }
    Ok(next)
}

/// What a runner is given to start an instance.
pub struct Context<'a> {
    /// The app's Action List: what its code runs against.
    pub list: &'a ActionList,
    /// What it may reach, from its manifest's capabilities.
    pub gate: Gate,
    /// What the instance it replaces saved ([`Instance::save_state`]).
    pub state: Option<Vec<u8>>,
    /// The app's contributions: where what it adds at run time goes (as
    /// the extension `owner`, checked against its manifest).
    pub contributions: &'a Contributions,
    /// Its session with the host service (`crate::hostapi`): what its
    /// code asks the app for, checked against what it may reach.
    pub api: crate::hostapi::ExtensionApi,
    /// Where its log lines go (`EXT_LOGS`): a process's output, a script's
    /// prints and errors.
    pub logs: crate::logs::Logs,
}

/// Runs one kind of extension.
pub trait Runner: Send + Sync {
    fn kind(&self) -> Kind;

    /// Starts `package`: checks what it can before anything is swapped in.
    ///
    /// # Errors
    ///
    /// It cannot start (a script that is not Luau, a component that does
    /// not instantiate, a process that does not say hello).
    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String>;
}

/// One running extension.
pub trait Instance: Send + Sync {
    /// The handler of its action `decl` (one with no preset).
    ///
    /// # Errors
    ///
    /// It does not do that action.
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String>;

    /// Its state, for the instance that replaces it on a reload.
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// What its code adds to the interface past its manifest's tables (an
    /// in-process extension's builder's).
    fn declared(&self) -> Option<crate::contrib::Contribution> {
        None
    }

    /// Runs its hook `decl`'s script (a `luau` extension's), as `caller`
    /// with the event as `args`: `None` when it runs no scripts.
    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Value) -> Option<ActionFuture> {
        let _ = (decl, caller, args);
        None
    }

    /// It is being unloaded or replaced.
    fn stop(&self) {}
}

/// Data extensions: no code, so every action is a preset.
#[derive(Clone, Copy, Debug, Default)]
pub struct DataRunner;

struct NoCode;

impl Instance for NoCode {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        Err(format!(
            "{} has no preset, and a data extension no code",
            decl.id
        ))
    }
}

impl Runner for DataRunner {
    fn kind(&self) -> Kind {
        Kind::Data
    }

    fn start(&self, _: &Package, _: Context<'_>) -> Result<Box<dyn Instance>, String> {
        Ok(Box::new(NoCode))
    }
}

/// Luau extensions.
///
/// Each action is a script of the package, run on
/// `architect-scripting`'s runtime as whoever ran the action, inside the
/// extension's capabilities, as one undo step. Scripts are read when the
/// extension starts; one running when it reloads finishes on the old code.
#[derive(Clone, Default)]
pub struct LuauRunner {
    transactions: Option<Arc<dyn Transactions>>,
    limits: Limits,
}

impl std::fmt::Debug for LuauRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuauRunner")
            .field("transactions", &self.transactions.is_some())
            .field("limits", &self.limits)
            .finish()
    }
}

impl LuauRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Each run one step of `transactions` (the document's undo).
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
}

struct Luau {
    runtime: Runtime,
    /// Its workers, cancelled when it stops (unloaded or replaced).
    workers: WorkerGroup,
    /// Each script action's source.
    scripts: BTreeMap<String, Arc<str>>,
    /// Each hook script's source, by its path.
    hooks: BTreeMap<String, Arc<str>>,
    /// The undo step's prefix: the extension's name.
    name: String,
    /// Its session with the host service: `host` in its scripts, and who
    /// their actions run as.
    api: crate::hostapi::ExtensionApi,
    logs: crate::logs::Logs,
}

impl Luau {
    /// The runtime for one run as `caller`: `ctx` says where it came from,
    /// `host` acts for whoever ran it; and the caller it runs as — the
    /// extension, for them ([`Principal::Extension`]).
    fn for_run(&self, caller: &ActionCaller) -> (Runtime, ActionCaller) {
        let api = self.api.clone().acting_for(caller.principal().cloned());
        let caller = caller.clone().with_principal(api.principal());
        let runtime = self
            .runtime
            .clone()
            .with_module(Arc::new(luau_ext::Ctx::of(caller.invocation())))
            .with_module(Arc::new(crate::luau_host::HostModule::new(api)));
        (runtime, caller)
    }
}

/// A script run's prints, and the error it stopped with, into its log.
fn log_report(
    logs: &crate::logs::Logs,
    id: &str,
    what: &str,
    report: &architect_scripting::luau::Report,
) {
    for line in &report.log {
        logs.push(id, crate::logs::Level::Info, "print", line.clone());
    }
    if let Err(e) = &report.result {
        logs.push(
            id,
            crate::logs::Level::Error,
            "script",
            format!("{what}: {e}"),
        );
    }
}

impl Runner for LuauRunner {
    fn kind(&self) -> Kind {
        Kind::Luau
    }

    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let mut scripts = BTreeMap::new();
        for decl in &package.manifest.actions {
            let Some(path) = &decl.script else { continue };
            let source = package.text(path).map_err(|e| e.to_string())?;
            Runtime::check(&source).map_err(|e| format!("{path}: {e}"))?;
            scripts.insert(decl.id.clone(), Arc::from(source));
        }
        let mut hooks = BTreeMap::new();
        for decl in &package.manifest.contributes.hooks {
            let Some(path) = &decl.script else { continue };
            let source = package.text(path).map_err(|e| e.to_string())?;
            Runtime::check(&source).map_err(|e| format!("{path}: {e}"))?;
            hooks.insert(path.clone(), Arc::from(source));
        }
        // Its `[[workers]]`, spawned by its scripts off their threads.
        let group = WorkerGroup::new();
        let mut workers = Workers::new().with_group(group.clone());
        for decl in &package.manifest.workers {
            let Some(path) = &decl.script else { continue };
            let source = package.text(path).map_err(|e| e.to_string())?;
            workers = workers.with_source(decl.id.clone(), source);
        }
        workers
            .check()
            .map_err(|(name, e)| format!("worker {name}: {e}"))?;
        let mut runtime = Runtime::new(cx.list.clone())
            .with_gate(cx.gate)
            .with_limits(self.limits)
            .with_workers(workers)
            .with_module(Arc::new(luau_ext::Ext::new(
                &package.manifest,
                cx.contributions.clone(),
            )));
        if let Some(t) = &self.transactions {
            runtime = runtime.with_transactions(Arc::clone(t));
        }
        // Its scripts' action calls run on the runtime it was loaded on,
        // even when a hook fires from a thread with none (the UI's).
        runtime = runtime.with_dispatch(architect_scripting::vm::Dispatch::here());
        Ok(Box::new(Luau {
            runtime,
            workers: group,
            scripts,
            hooks,
            name: package.manifest.name.clone(),
            api: cx.api,
            logs: cx.logs,
        }))
    }
}

impl Instance for Luau {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let source = self
            .scripts
            .get(&decl.id)
            .cloned()
            .ok_or_else(|| format!("{} has no script", decl.id))?;
        let me = Self {
            runtime: self.runtime.clone(),
            workers: self.workers.clone(),
            scripts: BTreeMap::new(),
            hooks: BTreeMap::new(),
            name: self.name.clone(),
            api: self.api.clone(),
            logs: self.logs.clone(),
        };
        let id = decl.id.clone();
        let step = format!("{}: {}", self.name, decl.name);
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args = json_args(&id, args)?;
            // Where it was run from, as `ctx`; as the extension, for its
            // caller.
            let (runtime, caller) = me.for_run(caller);
            let (source, id, step) = (Arc::clone(&source), id.clone(), step.clone());
            let (logs, owner) = (me.logs.clone(), me.api.id().to_owned());
            let run: ActionFuture = Box::pin(async move {
                let report = runtime.run(&caller, &source, args, &step).await;
                log_report(&logs, &owner, &id, &report);
                match report.result {
                    Ok(value) => Ok(value.to_string().into_bytes()),
                    Err(e) => Err(ActionError::Failed {
                        id,
                        message: e.to_string(),
                    }),
                }
            });
            Ok(run)
        }))
    }

    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Value) -> Option<ActionFuture> {
        let path = decl.script.as_ref()?;
        let file = self.hooks.get(path)?;
        // The whole file, or one function of the table it returns.
        let source: String = decl.function.as_ref().map_or_else(
            || file.to_string(),
            |f| {
                format!(
                    "local __hooks = (function(...) {file}\nend)(...)\nreturn __hooks[{f:?}](args)"
                )
            },
        );
        let (runtime, caller) = self.for_run(caller);
        let id = format!("{} hook {}", self.name, decl.on);
        let step = id.clone();
        let (logs, owner) = (self.logs.clone(), self.api.id().to_owned());
        Some(Box::pin(async move {
            let report = runtime.run(&caller, &source, args, &step).await;
            log_report(&logs, &owner, &step, &report);
            report
                .result
                .map(|v| v.to_string().into_bytes())
                .map_err(|e| ActionError::Failed {
                    id,
                    message: e.to_string(),
                })
        }))
    }

    fn stop(&self) {
        let cancelled = self.workers.cancel_all();
        if cancelled > 0 {
            tracing::debug!(workers = cancelled, "an extension's workers were cancelled");
        }
    }
}

/// What a Luau extension reaches past the Action List: `ext` (its id,
/// and what it adds at run time) and `ctx` (where the run came from).
mod luau_ext {
    use architect::action::Invocation;
    use architect_scripting::luau::mlua::{self, Lua, LuaSerdeExt as _, Value as LuaValue};
    use architect_scripting::luau::{Module, raise};

    use crate::contrib::{Contribution, Contributions, StateWrite};
    use crate::manifest::Manifest;

    /// `ext`: `id`; `contribute(table)` puts what it adds at run time (in
    /// place of what it added before; checked as its manifest's tables
    /// are); `state{key, text, kind?, icon?, target?, seq?, ttl_ms?}`
    /// writes its runtime state; `clear(key?)`.
    pub(super) struct Ext {
        manifest: Manifest,
        contributions: Contributions,
    }

    impl Ext {
        pub(super) fn new(manifest: &Manifest, contributions: Contributions) -> Self {
            Self {
                manifest: manifest.clone(),
                contributions,
            }
        }
    }

    impl Module for Ext {
        fn global(&self) -> &'static str {
            "ext"
        }

        fn path(&self) -> &'static str {
            "ext"
        }

        fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
            let t = lua.create_table()?;
            t.set("id", self.manifest.id.clone())?;
            let (m, c) = (self.manifest.clone(), self.contributions.clone());
            t.set(
                "contribute",
                lua.create_function(move |lua, value: LuaValue| {
                    let added: Contribution = lua.from_value(value)?;
                    let views = m.view_ids();
                    added
                        .check(&m.rules(&views))
                        .map_err(|e| raise(lua, e.to_string()))?;
                    let diff = c
                        .put_runtime(&m.id, added)
                        .ok_or_else(|| raise(lua, format!("{} is not loaded", m.id)))?;
                    lua.to_value(&diff)
                })?,
            )?;
            let (id, c) = (self.manifest.id.clone(), self.contributions.clone());
            t.set(
                "state",
                lua.create_function(move |lua, value: LuaValue| {
                    let mut write: StateWrite = lua.from_value(value)?;
                    write.source = format!("ext:{id}");
                    let applied = c
                        .set_state(write, super::now_ms())
                        .map_err(|e| raise(lua, e.to_string()))?;
                    lua.to_value(&applied)
                })?,
            )?;
            let (id, c) = (self.manifest.id.clone(), self.contributions.clone());
            t.set(
                "clear",
                lua.create_function(move |_, key: Option<String>| {
                    Ok(c.clear_state(&format!("ext:{id}"), key.as_deref()))
                })?,
            )?;
            Ok(LuaValue::Table(t))
        }
    }

    /// `ctx`: where this run came from (`ctx.source`, `ctx.target`,
    /// `ctx.pane`, `ctx.url`, `ctx.event`, …), `{source = "app"}` when
    /// the door said nothing.
    pub(super) struct Ctx(serde_json::Value);

    impl Ctx {
        pub(super) fn of(invocation: Option<&Invocation>) -> Self {
            Self(super::invocation_json(invocation))
        }
    }

    impl Module for Ctx {
        fn global(&self) -> &'static str {
            "ctx"
        }

        fn path(&self) -> &'static str {
            "ctx"
        }

        fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
            lua.to_value(&self.0)
        }
    }
}

/// The time now, ms since the epoch: the clock runtime state is kept on.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// An invocation as JSON (what `ctx` is, and what a process is sent):
/// `{"source": "app"}` for none.
#[must_use]
pub fn invocation_json(invocation: Option<&Invocation>) -> Value {
    let Some(i) = invocation else {
        return serde_json::json!({ "source": InvocationSource::App.name() });
    };
    let mut out = serde_json::Map::new();
    out.insert("source".into(), Value::from(i.source.name()));
    for (name, field) in [
        ("target", &i.target),
        ("id", &i.id),
        ("window", &i.window),
        ("pane", &i.pane),
        ("view", &i.view),
        ("selection", &i.selection),
        ("url", &i.url),
        ("path", &i.path),
        ("event", &i.event),
        ("detail", &i.detail),
    ] {
        if let Some(v) = field {
            out.insert(name.into(), Value::from(v.as_str()));
        }
    }
    Value::Object(out)
}

/// An action's arguments as JSON: nothing is `null`.
fn json_args(id: &str, args: &[u8]) -> Result<Value, ActionError> {
    if args.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
        id: id.to_owned(),
        message: e.to_string(),
        expected: None,
    })
}

/// What an extension may reach: its own actions, the namespaces it named,
/// up to the effect it named.
#[must_use]
pub fn gate(manifest: &Manifest) -> Gate {
    let own = manifest.action_prefix();
    let named = manifest.capabilities.actions.clone();
    let most = manifest.capabilities.effect;
    Arc::new(move |info: &ActionInfo| {
        if info.id.starts_with(&own) {
            return Ok(());
        }
        if !named
            .iter()
            .any(|p| p == "*" || info.id.starts_with(p.as_str()))
        {
            return Err("not in a namespace this extension may run".to_owned());
        }
        if reach(info.effect) > most {
            return Err(format!(
                "this extension may run {} actions at most",
                most.name()
            ));
        }
        Ok(())
    })
}

const fn reach(effect: Effect) -> Reach {
    match effect {
        Effect::ReadOnly => Reach::ReadOnly,
        Effect::Mutating => Reach::Mutating,
        Effect::Destructive => Reach::Destructive,
    }
}

const fn effect(reach: Reach) -> Effect {
    match reach {
        Reach::ReadOnly => Effect::ReadOnly,
        Reach::Mutating => Effect::Mutating,
        Reach::Destructive => Effect::Destructive,
    }
}

/// Why an extension was not loaded (the last good one, if any, stays).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    Package(PackageError),
    /// No runner for its kind in this app (a process on a phone).
    NoRunner(Kind),
    /// Its runner could not start it.
    Start(String),
    /// One of its actions could not be made.
    Action {
        id: String,
        why: String,
    },
    /// One of its action ids is another's.
    ActionTaken(String),
    /// One of its settings sections clashes with another's.
    Sections(String),
    /// One of its views is of a kind this app has no provider for.
    ViewKind {
        view: String,
        why: KindError,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Package(e) => e.fmt(f),
            Self::NoRunner(kind) => write!(f, "this app does not run {} extensions", kind.name()),
            Self::Start(why) => write!(f, "it did not start: {why}"),
            Self::Action { id, why } => write!(f, "{id}: {why}"),
            Self::ActionTaken(id) => write!(f, "{id} is another's action"),
            Self::Sections(why) => write!(f, "its settings: {why}"),
            Self::ViewKind { view, why } => write!(f, "its view {view}: {why}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<PackageError> for LoadError {
    fn from(e: PackageError) -> Self {
        Self::Package(e)
    }
}

/// What a load or reload changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub origin: Origin,
    /// Actions it added, took away, and kept (by id).
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
    /// Whether this replaced a loaded copy.
    pub reloaded: bool,
    pub took: Duration,
}

/// What happened to an extension: for a toast, the diagnostics, a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Loaded(Change),
    Unloaded {
        id: String,
    },
    /// It did not load or reload; a loaded copy stays as it was.
    Failed {
        id: String,
        error: LoadError,
    },
}

/// A loaded extension, as the extensions page shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub manifest: Manifest,
    pub origin: Origin,
    /// Its folder (to watch), when it has one.
    pub dir: Option<PathBuf>,
    /// Its actions on the list.
    pub actions: Vec<String>,
}

struct Running {
    loaded: Loaded,
    instance: Box<dyn Instance>,
}

type Listener = Arc<dyn Fn(&Event) + Send + Sync>;

/// What runs a hook's work: the app's executor, or a thread of its own.
pub type Spawn = Arc<dyn Fn(ActionFuture) + Send + Sync>;

/// A view an extension asks the app to open (`EXT_VIEW_OPEN`, an opener
/// with a view): the app's window opens it where it says.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewOpen {
    /// The extension.
    pub extension: String,
    /// Its view's id, and the workspace kind it is (`<extension>/<view>`).
    pub view: String,
    pub kind: String,
    pub title: String,
    /// What it shows (a file's path, a link): the pane's state.
    pub state: Option<String>,
    pub home: crate::manifest::Home,
    pub placement: crate::manifest::ViewPlacement,
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// Where the open came from.
    pub invocation: Invocation,
}

/// What [`Host::open`] opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    /// A file, by its path: claimed by its extension or its document kind.
    Path(String),
    /// A link, claimed by a pattern.
    Url(String),
    /// A document of a kind, and its path when it has one.
    DocKind { kind: String, path: Option<String> },
}

impl Opening {
    fn describe(&self) -> String {
        match self {
            Self::Path(p) => p.clone(),
            Self::Url(u) => u.clone(),
            Self::DocKind { kind, .. } => format!("a {kind} document"),
        }
    }
}

/// What opens a [`ViewOpen`] in the app's window: its answer, or why not.
pub type ViewOpener = Arc<dyn Fn(ViewOpen) -> Result<Value, String> + Send + Sync>;

struct Inner {
    list: ActionList,
    sections: Arc<dyn SectionSink>,
    runners: Mutex<Vec<Arc<dyn Runner>>>,
    /// The view kinds the app provides: what its views may name.
    kinds: Mutex<ViewKinds>,
    page_checker: Mutex<Option<crate::check::PageChecker>>,
    loaded: Mutex<BTreeMap<String, Running>>,
    listeners: Mutex<Vec<Listener>>,
    /// The app's themes, when it gave them: what extensions' themes and
    /// tokens go into.
    themes: Mutex<Option<ThemeStore>>,
    /// Hears the theme change, for `theme.changed` hooks.
    theme_heard: Mutex<Option<architect_theme::Subscription>>,
    /// What loaded extensions add to the interface.
    contributions: Contributions,
    /// Where hooks' work runs.
    spawn: Mutex<Spawn>,
    /// The ids whose `startup` hooks have run: once per extension per
    /// host, not again on a reload.
    started: Mutex<BTreeSet<String>>,
    /// The hooks running now (owner and hook): one does not start again
    /// before it ends.
    running: Arc<Mutex<BTreeSet<String>>>,
    /// What opens extensions' views in the app's window.
    view_opener: Mutex<Option<ViewOpener>>,
    /// The in-process extensions linked into the app, built, by id.
    #[cfg(feature = "linked")]
    linked: Mutex<BTreeMap<String, Arc<crate::linked::Builder>>>,
    /// The host service's side: consent, logs, the app's context, values.
    services: crate::hostapi::Services,
}

/// A [`Host`] held weakly: what an extension's session keeps, so an
/// extension does not keep the app's host alive.
#[derive(Clone)]
pub(crate) struct WeakHost(std::sync::Weak<Inner>);

impl WeakHost {
    pub(crate) fn upgrade(&self) -> Option<Host> {
        self.0.upgrade().map(|inner| Host { inner })
    }
}

/// Loads extensions onto one Action List and one settings registry.
/// Cheap to clone.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("loaded", &self.ids())
            .finish_non_exhaustive()
    }
}

impl PartialEq for Host {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Host {
    /// A host over `list`, putting settings in `sections`, running data
    /// and Luau extensions.
    pub fn new(list: ActionList, sections: impl SectionSink + 'static) -> Self {
        Self {
            inner: Arc::new(Inner {
                list,
                sections: Arc::new(sections),
                runners: Mutex::new(vec![Arc::new(DataRunner), Arc::new(LuauRunner::new())]),
                kinds: Mutex::new(ViewKinds::new()),
                page_checker: Mutex::new(None),
                loaded: Mutex::new(BTreeMap::new()),
                listeners: Mutex::new(Vec::new()),
                themes: Mutex::new(None),
                theme_heard: Mutex::new(None),
                contributions: Contributions::new(),
                spawn: Mutex::new(Arc::new(|work: ActionFuture| {
                    std::thread::spawn(move || {
                        let _ = futures::executor::block_on(work);
                    });
                })),
                started: Mutex::new(BTreeSet::new()),
                running: Arc::default(),
                view_opener: Mutex::new(None),
                #[cfg(feature = "linked")]
                linked: Mutex::new(BTreeMap::new()),
                services: crate::hostapi::Services::default(),
            }),
        }
    }

    pub(crate) fn services(&self) -> &crate::hostapi::Services {
        &self.inner.services
    }

    pub(crate) fn downgrade(&self) -> WeakHost {
        WeakHost(Arc::downgrade(&self.inner))
    }

    /// Puts what extensions add to the interface in `contributions` (the
    /// app's, shared with its window) in place of the host's own.
    #[must_use]
    pub fn with_contributions(mut self, contributions: Contributions) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.contributions = contributions;
        }
        self
    }

    /// Runs hooks' work with `spawn` (the app's executor) in place of a
    /// thread each.
    #[must_use]
    pub fn with_spawn(self, spawn: impl Fn(ActionFuture) + Send + Sync + 'static) -> Self {
        *self
            .inner
            .spawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(spawn);
        self
    }

    /// Opens extensions' views with `opener` (the app's window's), from
    /// now on.
    pub fn set_view_opener(
        &self,
        opener: impl Fn(ViewOpen) -> Result<Value, String> + Send + Sync + 'static,
    ) {
        *self
            .inner
            .view_opener
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(opener));
    }

    /// Opens extension `extension`'s view `view` (showing `state`), as its
    /// manifest places it unless `placement` says otherwise.
    ///
    /// # Errors
    ///
    /// It is not loaded, has no such view, or the app has no window to
    /// open it in.
    pub fn open_view(
        &self,
        extension: &str,
        view: &str,
        state: Option<String>,
        placement: Option<crate::manifest::ViewPlacement>,
        invocation: Invocation,
    ) -> Result<Value, String> {
        let decl = self
            .get(extension)
            .ok_or_else(|| format!("{extension} is not loaded"))?
            .manifest
            .views
            .into_iter()
            .find(|v| v.id == view)
            .ok_or_else(|| format!("{extension} has no view {view}"))?;
        let opener = self
            .inner
            .view_opener
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or("this app opens no extension views (no window)")?;
        opener(ViewOpen {
            extension: extension.to_owned(),
            kind: decl.workspace_kind(extension),
            view: decl.id,
            title: decl.title,
            state,
            home: decl.home,
            placement: placement.unwrap_or(decl.placement),
            width: decl.width,
            height: decl.height,
            invocation,
        })
    }

    /// Opens `what` (a file's path, a link, a document kind) with the
    /// first opener that claims it: in its view, or with its action
    /// (the invocation, and `{path}` or `{url}` to one that takes
    /// arguments). A file or document opened raises `doc.opened`. What
    /// opened it, and how.
    ///
    /// # Errors
    ///
    /// Nothing claims it, or its opener failed.
    pub async fn open(&self, what: Opening, mut invocation: Invocation) -> Result<Value, String> {
        let reg = &self.inner.contributions;
        let (claims, state, field) = match &what {
            Opening::Path(p) => (reg.open_path(p), p.clone(), "path"),
            Opening::Url(u) => (reg.open_url(u), u.clone(), "url"),
            Opening::DocKind { kind, path } => (
                reg.open_doc_kind(kind),
                path.clone().unwrap_or_default(),
                "path",
            ),
        };
        let by = claims
            .into_iter()
            .next()
            .ok_or_else(|| format!("nothing opens {}", what.describe()))?;
        if invocation.source == InvocationSource::App {
            invocation.source = if field == "url" {
                InvocationSource::Link
            } else {
                InvocationSource::Open
            };
        }
        if field == "url" {
            invocation.url = Some(state.clone());
        } else {
            invocation.path = Some(state.clone());
        }
        let opener = by.item.key().to_owned();
        let answer = if let Some(view) = &by.item.view {
            let placed = self.open_view(
                &by.owner,
                view,
                (!state.is_empty()).then(|| state.clone()),
                None,
                invocation,
            )?;
            serde_json::json!({ "opened_by": by.owner, "opener": opener, "view": view, "opened": placed })
        } else {
            let caller = ActionCaller::app().with_invocation(invocation);
            // The path or link is the invocation's (`ctx.path`, `ctx.url`);
            // it goes in the arguments too only to an action that takes
            // some — one that takes none refuses any name.
            let takes = self
                .inner
                .list
                .info(&by.item.action)
                .is_some_and(|i| i.params_schema.is_some());
            let args = if takes {
                serde_json::json!({ field: state }).to_string()
            } else {
                String::new()
            };
            let out = self
                .inner
                .list
                .run_json_async_with(&caller, &by.item.action, args.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let result: Value = serde_json::from_slice(&out).unwrap_or(Value::Null);
            serde_json::json!({ "opened_by": by.owner, "opener": opener, "action": by.item.action, "result": result })
        };
        if field == "path" {
            self.emit("doc.opened", &state);
        }
        Ok(answer)
    }

    /// Links in-process extension `built` as `id`'s built-in copy: when
    /// that copy runs, it runs here ([`crate::linked`]).
    #[cfg(feature = "linked")]
    pub fn link(&self, manifest: &Manifest, built: crate::linked::Builder) {
        built.handle().connect(Arc::new(linked::Sink {
            manifest: manifest.clone(),
            contributions: self.inner.contributions.clone(),
            api: self.api(&manifest.id),
        }));
        let id = &manifest.id;
        self.inner
            .linked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), Arc::new(built));
    }

    /// The component of view `view` of `extension`, when the copy of it
    /// running is the one linked in (a process copy's is drawn remotely).
    #[cfg(feature = "linked")]
    #[must_use]
    pub fn linked_view(
        &self,
        extension: &str,
        view: &str,
    ) -> Option<fn() -> dioxus::prelude::Element> {
        if self.get(extension)?.origin != Origin::Builtin {
            return None;
        }
        self.inner
            .linked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(extension)?
            .view_root(view)
    }

    /// The in-process extensions linked in, built: for their root context.
    #[cfg(feature = "linked")]
    #[must_use]
    pub fn linked(&self) -> Vec<(String, Arc<crate::linked::Builder>)> {
        self.inner
            .linked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(id, b)| (id.clone(), Arc::clone(b)))
            .collect()
    }

    /// What loaded extensions add to the interface: their openers, tabs,
    /// status items, menu and context entries, hooks, slots, and the
    /// runtime state beside them.
    #[must_use]
    pub fn contributions(&self) -> &Contributions {
        &self.inner.contributions
    }

    /// Runs `runner`'s kind with it, in place of the one there was.
    #[must_use]
    pub fn with_runner(self, runner: impl Runner + 'static) -> Self {
        {
            let mut runners = self
                .inner
                .runners
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            runners.retain(|r| r.kind() != runner.kind());
            runners.push(Arc::new(runner));
        }
        self
    }

    /// Runs `runner`'s kind with it from now, in place of the one there
    /// was ([`Self::with_runner`] on a host already shared).
    pub fn add_runner(&self, runner: impl Runner + 'static) {
        let mut runners = self
            .inner
            .runners
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        runners.retain(|r| r.kind() != runner.kind());
        runners.push(Arc::new(runner));
    }

    /// The view kinds it takes.
    #[must_use]
    pub fn view_kinds(&self) -> ViewKinds {
        self.inner
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// An `EXT_CHECK` checker that knows what this host knows: its view
    /// kinds, and whether it runs process extensions.
    #[must_use]
    pub fn checker(&self) -> crate::check::Checker {
        let checker = crate::check::Checker::new()
            .with_view_kinds(self.view_kinds())
            .with_processes(self.runs(Kind::Process));
        match self.page_checker() {
            Some(pages) => checker.with_page_checker(pages),
            None => checker,
        }
    }

    /// `EXT_CHECK` checks a view's page file with `pages` (the app's page
    /// runner: `architect_pages`' `Pages::check_file`).
    pub fn set_page_checker(&self, pages: crate::check::PageChecker) {
        *self
            .inner
            .page_checker
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(pages);
    }

    /// What checks a view's page file, when the app gave one.
    #[must_use]
    pub fn page_checker(&self) -> Option<crate::check::PageChecker> {
        self.inner
            .page_checker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Takes views of `kinds` (the app's [`ViewKinds`], the ones its
    /// linked crates registered); without it, only the built-in `dioxus`.
    #[must_use]
    pub fn with_view_kinds(self, kinds: ViewKinds) -> Self {
        *self
            .inner
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = kinds;
        self
    }

    /// Puts loaded extensions' themes and tokens in `store` (the app's):
    /// in as each loads, as a diff on a reload, out as it unloads.
    #[must_use]
    pub fn with_themes(self, store: ThemeStore) -> Self {
        let weak = Arc::downgrade(&self.inner);
        let heard = store.subscribe(move |r| {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.emit("theme.changed", r.id());
            }
        });
        *self
            .inner
            .theme_heard
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(heard);
        *self
            .inner
            .themes
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(store);
        self
    }

    /// The theme store it puts extensions' themes in, when it has one.
    #[must_use]
    pub fn themes(&self) -> Option<ThemeStore> {
        self.inner
            .themes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether it runs extensions of `kind`.
    #[must_use]
    pub fn runs(&self, kind: Kind) -> bool {
        self.runner(kind).is_some()
    }

    fn runner(&self, kind: Kind) -> Option<Arc<dyn Runner>> {
        self.inner
            .runners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|r| r.kind() == kind)
            .cloned()
    }

    /// The Action List it puts actions on.
    #[must_use]
    pub fn list(&self) -> &ActionList {
        &self.inner.list
    }

    /// Calls `listener` with every [`Event`] from now on.
    pub fn on_event(&self, listener: impl Fn(&Event) + Send + Sync + 'static) {
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(listener));
    }

    fn tell(&self, event: &Event) {
        let listeners = self
            .inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for l in listeners {
            l(event);
        }
    }

    /// The loaded extensions' ids.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// Every loaded extension.
    #[must_use]
    pub fn loaded(&self) -> Vec<Loaded> {
        self.inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|r| r.loaded.clone())
            .collect()
    }

    /// The loaded extension `id`.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Loaded> {
        self.inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .map(|r| r.loaded.clone())
    }

    /// Loads `package` (from `origin`), or reloads it in place of the
    /// copy loaded now, as a diff. When it fails, nothing changes.
    ///
    /// # Errors
    ///
    /// No runner for its kind, it does not start, an action of it cannot
    /// be made or is another's, or its settings clash.
    pub fn load(&self, package: &Package, origin: Origin) -> Result<Change, LoadError> {
        let started = Instant::now();
        let manifest = &package.manifest;
        let span = tracing::info_span!(
            "ext.load",
            ext.id = %manifest.id,
            ext.version = %manifest.version,
            ext.kind = manifest.kind.name(),
            ext.origin = origin.name(),
            ext.outcome = tracing::field::Empty,
            ext.took_ms = tracing::field::Empty,
        );
        let _entered = span.enter();
        let result = self.swap_in(package, origin, started);
        if result.is_ok() {
            let id = &manifest.id;
            let first = self
                .inner
                .started
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.clone());
            if first {
                self.emit_to(Some(id), "startup", id);
            }
            self.emit("extension.loaded", id);
        }
        span.record(
            "ext.outcome",
            match &result {
                Ok(_) => "loaded",
                Err(_) => "failed",
            },
        );
        span.record(
            "ext.took_ms",
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        match &result {
            Ok(change) => self.tell(&Event::Loaded(change.clone())),
            Err(error) => self.tell(&Event::Failed {
                id: manifest.id.clone(),
                error: error.clone(),
            }),
        }
        result
    }

    fn swap_in(
        &self,
        package: &Package,
        origin: Origin,
        started: Instant,
    ) -> Result<Change, LoadError> {
        let manifest = &package.manifest;
        manifest.check().map_err(|e| LoadError::Package(e.into()))?;
        let runner = match self.linked_runner(&manifest.id, origin) {
            Some(linked) => linked,
            None => self
                .runner(manifest.kind)
                .ok_or(LoadError::NoRunner(manifest.kind))?,
        };
        self.check_views(manifest)?;
        let list = &self.inner.list;
        let mut loaded = self
            .inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let old = loaded.get(&manifest.id);
        let reloaded = old.is_some();
        // 1–2: start the new one with the old one's state, and build
        // everything it adds, before anything is swapped.
        let instance = runner
            .start(
                package,
                Context {
                    list,
                    gate: self.live_gate(manifest, origin),
                    state: old.and_then(|o| o.instance.save_state()),
                    contributions: &self.inner.contributions,
                    api: self.api(&manifest.id),
                    logs: self.logs().clone(),
                },
            )
            .map_err(LoadError::Start)?;
        let mut actions = Vec::with_capacity(manifest.actions.len());
        for decl in &manifest.actions {
            let handler = match &decl.preset {
                Some(_) => preset(list, self.live_gate(manifest, origin), decl),
                None => instance.action(decl).map_err(|why| LoadError::Action {
                    id: decl.id.clone(),
                    why,
                })?,
            };
            actions.push((info(manifest, decl), handler));
        }
        let before: BTreeSet<String> = old
            .map(|o| o.loaded.actions.iter().cloned().collect())
            .unwrap_or_default();
        if let Some((info, _)) = actions
            .iter()
            .find(|(i, _)| !before.contains(&i.id) && list.info(&i.id).is_some())
        {
            return Err(LoadError::ActionTaken(info.id.clone()));
        }
        self.inner
            .sections
            .put(&manifest.id, manifest.sections())
            .map_err(LoadError::Sections)?;
        // 3: the actions, as a diff.
        let after: BTreeSet<String> = actions.iter().map(|(i, _)| i.id.clone()).collect();
        let removed: Vec<String> = before.difference(&after).cloned().collect();
        let kept: Vec<String> = before.intersection(&after).cloned().collect();
        let added: Vec<String> = after.difference(&before).cloned().collect();
        for id in &before {
            list.unregister(id);
        }
        for (info, handler) in actions {
            // Each id was free or ours a moment ago, under the lock.
            let _ = list.register_forwarded_with(&info, handler);
        }
        // Its themes and tokens, as a diff: never a reason to fail.
        if let Some(store) = self.themes() {
            store.contribute(&manifest.id, theme_contribution(package));
        }
        // What it adds to the interface, as a diff (what it added at run
        // time stays until its new instance says otherwise).
        let declared = instance.declared().map_or_else(
            || manifest.contributes.clone(),
            |code| manifest.contributes.merged(&code),
        );
        self.inner.contributions.put(&manifest.id, declared);
        if let Some(old) = loaded.remove(&manifest.id) {
            old.instance.stop();
        }
        loaded.insert(
            manifest.id.clone(),
            Running {
                loaded: Loaded {
                    manifest: manifest.clone(),
                    origin,
                    dir: package.dir().map(std::path::Path::to_path_buf),
                    actions: after.into_iter().collect(),
                },
                instance,
            },
        );
        drop(loaded);
        Ok(Change {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            origin,
            reloaded,
            added,
            removed,
            kept,
            took: started.elapsed(),
        })
    }

    /// The runner for an in-process extension `id` linked in, when the
    /// copy loading is the built-in one.
    #[allow(clippy::unused_self)]
    fn linked_runner(&self, id: &str, origin: Origin) -> Option<Arc<dyn Runner>> {
        #[cfg(feature = "linked")]
        if origin == Origin::Builtin {
            let built = self
                .inner
                .linked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .cloned()?;
            return Some(Arc::new(linked::Runner(built)));
        }
        let _ = (id, origin);
        None
    }

    /// Refuses a view of a kind nobody here provides, before anything of
    /// the extension starts.
    fn check_views(&self, manifest: &Manifest) -> Result<(), LoadError> {
        let kinds = self
            .inner
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for view in &manifest.views {
            kinds
                .resolve(&view.kind)
                .map_err(|why| LoadError::ViewKind {
                    view: view.id.clone(),
                    why,
                })?;
        }
        drop(kinds);
        Ok(())
    }

    /// Raises app event `on` (`workspace.changed`, `doc.opened`,
    /// `changed:<topic>`) with `detail`: every loaded extension's hooks on
    /// it run, each as the app with the event in its invocation and as
    /// `{event, detail}` arguments, inside its extension's capabilities.
    /// A hook still running from before is not started again. How many
    /// started.
    // How many is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn emit(&self, on: &str, detail: &str) -> usize {
        self.emit_to(None, on, detail)
    }

    /// [`Self::emit`] to `only` one extension's hooks.
    fn emit_to(&self, only: Option<&str>, on: &str, detail: &str) -> usize {
        let hooks: Vec<_> = self
            .inner
            .contributions
            .hooks(on)
            .into_iter()
            .filter(|h| only.is_none_or(|o| o == h.owner))
            .collect();
        if hooks.is_empty() {
            return 0;
        }
        let spawn = self
            .inner
            .spawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut started = 0_usize;
        for by in hooks {
            let key = format!(
                "{}/{}/{}",
                by.owner,
                by.item.on,
                by.item
                    .action
                    .as_deref()
                    .or(by.item.script.as_deref())
                    .unwrap_or_default()
            );
            let caller = ActionCaller::app().with_invocation(Invocation {
                event: Some(on.to_owned()),
                detail: Some(detail.to_owned()),
                ..Invocation::new(InvocationSource::Hook)
            });
            let args = serde_json::json!({ "event": on, "detail": detail });
            let Some(work) = self.hook_work(&by.owner, &by.item, &caller, args) else {
                continue;
            };
            {
                let mut running = self
                    .inner
                    .running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if !running.insert(key.clone()) {
                    continue;
                }
            }
            let running = Arc::clone(&self.inner.running);
            let (owner, hook) = (by.owner.clone(), on.to_owned());
            let logs = self.logs().clone();
            spawn(Box::pin(async move {
                let out = work.await;
                if let Err(e) = &out {
                    logs.push(
                        &owner,
                        crate::logs::Level::Error,
                        "hook",
                        format!("{hook}: {e}"),
                    );
                }
                let _span = tracing::info_span!(
                    "ext.hook",
                    ext.id = %owner,
                    ext.hook = %hook,
                    ext.outcome = if out.is_ok() { "ran" } else { "failed" },
                    ext.error = out.as_ref().err().map(ToString::to_string),
                )
                .entered();
                running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&key);
                out
            }));
            started = started.saturating_add(1);
        }
        started
    }

    /// A hook's work: its action through the list (inside its extension's
    /// gate), or its script on its instance.
    fn hook_work(
        &self,
        owner: &str,
        decl: &HookDecl,
        caller: &ActionCaller,
        args: Value,
    ) -> Option<ActionFuture> {
        let loaded = self
            .inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let running = loaded.get(owner)?;
        if decl.script.is_some() {
            return running.instance.hook(decl, caller, args);
        }
        let action = decl.action.clone()?;
        let gate = self.live_gate(&running.loaded.manifest, running.loaded.origin);
        drop(loaded);
        let list = self.inner.list.clone();
        let info = list.info(&action)?;
        if let Err(why) = gate(&info) {
            tracing::warn!(ext.id = %owner, ext.hook_action = %action, ext.why = %why, "a hook was refused");
            self.logs().push(
                owner,
                crate::logs::Level::Warn,
                "hook",
                format!("{} ran {action}: refused: {why}", decl.on),
            );
            return None;
        }
        let caller = caller.clone();
        // The event is the invocation's (`ctx.event`, `ctx.detail`); it
        // goes in the arguments too only to an action that takes some.
        let args = if info.params_schema.is_some() {
            args.to_string().into_bytes()
        } else {
            Vec::new()
        };
        Some(Box::pin(async move {
            list.run_json_async_with(&caller, &action, &args).await
        }))
    }

    /// Hears `list`'s events (the app's) on a thread of its own, for as
    /// long as the host lives: each `Changed{topic}` raises
    /// `changed:<topic>` and, for the app's topics, its name
    /// (`workspace` → `workspace.changed`, `theme`, `selection`, `doc` →
    /// `doc.opened`).
    pub fn watch(&self, list: &ActionList) {
        let weak = Arc::downgrade(&self.inner);
        let list = list.clone();
        let mut since = list.latest();
        let _ = std::thread::Builder::new()
            .name("ext-hooks".to_owned())
            .spawn(move || {
                loop {
                    let (events, next) = list.events_since(since, 64, Duration::from_millis(500));
                    since = next;
                    let Some(inner) = weak.upgrade() else { return };
                    let host = Self { inner };
                    for e in events {
                        if let EventKind::Changed { topic, detail } = e.kind {
                            host.emit_changed(&topic, &detail);
                        }
                    }
                }
            });
    }

    /// Raises `changed:<topic>`, and the app event the topic names.
    // How many is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn emit_changed(&self, topic: &str, detail: &str) -> usize {
        let named = match topic {
            "workspace" => Some("workspace.changed"),
            "theme" => Some("theme.changed"),
            "selection" => Some("selection.changed"),
            "doc" => Some("doc.opened"),
            _ => None,
        };
        let mut n = self.emit(&format!("changed:{topic}"), detail);
        if let Some(named) = named {
            n = n.saturating_add(self.emit(named, detail));
        }
        n
    }

    /// Unloads `id`: its actions off the list, its settings off the page.
    /// False when it was not loaded.
    pub fn unload(&self, id: &str) -> bool {
        let Some(running) = self
            .inner
            .loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
        else {
            return false;
        };
        for action in &running.loaded.actions {
            self.inner.list.unregister(action);
        }
        self.inner.sections.withdraw(id);
        if let Some(store) = self.themes() {
            store.withdraw(id);
        }
        self.inner.contributions.withdraw(id);
        running.instance.stop();
        tracing::info_span!("ext.unload", ext.id = %id).in_scope(|| {});
        self.tell(&Event::Unloaded { id: id.to_owned() });
        self.emit("extension.unloaded", id);
        true
    }
}

/// What `package` adds to the app's themes: each `[[themes]]` file read
/// (its id the file's name; [`Source::Extension`](architect_theme::Source::Extension)),
/// the ones that do not read as problems, and its `[tokens]`.
#[must_use]
pub fn theme_contribution(package: &Package) -> Contribution {
    let manifest = &package.manifest;
    let mut out = Contribution {
        tokens: manifest.tokens.clone(),
        ..Contribution::default()
    };
    for path in manifest.theme_files() {
        let source = architect_theme::Source::Extension {
            id: manifest.id.clone(),
            file: path.clone(),
        };
        let id = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&path)
            .to_owned();
        let read = package
            .text(&path)
            .map_err(|e| vec![Problem::new(&id, &source, format!("cannot be read: {e}"))])
            .and_then(|text| Theme::parse(&id, &text, source.clone()));
        match read {
            Ok(theme) => out.themes.push(theme),
            Err(problems) => out.failed.push((id, problems)),
        }
    }
    out
}

/// How the list describes an extension's action.
fn info(manifest: &Manifest, decl: &ActionDecl) -> ActionInfo {
    ActionInfo {
        id: decl.id.clone(),
        display_name: decl.name.clone(),
        description: if decl.description.is_empty() {
            format!("From the extension {}.", manifest.name)
        } else {
            decl.description.clone()
        },
        category: "Extensions".to_owned(),
        group: if decl.group.is_empty() {
            manifest.name.clone()
        } else {
            decl.group.clone()
        },
        shortcut: decl.shortcut.clone(),
        effect: effect(decl.effect),
        params_schema: decl.params.clone(),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

/// A preset: the action it names, with its fixed arguments (or the ones
/// it is given), inside the extension's capabilities.
fn preset(list: &ActionList, gate: Gate, decl: &ActionDecl) -> CallerAsyncHandler {
    let list = list.clone();
    let (id, target, fixed) = (
        decl.id.clone(),
        decl.preset
            .as_ref()
            .map(|p| p.action.clone())
            .unwrap_or_default(),
        decl.preset
            .as_ref()
            .and_then(|p| p.args.clone())
            .map(String::into_bytes),
    );
    Arc::new(move |caller: &ActionCaller, args: &[u8]| {
        let info = list
            .info(&target)
            .ok_or_else(|| ActionError::Unknown(target.clone()))?;
        gate(&info).map_err(|reason| ActionError::Denied {
            id: id.clone(),
            reason: format!("{target}: {reason}"),
        })?;
        let args = fixed.clone().unwrap_or_else(|| args.to_vec());
        let (list, caller, target) = (list.clone(), caller.clone(), target.clone());
        let run: ActionFuture =
            Box::pin(async move { list.run_json_async_with(&caller, &target, &args).await });
        Ok(run)
    })
}

/// In-process extensions' runner (feature `linked`).
#[cfg(feature = "linked")]
mod linked {
    use std::sync::Arc;

    use architect::action::{ActionCaller, ActionError, ActionFuture, CallerAsyncHandler};

    use super::{Context, Instance, invocation_json, json_args, now_ms};
    use crate::contrib::{Contribution, Contributions, StateWrite};
    use crate::linked::Builder;
    use crate::manifest::{ActionDecl, Kind, Manifest};
    use crate::package::Package;

    /// A linked extension's runtime additions, into the registry: checked
    /// as its manifest's tables are, its state its own.
    pub(super) struct Sink {
        pub(super) manifest: Manifest,
        pub(super) contributions: Contributions,
        pub(super) api: crate::hostapi::ExtensionApi,
    }

    impl crate::linked::Sink for Sink {
        fn contribute(&self, contribution: Contribution) {
            let views = self.manifest.view_ids();
            match contribution.check(&self.manifest.rules(&views)) {
                Ok(()) => {
                    self.contributions
                        .put_runtime(&self.manifest.id, contribution);
                }
                Err(why) => {
                    tracing::warn!(ext.id = %self.manifest.id, ext.why = %why, "a contribution was refused");
                }
            }
        }

        fn set_state(&self, mut write: StateWrite) {
            write.source = format!("ext:{}", self.manifest.id);
            if let Err(why) = self.contributions.set_state(write, now_ms()) {
                tracing::warn!(ext.id = %self.manifest.id, ext.why = %why, "state was refused");
            }
        }

        fn clear_state(&self, key: Option<&str>) {
            self.contributions
                .clear_state(&format!("ext:{}", self.manifest.id), key);
        }

        fn host(&self, request: crate::hostcall::HostRequest) -> crate::linked::HostFuture {
            let api = self.api.clone();
            Box::pin(async move { api.call(request).await })
        }
    }

    pub(super) struct Runner(pub(super) Arc<Builder>);

    impl super::Runner for Runner {
        fn kind(&self) -> Kind {
            // Whatever its manifest says (its process copy's kind): it is
            // chosen by its id, not its kind.
            Kind::Process
        }

        fn start(&self, package: &Package, _: Context<'_>) -> Result<Box<dyn Instance>, String> {
            let m = &package.manifest;
            if let Some(view) = self
                .0
                .view_ids()
                .into_iter()
                .find(|v| !m.views.iter().any(|d| &d.id == v))
            {
                return Err(format!(
                    "its code draws {view}, which its manifest does not declare"
                ));
            }
            if let Some(a) = m.actions.iter().find(|a| {
                a.preset.is_none() && a.script.is_none() && self.0.action_fn(&a.id).is_none()
            }) {
                return Err(format!("its code does not run its action {}", a.id));
            }
            let views = m.view_ids();
            self.0
                .contribution()
                .check(&m.rules(&views))
                .map_err(|e| e.to_string())?;
            Ok(Box::new(Linked(Arc::clone(&self.0))))
        }
    }

    struct Linked(Arc<Builder>);

    impl Instance for Linked {
        fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
            let run = self
                .0
                .action_fn(&decl.id)
                .ok_or_else(|| format!("its code does not run {}", decl.id))?;
            let id = decl.id.clone();
            Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
                let args = json_args(&id, args)?;
                let cx = invocation_json(caller.invocation());
                let (work, id) = (run(args, cx), id.clone());
                let out: ActionFuture = Box::pin(async move {
                    work.await
                        .map(|v| v.to_string().into_bytes())
                        .map_err(|message| ActionError::Failed { id, message })
                });
                Ok(out)
            }))
        }

        fn declared(&self) -> Option<Contribution> {
            Some(self.0.contribution().clone())
        }
    }
}
