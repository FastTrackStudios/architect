//! TSX and JS pages in a browser (feature `web`, on
//! `wasm32-unknown-unknown`; `docs/content/architecture/pages.md`, "On
//! the web").
//!
//! A browser has no `QuickJS` to give a page: each page runs in a Web
//! Worker of its own, on the browser's own engine, with the same runtime
//! a native page has (`js/prelude.js`, the same `architect.d.ts`) — its
//! `__host` bridge is messages to the main thread (`web/page.js`) instead
//! of calls into Rust. The main thread only answers it and draws:
//!
//! - **Its modules**: the page's folder is given as text
//!   ([`PageFolder`]); every script is transpiled here (oxc, in this
//!   wasm) and its imports resolved by the native loader's rules
//!   ([`crate::modules`]), and the worker links them as `blob:` URLs.
//! - **Its tree**: what the worker renders comes back as the element tree
//!   ([`Element`]); [`Mirror`] diffs it into `wire::Edit`s and the
//!   extensions host's checked applier ([`Pane`]) applies them under the
//!   pane's element — never an `on*` handler, never outside it. An event
//!   there goes back to its handler ([`crate::frame::page_event`]).
//! - **The app**: each host call is answered by the page's extension's
//!   [`WebHost`], after the same capability checks every host makes
//!   (`hostcall::check`): `actions.run` is its `run`, `net.fetch` and the
//!   rest its host-service calls, the actions a page lists only those it
//!   may run. Shared state, settings, the theme and notifications are the
//!   app's ([`WebPages`]). The worker has none of the browser's network,
//!   storage or workers of its own: a page reaches the network only
//!   through `net.fetch`.
//! - **Limits**: each unit of the page's work (an event, a host call's
//!   answer, a timer, with the renders it asks for) has
//!   [`Limits::frame`]; past it the main thread terminates the worker
//!   (a browser has no interrupt to stop one call), says to use a worker,
//!   and starts the page again from its files — the rest of the app never
//!   waited. Loading has [`Limits::load`]. A browser cannot cap one
//!   worker's memory.
//! - **Workers**: `worker.spawn("job.js")` is a nested Web Worker the
//!   page's own starts (`web/job.js`), with [`Limits::worker_time`].
//! - **Errors**: a page that does not load, or whose render throws,
//!   keeps its last good frame under a banner, its error at the source's
//!   line and column ([`crate::tsx::remap`]). [`WebPage::reload`] is a
//!   save: the page loads again in a fresh worker, swapped in once it
//!   has rendered.

// Nothing is `Send` in a browser: everything here is on its one thread.
#![allow(clippy::future_not_send)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::Duration;

use architect_extensions::hostcall::HostRequest;
use architect_extensions::web::{Pane, WebHost};
use architect_extensions::wire::ViewEvent;
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::api::{ActionSummary, Element, Notice, ThemeSnapshot};
use crate::diff::Mirror;
use crate::frame::{self, RENDER_FAILED};
use crate::modules::{self, ARCHITECT, ARCHITECT_WORKER, LINK};
use crate::tsx;

/// The page runtime (`js/prelude.js`), the same a native page runs.
const PRELUDE: &str = include_str!("../js/prelude.js");
/// A worker's runtime (`js/worker.js`).
const WORKER_PRELUDE: &str = include_str!("../js/worker.js");
/// A page's Web Worker, around the prelude.
const PAGE_JS: &str = include_str!("page.js");
/// A page's worker's Web Worker, around the worker runtime.
const JOB_JS: &str = include_str!("job.js");
/// Where the prelude goes in each.
const AT_PRELUDE: &str = "// @prelude";

/// What a page's modules are called in what it throws: under this root,
/// which [`tsx::remap`] takes away again.
const ROOT: &str = "/page";

/// How many times a page stopped past its time is started again before
/// it stays stopped (until it is reloaded).
const RESTARTS: u32 = 3;

