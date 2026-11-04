//! Workers: a script's heavy work in a Luau state of its own, on another
//! thread, spoken to by messages only (feature `luau`).
//!
//! `docs/content/architecture/extensions.md`, "Responsive by design:
//! views and workers". A view's script draws within a frame's budget; what takes longer — a
//! search, a render, a thousand primes — goes to a worker. A worker is a
//! script the app names ([`Workers`]: an extension's `[[workers]]`), run
//! on the pool ([`crate::vm`]) in a fresh state that shares no memory
//! with its owner: no actions, no `ImGui`, no theme, only JSON messages
//! each way (Zellij's plugin workers, Web Workers). Its owner is the run
//! or live script that spawned it; when that goes (a view closed, an
//! extension unloaded: [`WorkerGroup::cancel_all`]), the worker is
//! cancelled at its next step. Each worker has its own limits
//! ([`Limits::worker`]: steps, memory and CPU time).
//!
//! The owner's `worker`:
//!
//! | `worker.` | gives |
//! |---|---|
//! | `spawn(name, args?)` | a handle `w`; the worker starts at once |
//! | `post(w, msg)`, `w:post(msg)` | a message to it (JSON) |
//! | `cancel(w)`, `w:cancel()` | stops it at its next step |
//! | `status(w)`, `w:status()` | `running`, `done`, `failed` or `cancelled` |
//! | `progress(w)`, `w:progress()` | `{done, total, fraction, note}` it last reported, or `nil` |
//! | `result(w)`, `w:result()` | what it returned (`nil` until done), and why it failed |
//! | `messages(w)`, `w:messages()` | what it posted since the last call |
//! | `log(w)`, `w:log()` | what it printed (its last hundred lines) |
//! | `on_message(w, fn)`, `on_progress(w, fn)`, `on_done(w, fn)`, `w:on_message(fn)`… | a live script's callbacks, at the start of its next frame |
//! | `join(w, ms?)`, `next(w, ms?)` | a one-off run's: waits for it to end, or for its next news (never in a view) |
//!
//! Its news wakes a live owner: its host runs a frame
//! ([`crate::luau::Live::woken`]), so an `ImGui` window that reads
//! `worker.progress(w)` each frame moves as the work does, with no
//! callbacks at all.
//!
//! The worker's own `worker`:
//!
//! | `worker.` | gives |
//! |---|---|
//! | `post(msg)` | a message to its owner |
//! | `progress(done, total?, note?)` | how far it is (the latest wins) |
//! | `receive(ms?)`, `poll()` | the next message from its owner: waiting (at most `ms`), or not; `nil` for none |
//! | `cancelled()` | whether it was told to stop (it is stopped at its next step anyway) |
//! | `on_message(fn)` | after its script returns, it serves its owner's messages until cancelled |
//! | `id`, `name` | its handle's number, its name |
//!
//! What its script returns is its result; `args` is what `spawn` gave.
//!
//! An owner that is not a [`Runtime`]'s script — a Luau page
//! (`architect-pages`), on a thread of its own — installs `worker` with
//! [`Workers::install_owner`] and runs its callbacks with [`deliver`] when
//! its wake says news came.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use mlua::{Lua, LuaSerdeExt as _, MultiValue, UserData, UserDataRef, Value as LuaValue};
use serde_json::Value;

use crate::ScriptError;
use crate::luau::{Limits, Module, Owner, Runtime, interrupt, load, raise, script_error, to_lua};
use crate::vm::{self, Cancel, Guard, Wake};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The worker scripts an app names, by name, and the limits and group
/// they run under. Given to a runtime with [`Runtime::with_workers`].
#[derive(Clone, Debug, Default)]
pub struct Workers {
    sources: BTreeMap<String, Arc<str>>,
    limits: Option<Limits>,
    group: WorkerGroup,
}

