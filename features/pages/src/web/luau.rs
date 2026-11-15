//! Luau extensions in the web app's host (#812): the extensions host's
//! runner for `kind = "luau"` in a browser, [`WebLuauRunner`].
//!
//! Luau's VM is luau-web's emscripten build (`apps/web/assets/luau-web`,
//! `scripts/vendor-luau-web.sh`), fetched only when a Luau extension
//! first runs. Each action or hook runs in a Web Worker of its own
//! (`web/luau.js`) with the globals of `web/luau_runtime.luau`; every
//! global it calls is a message answered here, on the page, by the one
//! implementation a desk's runtime calls too:
//!
//! - `host.*`: [`architect_extensions::luau_calls::host_request`], then
//!   the app's host service for the extension ([`WebHost::through`] its
//!   `ExtensionApi`), checked against what the person granted;
//! - `ext.*`: [`architect_extensions::luau_calls::ext_call`] on the
//!   host's contributions;
//! - `actions.*` and `state.query`: [`architect_scripting::bindings`], as
//!   the extension for whoever ran it, inside its gate.
//!
//! A call waits by suspending the script (Asyncify), so nothing needs
//! `Atomics.wait` or cross-origin isolation. Its limits are a TypeScript
//! action's ([`super::Limits`]): loading within `load`, each unit of work
//! between host calls within `frame`, a worker within `worker_time`; past
//! one the Web Worker is terminated. A browser cannot cap its memory.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionList, CallerAsyncHandler, Principal,
};
use architect_extensions::contrib::{Contributions, HookDecl};
use architect_extensions::host::{
    Context, Gate, Instance, Runner, capped_answer, capped_error, json_args,
};
use architect_extensions::hostapi::ExtensionApi;
use architect_extensions::hostcall::HostAnswer;
use architect_extensions::luau_calls::{
    EXT_FUNCTIONS, HOST_FUNCTIONS, ext_call, host_request, raw_request,
};
use architect_extensions::manifest::{ActionDecl, Kind, Manifest};
use architect_extensions::package::Package;
use architect_extensions::web::WebHost;
use architect_scripting::ErrorKind;
use architect_scripting::bindings;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use super::{Limits, WebPages, from_js, millis, out_of_time, to_js};

/// The Web Worker one run is in.
const LUAU_JS: &str = include_str!("luau.js");
/// Its globals (sent with each run's start).
const LUAU_RUNTIME: &str = include_str!("luau_runtime.luau");

/// What one run needs, sent to the page's thread.
struct Job {
    lib: Arc<str>,
    run: Run,
    reply: oneshot::Sender<Result<String, String>>,
}

/// One run of a script: as whom, with what.
struct Run {
    /// Its name in errors and the log (the action's id, `<name> hook <on>`).
    what: String,
    source: Arc<str>,
    args: Json,
    ctx: Json,
    workers: Arc<BTreeMap<String, String>>,
    manifest: Manifest,
    list: ActionList,
    gate: Gate,
    contributions: Contributions,
    api: ExtensionApi,
    /// The caller its `actions` run as (the extension, for whoever ran it,
    /// as a script).
    caller: ActionCaller,
}

/// The web app's runner for Luau extensions: their actions and hooks in
/// Web Workers, on luau-web's Luau.
#[derive(Clone)]
pub struct WebLuauRunner {
    jobs: mpsc::UnboundedSender<Job>,
    lib: Arc<str>,
}

impl std::fmt::Debug for WebLuauRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebLuauRunner")
            .field("lib", &self.lib)
            .finish()
    }
}

impl WebLuauRunner {
    /// Runs with `pages`' limits, loading Luau from `lib` (luau-web's
    /// `index.js`, an absolute URL on this origin). Call it on the page's
    /// thread.
    #[must_use]
    pub fn new(pages: WebPages, lib: impl Into<String>) -> Self {
        let lib: Arc<str> = Arc::from(lib.into());
        let (jobs, mut rx) = mpsc::unbounded::<Job>();
        spawn_local(async move {
            let script: Rc<RefCell<Option<String>>> = Rc::default();
            while let Some(job) = rx.next().await {
                let (limits, script) = (pages.limits(), Rc::clone(&script));
                spawn_local(async move {
                    let out = match worker_url(&script) {
                        Ok(url) => run(&url, &job.lib, job.run, limits).await,
                        Err(e) => Err(e),
                    };
                    let _ = job.reply.send(out);
                });
            }
        });
        Self { jobs, lib }
    }
}

