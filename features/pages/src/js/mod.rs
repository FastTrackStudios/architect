//! TSX and JS pages on `QuickJS` (feature `js`).
//!
//! Each page has a `QuickJS` runtime of its own, made on its page's thread
//! (`crate::runner`) and living there: nothing of it is ever touched from
//! the window's thread. What the page renders leaves it as JSON, becomes
//! edits, and goes to the window as a frame.
//!
//! - **Its modules**: the page is an ES module; it imports what is in
//!   its own folder (`./list.tsx`, `../shared/ui.ts`), each `.ts`/`.tsx`/
//!   `.jsx` transpiled by oxc as it loads (`crate::tsx`), and
//!   `"architect"`, the runtime (`prelude.js`). No packages, no Node
//!   APIs: nothing outside its folder loads.
//! - **The app**: `actions`, `settings`, `state`, `theme`, `notify` and
//!   the raw `host` bind [`crate::host::AppHost`] (the [`PageHost`]
//!   service) in process. A call that awaits (an action) runs off the
//!   page's thread — the action on the app's Tokio runtime — and its
//!   answer resolves the page's promise when it comes back.
//! - **Limits**: its memory and stack are capped; each call into it (a
//!   render, a handler, a timer) has [`crate::runner::PageLimits::frame`] before `QuickJS`'s
//!   interrupt stops it with a message to use a worker, and its module's
//!   top level [`crate::runner::PageLimits::load`]. Closing the page stops it at once.
//! - **Workers**: `worker.spawn("job.js", args)` runs a script of its
//!   folder in a runtime of its own, on its own thread, with its own
//!   limits ([`crate::runner::PageLimits::worker_time`]); they speak by JSON messages
//!   only, and the worker goes when its page does.
//! - **Its log**: `console.log` lines go to the page's log
//!   ([`crate::runner::Log`]), under its extension.
//! - **Its errors** say the `.tsx` file's own line and column: each
//!   transpiled module keeps its source map ([`tsx::SourceMap`]), and what
//!   a page throws is remapped through them ([`tsx::remap`]) before it
//!   reaches `<file>.error`, `APP_STATUS` or `EXT_CHECK`.
//!
//! Phase two of #273 (a worker-side DOM so Preact or Solid run unchanged)
//! fits here: their DOM calls would build the same element tree this
//! runtime's `h` does, diffed into edits by the same [`crate::diff::Mirror`].

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use architect_scripting::vm::{self, Cancel};
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{CatchResultExt as _, Context, Ctx, Function, Module, Object, Runtime, Value};

use crate::api::{
    DataEvent, Element, Notice, PageEvent, PageHost as _, ThemeSnapshot, WorkerMessage,
};
use crate::files::PageFile;
use crate::runner::{Engine, Inbox, Load, Services};
use crate::tsx;

/// The page runtime, run first in every page's runtime.
const PRELUDE: &str = include_str!("prelude.js");
/// A worker's runtime.
const WORKER_PRELUDE: &str = include_str!("worker.js");
/// `import … from "architect"`.
const ARCHITECT: &str = "const a = globalThis.__architect.api;\n\
export const { h, Fragment, useState, useRef, useEffect, useData, useShared, useEmbed, useTheme, invalidate, \
theme, actions, settings, state, notify, ui, clipboard, events, documents, net, ports, worker, Overlay, host } = a;\nexport default a;\n";
/// `import … from "architect/worker"`.
const ARCHITECT_WORKER: &str = "const w = globalThis.__worker.api;\n\
export const { args, post, onMessage } = w;\nexport default w;\n";

/// `QuickJS`'s stack, per runtime.
const STACK: usize = 1024 * 1024;

/// The [`Load`] for TSX/TS/JSX/JS pages.
#[must_use]
pub fn loader() -> Load {
    Arc::new(|page: &PageFile, services: &Services| {
        let engine: Box<dyn Engine> = Box::new(JsEngine::load(page, services)?);
        Ok(engine)
    })
}