/// How far a page in a browser may go (the native
/// `runner::PageLimits`, less what a browser cannot limit: memory).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest one unit of its work may run: past it, its worker is
    /// terminated with a message to use a worker.
    pub frame: Duration,
    /// The longest loading it (its modules, its first render) may run.
    pub load: Duration,
    /// A worker's whole run.
    pub worker_time: Duration,
}

impl Default for Limits {
    /// The native defaults: 500 ms a unit, 5 s to load, 300 s a worker.
    fn default() -> Self {
        Self {
            frame: Duration::from_millis(500),
            load: Duration::from_secs(5),
            worker_time: Duration::from_secs(300),
        }
    }
}

fn millis(d: Duration) -> i32 {
    i32::try_from(d.as_millis()).unwrap_or(i32::MAX)
}

/// A page's folder, as text: every file it may import or spawn, by its
/// `/`-separated path in the folder, and the one that is the page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageFolder {
    /// The page's file (`board.tsx`).
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

impl PageFolder {
    /// `entry` among `files` (`(path, text)`, as an extension's `PAGES`
    /// lists them).
    #[must_use]
    pub fn new<'a>(entry: &str, files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            entry: entry.to_owned(),
            files: files
                .into_iter()
                .map(|(n, t)| (n.to_owned(), t.to_owned()))
                .collect(),
        }
    }
}

/// The folder prepared: what the worker is started with, and the maps
/// its errors are said through.
struct Prepared {
    start: Json,
    maps: Vec<(String, tsx::SourceMap)>,
}

fn prepare(folder: &PageFolder) -> Prepared {
    let prepared = modules::prepare(&folder.files);
    let mut maps = Vec::new();
    let mut out = serde_json::Map::new();
    for (name, m) in prepared {
        let imports: Vec<Json> = m
            .imports
            .iter()
            .map(|i| match &i.to {
                Ok(to) => json!([i.specifier, to, null]),
                Err(why) => json!([i.specifier, null, why]),
            })
            .collect();
        out.insert(
            name.clone(),
            json!({ "code": m.code, "imports": imports, "error": m.error }),
        );
        if let Some(map) = m.map {
            maps.push((format!("{ROOT}/{name}"), map));
        }
    }
    Prepared {
        start: json!({
            "t": "start",
            "entry": folder.entry,
            "modules": out,
            "architect": ARCHITECT,
            "architectWorker": ARCHITECT_WORKER,
            "link": LINK,
            "root": ROOT,
            "job": JOB_JS.replacen(AT_PRELUDE, WORKER_PRELUDE, 1),
        }),
        maps,
    }
}

/// What every page of an app shares in a browser: shared state (each
/// change told to every page), settings, the theme, where notifications
/// go, and the pages' limits. Cheap to clone; clones share it.
#[derive(Clone, Default)]
pub struct WebPages {
    inner: Rc<App>,
}

#[derive(Default)]
struct App {
    shared: RefCell<BTreeMap<String, String>>,
    settings: RefCell<BTreeMap<String, Json>>,
    theme: RefCell<ThemeSnapshot>,
    notify: RefCell<Option<Rc<dyn Fn(&Notice)>>>,
    limits: Cell<Limits>,
    pages: RefCell<Vec<Weak<PageInner>>>,
    /// The page worker's script, as a `blob:` URL: made once.
    script: RefCell<Option<String>>,
}

impl std::fmt::Debug for WebPages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPages")
            .field("pages", &self.inner.pages.borrow().len())
            .finish_non_exhaustive()
    }
}

impl WebPages {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_limits(self, limits: Limits) -> Self {
        self.inner.limits.set(limits);
        self
    }

    /// Where the pages' notifications (`notify`) go; else only their log.
    pub fn on_notify(&self, f: impl Fn(&Notice) + 'static) {
        *self.inner.notify.borrow_mut() = Some(Rc::new(f));
    }

    /// The theme showing: every page renders again in it.
    pub fn set_theme(&self, theme: ThemeSnapshot) {
        let text = serde_json::to_string(&theme).unwrap_or_default();
        *self.inner.theme.borrow_mut() = theme;
        self.each(|p| p.post_all(&json!({ "t": "theme", "text": text })));
    }