impl Workers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Worker `name` runs `source`.
    #[must_use]
    pub fn with_source(mut self, name: impl Into<String>, source: impl Into<Arc<str>>) -> Self {
        self.sources.insert(name.into(), source.into());
        self
    }

    /// Each worker runs within `limits` ([`Limits::worker`] when none).
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Every worker spawned goes in `group` too: cancelled with it (an
    /// extension's, on unload).
    #[must_use]
    pub fn with_group(mut self, group: WorkerGroup) -> Self {
        self.group = group;
        self
    }

    /// The group its workers go in.
    #[must_use]
    pub const fn group(&self) -> &WorkerGroup {
        &self.group
    }

    /// The names of the workers it knows.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }

    /// Whether each source is Luau, without running it.
    ///
    /// # Errors
    ///
    /// The first that is not, with its name.
    pub fn check(&self) -> Result<(), (String, ScriptError)> {
        for (name, source) in &self.sources {
            Runtime::check(source).map_err(|e| (name.clone(), e))?;
        }
        Ok(())
    }
}

impl Workers {
    /// `worker` for an owner outside a [`Runtime`] (a Luau page): its
    /// value, for the owner to make its global. Its workers' news wakes
    /// `wake`, and they stop when any of `cancel` is set or `lua` goes;
    /// the owner runs their callbacks with [`deliver`].
    ///
    /// # Errors
    ///
    /// When Luau does (out of memory).
    pub fn install_owner(
        self,
        lua: &Lua,
        wake: Option<Wake>,
        cancel: Vec<Cancel>,
    ) -> mlua::Result<LuaValue> {
        lua.set_app_data(Owner { wake, cancel });
        WorkerModule(Arc::new(self)).install(lua)
    }
}

/// Runs an owner's worker callbacks (`on_message`, `on_progress`,
/// `on_done`) with what its workers sent since the last call: what a live
/// script's frame does first, for an owner of [`Workers::install_owner`].
///
/// # Errors
///
/// A callback threw.
pub fn deliver(lua: &Lua) -> mlua::Result<()> {
    let links: Vec<Arc<Link>> = match lua.app_data_ref::<Owned>() {
        Some(owned) => lock(&owned.0).clone(),
        None => return Ok(()),
    };
    let all: mlua::Table = lua.named_registry_value(CALLBACKS)?;
    for link in links {
        let Some(cbs) = all.raw_get::<Option<mlua::Table>>(link.id)? else {
            continue;
        };
        let on_message: Option<mlua::Function> = cbs.get("message")?;
        let on_progress: Option<mlua::Function> = cbs.get("progress")?;
        let on_done: Option<mlua::Function> = cbs.get("done")?;
        if let Some(f) = &on_message {
            let drained: Vec<Value> = lock(&link.out).messages.drain(..).collect();
            for m in drained {
                f.call::<()>(to_lua(lua, &m)?)?;
            }
        }
        if let Some(f) = &on_progress {
            let fresh = {
                let mut out = lock(&link.out);
                std::mem::take(&mut out.progress_new)
                    .then(|| out.progress.clone())
                    .flatten()
            };
            if let Some(p) = fresh {
                f.call::<()>(progress_table(lua, &p)?)?;
            }
        }
        if let Some(f) = &on_done {
            let ended = {
                let mut out = lock(&link.out);
                let ended = out.status != WorkerStatus::Running && !out.done_told;
                if ended {
                    out.done_told = true;
                }
                ended
            };
            if ended {
                let (value, error) = result_of(lua, &link)?;
                f.call::<()>((value, error))?;
            }
        }
    }
    Ok(())
}

impl Runtime {
    /// Its scripts may spawn `workers` (`worker.spawn(name, args)`).
    #[must_use]
    pub fn with_workers(self, workers: Workers) -> Self {
        self.with_module(Arc::new(WorkerModule(Arc::new(workers))))
    }
}

/// Workers to stop together: an extension's, cancelled when it unloads.
/// Cheap to clone; clones share it.
#[derive(Clone, Debug, Default)]
pub struct WorkerGroup(Arc<Mutex<Vec<Weak<Link>>>>);