/// A process-wide origin for the deadlines kept as atomics.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn since_epoch() -> u64 {
    u64::try_from(epoch().elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Why the interrupt stopped a call.
const RUNNING: u8 = 0;
const OUT_OF_TIME: u8 = 1;
const STOPPED: u8 = 2;

/// What `QuickJS`'s interrupt checks: a deadline for the call running now,
/// and the flags that stop it.
#[derive(Debug, Default)]
struct Budget {
    /// Nanoseconds since [`epoch`]; 0 for none.
    deadline: AtomicU64,
    allowed: AtomicU64,
    why: AtomicU8,
    cancel: Vec<Cancel>,
}

impl Budget {
    fn new(cancel: Vec<Cancel>) -> Arc<Self> {
        Arc::new(Self {
            cancel,
            ..Self::default()
        })
    }

    fn arm(&self, allowed: Duration) {
        let allowed = u64::try_from(allowed.as_nanos()).unwrap_or(u64::MAX);
        self.why.store(RUNNING, Ordering::Relaxed);
        self.allowed.store(allowed, Ordering::Relaxed);
        self.deadline.store(
            since_epoch().saturating_add(allowed).max(1),
            Ordering::Relaxed,
        );
    }

    fn disarm(&self) {
        self.deadline.store(0, Ordering::Relaxed);
    }

    /// `QuickJS`'s interrupt: true stops the script.
    fn interrupt(&self) -> bool {
        if self.cancel.iter().any(Cancel::is_set) {
            self.why.store(STOPPED, Ordering::Relaxed);
            return true;
        }
        let at = self.deadline.load(Ordering::Relaxed);
        if at != 0 && since_epoch() > at {
            self.why.store(OUT_OF_TIME, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// What a call that failed failed of: the budget's reason, when it
    /// was the interrupt, else `error`.
    fn explain(&self, error: String) -> String {
        match self.why.swap(RUNNING, Ordering::Relaxed) {
            OUT_OF_TIME => {
                let ms = self.allowed.load(Ordering::Relaxed) / 1_000_000;
                format!("ran past its time of {ms} ms: hand heavy work to a worker (worker.spawn)")
            }
            STOPPED => "it was stopped".to_owned(),
            _ => error,
        }
    }
}

/// Finds and loads a page's modules: inside its folder only.
#[derive(Clone)]
struct Files {
    root: PathBuf,
    /// The page's (`"architect"`) or a worker's (`"architect/worker"`).
    worker: bool,
    /// The transpile errors of the last load, with their places.
    errors: Arc<Mutex<Vec<tsx::SourceError>>>,
    /// Each transpiled module's source map, by its module name.
    maps: Arc<Mutex<BTreeMap<String, tsx::SourceMap>>>,
}

impl Files {
    fn new(root: &Path, worker: bool) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        Self {
            root,
            worker,
            errors: Arc::default(),
            maps: Arc::default(),
        }
    }

    /// `why` (an error, a stack) at its sources' places, the page's
    /// folder's paths relative to it ([`tsx::remap`]).
    fn said(&self, why: &str) -> String {
        let root = self.root.display().to_string();
        let held = self.maps.lock().unwrap_or_else(PoisonError::into_inner);
        let maps: Vec<(&str, &tsx::SourceMap)> =
            held.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let said = tsx::remap(why, &root, &maps);
        drop(maps);
        drop(held);
        said
    }

    fn inside(&self, path: &Path) -> Option<PathBuf> {
        let real = path.canonicalize().ok()?;
        (real.starts_with(&self.root) && real.is_file()).then_some(real)
    }
}

/// `a/./b/../c` as `a/c`, without the file system.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

impl Resolver for Files {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let ours = if self.worker {
            "architect/worker"
        } else {
            "architect"
        };
        if name == ours {
            return Ok(name.to_owned());
        }
        let relative = name.starts_with("./") || name.starts_with("../");
        let wanted = if relative {
            let from = Path::new(base)
                .parent()
                .filter(|p| p.is_absolute())
                .map_or_else(|| self.root.clone(), Path::to_path_buf);
            normal(&from.join(name))
        } else if Path::new(name).is_absolute() {
            PathBuf::from(name)
        } else {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                format!(
                    "a page imports \"{ours}\" and files of its own folder (\"./x.tsx\"), nothing else"
                ),
            ));
        };
        let candidates = [
            wanted.clone(),
            wanted.with_extension("tsx"),
            wanted.with_extension("ts"),
            wanted.with_extension("jsx"),
            wanted.with_extension("js"),
            wanted.join("index.tsx"),
            wanted.join("index.ts"),
            wanted.join("index.js"),
        ];
        candidates
            .iter()
            .find_map(|c| self.inside(c))
            .map(|p| p.display().to_string())
            .ok_or_else(|| {
                rquickjs::Error::new_resolving_message(
                    base,
                    name,
                    "no such file in the page's folder",
                )
            })
    }
}

