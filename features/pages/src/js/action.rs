//! TypeScript actions (#536, feature `actions`).
//!
//! A `typescript` extension's actions and hooks are `.ts`/`.js` modules
//! beside its pages, run on the pages' `QuickJS` runtime — the same
//! prelude, the same `"architect"` module, the same bridge to the host
//! service — so a light extension needs no Rust and no WASM toolchain.
//!
//! ```toml
//! kind = "typescript"
//!
//! [[actions]]
//! id = "SHOWCASE_RECIPE_SCALE"
//! name = "Scale a recipe"
//! params = '{"type":"object","properties":{"servings":{"type":"number"}}}'
//! script = "actions/scale.ts"
//! ```
//!
//! ```ts
//! import { defineAction, documents } from "architect";
//! export default defineAction<{ servings?: number }>(async (args, ctx) => {
//!   const note = await documents.read("Recipes/Pancakes.md");
//!   return { servings: args.servings ?? 4, from: ctx.source };
//! });
//! ```
//!
//! [`TsRunner`] is the extensions host's runner for the kind
//! (`architect_extensions::host::Runner`). As it starts an extension,
//! every module of its folder is transpiled by oxc (`crate::modules::prepare`)
//! and each action's and hook's module, with what it imports, must
//! compile: a mistake fails the load at its file, line and column. Each
//! run then has a fresh runtime of its own on a thread of its own
//! ([`crate::js::JsEngine::bare`]), the module loaded from the folder:
//!
//! - **as its extension**, acting for whoever ran it: its action runs go
//!   through the extension's gate, its host-service calls (documents,
//!   `net.fetch`, prompts, pickers, menus, the clipboard, the editor,
//!   events, secrets) are checked against what was granted, as a page's
//!   and a Luau script's are;
//! - **within limits**: [`ActionLimits::memory`], each unit of its work
//!   (its start, an answer's continuation, a timer) within
//!   [`ActionLimits::unit`], its units in all within [`ActionLimits::cpu`]
//!   (20 s), the whole run, waits included, within [`ActionLimits::time`]
//!   (10 min), at most [`ActionLimits::concurrent`] runs of an extension
//!   at once; one that awaits nothing that will ever answer is stopped
//!   and says so; its answer within 1 MB and its error within 4 KB
//!   (`architect_extensions::host::MAX_ANSWER`, `MAX_ERROR`), as a Luau
//!   script's;
//! - **one undo step** when the runner has the document's transactions,
//!   taken back when it fails, as a Luau script's run is;
//! - **its answer** is what its handler returns, as JSON; what it throws
//!   is the run's error, at its `.ts` file's own line and column; its
//!   `console.log` lines go to the extension's log (`EXT_LOGS`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use architect::action::{
    ActionCaller, ActionError, ActionFuture, CallerAsyncHandler, InvocationSource,
};
use architect_extensions::contrib::HookDecl;
use architect_extensions::host::{Context, Instance, Runner, capped_answer, capped_error};
use architect_extensions::logs::{Level, Logs};
use architect_extensions::manifest::{ActionDecl, Kind, is_ts_script};
use architect_extensions::package::Package;
use architect_scripting::Transactions;
use architect_scripting::luau::Gate;
use architect_scripting::vm::{Cancel, Dispatch};
use serde_json::Value;

use crate::host::AppHost;
use crate::js::JsEngine;
use crate::modules::{self, is_script};
use crate::runner::{Log, PageLimits, Services};

/// The most files of an extension's folder read as it starts.
const MAX_FILES: usize = 2_000;