impl WorkerGroup {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&self, link: &Arc<Link>) {
        let mut all = lock(&self.0);
        all.retain(|w| w.strong_count() > 0);
        all.push(Arc::downgrade(link));
    }

    fn live(&self) -> Vec<Arc<Link>> {
        lock(&self.0).iter().filter_map(Weak::upgrade).collect()
    }

    /// Cancels every worker of the group still running: how many.
    #[allow(clippy::must_use_candidate)] // Most callers only cancel.
    pub fn cancel_all(&self) -> usize {
        let running: Vec<Arc<Link>> = self
            .live()
            .into_iter()
            .filter(|l| l.status() == WorkerStatus::Running)
            .collect();
        for link in &running {
            link.own.set();
        }
        running.len()
    }

    /// How many of its workers are running now.
    #[must_use]
    pub fn running(&self) -> usize {
        self.live()
            .iter()
            .filter(|l| l.status() == WorkerStatus::Running)
            .count()
    }
}

/// Where a worker is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkerStatus {
    #[default]
    Running,
    /// Its script returned (and, serving, was then cancelled).
    Done,
    /// Its script raised, or ran past a limit.
    Failed,
    /// It was cancelled before it finished: by its owner, its owner
    /// going, or its group.
    Cancelled,
}

impl WorkerStatus {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// How far a worker said it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub done: f64,
    pub total: Option<f64>,
    pub note: Option<String>,
}

impl Progress {
    /// `done / total`, in 0..=1, when there is a total.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|t| *t > 0.0)
            .map(|t| (self.done / t).clamp(0.0, 1.0))
    }
}

/// What a worker sent its owner, past what the owner read.
#[derive(Debug, Default)]
struct Out {
    messages: VecDeque<Value>,
    progress: Option<Progress>,
    /// Progress came since the owner last heard.
    progress_new: bool,
    status: WorkerStatus,
    result: Option<Value>,
    error: Option<String>,
    /// The owner was told it ended.
    done_told: bool,
    log: VecDeque<String>,
}

/// What one worker and its owner share: messages each way, its news,
/// its cancel.
#[derive(Debug)]
pub(crate) struct Link {
    id: u64,
    name: String,
    /// Its own cancel: its owner's `cancel`, the group's.
    own: Cancel,
    /// Its own and its owner's: any set stops it.
    cancels: Vec<Cancel>,
    inbox: Mutex<VecDeque<Value>>,
    arrived: Condvar,
    out: Mutex<Out>,
    news: Condvar,
    wake: Option<Wake>,
}

/// Lines of a worker's log kept.
const LOG_KEPT: usize = 100;

impl Link {
    fn cancelled(&self) -> bool {
        self.cancels.iter().any(Cancel::is_set)
    }

    fn status(&self) -> WorkerStatus {
        lock(&self.out).status
    }

    /// Changes what it sent, tells whoever waits, and wakes its owner.
    fn tell(&self, f: impl FnOnce(&mut Out)) {
        f(&mut lock(&self.out));
        self.news.notify_all();
        if let Some(wake) = &self.wake {
            wake.wake();
        }
    }