impl Loader for Files {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        if name == "architect" && !self.worker {
            return Module::declare(ctx.clone(), name, ARCHITECT);
        }
        if name == "architect/worker" && self.worker {
            return Module::declare(ctx.clone(), name, ARCHITECT_WORKER);
        }
        let path = Path::new(name);
        if self.inside(path).is_none() {
            return Err(rquickjs::Error::new_loading_message(
                name,
                "outside the page's folder",
            ));
        }
        let source = std::fs::read_to_string(path)
            .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
        let code = if tsx::needs_transpile(path) {
            let shown = path.strip_prefix(&self.root).unwrap_or(path);
            match tsx::transpile_mapped(shown, &source) {
                Ok((code, map)) => {
                    self.maps
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(name.to_owned(), map);
                    code
                }
                Err(errors) => {
                    let first = errors.first().map(ToString::to_string).unwrap_or_default();
                    self.errors
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend(errors);
                    return Err(rquickjs::Error::new_loading_message(name, first));
                }
            }
        } else {
            source
        };
        Module::declare(ctx.clone(), name, code)
    }
}

/// A worker the page spawned, as the page holds it.
struct WorkerLink {
    to: mpsc::Sender<String>,
    cancel: Cancel,
}

type Workers = Rc<RefCell<BTreeMap<u32, WorkerLink>>>;

/// A page's `QuickJS` runtime.
pub struct JsEngine {
    // The context goes before its runtime.
    ctx: Context,
    rt: Runtime,
    budget: Arc<Budget>,
    services: Services,
    workers: Workers,
    files: Files,
}

impl std::fmt::Debug for JsEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsEngine")
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

impl Drop for JsEngine {
    fn drop(&mut self) {
        for link in self.workers.borrow().values() {
            link.cancel.set();
        }
    }
}

/// A fresh runtime with a page's limits and loader.
fn runtime(
    memory: usize,
    budget: &Arc<Budget>,
    files: &Files,
) -> Result<(Runtime, Context), String> {
    let rt = Runtime::new().map_err(|e| e.to_string())?;
    rt.set_memory_limit(memory);
    rt.set_max_stack_size(STACK);
    let theirs = Arc::clone(budget);
    rt.set_interrupt_handler(Some(Box::new(move || theirs.interrupt())));
    rt.set_loader(files.clone(), files.clone());
    let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
    Ok((rt, ctx))
}