/// What one run of a TypeScript action may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionLimits {
    /// The most memory its runtime may hold.
    pub memory: usize,
    /// The longest one unit of its work may run: its start, the
    /// continuation of a host call's answer, a timer. Past it the run is
    /// stopped with a message to hand heavy work to a worker.
    pub unit: Duration,
    /// The longest its module's top level may take to load.
    pub load: Duration,
    /// The CPU time its units may take in all (the sum of them, waits for
    /// the app and the person left out): what stops a timer that renews
    /// itself for ever. `None` for no limit.
    pub cpu: Option<Duration>,
    /// The longest the whole run may take, waits for the app and the
    /// person included (a prompt left open): its undo step is held that
    /// long at most. `None` for no limit.
    pub time: Option<Duration>,
    /// How many runs of one extension's actions and hooks may be going at
    /// once: one more fails at once, saying so.
    pub concurrent: usize,
    /// Its workers' (`worker.spawn`): memory and their whole run's time.
    pub worker_memory: usize,
    pub worker_time: Duration,
}

impl Default for ActionLimits {
    fn default() -> Self {
        let page = PageLimits::default();
        Self {
            memory: page.memory,
            unit: Duration::from_secs(1),
            load: page.load,
            cpu: Some(Duration::from_secs(20)),
            time: Some(Duration::from_secs(600)),
            concurrent: 8,
            worker_memory: page.worker_memory,
            worker_time: page.worker_time,
        }
    }
}

impl ActionLimits {
    const fn page(&self) -> PageLimits {
        PageLimits {
            memory: self.memory,
            frame: self.unit,
            load: self.load,
            worker_memory: self.worker_memory,
            worker_time: self.worker_time,
        }
    }
}

/// The extensions host's runner for `kind = "typescript"`.
///
/// `host` is the page host its runs start from — the app's theme,
/// settings and shared state, as its pages have them; each run makes it
/// its extension's, acting for whoever ran it.
#[derive(Clone)]
pub struct TsRunner {
    host: AppHost,
    limits: ActionLimits,
    transactions: Option<Arc<dyn Transactions>>,
}

impl std::fmt::Debug for TsRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsRunner")
            .field("limits", &self.limits)
            .field("transactions", &self.transactions.is_some())
            .finish_non_exhaustive()
    }
}

impl TsRunner {
    /// Runs from `host` (the app's page host, `Pages::host`).
    #[must_use]
    pub fn new(host: AppHost) -> Self {
        Self {
            host,
            limits: ActionLimits::default(),
            transactions: None,
        }
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: ActionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Each run one step of `transactions` (the document's undo), taken
    /// back when it fails.
    #[must_use]
    pub fn with_transactions(mut self, transactions: Arc<dyn Transactions>) -> Self {
        self.transactions = Some(transactions);
        self
    }
}

impl Runner for TsRunner {
    fn kind(&self) -> Kind {
        Kind::TypeScript
    }

    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let manifest = &package.manifest;
        let root = package
            .dir()
            .ok_or("a typescript extension runs from its folder, and this package has none")?
            .to_path_buf();
        let files = read_scripts(&root)?;
        let prepared = modules::prepare(&files);
        let mut actions = BTreeMap::new();
        for decl in &manifest.actions {
            let Some(path) = &decl.script else { continue };
            compiles(&prepared, path)?;
            actions.insert(decl.id.clone(), path.clone());
        }
        let mut hooks = BTreeSet::new();
        for decl in &manifest.contributes.hooks {
            let Some(path) = decl.script.as_ref().filter(|p| is_ts_script(p)) else {
                continue;
            };
            compiles(&prepared, path)?;
            hooks.insert(path.clone());
        }
        Ok(Box::new(TsInstance(Arc::new(Shared {
            root,
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            actions,
            hooks,
            host: self.host.clone(),
            gate: cx.gate,
            dispatch: Dispatch::here(),
            api: cx.api,
            logs: cx.logs,
            limits: self.limits,
            transactions: self.transactions.clone(),
            stop: Cancel::default(),
            running: Arc::default(),
        }))))
    }
}

/// Every script of `root` (`.ts`, `.tsx`, `.js`…), by its `/`-separated
/// path; hidden folders and `node_modules` left out.
fn read_scripts(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(dir) = folders.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if kind.is_dir() {
                folders.push(path);
                continue;
            }
            let Some(rel) = path.strip_prefix(root).ok().map(slashed) else {
                continue;
            };
            if !is_script(&rel) {
                continue;
            }
            if out.len() >= MAX_FILES {
                return Err(format!("its folder has more than {MAX_FILES} scripts"));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
            out.insert(rel, text);
        }
    }
    Ok(out)
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// That module `path`, and every file of the folder it imports, compiles
/// and resolves: the first mistake, at its file, line and column.
fn compiles(prepared: &BTreeMap<String, modules::Module>, path: &str) -> Result<(), String> {
    let path = path.trim_start_matches("./");
    let mut seen = BTreeSet::new();
    let mut next = vec![path.to_owned()];
    while let Some(name) = next.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let module = prepared
            .get(&name)
            .ok_or_else(|| format!("{name}: no such file in the extension's folder"))?;
        if let Some(why) = &module.error {
            return Err(why.clone());
        }
        for import in &module.imports {
            match &import.to {
                Ok(to) if to == "architect" => {}
                Ok(to) if to.starts_with("architect/") => {
                    return Err(format!(
                        "{name}: an action imports \"architect\" and files of its folder, not {to:?}"
                    ));
                }
                Ok(to) => next.push(to.clone()),
                Err(why) => return Err(format!("{name}: import {:?}: {why}", import.specifier)),
            }
        }
    }
    Ok(())
}