    /// The next message from its owner: waiting at most `within` (none:
    /// until one comes), `None` when none came or it was cancelled.
    fn receive(&self, within: Option<Duration>) -> Option<Value> {
        let until = within.and_then(|d| Instant::now().checked_add(d));
        let mut inbox = lock(&self.inbox);
        loop {
            if let Some(msg) = inbox.pop_front() {
                return Some(msg);
            }
            if self.cancelled() {
                return None;
            }
            let wait = match until {
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return None;
                    }
                    left.min(POLL)
                }
                None => POLL,
            };
            inbox = self
                .arrived
                .wait_timeout(inbox, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// How often a waiting worker or owner looks at its cancels.
const POLL: Duration = Duration::from_millis(20);

/// The workers a state spawned (its app data): cancelled when it goes.
#[derive(Debug, Default)]
struct Owned(Mutex<Vec<Arc<Link>>>);

impl Drop for Owned {
    fn drop(&mut self) {
        for link in lock(&self.0).iter() {
            link.own.set();
        }
    }
}

/// A worker, as its owner's script holds it.
struct Handle(Arc<Link>);

/// The registry key of a live owner's callbacks: `{[id] = {message,
/// progress, done}}`.
const CALLBACKS: &str = "architect.worker.callbacks";
/// The registry key of a serving worker's handler.
const ON_MESSAGE: &str = "architect.worker.on_message";

static IDS: AtomicU64 = AtomicU64::new(1);

/// `worker`, for an owner.
struct WorkerModule(Arc<Workers>);

fn progress_table(lua: &Lua, p: &Progress) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    t.set("done", p.done)?;
    t.set("total", p.total)?;
    t.set("fraction", p.fraction())?;
    t.set("note", p.note.clone())?;
    Ok(t)
}

fn json_in(lua: &Lua, value: LuaValue) -> mlua::Result<Value> {
    lua.from_value::<Value>(value)
        .map_err(|e| raise(lua, format!("a message is JSON: {e}")))
}

fn millis(ms: Option<f64>) -> Option<Duration> {
    ms.filter(|m| m.is_finite() && *m >= 0.0)
        .map(|m| Duration::from_secs_f64(m / 1000.0))
}

/// Posts `msg` to the worker.
fn post(link: &Link, msg: Value) {
    lock(&link.inbox).push_back(msg);
    link.arrived.notify_all();
}

fn result_of(lua: &Lua, link: &Link) -> mlua::Result<(LuaValue, Option<String>)> {
    let out = lock(&link.out);
    let value = match &out.result {
        Some(v) => to_lua(lua, v)?,
        None => LuaValue::Nil,
    };
    Ok((value, out.error.clone()))
}

fn messages_of(lua: &Lua, link: &Link) -> mlua::Result<mlua::Table> {
    let drained: Vec<Value> = lock(&link.out).messages.drain(..).collect();
    let t = lua.create_table()?;
    for m in drained {
        t.push(to_lua(lua, &m)?)?;
    }
    Ok(t)
}

/// What the owner hears next: a message, new progress, or how it ended.
enum News {
    Message(Value),
    Progress(Progress),
    Ended(WorkerStatus, Option<Value>, Option<String>),
}

fn take_news(out: &mut Out) -> Option<News> {
    if let Some(m) = out.messages.pop_front() {
        return Some(News::Message(m));
    }
    if out.progress_new {
        out.progress_new = false;
        if let Some(p) = &out.progress {
            return Some(News::Progress(p.clone()));
        }
    }
    if out.status != WorkerStatus::Running && !out.done_told {
        out.done_told = true;
        return Some(News::Ended(
            out.status,
            out.result.clone(),
            out.error.clone(),
        ));
    }
    None
}

/// Waits, on the owner's thread, until `ready` says there is something,
/// at most `within`; `None` when the time ran out or the owner went.
fn wait_for<T>(
    lua: &Lua,
    link: &Link,
    within: Option<Duration>,
    mut ready: impl FnMut(&mut Out) -> Option<T>,
) -> Option<T> {
    let owner = lua.app_data_ref::<Owner>().map(|o| o.clone());
    let until = within.and_then(|d| Instant::now().checked_add(d));
    let mut out = lock(&link.out);
    loop {
        if let Some(found) = ready(&mut out) {
            return Some(found);
        }
        if owner.as_ref().is_some_and(Owner::gone) {
            return None;
        }
        let wait = match until {
            Some(until) => {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return None;
                }
                left.min(POLL)
            }
            None => POLL,
        };
        out = link
            .news
            .wait_timeout(out, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

fn join(lua: &Lua, link: &Link, ms: Option<f64>) -> mlua::Result<(LuaValue, Option<String>)> {
    let ended = wait_for(lua, link, millis(ms), |out| {
        (out.status != WorkerStatus::Running).then_some(())
    });
    if ended.is_none() {
        return Ok((LuaValue::Nil, Some("it is still running".to_owned())));
    }
    lock(&link.out).done_told = true;
    result_of(lua, link)
}

fn next(lua: &Lua, link: &Link, ms: Option<f64>) -> mlua::Result<(LuaValue, LuaValue)> {
    let Some(news) = wait_for(lua, link, millis(ms), take_news) else {
        return Ok((LuaValue::Nil, LuaValue::Nil));
    };
    let kind = |k: &str| lua.create_string(k).map(LuaValue::String);
    Ok(match news {
        News::Message(m) => (kind("message")?, to_lua(lua, &m)?),
        News::Progress(p) => (kind("progress")?, LuaValue::Table(progress_table(lua, &p)?)),
        News::Ended(status, result, error) => {
            let payload = match (status, result, error) {
                (WorkerStatus::Done, Some(v), _) => to_lua(lua, &v)?,
                (_, _, Some(e)) => LuaValue::String(lua.create_string(&e)?),
                _ => LuaValue::Nil,
            };
            (kind(status.name())?, payload)
        }
    })
}

impl UserData for Handle {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, h| Ok(h.0.id));
        fields.add_field_method_get("name", |_, h| Ok(h.0.name.clone()));
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("post", |lua, h, msg: LuaValue| {
            post(&h.0, json_in(lua, msg)?);
            Ok(())
        });
        methods.add_method("cancel", |_, h, ()| {
            h.0.own.set();
            Ok(())
        });
        methods.add_method("status", |_, h, ()| Ok(h.0.status().name()));
        methods.add_method("progress", |lua, h, ()| {
            let p = lock(&h.0.out).progress.clone();
            p.map(|p| progress_table(lua, &p)).transpose()
        });
        methods.add_method("result", |lua, h, ()| result_of(lua, &h.0));
        methods.add_method("messages", |lua, h, ()| messages_of(lua, &h.0));
        methods.add_method("log", |_, h, ()| {
            Ok(lock(&h.0.out).log.iter().cloned().collect::<Vec<_>>())
        });
        methods.add_method("join", |lua, h, ms: Option<f64>| join(lua, &h.0, ms));
        methods.add_method("next", |lua, h, ms: Option<f64>| next(lua, &h.0, ms));
        // `w:on_message(fn)`: `worker.on_message(w, fn)`, as a page's
        // handle says it (`w.onMessage(fn)` in TypeScript).
        for which in ["message", "progress", "done"] {
            methods.add_method(
                format!("on_{which}"),
                move |lua, h, f: mlua::Function| set_callback(lua, &h.0, which, f),
            );
        }
    }
}

/// Sets `name` on `table` to a function of a handle and `A`.
fn on_handle<A, R>(
    lua: &Lua,
    table: &mlua::Table,
    name: &str,
    f: impl Fn(&Lua, &Arc<Link>, A) -> mlua::Result<R> + Send + Sync + 'static,
) -> mlua::Result<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    table.set(
        name,
        lua.create_function(move |lua, (h, args): (UserDataRef<Handle>, A)| f(lua, &h.0, args))?,
    )
}

