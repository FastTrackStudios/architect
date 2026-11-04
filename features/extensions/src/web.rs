//! WASI component extensions in a browser (feature `web`, on
//! `wasm32-unknown-unknown`; `docs/content/architecture/extensions.md`,
//! "WASI components", "On the web").
//!
//! Wasmtime does not run in a browser, so a component is transpiled ahead
//! of time by jco (`features/extensions/web/transpile.mjs`) into core wasm
//! and an `instantiate` function, and the browser's own WebAssembly runs
//! it. The JS side (`features/extensions/web/runner.js`, linked in by
//! wasm-bindgen; `instance.js`, each instance's Web Worker) does what the
//! native runner's actor does: one call at a time, its limits, starting it
//! again after a trap, its workers and their queues. This module is the
//! rest of the host:
//!
//! - [`WebHost`] answers the world's host imports — every one arrives as a
//!   host-service request (`hostcall::HostRequest`) — after the decision
//!   every host makes ([`crate::hostcall::check`]: its manifest's
//!   capabilities, as a person granted them), running app actions on the
//!   app's [`ActionList`] as the extension;
//! - [`WebComponent`] starts one, runs its actions
//!   ([`WebComponent::register_actions`] puts them on the app's list) and
//!   mounts its views ([`WebComponent::mount`]): a view's edits are the
//!   remote-view wire's (`wire::Edit`), applied to the page's DOM under
//!   the pane's element, its events sent back as `wire::ViewEvent`s.

// Nothing is `Send` in a browser: its futures are spawned on the page's
// one thread (`spawn_local`).
#![allow(clippy::future_not_send)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use architect::action::{
    ActionCaller, ActionError, ActionFuture, ActionInfo, ActionList, Effect, Invocation,
    InvocationSource, Principal,
};
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

use crate::hostcall::{HostAnswer, HostError, HostRequest};
use crate::manifest::{ActionDecl, Capabilities, Manifest, Reach};
use crate::wire::{Edit, Value};

#[wasm_bindgen(module = "/web/runner.js")]
extern "C" {
    #[wasm_bindgen(js_name = Component)]
    type JsComponent;

    #[wasm_bindgen(constructor, js_class = "Component")]
    fn new(opts: &JsValue) -> JsComponent;
    #[wasm_bindgen(method)]
    fn start(this: &JsComponent, state: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn invoke(this: &JsComponent, action: &str, args: &str, invocation: &str) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn render(this: &JsComponent, pane: f64, view: &str) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn event(this: &JsComponent, pane: f64, event: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn close(this: &JsComponent, pane: f64) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn stop(this: &JsComponent);
    #[wasm_bindgen(method)]
    fn report(this: &JsComponent) -> JsValue;
}

/// An instance's Web Worker script: given to the runner as its source, so
/// the embedder serves only the transpiled component.
const INSTANCE_JS: &str = include_str!("../web/instance.js");

/// What one instance may use (the native `wasi::Limits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Its linear memories, in bytes, at most: written into its core
    /// modules as their maximum, so past it `memory.grow` fails.
    pub memory: usize,
    /// One call's own time, at most (its waits on the host aside): past it
    /// its Web Worker is terminated.
    pub call: Duration,
}

impl Default for Limits {
    /// 64 MiB, and 5 s a call.
    fn default() -> Self {
        Self {
            memory: 64 << 20,
            call: Duration::from_secs(5),
        }
    }
}

impl Limits {
    /// A worker's: 64 MiB, and 60 s of its own time for its whole run.
    #[must_use]
    pub const fn worker() -> Self {
        Self {
            memory: 64 << 20,
            call: Duration::from_secs(60),
        }
    }

    fn js(self) -> JsValue {
        let o = js_sys::Object::new();
        set(&o, "memory", &JsValue::from_f64(lossy(self.memory)));
        set(
            &o,
            "call",
            &JsValue::from_f64(lossy(
                usize::try_from(self.call.as_millis()).unwrap_or(usize::MAX),
            )),
        );
        o.into()
    }
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)] // Sizes far below 2^53.
const fn lossy(n: usize) -> f64 {
    n as f64
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)] // An id the runner made from a count.
const fn id_of(n: f64) -> u64 {
    n as u64
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)] // Pane and element ids: counts.
const fn num(n: u64) -> f64 {
    n as f64
}

fn set(o: &js_sys::Object, key: &str, value: &JsValue) {
    let _ = js_sys::Reflect::set(o, &JsValue::from_str(key), value);
}