/// The run's Web Worker script, as a `blob:` URL: made once.
fn worker_url(made: &RefCell<Option<String>>) -> Result<String, String> {
    if let Some(url) = made.borrow().clone() {
        return Ok(url);
    }
    let parts = js_sys::Array::of1(&JsValue::from_str(LUAU_JS));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
        .map_err(|e| format!("{e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
    *made.borrow_mut() = Some(url.clone());
    Ok(url)
}

impl Runner for WebLuauRunner {
    fn kind(&self) -> Kind {
        Kind::Luau
    }

    fn start(&self, package: &Package, cx: Context<'_>) -> Result<Box<dyn Instance>, String> {
        let manifest = &package.manifest;
        let read = |path: &str| package.text(path).map_err(|e| e.to_string());
        let mut scripts = BTreeMap::new();
        for decl in &manifest.actions {
            if let Some(path) = &decl.script {
                scripts.insert(decl.id.clone(), Arc::<str>::from(read(path)?));
            }
        }
        let mut hooks = BTreeMap::new();
        for decl in &manifest.contributes.hooks {
            if let Some(path) = &decl.script {
                hooks.insert(path.clone(), Arc::<str>::from(read(path)?));
            }
        }
        let mut workers = BTreeMap::new();
        for decl in &manifest.workers {
            if let Some(path) = &decl.script {
                workers.insert(decl.id.clone(), read(path)?);
            }
        }
        Ok(Box::new(LuauInstance(Arc::new(Shared {
            jobs: self.jobs.clone(),
            lib: Arc::clone(&self.lib),
            scripts,
            hooks,
            workers: Arc::new(workers),
            manifest: manifest.clone(),
            list: cx.list.clone(),
            gate: cx.gate,
            contributions: cx.contributions.clone(),
            api: cx.api,
        }))))
    }
}

struct Shared {
    jobs: mpsc::UnboundedSender<Job>,
    lib: Arc<str>,
    scripts: BTreeMap<String, Arc<str>>,
    hooks: BTreeMap<String, Arc<str>>,
    workers: Arc<BTreeMap<String, String>>,
    manifest: Manifest,
    list: ActionList,
    gate: Gate,
    contributions: Contributions,
    api: ExtensionApi,
}

/// A loaded Luau extension.
struct LuauInstance(Arc<Shared>);

impl Shared {
    /// Runs `source` as `what` for `caller` with `args`.
    fn run(
        &self,
        what: String,
        source: Arc<str>,
        caller: &ActionCaller,
        args: Json,
    ) -> ActionFuture {
        // As a desk's `Luau::for_run`: `host` acts for whoever ran it; its
        // actions run as the extension, for them, as a script.
        let api = self.api.clone().acting_for(caller.principal().cloned());
        let who: Principal = api.principal();
        let caller = caller.clone().with_principal(who);
        let ctx = architect_extensions::host::invocation_json(caller.invocation());
        let (reply, answer) = oneshot::channel();
        let sent = self.jobs.unbounded_send(Job {
            lib: Arc::clone(&self.lib),
            run: Run {
                what: what.clone(),
                source,
                args,
                ctx,
                workers: Arc::clone(&self.workers),
                manifest: self.manifest.clone(),
                list: self.list.clone(),
                gate: Arc::clone(&self.gate),
                contributions: self.contributions.clone(),
                api,
                caller: bindings::as_script(&caller),
            },
            reply,
        });
        Box::pin(async move {
            let failed = |message: String| ActionError::Failed {
                id: what.clone(),
                message: capped_error(message),
            };
            sent.map_err(|_| failed("the browser's Luau runner stopped".to_owned()))?;
            let text = answer
                .await
                .map_err(|_| failed("it was unloaded".to_owned()))?
                .map_err(failed)?;
            capped_answer(&what, text.into_bytes())
        })
    }
}

impl Instance for LuauInstance {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let source = self
            .0
            .scripts
            .get(&decl.id)
            .cloned()
            .ok_or_else(|| format!("{} has no script", decl.id))?;
        let me = Arc::clone(&self.0);
        let id = decl.id.clone();
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let args = json_args(&id, args)?;
            Ok(me.run(id.clone(), Arc::clone(&source), caller, args))
        }))
    }

    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Json) -> Option<ActionFuture> {
        let path = decl.script.as_ref()?;
        let file = self.0.hooks.get(path)?;
        // The whole file, or one function of the table it returns (a
        // desk's words for it).
        let source: Arc<str> = decl.function.as_ref().map_or_else(
            || Arc::clone(file),
            |f| {
                Arc::from(format!(
                    "local __hooks = (function(...) {file}\nend)(...)\nreturn __hooks[{f:?}](args)"
                ))
            },
        );
        let what = format!("{} hook {}", self.0.manifest.name, decl.on);
        Some(self.0.run(what, source, caller, args))
    }
}