    /// A shared-state key's value (`state.get`), if set.
    #[must_use]
    pub fn shared(&self, key: &str) -> Option<Json> {
        self.inner
            .shared
            .borrow()
            .get(key)
            .and_then(|v| serde_json::from_str(v).ok())
    }

    /// Sets a shared-state key (`null` clears it): every page hears.
    pub fn set_shared(&self, key: &str, value: &Json) {
        set_shared(&self.inner, key, &value.to_string());
    }

    /// Opens `folder`'s page as `host`'s extension's, drawn under `into`
    /// (emptied first). It loads in a Web Worker of its own; the handle
    /// closes it when it goes.
    #[must_use = "the page closes when its handle goes"]
    pub fn open(&self, folder: &PageFolder, host: WebHost, into: &web_sys::Element) -> WebPage {
        let name = Path::new(&folder.entry).file_name().map_or_else(
            || folder.entry.clone(),
            |n| n.to_string_lossy().into_owned(),
        );
        let inner = Rc::new_cyclic(|me: &Weak<PageInner>| {
            let events = me.clone();
            let pane = Pane::new(into.clone(), move |e: ViewEvent| {
                if let Some(page) = events.upgrade() {
                    page.event(&e);
                }
            });
            PageInner {
                me: me.clone(),
                app: Rc::downgrade(&self.inner),
                host,
                name,
                folder: RefCell::new(folder.clone()),
                prepared: RefCell::new(prepare(folder)),
                current: RefCell::new(None),
                loading: RefCell::new(None),
                pane: RefCell::new(pane),
                mirror: RefCell::new(Mirror::new()),
                last: RefCell::new(Vec::new()),
                error: RefCell::new(None),
                stopped: RefCell::new(None),
                first: Cell::new(true),
                next_run: Cell::new(0),
                restarts: Cell::new(0),
                renders: Cell::new(0),
                log: RefCell::new(VecDeque::new()),
                on_log: RefCell::new(None),
            }
        });
        self.inner.pages.borrow_mut().push(Rc::downgrade(&inner));
        inner.load();
        WebPage { inner }
    }

    fn each(&self, f: impl Fn(&PageInner)) {
        each(&self.inner, f);
    }
}

fn each(app: &App, f: impl Fn(&PageInner)) {
    let pages: Vec<Rc<PageInner>> = {
        let mut pages = app.pages.borrow_mut();
        pages.retain(|p| p.strong_count() > 0);
        pages.iter().filter_map(Weak::upgrade).collect()
    };
    for p in pages {
        f(&p);
    }
}

fn set_shared(app: &App, key: &str, value: &str) {
    {
        let mut shared = app.shared.borrow_mut();
        if value == "null" {
            shared.remove(key);
        } else {
            shared.insert(key.to_owned(), value.to_owned());
        }
    }
    let news = json!({ "t": "shared", "key": key, "value": value });
    each(app, |p| p.post_all(&news));
}

/// A page open in a browser.
#[derive(Clone)]
pub struct WebPage {
    inner: Rc<PageInner>,
}

impl std::fmt::Debug for WebPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebPage")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

impl WebPage {
    /// Its folder saved anew: the page loads again in a fresh worker,
    /// swapped in once it renders; the last good one stays (under a
    /// banner) when it does not.
    pub fn reload(&self, folder: &PageFolder) {
        *self.inner.folder.borrow_mut() = folder.clone();
        *self.inner.prepared.borrow_mut() = prepare(folder);
        *self.inner.stopped.borrow_mut() = None;
        self.inner.restarts.set(0);
        self.inner.load();
    }

