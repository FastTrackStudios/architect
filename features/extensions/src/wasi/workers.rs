//! A WASI component's workers (`docs/content/architecture/extensions.md`,
//! "WASI components", "Workers").
//!
//! There are no threads in a component: its parallelism is more instances
//! of it. A `[[workers]]` entry of a `wasm` package names what a worker
//! runs — an `entry` of the component itself (its `work.run` is given
//! the name), or a separate component in the package (`*.wasm`, given the
//! worker's id). Its owner, the extension's instance, spawns it
//! (`worker.spawn`); the host instantiates the compiled component again
//! (its code shared, from the compile cache) on a thread of its [`Pool`],
//! with a memory limit and an epoch deadline of its own
//! ([`WasiRunner::with_worker_limits`](super::WasiRunner::with_worker_limits)),
//! and the two speak by JSON messages only — the same API as a Luau
//! worker's (`architect_scripting::worker`) and a process's work
//! ([`crate::work`]):
//!
//! | the owner | the worker |
//! |---|---|
//! | `spawn(name, args)`: its handle | `run(entry, worker, args)`: its result |
//! | `post(w, msg)`: Err when its queue is full | `post(0, msg)`: waits while its owner's is full |
//! | `cancel(w)` | `cancelled()`, `receive(ms)` |
//! | `on-message`, `on-progress`, `on-done` | `progress(done, total, note)` |
//!
//! - **back-pressure**: each way a queue holds [`QUEUE`] messages; an
//!   owner (a view: it never waits) is told its post did not go, a
//!   worker waits (that time is the host's, not its own). An instance has
//!   at most [`MAX_WORKERS`] running or waiting for the pool.
//! - **news**: an owner hears its workers' messages in order, their
//!   progress coalesced (the latest), at most once each
//!   [`NEWS_EVERY`], between its own calls — so a click on its view is
//!   never queued behind more than one round of news.
//! - **cancellation**: a worker is stopped at its next epoch tick (10
//!   ms) when its owner cancels it, traps (and is started again) or is
//!   unloaded — whether it looks at `cancelled()` or not.
//! - **limits**: a worker past its memory or its time traps alone: its
//!   owner hears `failed` with why; its owner's instance runs on.
//! - **the pool**: threads start as they are wanted, up to its size, and
//!   stop once idle ([`Pool`]).

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use wasmtime::component::{Component, Linker};

use super::bindings::architect::extension::worker as wit_worker;
use super::bindings::exports::architect::extension::work as wit_work;
use super::{Limits, State, World, begin, end};
use crate::hostapi::ExtensionApi;
use crate::logs::{Level, Logs};

/// Messages a queue holds, each way, before back-pressure.
pub const QUEUE: usize = 64;

/// Workers one instance may have running or waiting for the pool.
pub const MAX_WORKERS: usize = 8;

/// How often, at most, an owner hears its workers' news.
pub const NEWS_EVERY: Duration = Duration::from_millis(16);

/// How long a worker waits at a time for room or a message before it
/// looks whether it was cancelled.
const WAIT_STEP: Duration = Duration::from_millis(10);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a worker is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkerStatus {
    /// Spawned, waiting for a thread of the pool.
    #[default]
    Waiting,
    Running,
    Done,
    /// It returned an error, or ran past a limit.
    Failed,
    /// Cancelled before it finished: by its owner, or its owner going.
    Cancelled,
}

impl WorkerStatus {
    /// `waiting`, `running`, `done`, `failed` or `cancelled`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether it has ended.
    #[must_use]
    pub const fn ended(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// One worker, as the host sees it ([`super::Stats::workers`]).
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerInfo {
    /// Its handle, as its owner has it.
    pub id: u64,
    /// Its `[[workers]]` id.
    pub name: String,
    pub status: WorkerStatus,
    /// Messages it posted its owner has not heard yet.
    pub to_owner: usize,
    /// Messages its owner posted it has not read yet.
    pub to_worker: usize,
    /// `done` and `total` it last said.
    pub progress: Option<(f64, f64)>,
    /// Its instance's memory now, in bytes.
    pub memory: usize,
}

/// How a worker ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Done(String),
    Failed(String),
    Cancelled,
}

impl Outcome {
    fn to_wit(&self) -> wit_work::Outcome {
        match self {
            Self::Done(r) => wit_work::Outcome::Done(r.clone()),
            Self::Failed(why) => wit_work::Outcome::Failed(why.clone()),
            Self::Cancelled => wit_work::Outcome::Cancelled,
        }
    }