/// What every run of one loaded extension shares.
struct Shared {
    root: PathBuf,
    id: String,
    name: String,
    /// Each action's module, by its id.
    actions: BTreeMap<String, String>,
    /// Its hooks' modules.
    hooks: BTreeSet<String>,
    host: AppHost,
    gate: Gate,
    /// Where its runs' action calls go: the runtime it was loaded on.
    dispatch: Dispatch,
    api: architect_extensions::hostapi::ExtensionApi,
    logs: Logs,
    limits: ActionLimits,
    transactions: Option<Arc<dyn Transactions>>,
    /// Set when it is unloaded or replaced: its runs stop.
    stop: Cancel,
    /// How many of its runs are going ([`ActionLimits::concurrent`]).
    running: Arc<AtomicUsize>,
}

/// One run counted while it goes.
struct Counted(Arc<AtomicUsize>);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct TsInstance(Arc<Shared>);

/// What one run runs.
struct Target {
    /// The module, in the extension's folder.
    file: String,
    /// Its export that handles it: `default`, or the one a hook names.
    export: String,
    /// What its `ctx.action` says.
    action: String,
    /// Its undo step's name.
    step: String,
}

impl Shared {
    /// One run of module `file`'s export `export` (`default`) as `caller`
    /// with `args`: off the calling thread, on a thread of its own, one
    /// undo step named `step`; `action` is what its `ctx.action` says.
    fn run(self: &Arc<Self>, target: Target, caller: &ActionCaller, args: Value) -> ActionFuture {
        let Target {
            file,
            export,
            action,
            step,
        } = target;
        let me = Arc::clone(self);
        // As the extension, for whoever ran it; its inner calls are a
        // script's, never the person's own press.
        let principal = me
            .api
            .clone()
            .acting_for(caller.principal().cloned())
            .principal();
        let mut context = architect_extensions::host::invocation_json(caller.invocation());
        if let Value::Object(map) = &mut context {
            map.insert("action".to_owned(), Value::String(action));
            map.insert("extension".to_owned(), Value::String(me.id.clone()));
        }
        let mut invocation = caller.invocation().cloned().unwrap_or_default();
        invocation.source = InvocationSource::Script;
        let inner = caller
            .clone()
            .with_principal(principal)
            .with_invocation(invocation);
        let (tx, rx) = futures::channel::oneshot::channel();
        let what = format!("{}: {file}", me.id);
        let going = me.running.fetch_add(1, Ordering::AcqRel);
        let counted = Counted(Arc::clone(&me.running));
        if going >= me.limits.concurrent {
            drop(counted);
            let limit = me.limits.concurrent;
            return Box::pin(async move {
                Err(ActionError::Failed {
                    id: what,
                    message: format!(
                        "{limit} of its runs are going already: it may run {limit} at once"
                    ),
                })
            });
        }
        let spawned = std::thread::Builder::new()
            .name(format!("ts action {file}"))
            .spawn(move || {
                let _counted = counted;
                let opened = me.transactions.as_ref().is_some_and(|t| t.begin(&step));
                let out = me.run_here(&file, &export, &inner, &args, &context);
                match (&me.transactions, opened, out.is_ok()) {
                    (Some(t), true, true) => {
                        t.commit();
                    }
                    (Some(t), true, false) => {
                        t.abort();
                    }
                    _ => {}
                }
                let _ = tx.send(out);
            });
        Box::pin(async move {
            spawned.map_err(|e| ActionError::Failed {
                id: what.clone(),
                message: format!("its thread did not start: {e}"),
            })?;
            match rx
                .await
                .unwrap_or_else(|_| Err("its run stopped before it answered".to_owned()))
            {
                Ok(answer) => capped_answer(&what, answer.into_bytes()),
                Err(message) => Err(ActionError::Failed {
                    id: what,
                    message: capped_error(message),
                }),
            }
        })
    }