fn set_callback(lua: &Lua, link: &Link, which: &str, f: mlua::Function) -> mlua::Result<()> {
    let all: mlua::Table = lua.named_registry_value(CALLBACKS)?;
    let mine: mlua::Table = if let Some(t) = all.raw_get::<Option<mlua::Table>>(link.id)? {
        t
    } else {
        let t = lua.create_table()?;
        all.raw_set(link.id, t.clone())?;
        t
    };
    mine.set(which, f)
}

impl Module for WorkerModule {
    fn global(&self) -> &'static str {
        "worker"
    }

    fn path(&self) -> &'static str {
        "worker"
    }

    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        lua.set_app_data(Owned::default());
        lua.set_named_registry_value(CALLBACKS, lua.create_table()?)?;
        let t = lua.create_table()?;
        let workers = Arc::clone(&self.0);
        t.set(
            "spawn",
            lua.create_function(move |lua, (name, args): (String, Option<LuaValue>)| {
                let Some(source) = workers.sources.get(&name).cloned() else {
                    let known = workers.names().join(", ");
                    return Err(raise(
                        lua,
                        format!("there is no worker {name:?} (there are: {known})"),
                    ));
                };
                let args = match args {
                    Some(a) => json_in(lua, a)?,
                    None => Value::Null,
                };
                let owner = lua
                    .app_data_ref::<Owner>()
                    .map(|o| o.clone())
                    .unwrap_or_default();
                let own = Cancel::default();
                let mut cancels = vec![own.clone()];
                cancels.extend(owner.cancel.iter().cloned());
                let link = Arc::new(Link {
                    id: IDS.fetch_add(1, Ordering::Relaxed),
                    name,
                    own,
                    cancels,
                    inbox: Mutex::new(VecDeque::new()),
                    arrived: Condvar::new(),
                    out: Mutex::new(Out::default()),
                    news: Condvar::new(),
                    wake: owner.wake.clone(),
                });
                if let Some(owned) = lua.app_data_ref::<Owned>() {
                    lock(&owned.0).push(Arc::clone(&link));
                }
                workers.group.add(&link);
                let limits = workers.limits.unwrap_or_else(Limits::worker);
                let theirs = Arc::clone(&link);
                vm::spawn(move || work(&theirs, &source, &args, limits));
                Ok(Handle(link))
            })?,
        )?;
        on_handle(lua, &t, "post", |lua, link, msg: LuaValue| {
            post(link, json_in(lua, msg)?);
            Ok(())
        })?;
        on_handle(lua, &t, "cancel", |_, link, ()| {
            link.own.set();
            Ok(())
        })?;
        on_handle(lua, &t, "status", |_, link, ()| Ok(link.status().name()))?;
        on_handle(lua, &t, "progress", |lua, link, ()| {
            let p = lock(&link.out).progress.clone();
            p.map(|p| progress_table(lua, &p)).transpose()
        })?;
        on_handle(lua, &t, "result", |lua, link, ()| result_of(lua, link))?;
        on_handle(lua, &t, "messages", |lua, link, ()| messages_of(lua, link))?;
        on_handle(lua, &t, "log", |_, link, ()| {
            Ok(lock(&link.out).log.iter().cloned().collect::<Vec<_>>())
        })?;
        on_handle(lua, &t, "join", |lua, link, ms: Option<f64>| {
            join(lua, link, ms)
        })?;
        on_handle(lua, &t, "next", |lua, link, ms: Option<f64>| {
            next(lua, link, ms)
        })?;
        for which in ["message", "progress", "done"] {
            on_handle(
                lua,
                &t,
                &format!("on_{which}"),
                move |lua, link, f: mlua::Function| set_callback(lua, link, which, f),
            )?;
        }
        Ok(LuaValue::Table(t))
    }

    /// A live owner's callbacks, with what its workers sent since.
    fn frame(&self, lua: &Lua) -> mlua::Result<()> {
        deliver(lua)
    }
}