/// One run's Web Worker, its limits and its answer's way back.
struct Running {
    worker: web_sys::Worker,
    run: Run,
    host: WebHost,
    limits: Limits,
    /// What the timer now armed is for: loading, a unit of work, a worker.
    armed: Cell<Duration>,
    timer: Cell<Option<i32>>,
    in_worker: Cell<bool>,
    done: RefCell<Option<oneshot::Sender<Result<String, String>>>>,
    on_time: RefCell<Option<Closure<dyn FnMut()>>>,
    on_message: RefCell<Option<Closure<dyn FnMut(web_sys::MessageEvent)>>>,
}

impl Running {
    fn arm(&self, limit: Duration) {
        self.disarm();
        self.armed.set(limit);
        let Some(f) = self.on_time.borrow().as_ref().map(|c| c.as_ref().clone()) else {
            return;
        };
        let id = web_sys::window().and_then(|w| {
            w.set_timeout_with_callback_and_timeout_and_arguments_0(
                f.unchecked_ref(),
                millis(limit),
            )
            .ok()
        });
        self.timer.set(id);
    }

    fn disarm(&self) {
        if let (Some(id), Some(w)) = (self.timer.take(), web_sys::window()) {
            w.clear_timeout_with_handle(id);
        }
    }

    /// Its unit of work's limit: a worker's while one runs.
    fn unit(&self) -> Duration {
        if self.in_worker.get() {
            self.limits.worker_time
        } else {
            self.limits.frame
        }
    }

    /// It is over: its worker goes, and its answer (or why) is said.
    fn end(&self, out: Result<String, String>) {
        self.disarm();
        self.worker.terminate();
        if let Err(why) = &out {
            self.host
                .log("error", "script", format!("{}: {why}", self.run.what));
        }
        if let Some(tx) = self.done.borrow_mut().take() {
            let _ = tx.send(out);
        }
        let on_message = self.on_message.borrow_mut().take();
        let on_time = self.on_time.borrow_mut().take();
        spawn_local(async move {
            drop((on_message, on_time));
        });
    }