fn get(o: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(o, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// A JS value as JSON.
fn to_json(v: &JsValue) -> Json {
    js_sys::JSON::stringify(v)
        .ok()
        .and_then(|s| s.as_string())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Json::Null)
}

/// JSON as a JS value.
fn from_json(v: &Json) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

/// What a rejected promise said.
fn why(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| get(e, "message").as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

/// A line in an extension's log, as `EXT_LOGS` shows one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// `debug`, `info`, `warn` or `error`.
    pub level: String,
    /// Where it came from: `log`, `stdout`, `stderr`, `host`, a worker.
    pub source: String,
    pub text: String,
}

type LogSink = Rc<dyn Fn(&LogLine)>;

/// The host service in a browser: what one extension may ask the app for,
/// each call checked against what it may reach.
///
/// The same decisions as `hostapi::ExtensionApi` (natively), from the
/// same [`crate::hostcall::check`]. What a browser host does not have it
/// says so: no documents or files (`open`, `read_document`, `read_file`,
/// `write_file` fail after the check), settings and data in memory, the
/// organization and selection from [`WebHost::with_context`].
#[derive(Clone)]
pub struct WebHost {
    inner: Rc<HostInner>,
}

struct HostInner {
    manifest: Manifest,
    may: Capabilities,
    list: ActionList,
    settings: RefCell<BTreeMap<String, Json>>,
    data: RefCell<BTreeMap<String, Json>>,
    logs: RefCell<Vec<LogLine>>,
    sink: RefCell<Option<LogSink>>,
    context: RefCell<Option<Rc<dyn Fn(&str) -> Option<Json>>>>,
    theme: RefCell<Json>,
}

impl std::fmt::Debug for WebHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebHost")
            .field("id", &self.inner.manifest.id)
            .finish_non_exhaustive()
    }
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

fn default_of(manifest: &Manifest, key: &str) -> Option<Json> {
    manifest
        .sections
        .iter()
        .flat_map(|s| &s.settings)
        .find(|s| s.key == key)
        .map(|s| serde_json::to_value(&s.default).unwrap_or(Json::Null))
}

impl WebHost {
    /// `manifest`'s extension, reaching what its manifest asks, its app
    /// actions run on `list`.
    #[must_use]
    pub fn new(manifest: Manifest, list: ActionList) -> Self {
        let may = manifest.capabilities.clone();
        Self {
            inner: Rc::new(HostInner {
                manifest,
                may,
                list,
                settings: RefCell::new(BTreeMap::new()),
                data: RefCell::new(BTreeMap::new()),
                logs: RefCell::new(Vec::new()),
                sink: RefCell::new(None),
                context: RefCell::new(None),
                theme: RefCell::new(Json::Null),
            }),
        }
    }

    /// What a person granted it (`consent`): it reaches what its manifest
    /// asks within that.
    #[must_use]
    pub fn with_granted(self, granted: &Capabilities) -> Self {
        let may = self.inner.manifest.capabilities.within(granted);
        let HostInner {
            manifest,
            list,
            settings,
            data,
            logs,
            sink,
            context,
            theme,
            ..
        } = Rc::try_unwrap(self.inner).unwrap_or_else(|rc| HostInner {
            manifest: rc.manifest.clone(),
            may: rc.may.clone(),
            list: rc.list.clone(),
            settings: RefCell::new(rc.settings.borrow().clone()),
            data: RefCell::new(rc.data.borrow().clone()),
            logs: RefCell::new(rc.logs.borrow().clone()),
            sink: RefCell::new(rc.sink.borrow().clone()),
            context: RefCell::new(rc.context.borrow().clone()),
            theme: RefCell::new(rc.theme.borrow().clone()),
        });
        Self {
            inner: Rc::new(HostInner {
                manifest,
                may,
                list,
                settings,
                data,
                logs,
                sink,
                context,
                theme,
            }),
        }
    }

    /// What the app knows that it may ask for: `org` and `selection`
    /// (`f("org")`), each only with its capability.
    #[must_use]
    pub fn with_context(self, f: impl Fn(&str) -> Option<Json> + 'static) -> Self {
        *self.inner.context.borrow_mut() = Some(Rc::new(f));
        self
    }

    /// Each line of its log as it is written, too.
    pub fn on_log(&self, f: impl Fn(&LogLine) + 'static) {
        *self.inner.sink.borrow_mut() = Some(Rc::new(f));
    }

    /// The app's theme as data (`wire::ThemeData`'s fields: `id`,
    /// `scheme`, `revision`, `tokens` of `{name, value}`), for its
    /// `theme` import.
    pub fn set_theme(&self, theme: Json) {
        *self.inner.theme.borrow_mut() = theme;
    }

    /// Its log so far.
    #[must_use]
    pub fn logs(&self) -> Vec<LogLine> {
        self.inner.logs.borrow().clone()
    }