    /// Why it is broken or was stopped, if it is: at its source's line
    /// and column.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        self.inner.problem()
    }

    /// Whether a page is running (loaded, and not stopped).
    #[must_use]
    pub fn running(&self) -> bool {
        self.inner.current.borrow().is_some()
    }

    /// How many frames it has drawn.
    #[must_use]
    pub fn renders(&self) -> u64 {
        self.inner.renders.get()
    }

    /// Its log: what it printed and what failed, the last few hundred
    /// lines (`[level] text`).
    #[must_use]
    pub fn log(&self) -> Vec<String> {
        self.inner.log.borrow().iter().cloned().collect()
    }

    /// Each line of its log as it is written, too (`level`, `text`).
    pub fn on_log(&self, f: impl Fn(&str, &str) + 'static) {
        *self.inner.on_log.borrow_mut() = Some(Rc::new(f));
    }
}

/// A page's log, kept: as the native runner's.
const KEPT: usize = 300;

struct PageInner {
    me: Weak<Self>,
    app: Weak<App>,
    host: WebHost,
    /// Its file's name, for its banner.
    name: String,
    folder: RefCell<PageFolder>,
    prepared: RefCell<Prepared>,
    /// The worker the page runs in.
    current: RefCell<Option<Run>>,
    /// One loading (a reload, a restart), swapped in once it renders.
    loading: RefCell<Option<Run>>,
    pane: RefCell<Pane>,
    mirror: RefCell<Mirror>,
    /// What it rendered last, to draw under a banner.
    last: RefCell<Vec<Element>>,
    /// Why it does not load or render.
    error: RefCell<Option<String>>,
    /// Why it was stopped (past its time).
    stopped: RefCell<Option<String>>,
    first: Cell<bool>,
    next_run: Cell<u64>,
    restarts: Cell<u32>,
    renders: Cell<u64>,
    log: RefCell<VecDeque<String>>,
    on_log: RefCell<Option<Rc<dyn Fn(&str, &str)>>>,
}

impl Drop for PageInner {
    fn drop(&mut self) {
        for run in [self.current.get_mut().take(), self.loading.get_mut().take()]
            .into_iter()
            .flatten()
        {
            run.stop();
        }
    }
}

/// One Web Worker a page runs (or loads) in.
struct Run {
    id: u64,
    worker: web_sys::Worker,
    loaded: Cell<bool>,
    timer: Cell<Option<i32>>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    on_time: Closure<dyn FnMut()>,
}

impl Run {
    fn post(&self, message: &Json) {
        let _ = self.worker.post_message(&to_js(message));
    }