impl JsEngine {
    /// Loads `page` in a runtime of its own: the runtime first, then the
    /// page's module, its default export mounted.
    ///
    /// # Errors
    ///
    /// It does not parse (with its line), its top level throws or runs
    /// past [`crate::runner::PageLimits::load`], or it exports no component.
    pub fn load(page: &PageFile, services: &Services) -> Result<Self, String> {
        let budget = Budget::new(vec![services.stop.clone()]);
        let files = Files::new(&page.root(), false);
        let (rt, ctx) = runtime(services.limits.memory, &budget, &files)?;
        let mut engine = Self {
            ctx,
            rt,
            budget,
            services: services.clone(),
            workers: Rc::default(),
            files: files.clone(),
        };
        engine.install(&files)?;
        let path = page
            .path
            .canonicalize()
            .unwrap_or_else(|_| page.path.clone())
            .display()
            .to_string();
        engine.budget.arm(services.limits.load);
        let mounted = engine.ctx.with(|ctx| -> Result<(), String> {
            let ns: Object = Module::import(&ctx, path.clone())
                .and_then(|p| p.finish::<Object>())
                .catch(&ctx)
                .map_err(|e| match e.to_string() {
                    s if s.contains("WouldBlock") || s.contains("would block") => {
                        "its top level waits on the app: do that in an effect (useEffect), not as it loads"
                            .to_owned()
                    }
                    s => s,
                })?;
            let component: Value = ns.get("default").map_err(|e| e.to_string())?;
            let arch: Object = ctx.globals().get("__architect").map_err(|e| e.to_string())?;
            let mount: Function = arch.get("mount").map_err(|e| e.to_string())?;
            mount
                .call::<_, ()>((component,))
                .catch(&ctx)
                .map_err(|e| e.to_string())
        });
        engine.budget.disarm();
        if let Err(why) = mounted {
            let errors = files.errors.lock().unwrap_or_else(PoisonError::into_inner);
            let why = errors.first().map_or_else(
                || files.said(&engine.budget.explain(why)),
                ToString::to_string,
            );
            drop(errors);
            return Err(why);
        }
        engine.drain();
        let snapshot = services.host.theme_snapshot();
        engine.theme(&snapshot)?;
        Ok(engine)
    }

    /// `__host` and the runtime, in the fresh context.
    fn install(&self, files: &Files) -> Result<(), String> {
        let services = self.services.clone();
        let workers = Rc::clone(&self.workers);
        self.ctx.with(|ctx| -> Result<(), String> {
            let host = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            bind(&ctx, &host, &services, &workers, files).map_err(|e| e.to_string())?;
            ctx.globals()
                .set("__host", host)
                .map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(PRELUDE)
                .catch(&ctx)
                .map_err(|e| format!("the page runtime did not start: {e}"))
        })
    }

    /// Calls `__architect[name](args)` within a frame's budget, then what
    /// it queued; what it answered, if a string.
    fn call<A>(&self, name: &str, args: A) -> Result<Option<String>, String>
    where
        A: for<'js> rquickjs::function::IntoArgs<'js>,
    {
        self.budget.arm(self.services.limits.frame);
        let out = self.ctx.with(|ctx| -> Result<Option<String>, String> {
            let arch: Object = ctx
                .globals()
                .get("__architect")
                .map_err(|e| e.to_string())?;
            let f: Function = arch.get(name).map_err(|e| e.to_string())?;
            let v: Value = f.call(args).catch(&ctx).map_err(|e| e.to_string())?;
            Ok(v.as_string().and_then(|s| s.to_string().ok()))
        });
        let out = out.map_err(|e| self.files.said(&self.budget.explain(e)));
        self.drain();
        self.budget.disarm();
        out
    }

    /// Runs the promise jobs that are due (an answer's `then`).
    fn drain(&self) {
        loop {
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => return,
                Err(e) => {
                    let why = e.0.with(|ctx| {
                        let caught = ctx.catch();
                        caught
                            .as_exception()
                            .map(|x| x.message().unwrap_or_default())
                            .or_else(|| caught.as_string().and_then(|s| s.to_string().ok()))
                            .unwrap_or_else(|| "a promise job failed".to_owned())
                    });
                    let why = self.files.said(&self.budget.explain(why));
                    self.services
                        .log
                        .line(&self.services.extension, "error", &why);
                    if self.services.stop.is_set() {
                        return;
                    }
                }
            }
        }
    }
}

impl Engine for JsEngine {
    fn render(&mut self) -> Result<Vec<Element>, String> {
        let text = self.call("render", ())?.unwrap_or_default();
        serde_json::from_str(&text).map_err(|e| format!("what it rendered is not a tree: {e}"))
    }