    /// A run, on its own thread: a fresh runtime, the module, its answer.
    fn run_here(
        &self,
        file: &str,
        export: &str,
        caller: &ActionCaller,
        args: &Value,
        context: &Value,
    ) -> Result<String, String> {
        let (inbox, from) = mpsc::channel();
        let host = self
            .host
            .clone()
            .with_extension(self.id.clone())
            .with_caller(caller.clone())
            .with_gate(Arc::clone(&self.gate))
            .with_dispatch(self.dispatch.clone());
        let (logs, owner) = (self.logs.clone(), self.id.clone());
        let log = Log::default().with_sink(Arc::new(move |level: &str, text: &str| {
            let level = match level {
                "error" => Level::Error,
                "warn" => Level::Warn,
                _ => Level::Info,
            };
            logs.push(&owner, level, "console", text.to_owned());
        }));
        let services = Services {
            host: Arc::new(host),
            inbox,
            log,
            stop: self.stop.clone(),
            limits: self.limits.page(),
            extension: self.id.clone(),
            embed: None,
        };
        let until = self.limits.time.and_then(|t| Instant::now().checked_add(t));
        let mut engine = JsEngine::bare(&self.root, &services)?;
        let out = engine.run_action(
            &self.root.join(file),
            export,
            &args.to_string(),
            &context.to_string(),
            &from,
            until,
            self.limits.cpu,
        );
        if let Err(why) = &out {
            self.logs
                .push(&self.id, Level::Error, "script", format!("{file}: {why}"));
        }
        drop(engine);
        out
    }
}

impl Instance for TsInstance {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let file = self
            .0
            .actions
            .get(&decl.id)
            .cloned()
            .ok_or_else(|| format!("{} has no script", decl.id))?;
        let shared = Arc::clone(&self.0);
        let id = decl.id.clone();
        let step = format!("{}: {}", self.0.name, decl.name);
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args = if args.iter().all(u8::is_ascii_whitespace) {
                Value::Null
            } else {
                serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                    id: id.clone(),
                    message: e.to_string(),
                    expected: None,
                })?
            };
            let target = Target {
                file: file.clone(),
                export: "default".to_owned(),
                action: id.clone(),
                step: step.clone(),
            };
            Ok(shared.run(target, caller, args))
        }))
    }

    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Value) -> Option<ActionFuture> {
        let path = decl.script.as_ref()?;
        if !self.0.hooks.contains(path) {
            return None;
        }
        let target = Target {
            file: path.clone(),
            // A hook may name one export of its module (`function`).
            export: decl
                .function
                .clone()
                .unwrap_or_else(|| "default".to_owned()),
            action: format!("hook {}", decl.on),
            step: format!("{} hook {}", self.0.name, decl.on),
        };
        Some(self.0.run(target, caller, args))
    }

    fn stop(&self) {
        self.0.stop.set();
    }
}