    /// Times what it does now: past `limit` it is stopped.
    fn arm(&self, limit: Duration) {
        self.disarm();
        let id = web_sys::window().and_then(|w| {
            w.set_timeout_with_callback_and_timeout_and_arguments_0(
                self.on_time.as_ref().unchecked_ref(),
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

    fn stop(&self) {
        self.disarm();
        self.worker.terminate();
    }
}

fn to_js(v: &Json) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

fn from_js(v: &JsValue) -> Json {
    js_sys::JSON::stringify(v)
        .ok()
        .and_then(|s| s.as_string())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Json::Null)
}

/// Why a unit or a load ran past `limit`: the native runner's words.
fn out_of_time(limit: Duration) -> String {
    format!(
        "ran past its time of {} ms: hand heavy work to a worker (worker.spawn)",
        limit.as_millis()
    )
}

impl PageInner {
    fn limits(&self) -> Limits {
        self.app
            .upgrade()
            .map(|a| a.limits.get())
            .unwrap_or_default()
    }

    fn problem(&self) -> Option<String> {
        self.error
            .borrow()
            .clone()
            .or_else(|| self.stopped.borrow().clone())
    }

    /// `text` (an error, a stack) at its sources' places.
    fn said(&self, text: &str) -> String {
        let prepared = self.prepared.borrow();
        let maps: Vec<(&str, &tsx::SourceMap)> =
            prepared.maps.iter().map(|(k, v)| (k.as_str(), v)).collect();
        tsx::remap(text, ROOT, &maps)
    }

    fn line(&self, level: &str, text: &str) {
        {
            let mut log = self.log.borrow_mut();
            log.push_back(format!("[{level}] {text}"));
            while log.len() > KEPT {
                log.pop_front();
            }
        }
        let sink = self.on_log.borrow().clone();
        if let Some(sink) = sink {
            sink(level, text);
        }
    }

    /// Starts a worker for the page's folder as it is now: it becomes the
    /// page once it has rendered.
    fn load(&self) {
        if let Some(old) = self.loading.borrow_mut().take() {
            old.stop();
        }
        match self.start() {
            Ok(run) => {
                run.arm(self.limits().load);
                *self.loading.borrow_mut() = Some(run);
            }
            Err(why) => self.fail(why),
        }
    }

    fn script(&self) -> Result<String, String> {
        let app = self.app.upgrade().ok_or("its app is gone")?;
        if let Some(url) = app.script.borrow().clone() {
            return Ok(url);
        }
        let source = PAGE_JS.replacen(AT_PRELUDE, PRELUDE, 1);
        let parts = js_sys::Array::of1(&JsValue::from_str(&source));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("text/javascript");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)
            .map_err(|e| format!("{e:?}"))?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
        *app.script.borrow_mut() = Some(url.clone());
        Ok(url)
    }

    fn start(&self) -> Result<Run, String> {
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        let worker = web_sys::Worker::new_with_options(&self.script()?, &options)
            .map_err(|e| format!("its Web Worker did not start: {e:?}"))?;
        let id = self.next_run.get().wrapping_add(1);
        self.next_run.set(id);
        let me = self.me.clone();
        let on_message =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                if let Some(page) = me.upgrade() {
                    page.message(id, &from_js(&e.data()));
                }
            });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        let me = self.me.clone();
        let on_time = Closure::<dyn FnMut()>::new(move || {
            if let Some(page) = me.upgrade() {
                page.out_of_time(id);
            }
        });
        let run = Run {
            id,
            worker,
            loaded: Cell::new(false),
            timer: Cell::new(None),
            _on_message: on_message,
            on_time,
        };
        let app = self.app.upgrade().ok_or("its app is gone")?;
        let mut start = self.prepared.borrow().start.clone();
        let shared: Vec<Json> = app
            .shared
            .borrow()
            .iter()
            .map(|(k, v)| json!([k, v]))
            .collect();
        if let Some(fields) = start.as_object_mut() {
            fields.insert("shared".to_owned(), Json::Array(shared));
            fields.insert("embed".to_owned(), json!("null"));
            fields.insert(
                "theme".to_owned(),
                json!(serde_json::to_string(&*app.theme.borrow()).unwrap_or_default()),
            );
            fields.insert(
                "workerTime".to_owned(),
                json!(self.limits().worker_time.as_millis()),
            );
        }
        run.post(&start);
        Ok(run)
    }

    /// Posts `message` to its workers, the running and the loading.
    fn post_all(&self, message: &Json) {
        for slot in [&self.current, &self.loading] {
            if let Some(run) = slot.borrow().as_ref() {
                run.post(message);
            }
        }
    }

    /// Which of its workers `id` is: `Some(true)` the running one,
    /// `Some(false)` the loading one.
    fn which(&self, id: u64) -> Option<bool> {
        if self.current.borrow().as_ref().is_some_and(|r| r.id == id) {
            Some(true)
        } else if self.loading.borrow().as_ref().is_some_and(|r| r.id == id) {
            Some(false)
        } else {
            None
        }
    }

    fn with_run<T>(&self, id: u64, f: impl FnOnce(&Run) -> T) -> Option<T> {
        let current = self.current.borrow();
        if let Some(run) = current.as_ref().filter(|r| r.id == id) {
            return Some(f(run));
        }
        drop(current);
        self.loading.borrow().as_ref().filter(|r| r.id == id).map(f)
    }