    fn event(&mut self, event: PageEvent) -> Result<(), String> {
        let text = serde_json::to_string(&event).map_err(|e| e.to_string())?;
        self.call("event", (text,)).map(drop)
    }

    fn answer(&mut self, call: u32, result: Result<String, String>) -> Result<(), String> {
        let (ok, text) = match result {
            Ok(t) => (true, t),
            Err(e) => (false, e),
        };
        self.call("answer", (call, ok, text)).map(drop)
    }

    fn shared(&mut self, event: &DataEvent) -> Result<(), String> {
        self.call("shared", (event.topic.clone(), event.value.clone()))
            .map(drop)
    }

    fn worker(&mut self, message: &WorkerMessage) -> Result<(), String> {
        if message.kind != "message" {
            self.workers.borrow_mut().remove(&message.worker);
        }
        self.call(
            "worker",
            (message.worker, message.kind.clone(), message.value.clone()),
        )
        .map(drop)
    }

    fn theme(&mut self, theme: &ThemeSnapshot) -> Result<(), String> {
        let text = serde_json::to_string(theme).map_err(|e| e.to_string())?;
        self.call("setTheme", (text,)).map(drop)
    }

    fn tick(&mut self) -> Result<Option<Duration>, String> {
        let ms = {
            self.budget.arm(self.services.limits.frame);
            let out = self.ctx.with(|ctx| -> Result<f64, String> {
                let arch: Object = ctx
                    .globals()
                    .get("__architect")
                    .map_err(|e| e.to_string())?;
                let f: Function = arch.get("tick").map_err(|e| e.to_string())?;
                f.call::<_, f64>(()).catch(&ctx).map_err(|e| e.to_string())
            });
            let out = out.map_err(|e| self.files.said(&self.budget.explain(e)));
            self.drain();
            self.budget.disarm();
            out?
        };
        Ok((ms >= 0.0).then(|| Duration::from_secs_f64(ms / 1000.0)))
    }

    fn dirty(&self) -> bool {
        self.ctx.with(|ctx| {
            ctx.globals()
                .get::<_, Object>("__architect")
                .and_then(|a| a.get::<_, Function>("dirty"))
                .and_then(|f| f.call::<_, bool>(()))
                .unwrap_or(false)
        })
    }
}