    /// The extension's manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.inner.manifest
    }

    /// A line in its log.
    pub fn log(&self, level: &str, source: &str, text: impl Into<String>) {
        let line = LogLine {
            level: level.to_owned(),
            source: source.to_owned(),
            text: text.into(),
        };
        let sink = self.inner.sink.borrow().clone();
        if let Some(sink) = sink {
            sink(&line);
        }
        self.inner.logs.borrow_mut().push(line);
    }

    fn principal(&self) -> Principal {
        Principal::extension(self.inner.manifest.id.clone(), None)
    }

    /// Answers `request`, checked against what the extension may reach.
    ///
    /// # Errors
    ///
    /// [`HostError::Refused`] outside its capabilities, and what the call
    /// itself met.
    pub async fn call(&self, request: HostRequest) -> Result<Json, HostError> {
        let op = request.op();
        let answer = self.answer(request).await;
        if let Err(e) = &answer {
            let level = if matches!(e, HostError::Refused { .. }) {
                "warn"
            } else {
                "error"
            };
            self.log(level, "host", format!("{op}: {e}"));
        }
        answer
    }

    /// [`Self::call`] from and to JSON text (what the runner sends).
    pub async fn call_json(&self, request: &str) -> String {
        let answer = match serde_json::from_str::<HostRequest>(request) {
            Ok(request) => self.call(request).await,
            Err(e) => Err(HostError::Failed {
                message: format!("not a host request: {e}"),
            }),
        };
        HostAnswer::from(answer).to_json()
    }

    async fn answer(&self, request: HostRequest) -> Result<Json, HostError> {
        let me = &self.inner;
        let effect = match &request {
            HostRequest::Run { action, .. } => me
                .list
                .info(action)
                .map(|i| reach(i.effect))
                .ok_or_else(|| HostError::NotFound {
                    what: format!("action {action}"),
                })?,
            _ => Reach::ReadOnly,
        };
        let (_, check) = crate::hostcall::check(&me.manifest, &me.may, &request, effect);
        check.map_err(|(capability, why)| HostError::Refused { capability, why })?;
        let context = |what: &str| {
            me.context
                .borrow()
                .as_ref()
                .and_then(|f| f(what))
                .unwrap_or(Json::Null)
        };
        match request {
            HostRequest::Run {
                action,
                args,
                ticket,
            } => self.run(&action, &args, ticket).await,
            HostRequest::Org => Ok(context("org")),
            HostRequest::Selection => Ok(context("selection")),
            HostRequest::Open { .. }
            | HostRequest::ReadDocument { .. }
            | HostRequest::ReadFile { .. }
            | HostRequest::WriteFile { .. } => Err(HostError::Failed {
                message: "a browser host has no documents or files to reach".to_owned(),
            }),
            HostRequest::Setting { key } => Ok(me
                .settings
                .borrow()
                .get(&key)
                .cloned()
                .or_else(|| default_of(&me.manifest, &key))
                .unwrap_or(Json::Null)),
            HostRequest::SetSetting { key, value } => {
                if default_of(&me.manifest, &key).is_none() {
                    return Err(HostError::NotFound {
                        what: format!("setting {key} in its manifest"),
                    });
                }
                me.settings.borrow_mut().insert(key.clone(), value);
                me.list.changed("settings", &key);
                Ok(Json::Null)
            }
            HostRequest::Data { key } => {
                Ok(me.data.borrow().get(&key).cloned().unwrap_or(Json::Null))
            }
            HostRequest::SetData { key, value } => {
                let mut data = me.data.borrow_mut();
                match value {
                    Some(v) => data.insert(key, v),
                    None => data.remove(&key),
                };
                Ok(Json::Null)
            }
            HostRequest::Log { text, level } => {
                self.log(level.as_deref().unwrap_or("info"), "log", text);
                Ok(Json::Null)
            }
            // Services, points and windows are the native host's
            // (`crate::interop`); the browser host has no other extensions
            // to call or windows to open yet.
            HostRequest::Service { .. } | HostRequest::Point { .. } | HostRequest::Window { .. } => {
                Err(HostError::Failed {
                    message: "services, points and windows are not in the browser yet".to_owned(),
                })
            }
        }
    }

    async fn run(
        &self,
        action: &str,
        args: &Json,
        ticket: Option<String>,
    ) -> Result<Json, HostError> {
        let mut caller = ActionCaller::new(self.principal())
            .with_invocation(Invocation::new(InvocationSource::Script));
        if let Some(t) = ticket {
            caller = caller.with_ticket(t);
        }
        let bytes = if args.is_null() {
            Vec::new()
        } else {
            args.to_string().into_bytes()
        };
        match self
            .inner
            .list
            .run_json_async_with(&caller, action, &bytes)
            .await
        {
            Ok(out) => Ok(serde_json::from_slice(&out).unwrap_or(Json::Null)),
            Err(ActionError::ConfirmationRequired { id, ticket, .. }) => {
                Err(HostError::ConfirmationRequired { action: id, ticket })
            }
            Err(ActionError::Denied { reason, .. }) => Err(HostError::Refused {
                capability: "authorizer".to_owned(),
                why: reason,
            }),
            Err(ActionError::Unknown(id)) => Err(HostError::NotFound {
                what: format!("action {id}"),
            }),
            Err(e) => Err(HostError::Failed {
                message: e.to_string(),
            }),
        }
    }
}

/// How the list describes an extension's action (as the native host's).
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