/// A worker's run, on its pool thread: its script, then its owner's
/// messages while it serves them; how it ended, to its owner.
fn work(link: &Arc<Link>, source: &str, args: &Value, limits: Limits) {
    let guard = Arc::new(Guard::new(limits.steps, link.cancels.clone()));
    guard.deadline(limits.time);
    let ran = run_worker(link, source, args, limits, &guard);
    link.tell(|out| match ran {
        Ok(value) => {
            out.status = WorkerStatus::Done;
            out.result = Some(value);
        }
        Err(_) if link.cancelled() => out.status = WorkerStatus::Cancelled,
        Err(e) => {
            out.status = WorkerStatus::Failed;
            out.error = Some(e.to_string());
        }
    });
}

fn run_worker(
    link: &Arc<Link>,
    source: &str,
    args: &Value,
    limits: Limits,
    guard: &Arc<Guard>,
) -> Result<Value, ScriptError> {
    let lua = worker_state(link, args, limits).map_err(|e| script_error(&e))?;
    interrupt(&lua, Arc::clone(guard));
    let main = load(&lua, source)?;
    let out: MultiValue = main.call(()).map_err(|e| script_error(&e))?;
    let first = out.into_iter().next().unwrap_or(LuaValue::Nil);
    let value: Value = lua.from_value(first).map_err(|e| script_error(&e))?;
    let serving: Option<mlua::Function> = lua.named_registry_value(ON_MESSAGE).ok();
    if let Some(handler) = serving {
        while let Some(msg) = link.receive(None) {
            guard.reset();
            let msg = to_lua(&lua, &msg).map_err(|e| script_error(&e))?;
            handler.call::<()>(msg).map_err(|e| script_error(&e))?;
        }
        if link.cancelled() {
            return Err(ScriptError::new(
                crate::ErrorKind::Limit,
                None,
                "it was stopped",
            ));
        }
    }
    Ok(value)
}

