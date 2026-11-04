//! A page, running (feature `runner`): one thread per page, whatever
//! runs it (`QuickJS`, Luau), and the [`Pages`] an app keeps.
//!
//! The page's thread owns its runtime ([`Engine`]) and its [`Mirror`]:
//! it waits on its inbox ([`Inbox`]: the pane's events, its host calls'
//! answers, shared state, its workers' news), calls into the page, and
//! when the page asked to render, renders it and sends the difference as
//! a [`Frame`] of edits — the window only applies frames. Every event is
//! answered with a frame (empty when nothing changed), so its host times
//! each one (`architect_workspace::ViewMeter`) and marks the pane busy
//! past its budget; a page that runs past [`PageLimits::frame`] in one
//! call is stopped there by its runtime's interrupt.
//!
//! The thread watches the page's folder: a save loads the page again in
//! a fresh runtime and swaps it in once it has rendered. One that fails
//! (it does not parse, its first render throws) leaves the last good page
//! on screen with a banner saying why, and writes `<file>.error`; the
//! next good save takes both away. The theme is followed too: a change
//! renders the page again with the new snapshot.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use architect_extensions::wire::{Edit, EventData, Frame, Value as WireValue, ViewEvent};
use architect_scripting::luau::Gate;
use architect_scripting::vm::Cancel;
use futures::channel::mpsc as fmpsc;

use crate::api::{Attr, DataEvent, Element, PageEvent, ThemeSnapshot, WorkerMessage};
use crate::diff::{Mirror, TEXT};
use crate::files::{self, PageFile, PageKind};
use crate::host::AppHost;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How often a page's folder is looked at for a save.
pub const WATCH_EVERY: Duration = Duration::from_millis(200);

/// How far a page may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageLimits {
    /// The most memory its runtime may hold.
    pub memory: usize,
    /// The longest one call into it (a render, a handler) may run: past
    /// it, the call is stopped with a message to use a worker. Its host
    /// shows it busy long before (a frame's 16 ms).
    pub frame: Duration,
    /// The longest loading it (its module's top level) may run.
    pub load: Duration,
    /// A worker's: memory and its whole run's time.
    pub worker_memory: usize,
    pub worker_time: Duration,
}

impl Default for PageLimits {
    fn default() -> Self {
        Self {
            memory: 64 * 1024 * 1024,
            frame: Duration::from_millis(500),
            load: Duration::from_secs(5),
            worker_memory: 256 * 1024 * 1024,
            worker_time: Duration::from_secs(300),
        }
    }
}

/// What reaches a page's thread.
#[derive(Debug)]
pub enum Inbox {
    /// An event in its pane.
    Event(ViewEvent),
    /// A host call's answer: JSON, or why it failed.
    Answer {
        call: u32,
        result: Result<String, String>,
    },
    /// A shared-state key changed.
    Shared(DataEvent),
    /// One of its workers' news.
    Worker(WorkerMessage),
    /// Look again: something changed (a render was asked from outside).
    Wake,
}

/// A page's runtime, on its thread: what [`Pages`] calls into. Each call
/// runs under the page's limits; an `Err` is why it failed (with its
/// line where the runtime knows it).
pub trait Engine {
    /// The page's top-level nodes now.
    ///
    /// # Errors
    ///
    /// The page threw, or ran past its time.
    fn render(&mut self) -> Result<Vec<Element>, String>;

    /// An event for one of its handlers.
    ///
    /// # Errors
    ///
    /// The handler threw.
    fn event(&mut self, event: PageEvent) -> Result<(), String>;

    /// A host call's answer.
    ///
    /// # Errors
    ///
    /// What it ran then threw.
    fn answer(&mut self, call: u32, result: Result<String, String>) -> Result<(), String>;

    /// A shared-state change.
    ///
    /// # Errors
    ///
    /// A watcher threw.
    fn shared(&mut self, event: &DataEvent) -> Result<(), String>;

    /// A worker's news.
    ///
    /// # Errors
    ///
    /// A callback threw.
    fn worker(&mut self, message: &WorkerMessage) -> Result<(), String>;

    /// The theme changed: its new snapshot.
    ///
    /// # Errors
    ///
    /// A subscriber threw.
    fn theme(&mut self, theme: &ThemeSnapshot) -> Result<(), String>;

    /// Runs what is due (its timers); how soon it wants to run next.
    ///
    /// # Errors
    ///
    /// A timer's function threw.
    fn tick(&mut self) -> Result<Option<Duration>, String>;