/// An action run from the app's list, to the component.
struct Run {
    action: String,
    args: String,
    invocation: String,
    reply: oneshot::Sender<Result<String, String>>,
}

/// A WASI component running in this page: its instance in a Web Worker,
/// its workers in more.
#[derive(Clone)]
pub struct WebComponent {
    inner: Rc<Inner>,
}

struct Inner {
    js: JsComponent,
    host: WebHost,
    panes: RefCell<HashMap<u64, Pane>>,
    next_pane: RefCell<u64>,
    actions: RefCell<Vec<String>>,
    views: RefCell<Vec<String>>,
    /// The callbacks the runner holds.
    _kept: RefCell<Vec<JsValue>>,
}

impl std::fmt::Debug for WebComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebComponent")
            .field("id", &self.inner.host.manifest().id)
            .finish_non_exhaustive()
    }
}

/// How a component is started.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Its instance's limits.
    pub limits: Limits,
    /// Each worker's.
    pub worker_limits: Option<Limits>,
    /// A separate component's module for a worker whose `entry` is one
    /// (`workers/primes.wasm`): its transpiled module's URL, by the
    /// worker's id.
    pub worker_urls: BTreeMap<String, String>,
}

impl WebComponent {
    /// Starts the component transpiled to `url` (jco's module: the
    /// `instantiate` of `transpile.mjs`'s output) for `host`'s extension.
    ///
    /// # Errors
    ///
    /// It could not be loaded, or its `init` failed.
    pub async fn start(url: &str, host: WebHost, options: Options) -> Result<Self, String> {
        let url = web_sys::window()
            .and_then(|w| w.location().href().ok())
            .and_then(|base| web_sys::Url::new_with_base(url, &base).ok())
            .map_or_else(|| url.to_owned(), |u| u.href());
        let manifest = host.manifest().clone();
        let opts = js_sys::Object::new();
        set(&opts, "url", &JsValue::from_str(&url));
        set(&opts, "instance", &JsValue::from_str(INSTANCE_JS));
        set(&opts, "limits", &options.limits.js());
        set(
            &opts,
            "workerLimits",
            &options.worker_limits.unwrap_or_else(Limits::worker).js(),
        );
        let workers = js_sys::Object::new();
        for w in &manifest.workers {
            let decl = js_sys::Object::new();
            set(
                &decl,
                "entry",
                &JsValue::from_str(w.entry.as_deref().unwrap_or(&w.id)),
            );
            if let Some(u) = options.worker_urls.get(&w.id) {
                set(&decl, "url", &JsValue::from_str(u));
            }
            set(&workers, &w.id, &decl);
        }
        set(&opts, "workers", &workers);

        let mut kept = Vec::new();
        let asks = host.clone();
        let ask = Closure::<dyn FnMut(String) -> js_sys::Promise>::new(move |request: String| {
            let host = asks.clone();
            future_to_promise(async move { Ok(JsValue::from_str(&host.call_json(&request).await)) })
        });
        set(&opts, "host", ask.as_ref());
        kept.push(ask.into_js_value());
        let logs = host.clone();
        let log = Closure::<dyn FnMut(String, String, String)>::new(
            move |level: String, source: String, text: String| logs.log(&level, &source, text),
        );
        set(&opts, "log", log.as_ref());
        kept.push(log.into_js_value());
        let themes = host.clone();
        let theme = Closure::<dyn FnMut() -> JsValue>::new(move || {
            let t = themes.inner.theme.borrow().clone();
            if t.is_null() {
                JsValue::NULL
            } else {
                from_json(&t)
            }
        });
        set(&opts, "theme", theme.as_ref());
        kept.push(theme.into_js_value());

        let inner = Rc::new_cyclic(|weak: &std::rc::Weak<Inner>| {
            let frames = weak.clone();
            let frame = Closure::<dyn FnMut(f64, JsValue)>::new(move |pane: f64, f: JsValue| {
                if let Some(inner) = frames.upgrade() {
                    inner.frame(id_of(pane), &f);
                }
            });
            set(&opts, "frame", frame.as_ref());
            kept.push(frame.into_js_value());
            let again = weak.clone();
            let restarted = Closure::<dyn FnMut(String)>::new(move |_why: String| {
                if let Some(inner) = again.upgrade() {
                    Self { inner }.remount();
                }
            });
            set(&opts, "restarted", restarted.as_ref());
            kept.push(restarted.into_js_value());
            Inner {
                js: JsComponent::new(&opts),
                host,
                panes: RefCell::new(HashMap::new()),
                next_pane: RefCell::new(0),
                actions: RefCell::new(Vec::new()),
                views: RefCell::new(Vec::new()),
                _kept: RefCell::new(kept),
            }
        });
        let started = JsFuture::from(inner.js.start(&JsValue::NULL))
            .await
            .map_err(|e| why(&e))?;
        let listed = |key: &str| -> Vec<String> {
            serde_json::from_value(to_json(&get(&started, key))).unwrap_or_default()
        };
        *inner.actions.borrow_mut() = listed("actions");
        *inner.views.borrow_mut() = listed("views");
        for a in inner.actions.borrow().iter() {
            if !manifest.actions.iter().any(|d| &d.id == a) {
                inner.host.log(
                    "warn",
                    "wasi",
                    format!("it runs {a}, which its manifest does not declare"),
                );
            }
        }
        Ok(Self { inner })
    }