/// `__host`'s functions: the page runtime's way to the app.
fn bind<'js>(
    ctx: &Ctx<'js>,
    host: &Object<'js>,
    services: &Services,
    workers: &Workers,
    files: &Files,
) -> rquickjs::Result<()> {
    let root = files.root.as_path();
    let next = Rc::new(Cell::new(0_u32));
    {
        let services = services.clone();
        host.set(
            "call",
            Function::new(ctx.clone(), move |kind: String, a: String, b: String| {
                let id = next.get();
                next.set(id.wrapping_add(1));
                start_call(&services, id, kind, a, b);
                id
            })?,
        )?;
    }
    {
        let services = services.clone();
        host.set(
            "sync",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>,
                      kind: String,
                      a: String,
                      b: String|
                      -> rquickjs::Result<Option<String>> {
                    let h = &services.host;
                    match kind.as_str() {
                        "shared" => Ok(Some(h.shared_json(&a))),
                        "embed" => Ok(Some(services.embed.as_ref().map_or_else(
                            || "null".to_owned(),
                            |cell| {
                                let now = cell.lock().unwrap_or_else(PoisonError::into_inner);
                                serde_json::to_string(&*now).unwrap_or_else(|_| "null".to_owned())
                            },
                        ))),
                        "set_shared" => h
                            .set_shared_json(&a, &b)
                            .map(|()| None)
                            .map_err(|e| rquickjs::Exception::throw_message(&ctx, &e.to_string())),
                        "notify" => {
                            h.show(&Notice { text: a, kind: b });
                            Ok(None)
                        }
                        other => Err(rquickjs::Exception::throw_message(
                            &ctx,
                            &format!("no host call {other}"),
                        )),
                    }
                },
            )?,
        )?;
    }
    {
        let (services, files) = (services.clone(), files.clone());
        host.set(
            "log",
            Function::new(ctx.clone(), move |level: String, text: String| {
                // What it reports (a handler's throw, `console.error(e)`)
                // says its source's places.
                let text = if level == "error" {
                    files.said(&text)
                } else {
                    text
                };
                services.log.line(&services.extension, &level, &text);
            })?,
        )?;
    }
    let started = Instant::now();
    host.set(
        "now",
        Function::new(ctx.clone(), move || {
            started.elapsed().as_secs_f64() * 1000.0
        })?,
    )?;
    let next_worker = Rc::new(Cell::new(1_u32));
    {
        let (services, workers) = (services.clone(), Rc::clone(workers));
        let root = root.to_path_buf();
        host.set(
            "spawn",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, file: String, args: String| -> rquickjs::Result<u32> {
                    let id = next_worker.get();
                    next_worker.set(id.wrapping_add(1));
                    let link = spawn_worker(&services, &root, id, &file, args)
                        .map_err(|e| rquickjs::Exception::throw_message(&ctx, &e))?;
                    workers.borrow_mut().insert(id, link);
                    Ok(id)
                },
            )?,
        )?;
    }
    {
        let workers = Rc::clone(workers);
        host.set(
            "post",
            Function::new(ctx.clone(), move |id: u32, text: String| {
                if let Some(link) = workers.borrow().get(&id) {
                    let _ = link.to.send(text);
                }
            })?,
        )?;
    }
    {
        let workers = Rc::clone(workers);
        host.set(
            "terminate",
            Function::new(ctx.clone(), move |id: u32| {
                if let Some(link) = workers.borrow_mut().remove(&id) {
                    link.cancel.set();
                }
            })?,
        )?;
    }
    Ok(())
}

/// Starts host call `id` off the page's thread; its answer goes to the
/// page's inbox.
fn start_call(services: &Services, id: u32, kind: String, a: String, b: String) {
    let (host, inbox) = (Arc::clone(&services.host), services.inbox.clone());
    vm::spawn(move || {
        let result = futures::executor::block_on(async {
            match kind.as_str() {
                "run" => host.run(a, b).await,
                "actions" => host
                    .actions(a)
                    .await
                    .and_then(|v| serde_json::to_string(&v).map_err(|e| failed(&e))),
                "setting" => host.setting(a).await,
                "set_setting" => host.set_setting(a, b).await.map(|()| String::new()),
                "host" => host.host_call(a).await,
                other => Err(crate::api::PageError::Unknown(format!(
                    "no host call {other}"
                ))),
            }
        });
        let _ = inbox.send(Inbox::Answer {
            call: id,
            result: result.map_err(|e| e.to_string()),
        });
    });
}

fn failed(e: &impl std::fmt::Display) -> crate::api::PageError {
    crate::api::PageError::Failed(e.to_string())
}

/// Starts `file` (in the page's folder) as worker `id` on a thread of its
/// own, with `args` (JSON).
fn spawn_worker(
    services: &Services,
    root: &Path,
    id: u32,
    file: &str,
    args: String,
) -> Result<WorkerLink, String> {
    let (to, from) = mpsc::channel::<String>();
    let cancel = Cancel::default();
    let services = services.clone();
    let mine = cancel.clone();
    let file = file.to_owned();
    let root = root.to_path_buf();
    std::thread::Builder::new()
        .name(format!("page worker {file}"))
        .spawn(move || {
            let tell = |kind: &str, value: String| {
                let _ = services.inbox.send(Inbox::Worker(WorkerMessage {
                    worker: id,
                    kind: kind.to_owned(),
                    value,
                }));
            };
            match run_worker(&services, &root, &file, &args, id, &from, &mine) {
                Ok(result) => tell("done", result),
                Err(why) => tell("failed", serde_json::Value::String(why).to_string()),
            }
        })
        .map_err(|e| format!("its thread did not start: {e}"))?;
    Ok(WorkerLink { to, cancel })
}