/// A worker's state: `args`, `print`, and its own `worker`; sandboxed.
fn worker_state(link: &Arc<Link>, args: &Value, limits: Limits) -> mlua::Result<Lua> {
    let lua = Lua::new();
    lua.set_memory_limit(limits.memory)?;
    let globals = lua.globals();
    globals.set("args", to_lua(&lua, args)?)?;
    let me = Arc::clone(link);
    let print = lua.create_function(move |_, values: MultiValue| {
        let words: Vec<String> = values
            .iter()
            .map(|v| v.to_string().unwrap_or_else(|_| v.type_name().to_owned()))
            .collect();
        let mut out = lock(&me.out);
        if out.log.len() == LOG_KEPT {
            out.log.pop_front();
        }
        out.log.push_back(words.join("\t"));
        drop(out);
        Ok(())
    })?;
    globals.set("print", print.clone())?;
    globals.set("log", print)?;
    let t = lua.create_table()?;
    t.set("id", link.id)?;
    t.set("name", link.name.clone())?;
    let me = Arc::clone(link);
    t.set(
        "post",
        lua.create_function(move |lua, msg: LuaValue| {
            let msg = json_in(lua, msg)?;
            me.tell(|out| out.messages.push_back(msg));
            Ok(())
        })?,
    )?;
    let me = Arc::clone(link);
    t.set(
        "progress",
        lua.create_function(
            move |_, (done, total, note): (f64, Option<f64>, Option<String>)| {
                me.tell(|out| {
                    out.progress = Some(Progress { done, total, note });
                    out.progress_new = true;
                });
                Ok(())
            },
        )?,
    )?;
    let me = Arc::clone(link);
    t.set(
        "receive",
        lua.create_function(move |lua, ms: Option<f64>| {
            let within = millis(ms);
            me.receive(within)
                .map_or(Ok(LuaValue::Nil), |m| to_lua(lua, &m))
        })?,
    )?;
    let me = Arc::clone(link);
    t.set(
        "poll",
        lua.create_function(move |lua, ()| {
            let next = lock(&me.inbox).pop_front();
            next.map_or(Ok(LuaValue::Nil), |m| to_lua(lua, &m))
        })?,
    )?;
    let me = Arc::clone(link);
    t.set(
        "cancelled",
        lua.create_function(move |_, ()| Ok(me.cancelled()))?,
    )?;
    t.set(
        "on_message",
        lua.create_function(|lua, f: mlua::Function| lua.set_named_registry_value(ON_MESSAGE, f))?,
    )?;
    globals.set("worker", t)?;
    lua.sandbox(true)?;
    Ok(lua)
}