    /// The host it asks.
    #[must_use]
    pub fn host(&self) -> &WebHost {
        &self.inner.host
    }

    /// The actions it runs, by id.
    #[must_use]
    pub fn actions(&self) -> Vec<String> {
        self.inner.actions.borrow().clone()
    }

    /// The views it renders, by id.
    #[must_use]
    pub fn views(&self) -> Vec<String> {
        self.inner.views.borrow().clone()
    }

    /// Runs its action `action` with `args`, invoked as `invocation`.
    ///
    /// # Errors
    ///
    /// What it returned as its error, or why it trapped.
    pub async fn invoke(
        &self,
        action: &str,
        args: &Json,
        invocation: &Json,
    ) -> Result<Json, String> {
        let args = if args.is_null() {
            String::new()
        } else {
            args.to_string()
        };
        let out = JsFuture::from(self.inner.js.invoke(action, &args, &invocation.to_string()))
            .await
            .map_err(|e| why(&e))?;
        let text = out.as_string().unwrap_or_default();
        Ok(serde_json::from_str(&text).unwrap_or(Json::String(text)))
    }

    /// Puts its actions (each its manifest declares and it runs) on
    /// `list`: run there as whoever ran them, as the native host's are.
    ///
    /// # Errors
    ///
    /// One is on the list already.
    pub fn register_actions(&self, list: &ActionList) -> Result<(), ActionError> {
        let (tx, mut rx) = mpsc::unbounded::<Run>();
        let me = self.clone();
        spawn_local(async move {
            while let Some(run) = rx.next().await {
                let args: Json = serde_json::from_str(&run.args).unwrap_or(Json::Null);
                let invocation: Json = serde_json::from_str(&run.invocation).unwrap_or(Json::Null);
                let answer = me
                    .invoke(&run.action, &args, &invocation)
                    .await
                    .map(|v| v.to_string());
                let _ = run.reply.send(answer);
            }
        });
        let manifest = self.inner.host.manifest();
        let runs = self.actions();
        for decl in manifest.actions.iter().filter(|d| runs.contains(&d.id)) {
            let tx = tx.clone();
            let id = decl.id.clone();
            let handler = std::sync::Arc::new(move |caller: &ActionCaller, args: &[u8]| {
                let (reply, answer) = oneshot::channel();
                let sent = tx.unbounded_send(Run {
                    action: id.clone(),
                    args: String::from_utf8_lossy(args).into_owned(),
                    invocation: crate::invocation::invocation_json(caller.invocation()).to_string(),
                    reply,
                });
                let id = id.clone();
                let run: ActionFuture = Box::pin(async move {
                    let failed = |message: String| ActionError::Failed {
                        id: id.clone(),
                        message,
                    };
                    sent.map_err(|_| failed("it was unloaded".to_owned()))?;
                    answer
                        .await
                        .map_err(|_| failed("it was unloaded".to_owned()))?
                        .map(String::into_bytes)
                        .map_err(failed)
                });
                Ok(run)
            });
            list.register_forwarded_with(&info(manifest, decl), handler)?;
        }
        Ok(())
    }

    /// Mounts its view `view` under `into` (emptied first): the pane's id.
    /// Its edits are applied there as they come; events on its elements go
    /// back to it.
    #[must_use = "the pane's id unmounts it"]
    pub fn mount(&self, view: &str, into: &web_sys::Element) -> u64 {
        let pane = {
            let mut next = self.inner.next_pane.borrow_mut();
            *next = next.saturating_add(1);
            *next
        };
        let p = Pane::new(
            pane,
            view.to_owned(),
            into.clone(),
            Rc::downgrade(&self.inner),
        );
        self.inner.panes.borrow_mut().insert(pane, p);
        self.render(pane);
        pane
    }

    fn render(&self, pane: u64) {
        let Some(view) = self.inner.panes.borrow().get(&pane).map(|p| p.view.clone()) else {
            return;
        };
        if let Some(p) = self.inner.panes.borrow_mut().get_mut(&pane) {
            p.reset();
        }
        let _ = self.inner.js.render(num(pane), &view);
    }

    /// Its views mount again (it was started again).
    fn remount(&self) {
        let panes: Vec<u64> = self.inner.panes.borrow().keys().copied().collect();
        for pane in panes {
            self.render(pane);
        }
    }

    /// Unmounts pane `pane`.
    pub fn unmount(&self, pane: u64) {
        if let Some(mut p) = self.inner.panes.borrow_mut().remove(&pane) {
            p.reset();
        }
        let _ = self.inner.js.close(num(pane));
    }