    fn message(&self, id: u64, m: &Json) {
        let Some(running) = self.which(id) else {
            return;
        };
        let text = |k: &str| {
            m.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match m.get("t").and_then(Json::as_str).unwrap_or_default() {
            "busy" => {
                let frame = self.limits().frame;
                self.with_run(id, |r| {
                    if r.loaded.get() {
                        r.arm(frame);
                    }
                });
            }
            "idle" => {
                self.with_run(id, Run::disarm);
                if let Some(why) = m.get("error").and_then(Json::as_str) {
                    let why = self.said(why);
                    self.line("error", &format!("{RENDER_FAILED}{why}"));
                    if running {
                        *self.error.borrow_mut() = Some(format!("{RENDER_FAILED}{why}"));
                        self.send_tree();
                    } else {
                        self.drop_loading();
                        self.fail(why);
                    }
                    return;
                }
                if !running {
                    self.swap_in();
                }
                if let Some(tree) = m.get("tree").and_then(Json::as_str) {
                    match serde_json::from_str::<Vec<Element>>(tree) {
                        Ok(tree) => {
                            *self.last.borrow_mut() = tree;
                            let stale = self
                                .error
                                .borrow()
                                .as_deref()
                                .is_some_and(|e| e.starts_with(RENDER_FAILED));
                            if stale || !running {
                                *self.error.borrow_mut() = None;
                            }
                            self.send_tree();
                        }
                        Err(e) => {
                            self.line("error", &format!("what it rendered is not a tree: {e}"))
                        }
                    }
                }
            }
            "failed" => {
                let why = self.said(&text("why"));
                if running {
                    self.line("error", &why);
                } else {
                    self.drop_loading();
                    self.fail(why);
                }
            }
            "call" => self.call(id, m),
            "set_shared" => {
                if let Some(app) = self.app.upgrade() {
                    set_shared(&app, &text("key"), &text("value"));
                }
            }
            "notify" => {
                let notice = Notice {
                    text: text("text"),
                    kind: text("kind"),
                };
                self.line(
                    "info",
                    &format!("notify ({}): {}", notice.kind, notice.text),
                );
                let sink = self.app.upgrade().and_then(|a| a.notify.borrow().clone());
                if let Some(sink) = sink {
                    sink(&notice);
                }
            }
            "log" => {
                let level = text("level");
                let said = if level == "error" {
                    self.said(&text("text"))
                } else {
                    text("text")
                };
                self.line(&level, &said);
            }
            _ => {}
        }
    }

    /// The loading worker rendered: it is the page now.
    fn swap_in(&self) {
        let Some(run) = self.loading.borrow_mut().take() else {
            return;
        };
        run.loaded.set(true);
        if let Some(old) = self.current.borrow_mut().replace(run) {
            old.stop();
        }
    }

    fn drop_loading(&self) {
        if let Some(run) = self.loading.borrow_mut().take() {
            run.stop();
        }
    }

    /// Worker `id` ran past its time.
    fn out_of_time(&self, id: u64) {
        let Some(running) = self.which(id) else {
            return;
        };
        let limits = self.limits();
        if running {
            if let Some(run) = self.current.borrow_mut().take() {
                run.stop();
            }
            let why = out_of_time(limits.frame);
            self.line("error", &why);
            *self.stopped.borrow_mut() = Some(why);
            self.send_tree();
            // Started again from its files, a few times.
            let restarts = self.restarts.get();
            if restarts < RESTARTS && self.loading.borrow().is_none() {
                self.restarts.set(restarts.saturating_add(1));
                self.load();
            }
        } else {
            self.drop_loading();
            self.fail(out_of_time(limits.load));
        }
    }

    /// The page as given does not load: the last good one (or nothing)
    /// under a banner.
    fn fail(&self, why: String) {
        self.line("error", &why);
        *self.error.borrow_mut() = Some(why);
        self.send_tree();
    }

    /// Draws the last good tree, under a banner when the page is broken.
    fn send_tree(&self) {
        let mut tree = self.last.borrow().clone();
        if let Some(why) = self.problem() {
            let then = if self.error.borrow().is_none() {
                "it was stopped and started again; reload it to take this away"
            } else {
                frame::showing(self.current.borrow().is_some())
            };
            tree.push(frame::banner(&self.name, &why, then));
        }
        let mut edits = Vec::new();
        if self.first.replace(false) {
            edits.extend(frame::root_style());
        }
        edits.extend(self.mirror.borrow_mut().update(&tree));
        self.renders.set(self.renders.get().saturating_add(1));
        let applied = self.pane.borrow_mut().apply(edits);
        if let Err(why) = applied {
            self.line("error", &format!("its edits could not be applied: {why}"));
        }
    }

    /// An event in its pane, for the running page's handler.
    fn event(&self, e: &ViewEvent) {
        let Ok(mirror) = self.mirror.try_borrow() else {
            return;
        };
        let Some(event) = frame::page_event(&mirror, e) else {
            return;
        };
        drop(mirror);
        let text = serde_json::to_string(&event).unwrap_or_default();
        if let Some(run) = self.current.borrow().as_ref() {
            run.post(&json!({ "t": "event", "text": text }));
        }
    }

    /// A host call from worker `id`: answered to it when it is done.
    fn call(&self, id: u64, m: &Json) {
        let text = |k: &str| {
            m.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let (call, kind, a, b) = (
            m.get("id").cloned().unwrap_or(Json::Null),
            text("kind"),
            text("a"),
            text("b"),
        );
        let me = self.me.clone();
        let host = self.host.clone();
        let app = self.app.clone();
        spawn_local(async move {
            let result = answer(&host, &app, &kind, &a, &b).await;
            let Some(page) = me.upgrade() else { return };
            let (ok, text) = match result {
                Ok(t) => (true, t),
                Err(e) => (false, e),
            };
            page.with_run(id, |r| {
                r.post(&json!({ "t": "answer", "id": call, "ok": ok, "text": text }));
            });
        });
    }
}

/// A page's host call (the prelude's `call(kind, a, b)`), answered as its
/// extension: its answer as JSON, or why it failed.
async fn answer(
    host: &WebHost,
    app: &Weak<App>,
    kind: &str,
    a: &str,
    b: &str,
) -> Result<String, String> {
    let json_of = |text: &str| -> Result<Json, String> {
        if text.is_empty() {
            Ok(Json::Null)
        } else {
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))
        }
    };
    match kind {
        "run" => host
            .call(HostRequest::Run {
                action: a.to_owned(),
                args: json_of(b)?,
                ticket: None,
            })
            .await
            .map(|v| v.to_string())
            // Said as the native host says it: the action first.
            .map_err(|e| match e.to_string() {
                said if said.starts_with(a) => said,
                said => format!("{a}: {said}"),
            }),
        "host" => {
            let request: HostRequest =
                serde_json::from_str(a).map_err(|e| format!("not a host request: {e}"))?;
            host.call(request)
                .await
                .map(|v| v.to_string())
                .map_err(|e| e.to_string())
        }
        "actions" => {
            let filter = a.to_lowercase();
            let list: Vec<ActionSummary> = host
                .list()
                .infos()
                .into_iter()
                .filter(|i| host.may_run(i))
                .filter(|i| {
                    filter.is_empty()
                        || [&i.id, &i.display_name, &i.description, &i.group]
                            .iter()
                            .any(|f| f.to_lowercase().contains(&filter))
                })
                .map(|i| ActionSummary {
                    effect: match i.effect {
                        architect::action::Effect::ReadOnly => "read_only",
                        architect::action::Effect::Mutating => "mutating",
                        architect::action::Effect::Destructive => "destructive",
                    }
                    .to_owned(),
                    id: i.id,
                    name: i.display_name,
                    description: i.description,
                    group: i.group,
                    enabled: i.enabled,
                })
                .collect();
            serde_json::to_string(&list).map_err(|e| e.to_string())
        }
        "setting" => Ok(app
            .upgrade()
            .and_then(|app| app.settings.borrow().get(a).cloned())
            .unwrap_or(Json::Null)
            .to_string()),
        "set_setting" => {
            let value = json_of(b)?;
            if let Some(app) = app.upgrade() {
                app.settings.borrow_mut().insert(a.to_owned(), value);
            }
            Ok(String::new())
        }
        other => Err(format!("no host call {other}")),
    }
}