    const fn status(&self) -> WorkerStatus {
        match self {
            Self::Done(_) => WorkerStatus::Done,
            Self::Failed(_) => WorkerStatus::Failed,
            Self::Cancelled => WorkerStatus::Cancelled,
        }
    }
}

/// Wakes an owner for its workers' news: once, until it has heard.
#[derive(Clone, Debug)]
pub(super) struct Nudge {
    pending: Arc<AtomicBool>,
    to: mpsc::UnboundedSender<()>,
}

impl Nudge {
    pub(super) fn new() -> (Self, mpsc::UnboundedReceiver<()>) {
        let (to, from) = mpsc::unbounded();
        (
            Self {
                pending: Arc::new(AtomicBool::new(false)),
                to,
            },
            from,
        )
    }

    fn nudge(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let _ = self.to.unbounded_send(());
        }
    }

    /// The owner is hearing it now: news from here on wakes it again.
    fn heard(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

#[derive(Default)]
struct Mail {
    status: WorkerStatus,
    to_owner: VecDeque<String>,
    to_worker: VecDeque<String>,
    progress: Option<wit_worker::ProgressReport>,
    /// Said since its owner last heard.
    progress_new: bool,
    outcome: Option<Outcome>,
    /// Its owner heard how it ended.
    told: bool,
}

/// What a worker reports, since its owner last heard.
pub(super) struct News {
    pub(super) messages: Vec<String>,
    pub(super) progress: Option<wit_worker::ProgressReport>,
    pub(super) outcome: Option<Outcome>,
}

/// One worker: its two queues, its progress, how it ended.
pub(super) struct Link {
    id: u64,
    name: String,
    entry: String,
    args: String,
    component: Component,
    cancel: AtomicBool,
    mail: Mutex<Mail>,
    /// A queue changed, or it was cancelled.
    changed: Condvar,
    memory: Arc<AtomicUsize>,
    nudge: Nudge,
}

impl Link {
    pub(super) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    fn status(&self) -> WorkerStatus {
        lock(&self.mail).status
    }

    fn set_status(&self, status: WorkerStatus) {
        lock(&self.mail).status = status;
    }

    /// The worker's message to its owner: waits while its owner's queue is
    /// full.
    pub(super) fn post_to_owner(&self, message: String) -> Result<(), String> {
        let mut mail = lock(&self.mail);
        loop {
            if self.cancelled() {
                return Err("cancelled".to_owned());
            }
            if mail.to_owner.len() < QUEUE {
                mail.to_owner.push_back(message);
                drop(mail);
                self.nudge.nudge();
                return Ok(());
            }
            mail = self
                .changed
                .wait_timeout(mail, WAIT_STEP)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The next message from its owner, waiting at most `wait`.
    pub(super) fn receive(&self, wait: Duration) -> Option<String> {
        let until = Instant::now().checked_add(wait).unwrap_or_else(Instant::now);
        let mut mail = lock(&self.mail);
        loop {
            if let Some(m) = mail.to_worker.pop_front() {
                return Some(m);
            }
            let left = until.saturating_duration_since(Instant::now());
            if self.cancelled() || left.is_zero() {
                return None;
            }
            mail = self
                .changed
                .wait_timeout(mail, left.min(WAIT_STEP))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub(super) fn progress(&self, report: wit_worker::ProgressReport) {
        {
            let mut mail = lock(&self.mail);
            mail.progress = Some(report);
            mail.progress_new = true;
        }
        self.nudge.nudge();
    }

    /// The owner's message to it: never waits.
    fn post_to_worker(&self, message: String) -> Result<(), String> {
        let mut mail = lock(&self.mail);
        if mail.status.ended() || self.cancelled() {
            return Err(format!("worker {} ({}) has ended", self.id, self.name));
        }
        if mail.to_worker.len() >= QUEUE {
            return Err(format!(
                "worker {}'s queue is full: {QUEUE} messages it has not read",
                self.id
            ));
        }
        mail.to_worker.push_back(message);
        drop(mail);
        self.changed.notify_all();
        Ok(())
    }

    fn finish(&self, outcome: Outcome) {
        {
            let mut mail = lock(&self.mail);
            mail.status = outcome.status();
            mail.outcome = Some(outcome);
            mail.to_worker.clear();
        }
        self.changed.notify_all();
        self.nudge.nudge();
    }

    /// What it said since its owner last heard; its end, once.
    fn news(&self) -> News {
        let news = {
            let mut mail = lock(&self.mail);
            let progress = if mail.progress_new {
                mail.progress_new = false;
                mail.progress.clone()
            } else {
                None
            };
            let outcome = if mail.told {
                None
            } else {
                mail.outcome.clone()
            };
            if outcome.is_some() {
                mail.told = true;
            }
            News {
                messages: mail.to_owner.drain(..).collect(),
                progress,
                outcome,
            }
        };
        // Room in its owner's queue: a worker waiting to post goes on.
        self.changed.notify_all();
        news
    }

    fn info(&self) -> WorkerInfo {
        let mail = lock(&self.mail);
        WorkerInfo {
            id: self.id,
            name: self.name.clone(),
            status: mail.status,
            to_owner: mail.to_owner.len(),
            to_worker: mail.to_worker.len(),
            progress: mail.progress.as_ref().map(|p| (p.done, p.total)),
            memory: self.memory.load(Ordering::Relaxed),
        }
    }
}

/// What a worker of one `[[workers]]` entry runs.
#[derive(Clone)]
pub(super) struct Entry {
    /// The component: the extension's own, or a separate one.
    pub(super) component: Component,
    /// What its `run` is given.
    pub(super) entry: String,
}

/// What every worker of one extension is made from.
pub(super) struct WorkerRecipe {
    pub(super) id: String,
    pub(super) entries: BTreeMap<String, Entry>,
    pub(super) linker: Arc<Linker<State>>,
    pub(super) limits: Limits,
    pub(super) api: ExtensionApi,
    pub(super) logs: Logs,
    pub(super) pool: Pool,
}

impl WorkerRecipe {
    fn log(&self, level: Level, text: impl Into<String>) {
        self.logs.push(&self.id, level, "worker", text);
    }
}

#[derive(Default)]
struct CrewInner {
    links: Mutex<BTreeMap<u64, Arc<Link>>>,
    next: AtomicU64,
    /// Worker instances alive (their stores held).
    alive: Arc<AtomicUsize>,
}

/// One instance's workers: what its `worker` imports reach, and its
/// actor drains. Cheap to clone; clones share it.
#[derive(Clone)]
pub(super) struct Crew {
    inner: Arc<CrewInner>,
    recipe: Arc<WorkerRecipe>,
    nudge: Nudge,
}

impl std::fmt::Debug for Crew {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crew")
            .field("workers", &lock(&self.inner.links).len())
            .finish_non_exhaustive()
    }
}

impl Crew {
    pub(super) fn new(recipe: WorkerRecipe, nudge: Nudge) -> Self {
        Self {
            inner: Arc::default(),
            recipe: Arc::new(recipe),
            nudge,
        }
    }

    /// Starts worker `name` with `args`: its handle.
    pub(super) fn spawn(&self, name: &str, args: String) -> Result<u64, String> {
        let entry = self
            .recipe
            .entries
            .get(name)
            .ok_or_else(|| format!("it has no worker {name} (its [[workers]])"))?
            .clone();
        let mut links = lock(&self.inner.links);
        let live = links.values().filter(|l| !l.status().ended()).count();
        if live >= MAX_WORKERS {
            return Err(format!(
                "it has {MAX_WORKERS} workers running already: one must end first"
            ));
        }
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let link = Arc::new(Link {
            id,
            name: name.to_owned(),
            entry: entry.entry,
            args,
            component: entry.component,
            cancel: AtomicBool::new(false),
            mail: Mutex::new(Mail::default()),
            changed: Condvar::new(),
            memory: Arc::new(AtomicUsize::new(0)),
            nudge: self.nudge.clone(),
        });
        links.insert(id, Arc::clone(&link));
        drop(links);
        self.inner.alive.fetch_add(1, Ordering::AcqRel);
        let (recipe, alive) = (Arc::clone(&self.recipe), Arc::clone(&self.inner.alive));
        let theirs = Arc::clone(&link);
        self.recipe.pool.submit(Task {
            link,
            run: Box::new(move |rt| {
                let outcome = rt.block_on(work(&recipe, &theirs));
                if let Outcome::Failed(why) = &outcome {
                    recipe.log(Level::Warn, format!("worker {} failed: {why}", theirs.name));
                }
                theirs.finish(outcome);
                alive.fetch_sub(1, Ordering::AcqRel);
            }),
            alive: Arc::clone(&self.inner.alive),
        });
        Ok(id)
    }

    pub(super) fn post(&self, to: u64, message: String) -> Result<(), String> {
        let link = lock(&self.inner.links)
            .get(&to)
            .cloned()
            .ok_or_else(|| format!("it has no worker {to}"))?;
        link.post_to_worker(message)
    }

    pub(super) fn cancel(&self, worker: u64) {
        if let Some(link) = lock(&self.inner.links).get(&worker) {
            link.cancel();
        }
    }

    /// Cancels every worker: its owner went, or is starting again. They
    /// are forgotten: a fresh instance hears nothing of them.
    pub(super) fn cancel_all(&self) -> usize {
        let links = std::mem::take(&mut *lock(&self.inner.links));
        let mut running: usize = 0;
        for link in links.values() {
            if !link.status().ended() {
                running = running.saturating_add(1);
            }
            link.cancel();
        }
        running
    }

    /// Every worker's news since the owner last heard, in the order they
    /// were spawned; the ended ones are forgotten once heard.
    pub(super) fn news(&self) -> Vec<(u64, News)> {
        self.nudge.heard();
        let links: Vec<Arc<Link>> = lock(&self.inner.links).values().cloned().collect();
        let mut out = Vec::new();
        for link in links {
            let news = link.news();
            if news.outcome.is_some() {
                lock(&self.inner.links).remove(&link.id);
            }
            if !news.messages.is_empty() || news.progress.is_some() || news.outcome.is_some() {
                out.push((link.id, news));
            }
        }
        out
    }

    pub(super) fn infos(&self) -> Vec<WorkerInfo> {
        lock(&self.inner.links).values().map(|l| l.info()).collect()
    }

    pub(super) fn alive(&self) -> usize {
        self.inner.alive.load(Ordering::Acquire)
    }
}

/// A worker's instance, from start to end: how it ended.
async fn work(recipe: &WorkerRecipe, link: &Arc<Link>) -> Outcome {
    if link.cancelled() {
        return Outcome::Cancelled;
    }
    link.set_status(WorkerStatus::Running);
    let data = State::new(
        &recipe.id,
        &recipe.logs,
        recipe.limits,
        Arc::clone(&link.memory),
        recipe.api.clone(),
        std::collections::BTreeSet::new(),
        None,
        super::Side::Worker(Arc::clone(link)),
    );
    let ended = |why: String| {
        if link.cancelled() {
            Outcome::Cancelled
        } else {
            Outcome::Failed(why)
        }
    };
    let mut store = match super::store(data) {
        Ok(store) => store,
        Err(why) => return Outcome::Failed(why),
    };
    begin(&mut store);
    let made = World::instantiate_async(&mut store, &link.component, &recipe.linker).await;
    let world = match end(&mut store, made) {
        Ok(world) => world,
        Err(why) => return ended(format!("it does not instantiate: {why}")),
    };
    begin(&mut store);
    let init = world
        .architect_extension_guest()
        .call_init(&mut store, None)
        .await;
    match end(&mut store, init) {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return ended(format!("its init failed: {why}")),
        Err(why) => return ended(why),
    }
    begin(&mut store);
    let ran = world
        .architect_extension_work()
        .call_run(&mut store, &link.entry, link.id, &link.args)
        .await;
    match end(&mut store, ran) {
        Ok(Ok(result)) => Outcome::Done(result),
        Ok(Err(why)) | Err(why) => ended(why),
    }
}

/// A worker waiting for a thread: what it runs, on that thread's runtime.
struct Task {
    link: Arc<Link>,
    run: Box<dyn FnOnce(&tokio::runtime::Runtime) + Send>,
    alive: Arc<AtomicUsize>,
}

impl Task {
    /// It will not run: its owner hears why.
    fn refuse(self, why: String) {
        self.link.finish(Outcome::Failed(why));
        self.alive.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
struct Queue {
    tasks: VecDeque<Task>,
    threads: usize,
    idle: usize,
}

struct PoolInner {
    queue: Mutex<Queue>,
    ready: Condvar,
    size: usize,
    idle: Duration,
}

/// The threads workers run on: started as they are wanted, at most
/// `size`, each stopping once it has been idle for `idle`. One per
/// runner, shared by its extensions. Cheap to clone.
#[derive(Clone)]
pub struct Pool(Arc<PoolInner>);

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("size", &self.0.size)
            .field("idle", &self.0.idle)
            .field("threads", &self.threads())
            .finish()
    }
}

impl Default for Pool {
    /// One thread fewer than the machine has (one at least, four at
    /// most), each stopping after 30 s idle.
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        Self::new(cores.saturating_sub(1).clamp(1, 4), Duration::from_secs(30))
    }
}

impl Pool {
    #[must_use]
    pub fn new(size: usize, idle: Duration) -> Self {
        Self(Arc::new(PoolInner {
            queue: Mutex::new(Queue::default()),
            ready: Condvar::new(),
            size: size.max(1),
            idle,
        }))
    }

    /// Its threads now (none until a worker is spawned, none again once
    /// they are idle).
    #[must_use]
    pub fn threads(&self) -> usize {
        lock(&self.0.queue).threads
    }

    fn submit(&self, task: Task) {
        let mut q = lock(&self.0.queue);
        q.tasks.push_back(task);
        if q.idle > 0 || q.threads >= self.0.size {
            drop(q);
            self.0.ready.notify_one();
            return;
        }
        q.threads = q.threads.saturating_add(1);
        let n = q.threads;
        drop(q);
        let pool = Arc::clone(&self.0);
        let spawned = std::thread::Builder::new()
            .name(format!("ext-wasi-worker {n}"))
            .spawn(move || serve(&pool));
        if let Err(e) = spawned {
            let mut q = lock(&self.0.queue);
            q.threads = q.threads.saturating_sub(1);
            if q.threads == 0 {
                let tasks: Vec<Task> = q.tasks.drain(..).collect();
                drop(q);
                for t in tasks {
                    t.refuse(format!("no thread to run it on: {e}"));
                }
            }
        }
    }
}

/// One thread of the pool: tasks until it has been idle long enough.
// The queue's lock is held to wait on it, and let go to run each task.
#[allow(clippy::significant_drop_tightening)]
fn serve(pool: &PoolInner) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let mut q = lock(&pool.queue);
    loop {
        if let Some(task) = q.tasks.pop_front() {
            drop(q);
            match &rt {
                Ok(rt) => (task.run)(rt),
                Err(e) => task.refuse(format!("its runtime did not start: {e}")),
            }
            q = lock(&pool.queue);
            continue;
        }
        q.idle = q.idle.saturating_add(1);
        let (back, waited) = pool
            .ready
            .wait_timeout(q, pool.idle)
            .unwrap_or_else(PoisonError::into_inner);
        q = back;
        q.idle = q.idle.saturating_sub(1);
        if waited.timed_out() && q.tasks.is_empty() {
            q.threads = q.threads.saturating_sub(1);
            return;
        }
    }
}

impl wit_worker::Host for State {
    async fn spawn(&mut self, name: String, args: String) -> Result<u64, String> {
        match &self.side {
            super::Side::Owner(Some(crew)) => crew.spawn(&name, args),
            super::Side::Owner(None) => Err("it has no workers".to_owned()),
            super::Side::Worker(_) => Err("a worker spawns no workers".to_owned()),
        }
    }

    async fn post(&mut self, to: u64, message: String) -> Result<(), String> {
        match &self.side {
            super::Side::Owner(Some(crew)) => crew.post(to, message),
            super::Side::Owner(None) => Err(format!("it has no worker {to}")),
            super::Side::Worker(link) => {
                let started = Instant::now();
                let posted = link.post_to_owner(message);
                self.clock.host = self.clock.host.saturating_add(started.elapsed());
                posted
            }
        }
    }

    async fn cancel(&mut self, worker: u64) {
        if let super::Side::Owner(Some(crew)) = &self.side {
            crew.cancel(worker);
        }
    }

    async fn progress(&mut self, done: f64, total: f64, note: String) {
        if let super::Side::Worker(link) = &self.side {
            link.progress(wit_worker::ProgressReport { done, total, note });
        }
    }

    async fn cancelled(&mut self) -> bool {
        matches!(&self.side, super::Side::Worker(link) if link.cancelled())
    }

    async fn receive(&mut self, wait_ms: u32) -> Option<String> {
        let super::Side::Worker(link) = &self.side else {
            return None;
        };
        let started = Instant::now();
        let got = link.receive(Duration::from_millis(u64::from(wait_ms)));
        self.clock.host = self.clock.host.saturating_add(started.elapsed());
        got
    }
}

/// The owner's news, into its instance: `on-message` for each message,
/// `on-progress` for the latest progress, `on-done` for an end. `Err`
/// when a call trapped.
pub(super) async fn deliver(
    store: &mut wasmtime::Store<State>,
    world: &World,
    news: Vec<(u64, News)>,
) -> Result<(), String> {
    let work = world.architect_extension_work();
    for (id, n) in news {
        for m in &n.messages {
            begin(store);
            let heard = work.call_on_message(&mut *store, id, m).await;
            end(store, heard)?;
        }
        if let Some(p) = &n.progress {
            begin(store);
            let heard = work.call_on_progress(&mut *store, id, p).await;
            end(store, heard)?;
        }
        if let Some(o) = &n.outcome {
            begin(store);
            let heard = work.call_on_done(&mut *store, id, &o.to_wit()).await;
            end(store, heard)?;
        }
    }
    Ok(())
}

/// Puts `crew` on its log: a line per worker cancelled, for `EXT_LOGS`.
pub(super) fn cancelled_line(n: usize, why: &str) -> Option<String> {
    (n > 0).then(|| format!("{n} worker(s) cancelled: {why}"))
}