    /// What it runs now, as JSON: `calls`, `traps`, `lastTrap`, `alive`,
    /// `generation`, `memory` (its limit), `workers` (`id`, `name`,
    /// `status`, `progress`).
    #[must_use]
    pub fn report(&self) -> Json {
        to_json(&self.inner.js.report())
    }

    /// Unloads it: its instance and its workers end.
    pub fn stop(&self) {
        self.inner.js.stop();
        for (_, mut p) in self.inner.panes.borrow_mut().drain() {
            p.reset();
        }
    }
}

impl Inner {
    fn frame(&self, pane: u64, f: &JsValue) {
        let mut panes = self.panes.borrow_mut();
        let Some(p) = panes.get_mut(&pane) else {
            return;
        };
        let failed = get(f, "failed");
        if let Some(why) = failed.as_string() {
            p.failed(&why);
            return;
        }
        let edits: Vec<Json> =
            serde_json::from_value(to_json(&get(f, "edits"))).unwrap_or_default();
        let applied = edits
            .iter()
            .map(edit_from)
            .collect::<Result<Vec<_>, _>>()
            .and_then(|edits| edits.into_iter().try_for_each(|e| p.edit(e)));
        if let Err(why) = applied {
            p.failed(&format!("its edits could not be applied: {why}"));
        }
    }

    fn event(&self, pane: u64, event: &Json) {
        let _ = self.js.event(num(pane), &from_json(event));
    }
}

/// One of the world's `edit`s (jco's shape: `{tag, val}`) as the wire's.
///
/// # Errors
///
/// It is not one.
pub fn edit_from(e: &Json) -> Result<Edit, String> {
    let tag = e.get("tag").and_then(Json::as_str).unwrap_or("");
    let val = e.get("val").unwrap_or(&Json::Null);
    let n = || {
        val.as_u64()
            .ok_or_else(|| format!("{tag} without a number"))
    };
    let m = || n().and_then(|n| u32::try_from(n).map_err(|e| e.to_string()));
    let text = |v: &Json| v.as_str().map(str::to_owned);
    Ok(match tag {
        "push-id" => Edit::PushId { id: n()? },
        "set-id" => Edit::SetId { id: n()? },
        "child" => Edit::Child { index: m()? },
        "pop" => Edit::Pop,
        "create-element" => Edit::CreateElement {
            tag: text(&val["tag"]).ok_or("an element without a tag")?,
            namespace: text(&val["namespace"]),
        },
        "create-text" => Edit::CreateText {
            value: text(val).unwrap_or_default(),
        },
        "clone" => Edit::Clone,
        "append-children" => Edit::AppendChildren { m: m()? },
        "replace-with" => Edit::ReplaceWith { m: m()? },
        "insert-after" => Edit::InsertAfter { m: m()? },
        "insert-before" => Edit::InsertBefore { m: m()? },
        "set-attribute" => Edit::SetAttribute {
            name: text(&val["name"]).ok_or("an attribute without a name")?,
            namespace: text(&val["namespace"]),
            value: value_from(&val["value"]),
        },
        "set-text" => Edit::SetText {
            value: text(val).unwrap_or_default(),
        },
        "listen" => Edit::Listen {
            name: text(val).unwrap_or_default(),
        },
        "unlisten" => Edit::Unlisten {
            name: text(val).unwrap_or_default(),
        },
        "remove" => Edit::Remove,
        other => return Err(format!("no edit {other}")),
    })
}

fn value_from(v: &Json) -> Value {
    let val = &v["val"];
    match v.get("tag").and_then(Json::as_str) {
        Some("text") => Value::Text(val.as_str().unwrap_or_default().to_owned()),
        Some("float") => Value::Float(val.as_f64().unwrap_or_default()),
        Some("int") => Value::Int(val.as_i64().unwrap_or_default()),
        Some("bool") => Value::Bool(val.as_bool().unwrap_or_default()),
        _ => Value::None,
    }
}

/// The attribute an element a view listens on carries its id in, as
/// Dioxus's own renderers.
const DIOXUS_ID: &str = "data-dioxus-id";

/// One mounted view: the wire's stack machine over the page's DOM, under
/// its element (id 0).
struct Pane {
    id: u64,
    view: String,
    root: web_sys::Element,
    nodes: HashMap<u64, web_sys::Node>,
    stack: Vec<(web_sys::Node, Option<u64>)>,
    listening: HashMap<String, Closure<dyn FnMut(web_sys::Event)>>,
    owner: std::rc::Weak<Inner>,
}

impl Pane {
    fn new(id: u64, view: String, root: web_sys::Element, owner: std::rc::Weak<Inner>) -> Self {
        let mut p = Self {
            id,
            view,
            root,
            nodes: HashMap::new(),
            stack: Vec::new(),
            listening: HashMap::new(),
            owner,
        };
        p.reset();
        p
    }