    fn message(self: &Rc<Self>, m: &Json) {
        let text = |k: &str| {
            m.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match m.get("t").and_then(Json::as_str).unwrap_or_default() {
            "loaded" => self.arm(self.unit()),
            "worker" => {
                self.in_worker
                    .set(m.get("on").and_then(Json::as_bool).unwrap_or(false));
                self.arm(self.unit());
            }
            "log" => self.host.log("info", "print", text("text")),
            "call" => {
                // Waiting on the page is not the script's time.
                self.disarm();
                let (id, g, name) = (
                    m.get("id").cloned().unwrap_or(Json::Null),
                    text("g"),
                    text("name"),
                );
                let args: Vec<Json> = serde_json::from_str(&text("args")).unwrap_or_default();
                let me = Rc::clone(self);
                spawn_local(async move {
                    let (ok, text) = match me.answer(&g, &name, &args).await {
                        Ok(v) => (true, v.to_string()),
                        Err(e) => (false, e),
                    };
                    if me.done.borrow().is_none() {
                        return;
                    }
                    me.arm(me.unit());
                    let _ = me.worker.post_message(&to_js(
                        &json!({ "t": "answer", "id": id, "ok": ok, "text": text }),
                    ));
                });
            }
            "done" => {
                let said = text("text");
                if m.get("ok").and_then(Json::as_bool).unwrap_or(false) {
                    self.end(Ok(said));
                } else {
                    // `script:3: …` as a desk says it: `line 3: …`.
                    self.end(Err(bindings::located(ErrorKind::Runtime, &said).to_string()));
                }
            }
            _ => {}
        }
    }

    /// One global's call, answered as a desk's runtime answers it.
    async fn answer(&self, group: &str, name: &str, args: &[Json]) -> Result<Json, String> {
        let run = &self.run;
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Json::Null);
        match group {
            "host" if name == "call" => {
                let answer = match raw_request(arg(0)) {
                    Ok(request) => HostAnswer::from(self.host.call(request).await),
                    Err(message) => {
                        HostAnswer::Ok(json!({ "error": { "kind": "failed", "message": message } }))
                    }
                };
                serde_json::to_value(&answer).map_err(|e| e.to_string())
            }
            "host" if HOST_FUNCTIONS.contains(&name) => {
                let request = host_request(name, args)?;
                self.host.call(request).await.map_err(|e| e.to_string())
            }
            "ext" if EXT_FUNCTIONS.contains(&name) => ext_call(
                &run.manifest,
                &run.contributions,
                name,
                args,
                architect_extensions::host::now_ms(),
            ),
            "actions" | "state" => {
                let gate = Some(&run.gate);
                let id = arg(0);
                let id = id.as_str().unwrap_or_default();
                match (group, name) {
                    ("actions", "list") => Ok(bindings::list(
                        &run.list,
                        &run.caller,
                        gate,
                        arg(0).as_str(),
                    )),
                    ("actions", "describe") => {
                        Ok(bindings::describe(&run.list, &run.caller, gate, id))
                    }
                    ("actions", "enabled") => {
                        let (on, why) = bindings::enabled(&run.list, &run.caller, gate, id);
                        Ok(json!([on, why]))
                    }
                    ("actions", "run") | ("state", "query") => {
                        let args = Some(arg(1)).filter(|a| !a.is_null());
                        bindings::run(
                            &run.list,
                            &run.caller,
                            gate,
                            id,
                            args.as_ref(),
                            group == "state",
                        )
                        .await
                    }
                    _ => Err(format!("{group} has no function {name}")),
                }
            }
            _ => Err(format!("{group} has no function {name}")),
        }
    }
}

/// Runs `run` in a Web Worker from `script`, Luau from `lib`, within
/// `limits`: what its script returned, as JSON, or why it stopped.
async fn run(script: &str, lib: &str, run: Run, limits: Limits) -> Result<String, String> {
    let options = web_sys::WorkerOptions::new();
    options.set_type(web_sys::WorkerType::Module);
    let worker = web_sys::Worker::new_with_options(script, &options)
        .map_err(|e| format!("its Web Worker did not start: {e:?}"))?;
    let host = WebHost::new(run.manifest.clone(), run.list.clone()).through(run.api.clone());
    let start = json!({
        "t": "start",
        "lib": lib,
        "prelude": LUAU_RUNTIME,
        "source": &*run.source,
        "args": run.args.to_string(),
        "ctx": run.ctx.to_string(),
        "id": run.manifest.id,
        "host": HOST_FUNCTIONS,
        "ext": EXT_FUNCTIONS,
        "workers": &*run.workers,
    });
    let (tx, rx) = oneshot::channel();
    let running = Rc::new(Running {
        worker,
        run,
        host,
        limits,
        armed: Cell::new(limits.load),
        timer: Cell::new(None),
        in_worker: Cell::new(false),
        done: RefCell::new(Some(tx)),
        on_time: RefCell::new(None),
        on_message: RefCell::new(None),
    });
    let weak = Rc::downgrade(&running);
    *running.on_message.borrow_mut() = Some(Closure::new(move |e: web_sys::MessageEvent| {
        if let Some(r) = weak.upgrade() {
            r.message(&from_js(&e.data()));
        }
    }));
    let weak = Rc::downgrade(&running);
    *running.on_time.borrow_mut() = Some(Closure::new(move || {
        if let Some(r) = weak.upgrade() {
            r.end(Err(out_of_time(r.armed.get())));
        }
    }));
    if let Some(f) = running.on_message.borrow().as_ref() {
        running
            .worker
            .set_onmessage(Some(f.as_ref().unchecked_ref()));
    }
    running.arm(limits.load);
    let _ = running.worker.post_message(&to_js(&start));
    let out = rx
        .await
        .unwrap_or_else(|_| Err("its run stopped before it answered".to_owned()));
    drop(running);
    out
}