/// A worker's whole run, on its thread: its result as JSON.
fn run_worker(
    services: &Services,
    root: &Path,
    file: &str,
    args: &str,
    id: u32,
    messages: &mpsc::Receiver<String>,
    cancel: &Cancel,
) -> Result<String, String> {
    let budget = Budget::new(vec![cancel.clone(), services.stop.clone()]);
    let files = Files::new(root, true);
    let (rt, ctx) = runtime(services.limits.worker_memory, &budget, &files)?;
    budget.arm(services.limits.worker_time);
    let path = files
        .inside(&root.join(file))
        .ok_or_else(|| format!("{file}: no such file in the page's folder"))?
        .display()
        .to_string();
    let out = ctx.with(|ctx| -> Result<(String, bool), String> {
        let host = Object::new(ctx.clone()).map_err(|e| e.to_string())?;
        let (log, ext) = (services.log.clone(), services.extension.clone());
        let inbox = services.inbox.clone();
        let set = || -> rquickjs::Result<()> {
            host.set("args", args)?;
            host.set(
                "log",
                Function::new(ctx.clone(), move |level: String, text: String| {
                    log.line(&ext, &level, &format!("worker {id}: {text}"));
                })?,
            )?;
            host.set(
                "post",
                Function::new(ctx.clone(), move |text: String| {
                    let _ = inbox.send(Inbox::Worker(WorkerMessage {
                        worker: id,
                        kind: "message".to_owned(),
                        value: text,
                    }));
                })?,
            )?;
            ctx.globals().set("__host", host.clone())
        };
        set().map_err(|e| e.to_string())?;
        ctx.eval::<(), _>(WORKER_PRELUDE)
            .catch(&ctx)
            .map_err(|e| e.to_string())?;
        let ns: Object = Module::import(&ctx, path.clone())
            .and_then(|p| p.finish::<Object>())
            .catch(&ctx)
            .map_err(|e| e.to_string())?;
        let main: Value = ns.get("default").map_err(|e| e.to_string())?;
        let api: Object = ctx
            .globals()
            .get::<_, Object>("__worker")
            .and_then(|w| w.get("api"))
            .map_err(|e| e.to_string())?;
        let given: Value = api.get("args").map_err(|e| e.to_string())?;
        let result: Value = match main.as_function() {
            Some(f) => {
                let v: Value = f.call((given,)).catch(&ctx).map_err(|e| e.to_string())?;
                match v.as_promise() {
                    Some(p) => p.finish::<Value>().catch(&ctx).map_err(|e| e.to_string())?,
                    None => v,
                }
            }
            None => main,
        };
        let text = ctx
            .json_stringify(result)
            .map_err(|e| e.to_string())?
            .and_then(|s| s.to_string().ok())
            .unwrap_or_else(|| "null".to_owned());
        let serving = ctx
            .globals()
            .get::<_, Object>("__worker")
            .and_then(|w| w.get::<_, Function>("serving"))
            .and_then(|f| f.call::<_, bool>(()))
            .unwrap_or(false);
        Ok((text, serving))
    });
    let (result, serving) = out.map_err(|e| files.said(&budget.explain(e)))?;
    if serving {
        // It serves its page's messages until it is told to stop.
        while !cancel.is_set() && !services.stop.is_set() {
            match messages.recv_timeout(Duration::from_millis(100)) {
                Ok(text) => {
                    let handled = ctx.with(|ctx| {
                        ctx.globals()
                            .get::<_, Object>("__worker")
                            .and_then(|w| w.get::<_, Function>("message"))
                            .and_then(|f| f.call::<_, ()>((text,)))
                            .catch(&ctx)
                            .map_err(|e| e.to_string())
                    });
                    if let Err(why) = handled {
                        return Err(files.said(&budget.explain(why)));
                    }
                    while rt.execute_pending_job().unwrap_or(false) {}
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
    drop(ctx);
    Ok(result)
}