    /// Empties it, for a first render.
    fn reset(&mut self) {
        self.root.set_inner_html("");
        self.nodes.clear();
        self.nodes.insert(0, self.root.clone().into());
        self.stack.clear();
        for (name, f) in self.listening.drain() {
            let _ = self
                .root
                .remove_event_listener_with_callback(&name, f.as_ref().unchecked_ref());
        }
    }

    fn failed(&mut self, why: &str) {
        self.reset();
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Ok(p) = doc.create_element("p")
        {
            let _ = p.set_attribute("class", "wasi-stopped");
            p.set_text_content(Some(&format!("It stopped: {why}")));
            let _ = self.root.append_child(&p);
        }
    }

    fn top(&self) -> Result<web_sys::Node, String> {
        self.stack
            .last()
            .map(|(n, _)| n.clone())
            .ok_or_else(|| "an edit on an empty stack".to_owned())
    }

    fn pop(&mut self, m: u32) -> Result<Vec<web_sys::Node>, String> {
        let m = usize::try_from(m).map_err(|e| e.to_string())?;
        let at = self
            .stack
            .len()
            .checked_sub(m)
            .ok_or("fewer nodes on the stack than an edit takes")?;
        Ok(self
            .stack
            .split_off(at)
            .into_iter()
            .map(|(n, _)| n)
            .collect())
    }

    /// Whether an edit may change `node`: one under the view's element,
    /// or one the view made that is not in the page yet. Never the rest
    /// of the page.
    fn inside(&self, node: &web_sys::Node) -> bool {
        self.root.contains(Some(node)) || !node.is_connected()
    }

    fn edit(&mut self, edit: Edit) -> Result<(), String> {
        let doc = web_sys::window()
            .and_then(|w| w.document())
            .ok_or("no document")?;
        let js = |e: JsValue| why(&e);
        match edit {
            Edit::PushId { id } => {
                let node = self
                    .nodes
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| format!("no node {id}"))?;
                self.stack.push((node, Some(id)));
            }
            Edit::SetId { id } => {
                let node = self.top()?;
                if id != 0 && node.is_same_node(Some(&self.root)) {
                    return Err("an edit reached outside the view".to_owned());
                }
                self.nodes.insert(id, node);
                if let Some(top) = self.stack.last_mut() {
                    top.1 = Some(id);
                }
            }
            Edit::Child { index } => {
                let child = self
                    .top()?
                    .child_nodes()
                    .item(index)
                    .ok_or("a child that is not there")?;
                if let Some(top) = self.stack.last_mut() {
                    *top = (child, None);
                }
            }
            Edit::Pop => {
                self.stack.pop().ok_or("an edit on an empty stack")?;
            }
            Edit::CreateElement { tag, namespace } => {
                let el = namespace
                    .map_or_else(
                        || doc.create_element(&tag),
                        |ns| doc.create_element_ns(Some(&ns), &tag),
                    )
                    .map_err(js)?;
                self.stack.push((el.into(), None));
            }
            Edit::CreateText { value } => {
                self.stack.push((doc.create_text_node(&value).into(), None));
            }
            Edit::Clone => {
                let copy = self.top()?.clone_node_with_deep(true).map_err(js)?;
                if let Some(top) = self.stack.last_mut() {
                    *top = (copy, None);
                }
            }
            Edit::AppendChildren { m } => {
                let kids = self.pop(m)?;
                let parent = self.top()?;
                if !self.inside(&parent) {
                    return Err("an edit reached outside the view".to_owned());
                }
                for k in kids {
                    parent.append_child(&k).map_err(js)?;
                }
            }
            Edit::ReplaceWith { m } => {
                let kids = self.pop(m)?;
                let (anchor, _) = self.stack.pop().ok_or("an edit on an empty stack")?;
                let parent = self.parent_of(&anchor)?;
                for k in kids {
                    parent.insert_before(&k, Some(&anchor)).map_err(js)?;
                }
                parent.remove_child(&anchor).map_err(js)?;
            }
            Edit::InsertAfter { m } | Edit::InsertBefore { m } => {
                let after = matches!(edit, Edit::InsertAfter { .. });
                let kids = self.pop(m)?;
                let anchor = self.top()?;
                let parent = self.parent_of(&anchor)?;
                let next = if after {
                    anchor.next_sibling()
                } else {
                    Some(anchor)
                };
                for k in kids {
                    parent.insert_before(&k, next.as_ref()).map_err(js)?;
                }
            }
            Edit::SetAttribute {
                name,
                namespace,
                value,
            } => self.set_attribute(&name, namespace.as_deref(), value)?,
            Edit::SetText { value } => self.top()?.set_text_content(Some(&value)),
            Edit::Listen { name } => {
                let (node, id) = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or("an edit on an empty stack")?;
                let id = id.ok_or("a handler on a node with no element id")?;
                if let Some(el) = node.dyn_ref::<web_sys::Element>() {
                    el.set_attribute(DIOXUS_ID, &id.to_string()).map_err(js)?;
                }
                self.listen(&name)?;
            }
            Edit::Unlisten { .. } => {}
            Edit::Remove => {
                let (node, _) = self.stack.pop().ok_or("an edit on an empty stack")?;
                if node.is_same_node(Some(&self.root)) {
                    return Err("an edit reached outside the view".to_owned());
                }
                self.nodes.retain(|_, n| !node.contains(Some(n)));
                if let Some(parent) = node.parent_node() {
                    parent.remove_child(&node).map_err(js)?;
                }
            }
        }
        Ok(())
    }

    fn parent_of(&self, node: &web_sys::Node) -> Result<web_sys::Node, String> {
        let parent = node.parent_node().ok_or("a node with no parent")?;
        if self.inside(&parent) {
            Ok(parent)
        } else {
            Err("an edit reached outside the view".to_owned())
        }
    }

    /// As Dioxus's web renderer: the `style` namespace is one CSS property.
    fn set_attribute(
        &self,
        name: &str,
        namespace: Option<&str>,
        value: Value,
    ) -> Result<(), String> {
        let node = self.top()?;
        let Some(el) = node.dyn_ref::<web_sys::Element>() else {
            return Ok(());
        };
        if name == "dangerous_inner_html" || name.starts_with("on") {
            return Err(format!("a view may not set {name}"));
        }
        let value = match value {
            Value::None => None,
            Value::Bool(false) if name == "checked" => None,
            Value::Text(t) => Some(t),
            Value::Float(f) => Some(f.to_string()),
            Value::Int(i) => Some(i.to_string()),
            Value::Bool(b) => Some(b.to_string()),
        };
        let js = |e: JsValue| why(&e);
        if namespace == Some("style") {
            let style = el
                .dyn_ref::<web_sys::HtmlElement>()
                .map(web_sys::HtmlElement::style);
            if let Some(style) = style {
                match value {
                    Some(v) => style.set_property(name, &v).map_err(js)?,
                    None => {
                        style.remove_property(name).map_err(js)?;
                    }
                }
            }
            return Ok(());
        }
        match value {
            Some(v) => el.set_attribute(name, &v).map_err(js)?,
            None => el.remove_attribute(name).map_err(js)?,
        }
        Ok(())
    }

    /// Events `name` under its element, to the component: from the nearest
    /// element with an id (it bubbles them through its own tree).
    fn listen(&mut self, name: &str) -> Result<(), String> {
        if self.listening.contains_key(name) {
            return Ok(());
        }
        let owner = self.owner.clone();
        let pane = self.id;
        let root = self.root.clone();
        let f = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some(inner) = owner.upgrade() else {
                return;
            };
            let mut at = e
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
            let element = loop {
                let Some(el) = at else { return };
                if let Some(id) = el
                    .get_attribute(DIOXUS_ID)
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    break id;
                }
                if el.is_same_node(Some(&root)) {
                    return;
                }
                at = el.parent_element();
            };
            inner.event(pane, &view_event(element, &e));
        });
        self.root
            .add_event_listener_with_callback(name, f.as_ref().unchecked_ref())
            .map_err(|e| why(&e))?;
        self.listening.insert(name.to_owned(), f);
        Ok(())
    }
}