    /// Whether it asked to render.
    fn dirty(&self) -> bool;
}

/// What a page's runtime is given: its host, its inbox (for the answers
/// of what it starts), its log, its stop, its limits.
#[derive(Clone)]
pub struct Services {
    pub host: Arc<AppHost>,
    pub inbox: mpsc::Sender<Inbox>,
    pub log: Log,
    pub stop: Cancel,
    pub limits: PageLimits,
    /// The extension it is a page of (its log's name).
    pub extension: String,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field("extension", &self.extension)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// Makes a page's runtime for a file, on the page's thread.
pub type Load = Arc<dyn Fn(&PageFile, &Services) -> Result<Box<dyn Engine>, String> + Send + Sync>;

/// A page's log: what it printed (`console.log`, `print`), the last few
/// hundred lines, each also a `tracing` event under its extension.
#[derive(Clone, Debug, Default)]
pub struct Log(Arc<Mutex<VecDeque<String>>>);

impl Log {
    pub const KEPT: usize = 300;

    pub fn line(&self, extension: &str, level: &str, text: &str) {
        match level {
            "error" => tracing::error!(extension, "{text}"),
            "warn" => tracing::warn!(extension, "{text}"),
            _ => tracing::info!(extension, "{text}"),
        }
        let mut lines = lock(&self.0);
        lines.push_back(format!("[{level}] {text}"));
        while lines.len() > Self::KEPT {
            lines.pop_front();
        }
        drop(lines);
    }

    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        lock(&self.0).iter().cloned().collect()
    }
}

/// How a running page is: what [`Pages::problems`] and a pane read.
#[derive(Debug)]
pub struct PageStatus {
    pub page: PageFile,
    pub extension: String,
    pub view: String,
    error: Mutex<Option<String>>,
    renders: AtomicU64,
    reloads: AtomicU64,
    /// Nanoseconds: the last render on its thread, and the last reload
    /// (from noticing the save to its frame).
    last_render: AtomicU64,
    last_reload: AtomicU64,
    pub log: Log,
}

impl PageStatus {
    /// Why it is not the page as saved (the last good one shows), if so.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }

    #[must_use]
    pub fn renders(&self) -> u64 {
        self.renders.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn reloads(&self) -> u64 {
        self.reloads.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn last_render(&self) -> Duration {
        Duration::from_nanos(self.last_render.load(Ordering::Relaxed))
    }

    /// From noticing a save to the frame that shows it.
    #[must_use]
    pub fn last_reload(&self) -> Duration {
        Duration::from_nanos(self.last_reload.load(Ordering::Relaxed))
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// A page open in a pane: its frames, and the way its events go in.
/// Dropping it stops the page and ends its thread.
pub struct PageSession {
    /// What it renders, as edits; ends when it stops.
    pub frames: fmpsc::UnboundedReceiver<Frame>,
    inbox: mpsc::Sender<Inbox>,
    stop: Cancel,
    status: Arc<PageStatus>,
}

impl std::fmt::Debug for PageSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageSession")
            .field("page", &self.status.page.path)
            .finish_non_exhaustive()
    }
}

impl PageSession {
    /// An event in its pane, for its handlers.
    pub fn event(&self, event: ViewEvent) {
        let _ = self.inbox.send(Inbox::Event(event));
    }

    /// A way to send it events from elsewhere (the pane's forwarder).
    #[must_use]
    pub fn sender(&self) -> PageSender {
        PageSender(self.inbox.clone())
    }

    #[must_use]
    pub const fn status(&self) -> &Arc<PageStatus> {
        &self.status
    }
}

impl Drop for PageSession {
    fn drop(&mut self) {
        self.stop.set();
        let _ = self.inbox.send(Inbox::Wake);
    }
}

/// Sends a page its pane's events.
#[derive(Clone, Debug)]
pub struct PageSender(mpsc::Sender<Inbox>);

impl PageSender {
    /// False once the page has stopped.
    #[must_use]
    pub fn event(&self, event: ViewEvent) -> bool {
        self.0.send(Inbox::Event(event)).is_ok()
    }
}

/// A page's problem, for the app's status (`APP_STATUS` lists them: the
/// seam #270's tools read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageProblem {
    pub extension: String,
    pub view: String,
    pub file: PathBuf,
    pub error: String,
}

/// What rendering a page once found ([`Pages::check`], `EXT_CHECK`'s
/// seam).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// How many nodes its first render drew.
    pub nodes: usize,
    pub took: Duration,
}

