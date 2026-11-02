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

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, CallerAsyncHandler, Effect,
};
use architect_scripting::Transactions;
use architect_scripting::luau::{Gate, Limits, Runtime};
use architect_settings::{Owner, Registry, Section};
use architect_workspace::{KindError, ViewKinds};
use semver::Version;
use serde_json::Value;

use crate::catalog::Origin;
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
    /// Each script action's source.
    scripts: BTreeMap<String, Arc<str>>,
    /// The undo step's prefix: the extension's name.
    name: String,
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
        let mut runtime = Runtime::new(cx.list.clone())
            .with_gate(cx.gate)
            .with_limits(self.limits);
        if let Some(t) = &self.transactions {
            runtime = runtime.with_transactions(Arc::clone(t));
        }
        Ok(Box::new(Luau {
            runtime,
            scripts,
            name: package.manifest.name.clone(),
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
        let runtime = self.runtime.clone();
        let id = decl.id.clone();
        let step = format!("{}: {}", self.name, decl.name);
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args = json_args(&id, args)?;
            let (runtime, source, caller, id, step) = (
                runtime.clone(),
                Arc::clone(&source),
                caller.clone(),
                id.clone(),
                step.clone(),
            );
            let run: ActionFuture = Box::pin(async move {
                let report = runtime.run(&caller, &source, args, &step).await;
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

struct Inner {
    list: ActionList,
    sections: Arc<dyn SectionSink>,
    runners: Mutex<Vec<Arc<dyn Runner>>>,
    /// The view kinds the app provides: what its views may name.
    kinds: Mutex<ViewKinds>,
    loaded: Mutex<BTreeMap<String, Running>>,
    listeners: Mutex<Vec<Listener>>,
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
                loaded: Mutex::new(BTreeMap::new()),
                listeners: Mutex::new(Vec::new()),
            }),
        }
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

    fn emit(&self, event: &Event) {
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
            Ok(change) => self.emit(&Event::Loaded(change.clone())),
            Err(error) => self.emit(&Event::Failed {
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
        let runner = self
            .runner(manifest.kind)
            .ok_or(LoadError::NoRunner(manifest.kind))?;
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
                    gate: gate(manifest),
                    state: old.and_then(|o| o.instance.save_state()),
                },
            )
            .map_err(LoadError::Start)?;
        let mut actions = Vec::with_capacity(manifest.actions.len());
        for decl in &manifest.actions {
            let handler = match &decl.preset {
                Some(_) => preset(list, manifest, decl),
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
        running.instance.stop();
        tracing::info_span!("ext.unload", ext.id = %id).in_scope(|| {});
        self.emit(&Event::Unloaded { id: id.to_owned() });
        true
    }
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
fn preset(list: &ActionList, manifest: &Manifest, decl: &ActionDecl) -> CallerAsyncHandler {
    let (list, gate) = (list.clone(), gate(manifest));
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