/// A DOM event as the world's `view-event` (jco's shape).
fn view_event(element: u64, e: &web_sys::Event) -> Json {
    json!({ "element": element, "name": e.type_(), "bubbles": e.bubbles(), "data": event_data(e) })
}

/// What a DOM event carries, as the world's `event-data`.
fn event_data(e: &web_sys::Event) -> Json {
    let mods = |alt: bool, ctrl: bool, meta: bool, shift: bool| json!({ "alt": alt, "ctrl": ctrl, "meta": meta, "shift": shift });
    if let Some(m) = e.dyn_ref::<web_sys::MouseEvent>() {
        return json!({ "tag": "mouse", "val": {
            "clientX": m.client_x(), "clientY": m.client_y(),
            "pageX": m.page_x(), "pageY": m.page_y(),
            "screenX": m.screen_x(), "screenY": m.screen_y(),
            "elementX": m.offset_x(), "elementY": m.offset_y(),
            "button": m.button(), "buttons": m.buttons(),
            "mods": mods(m.alt_key(), m.ctrl_key(), m.meta_key(), m.shift_key()),
        }});
    }
    if let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() {
        return json!({ "tag": "key", "val": {
            "key": k.key(), "code": k.code(), "location": k.location(),
            "repeat": k.repeat(), "composing": k.is_composing(),
            "mods": mods(k.alt_key(), k.ctrl_key(), k.meta_key(), k.shift_key()),
        }});
    }
    if matches!(e.type_().as_str(), "input" | "change") {
        let value = e
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|i| i.value())
            .unwrap_or_default();
        return json!({ "tag": "form", "val": value });
    }
    json!({ "tag": "focus" })
}