/// An app's pages: where each view's page file is, what runs each kind,
/// and the pages running now. Cheap to clone; clones share it.
#[derive(Clone)]
pub struct Pages(Arc<PagesInner>);

struct PagesInner {
    host: AppHost,
    app_dir: Option<PathBuf>,
    loaders: BTreeMap<&'static str, Load>,
    /// A view's own page (its manifest's `page`), by extension and view.
    declared: Mutex<BTreeMap<(String, String), PageFile>>,
    running: Mutex<Vec<Weak<PageStatus>>>,
    limits: PageLimits,
}

impl std::fmt::Debug for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pages")
            .field("app_dir", &self.0.app_dir)
            .field("kinds", &self.0.loaders.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Pages {
    /// Pages over `host` (the app's Action List, theme, shared state):
    /// none runnable until a kind's runner is added.
    #[must_use]
    pub fn new(host: AppHost) -> Self {
        Self(Arc::new(PagesInner {
            host,
            app_dir: None,
            loaders: BTreeMap::new(),
            declared: Mutex::new(BTreeMap::new()),
            running: Mutex::new(Vec::new()),
            limits: PageLimits::default(),
        }))
    }

    /// Configures it: before it is cloned (a clone's configuration is
    /// shared, so a change after is refused, and said).
    fn configure(mut self, f: impl FnOnce(&mut PagesInner)) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.0) {
            f(inner);
        } else {
            tracing::warn!("Pages is configured before it is cloned: this change is dropped");
        }
        self
    }

    /// Its overrides are in `dir`'s `pages/`.
    #[must_use]
    pub fn with_app_dir(self, dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        self.configure(|p| p.app_dir = Some(dir))
    }

    #[must_use]
    pub fn with_limits(self, limits: PageLimits) -> Self {
        self.configure(|p| p.limits = limits)
    }

    /// Pages of `kind` run with `load`.
    #[must_use]
    pub fn with_runner(self, kind: PageKind, load: Load) -> Self {
        self.configure(|p| {
            p.loaders.insert(kind.extension(), load);
        })
    }

    /// TSX/TS/JSX/JS pages run on `QuickJS` (`crate::js`).
    #[cfg(feature = "js")]
    #[must_use]
    pub fn with_js(self) -> Self {
        let load = crate::js::loader();
        [PageKind::Tsx, PageKind::Ts, PageKind::Jsx, PageKind::Js]
            .into_iter()
            .fold(self, |pages, kind| {
                pages.with_runner(kind, Arc::clone(&load))
            })
    }

    /// Luau element pages (`crate::luau`).
    #[cfg(feature = "luau")]
    #[must_use]
    pub fn with_luau(self) -> Self {
        self.with_runner(PageKind::Luau, crate::luau::loader())
    }

    #[must_use]
    pub fn app_dir(&self) -> Option<&Path> {
        self.0.app_dir.as_deref()
    }

    #[must_use]
    pub fn host(&self) -> &AppHost {
        &self.0.host
    }

    /// Whether it runs pages of `kind`.
    #[must_use]
    pub fn runs(&self, kind: PageKind) -> bool {
        kind == PageKind::A2ui || self.0.loaders.contains_key(kind.extension())
    }

    /// `extension`'s view `view` has its own page, `page` (its manifest's
    /// `page = "pages/x.tsx"`, or a built-in's read-only copy).
    pub fn declare(&self, extension: &str, view: &str, page: PageFile) {
        lock(&self.0.declared).insert((extension.to_owned(), view.to_owned()), page);
    }

    /// Forgets `extension`'s pages (it unloaded).
    pub fn forget(&self, extension: &str) {
        lock(&self.0.declared).retain(|(e, _), _| e != extension);
    }

    /// The page `extension`'s view `view` shows now: an override in the
    /// app's folder, else its own page, else none (its compiled view).
    #[must_use]
    pub fn page_for(&self, extension: &str, view: &str) -> Option<PageFile> {
        self.0
            .app_dir
            .as_deref()
            .and_then(|dir| files::find_override(dir, extension, view))
            .filter(|p| self.runs(p.kind))
            .or_else(|| {
                lock(&self.0.declared)
                    .get(&(extension.to_owned(), view.to_owned()))
                    .cloned()
            })
    }

    /// Opens `page` as `extension`'s view `view`: its thread starts, its
    /// first frame follows. `gate` keeps it to its extension's reach.
    ///
    /// # Errors
    ///
    /// Nothing runs pages of its kind, or its thread did not start.
    pub fn open(
        &self,
        page: &PageFile,
        extension: &str,
        view: &str,
        gate: Option<Gate>,
    ) -> Result<PageSession, String> {
        let load = self
            .0
            .loaders
            .get(page.kind.extension())
            .cloned()
            .ok_or_else(|| format!("this app does not run .{} pages", page.kind.extension()))?;
        let mut host = self.0.host.clone();
        if let Some(gate) = gate {
            host = host.with_gate(gate);
        }
        let (inbox, asked) = mpsc::channel();
        let (frames_to, frames) = fmpsc::unbounded();
        let stop = Cancel::default();
        let status = Arc::new(PageStatus {
            page: page.clone(),
            extension: extension.to_owned(),
            view: view.to_owned(),
            error: Mutex::new(None),
            renders: AtomicU64::new(0),
            reloads: AtomicU64::new(0),
            last_render: AtomicU64::new(0),
            last_reload: AtomicU64::new(0),
            log: Log::default(),
        });
        {
            let mut running = lock(&self.0.running);
            running.retain(|w| w.strong_count() > 0);
            running.push(Arc::downgrade(&status));
        }
        let services = Services {
            host: Arc::new(host),
            inbox: inbox.clone(),
            log: status.log.clone(),
            stop: stop.clone(),
            limits: self.0.limits,
            extension: extension.to_owned(),
        };
        let theirs = Arc::clone(&status);
        std::thread::Builder::new()
            .name(format!("page {extension}/{view}"))
            .spawn(move || PageThread::new(theirs, load, services, frames_to).run(&asked))
            .map_err(|e| format!("its thread did not start: {e}"))?;
        Ok(PageSession {
            frames,
            inbox,
            stop,
            status,
        })
    }

    /// Every running page that is not the page as saved, and why.
    #[must_use]
    pub fn problems(&self) -> Vec<PageProblem> {
        lock(&self.0.running)
            .iter()
            .filter_map(Weak::upgrade)
            .filter_map(|s| {
                s.error().map(|error| PageProblem {
                    extension: s.extension.clone(),
                    view: s.view.clone(),
                    file: s.page.path.clone(),
                    error,
                })
            })
            .collect()
    }

    /// The pages running now.
    #[must_use]
    pub fn running(&self) -> Vec<Arc<PageStatus>> {
        lock(&self.0.running)
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// Loads `page` and renders it once, on a thread of its own, as
    /// `EXT_CHECK` does for an extension's views: how many nodes it drew.
    ///
    /// # Errors
    ///
    /// Why it would not load or render, with its line where known.
    pub fn check(&self, page: &PageFile) -> Result<Checked, String> {
        let load = self
            .0
            .loaders
            .get(page.kind.extension())
            .cloned()
            .ok_or_else(|| format!("this app does not run .{} pages", page.kind.extension()))?;
        let (inbox, _asked) = mpsc::channel();
        let services = Services {
            host: Arc::new(self.0.host.clone()),
            inbox,
            log: Log::default(),
            stop: Cancel::default(),
            limits: self.0.limits,
            extension: "check".to_owned(),
        };
        let page = page.clone();
        std::thread::Builder::new()
            .name("page check".to_owned())
            .spawn(move || {
                let started = Instant::now();
                let mut engine = load(&page, &services)?;
                let tree = engine.render()?;
                let mut mirror = Mirror::new();
                let _ = mirror.update(&tree);
                Ok(Checked {
                    nodes: mirror.len(),
                    took: started.elapsed(),
                })
            })
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "its check panicked".to_owned())?
    }
}

/// A page's thread.
struct PageThread {
    status: Arc<PageStatus>,
    load: Load,
    services: Services,
    frames: fmpsc::UnboundedSender<Frame>,
    engine: Option<Box<dyn Engine>>,
    mirror: Mirror,
    /// The page's nodes as last drawn, to draw under a banner.
    last: Vec<Element>,
    print: u64,
    theme: u64,
    first: bool,
}

impl PageThread {
    fn new(
        status: Arc<PageStatus>,
        load: Load,
        services: Services,
        frames: fmpsc::UnboundedSender<Frame>,
    ) -> Self {
        Self {
            print: files::fingerprint(&status.page),
            theme: services.host.theme_revision(),
            status,
            load,
            services,
            frames,
            engine: None,
            mirror: Mirror::new(),
            last: Vec::new(),
            first: true,
        }
    }

    fn run(mut self, asked: &mpsc::Receiver<Inbox>) {
        self.reload(None);
        let mut next_tick = self.tick();
        let mut watched = Instant::now();
        while !self.services.stop.is_set() {
            let now = Instant::now();
            let until_watch = WATCH_EVERY.saturating_sub(now.duration_since(watched));
            let wait = next_tick.map_or(until_watch, |t| {
                t.saturating_duration_since(now).min(until_watch)
            });
            let mut answered = false;
            match asked.recv_timeout(wait) {
                Ok(first) => {
                    answered |= self.handle(first);
                    while let Ok(more) = asked.try_recv() {
                        answered |= self.handle(more);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if self.services.stop.is_set() {
                return;
            }
            // Its timers that are due, then the folder and the theme.
            let _ = self.tick();
            if watched.elapsed() >= WATCH_EVERY {
                watched = Instant::now();
                self.watch();
            }
            self.frame(answered);
            // What the render started (an effect's timer) is due when.
            next_tick = self.tick();
            if self.engine.as_ref().is_some_and(|e| e.dirty()) {
                next_tick = Some(Instant::now());
            }
        }
    }

    /// One message; whether it was an event (which a frame answers).
    fn handle(&mut self, msg: Inbox) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return matches!(msg, Inbox::Event(_));
        };
        let (result, event) = match msg {
            Inbox::Event(e) => (
                page_event(&self.mirror, &e).map_or(Ok(()), |pe| engine.event(pe)),
                true,
            ),
            Inbox::Answer { call, result } => (engine.answer(call, result), false),
            Inbox::Shared(e) => (engine.shared(&e), false),
            Inbox::Worker(m) => (engine.worker(&m), false),
            Inbox::Wake => (Ok(()), false),
        };
        if let Err(why) = result {
            self.failed_call(&why);
        }
        event
    }

    fn tick(&mut self) -> Option<Instant> {
        let engine = self.engine.as_mut()?;
        match engine.tick() {
            Ok(next) => next.and_then(|d| Instant::now().checked_add(d)),
            Err(why) => {
                self.failed_call(&why);
                None
            }
        }
    }

    /// A handler or timer threw: logged, the page runs on (a render that
    /// throws is a failed page; a handler that throws is one bad click).
    fn failed_call(&self, why: &str) {
        self.status.log.line(&self.services.extension, "error", why);
    }

    /// Looks at the page's folder and the theme.
    fn watch(&mut self) {
        let print = files::fingerprint(&self.status.page);
        if print != self.print {
            self.print = print;
            let noticed = Instant::now();
            self.reload(Some(noticed));
        }
        let theme = self.services.host.theme_revision();
        if theme != self.theme {
            self.theme = theme;
            let snapshot = self.services.host.theme_snapshot();
            if let Some(engine) = self.engine.as_mut()
                && let Err(why) = engine.theme(&snapshot)
            {
                self.failed_call(&why);
            }
        }
    }

    /// Loads the page in a fresh runtime and renders it; swapped in when
    /// both work, the last good one kept (and `.error` written) when not.
    fn reload(&mut self, noticed: Option<Instant>) {
        let page = self.status.page.clone();
        if !page.path.is_file() {
            self.fail(format!("{} is gone", page.path.display()));
            return;
        }
        let loaded = (self.load)(&page, &self.services).and_then(|mut engine| {
            let started = Instant::now();
            let tree = engine.render()?;
            self.status
                .last_render
                .store(nanos(started.elapsed()), Ordering::Relaxed);
            Ok((engine, tree))
        });
        match loaded {
            Ok((engine, tree)) => {
                self.engine = Some(engine);
                *lock(&self.status.error) = None;
                files::clear_error(&page);
                self.last = tree;
                self.send_tree();
                if let Some(at) = noticed {
                    self.status.reloads.fetch_add(1, Ordering::Relaxed);
                    self.status
                        .last_reload
                        .store(nanos(at.elapsed()), Ordering::Relaxed);
                }
            }
            Err(why) => self.fail(why),
        }
    }

    /// The page as saved does not work: the last good one (or nothing)
    /// under a banner, and `.error` beside it.
    fn fail(&mut self, why: String) {
        files::write_error(&self.status.page, &why);
        self.status
            .log
            .line(&self.services.extension, "error", &why);
        *lock(&self.status.error) = Some(why);
        self.send_tree();
    }

    /// Renders when the page asked (or an event wants its answer).
    fn frame(&mut self, answered: bool) {
        let dirty = self.engine.as_ref().is_some_and(|e| e.dirty());
        if dirty && let Some(engine) = self.engine.as_mut() {
            let started = Instant::now();
            match engine.render() {
                Ok(tree) => {
                    self.status
                        .last_render
                        .store(nanos(started.elapsed()), Ordering::Relaxed);
                    self.last = tree;
                    // A render that works again clears a render's error,
                    // not a load's (its file is still broken).
                    let stale = lock(&self.status.error)
                        .as_deref()
                        .is_some_and(|e| e.starts_with(RENDER_FAILED));
                    if stale {
                        *lock(&self.status.error) = None;
                        files::clear_error(&self.status.page);
                    }
                }
                Err(why) => {
                    let why = format!("{RENDER_FAILED}{why}");
                    files::write_error(&self.status.page, &why);
                    *lock(&self.status.error) = Some(why);
                }
            }
            self.send_tree();
        } else if answered || self.first {
            self.send(Vec::new());
        }
    }

    /// Draws the last good tree, under a banner when the page is broken.
    fn send_tree(&mut self) {
        let mut tree = self.last.clone();
        if let Some(why) = self.status.error() {
            tree.push(banner(&self.status.page, &why, self.engine.is_some()));
        }
        let mut edits = Vec::new();
        if self.first {
            edits.extend(root_style());
        }
        edits.extend(self.mirror.update(&tree));
        self.status.renders.fetch_add(1, Ordering::Relaxed);
        self.send(edits);
    }

    fn send(&mut self, mut edits: Vec<Edit>) {
        if self.first {
            self.first = false;
            if edits.is_empty() {
                edits.extend(root_style());
            }
        }
        if self.frames.unbounded_send(Frame::Edits { edits }).is_err() {
            self.services.stop.set();
        }
    }
}

/// A render that threw: its error says so (and goes when one works).
const RENDER_FAILED: &str = "its render failed: ";

/// The pane's own element: filling it, in the theme's colours.
fn root_style() -> Vec<Edit> {
    vec![
        Edit::PushId { id: 0 },
        Edit::SetAttribute {
            name: "style".to_owned(),
            namespace: None,
            value: WireValue::Text(
                "display: block; position: relative; min-height: 100%; box-sizing: border-box; \
                 color: var(--foreground); font-family: -apple-system, system-ui, sans-serif; font-size: 13px;"
                    .to_owned(),
            ),
        },
        Edit::Pop,
    ]
}

/// The banner over a broken page.
fn banner(page: &PageFile, why: &str, kept: bool) -> Element {
    let name = page
        .path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let then = if kept {
        "the last good page is showing"
    } else {
        "nothing to show until it loads"
    };
    let first = why.lines().next().unwrap_or(why);
    let text = |t: String| Element {
        tag: TEXT.to_owned(),
        text: Some(t),
        ..Element::default()
    };
    Element {
        tag: "div".to_owned(),
        key: Some("architect-page-error".to_owned()),
        attrs: vec![
            Attr {
                name: "class".to_owned(),
                value: "page-error".to_owned(),
            },
            Attr {
                name: "style".to_owned(),
                value: "position: absolute; left: 8px; right: 8px; bottom: 8px; padding: 8px 12px; \
                        border-radius: 8px; font-size: 12px; white-space: pre-wrap; \
                        background: var(--destructive, #b91c1c); color: var(--destructive-foreground, #fff);"
                    .to_owned(),
            },
        ],
        children: vec![text(format!("⚠ {name}: {first}\n{then}; why is in {name}.error"))],
        ..Element::default()
    }
}

/// The page's view of a pane event: the handler it is for, and what it
/// carries. `None` when nothing listens for it there.
fn page_event(mirror: &Mirror, e: &ViewEvent) -> Option<PageEvent> {
    let handler = mirror.handler(e.element, &e.name)?;
    let mut out = PageEvent {
        handler,
        event: e.name.clone(),
        ..PageEvent::default()
    };
    match &e.data {
        EventData::Mouse(p) => {
            out.x = p.element_x;
            out.y = p.element_y;
            (out.alt, out.ctrl, out.meta, out.shift) =
                (p.mods.alt, p.mods.ctrl, p.mods.meta, p.mods.shift);
        }
        EventData::Key(k) => {
            out.key = Some(k.key.clone());
            (out.alt, out.ctrl, out.meta, out.shift) =
                (k.mods.alt, k.mods.ctrl, k.mods.meta, k.mods.shift);
        }
        EventData::Form(value) => out.value = Some(value.clone()),
        EventData::Focus => {}
    }
    Some(out)
}
