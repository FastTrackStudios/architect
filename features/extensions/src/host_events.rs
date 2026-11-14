//! The host's side of timers, events and background extensions (#308;
//! `crate::events`, `docs/content/architecture/extensions.md`, "Timers,
//! events and background extensions").
//!
//! - **The clock**: the host reads the time on one [`Clock`]
//!   ([`Host::with_clock`]; the system's unless a test gives a
//!   [`crate::events::FakeClock`]). [`Host::tick`] runs what is due on it:
//!   scheduled hooks (`every`, `at`) and idle background extensions'
//!   stops. The app calls it from a thread ([`Host::run_clock`]); a test
//!   moves its clock and calls it.
//! - **Events**: document events ([`Host::raise_doc`], or a
//!   [`DocEvent`] raised on the Action List), focus ([`Host::focus`]) and
//!   the events the app defines ([`Host::define_event`],
//!   [`Host::raise`]), run through the same hooks as every app event.
//! - **Hooks made at run time** ([`Host::subscribe`]): the host service's
//!   `subscribe`, checked as a manifest's `[[hooks]]` are.
//! - **Background extensions** (`[background]`): [`Lazy`] stands in for
//!   the extension's instance: the first action or hook starts it, and
//!   [`Host::tick`] stops it once it has been idle its timeout. Each start
//!   and stop is a line in its log (`EXT_LOGS`); what each is doing now is
//!   [`Host::background`] and `APP_STATUS`'s `background`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionCaller, ActionError, ActionFuture, CallerAsyncHandler};
use serde_json::{Value, json};

use crate::contrib::{Contribution, HookDecl};
use crate::events::{
    APP_BLURRED, APP_FOCUSED, AppEvent, BackgroundState, BackgroundStatus, Clock, DocEvent,
    SCHEDULE, Schedule, SharedClock, SystemClock, TypedEvent,
};
use crate::host::{Host, Instance};
use crate::logs::{Level, Logs};
use crate::manifest::ActionDecl;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the host keeps for timers, events and background extensions.
pub(crate) struct EventState {
    clock: Mutex<SharedClock>,
    /// The events the app defined, by name.
    defined: Mutex<BTreeMap<String, AppEvent>>,
    /// Hooks made at run time, by owner and key.
    subscriptions: Mutex<BTreeMap<String, BTreeMap<String, HookDecl>>>,
    /// Each scheduled hook's next run (ms on the clock), by owner and key.
    due: Mutex<BTreeMap<(String, String), u64>>,
    /// The background extensions loaded, by id.
    background: Mutex<BTreeMap<String, Arc<Core>>>,
    /// `APP_STATUS` has a `background` section.
    shown: AtomicBool,
    /// [`Host::run_clock`] runs.
    ticking: AtomicBool,
    /// What hears events as they are raised, beside hooks
    /// ([`Host::hear`]): a web view's bridge, by key, with its owner.
    heard: Mutex<BTreeMap<u64, (String, Heard)>>,
    next_heard: std::sync::atomic::AtomicU64,
}

/// What hears an extension's events as they are raised ([`Host::hear`]):
/// the event's name and its detail.
pub type Heard = Arc<dyn Fn(&str, &Value) + Send + Sync>;

impl Default for EventState {
    fn default() -> Self {
        Self {
            clock: Mutex::new(Arc::new(SystemClock::default())),
            defined: Mutex::default(),
            subscriptions: Mutex::default(),
            due: Mutex::default(),
            background: Mutex::default(),
            shown: AtomicBool::new(false),
            ticking: AtomicBool::new(false),
            heard: Mutex::default(),
            next_heard: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

/// What one [`Host::tick`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// The scheduled hooks it started, as `<owner> <when>` (`lab.x every 5m`).
    pub ran: Vec<String>,
    /// The background extensions it stopped, idle.
    pub stopped: Vec<String>,
}

/// Timers, events and background extensions.
impl Host {
    /// Reads the time on `clock` from now on (a test's
    /// [`crate::events::FakeClock`]).
    #[must_use]
    pub fn with_clock(self, clock: impl Clock + 'static) -> Self {
        self.set_clock(Arc::new(clock));
        self
    }

    /// Reads the time on `clock` from now on.
    pub fn set_clock(&self, clock: SharedClock) {
        *lock(&self.event_state().clock) = clock;
    }

    /// The clock it reads the time on.
    #[must_use]
    pub fn clock(&self) -> SharedClock {
        Arc::clone(&lock(&self.event_state().clock))
    }

    // ── App-defined events ──────────────────────────────────────────

    /// Defines app event `event`: an extension that asks for it
    /// (`capabilities.events`) and was granted it hears it with a hook on
    /// `app:<name>`. A second definition by the same name replaces the
    /// first.
    ///
    /// # Errors
    ///
    /// Its name is not `a-z 0-9 . _ -`.
    pub fn define_event(&self, event: AppEvent) -> Result<(), String> {
        if !crate::events::valid_name(&event.name) {
            return Err(format!(
                "{:?} is not an app event's name (a-z 0-9 . _ -)",
                event.name
            ));
        }
        lock(&self.event_state().defined).insert(event.name.clone(), event);
        Ok(())
    }

    /// Defines `E` ([`Self::define_event`]).
    ///
    /// # Errors
    ///
    /// Its name is not `a-z 0-9 . _ -`.
    pub fn define<E: TypedEvent>(&self) -> Result<(), String> {
        self.define_event(E::definition())
    }

    /// The events the app defined.
    #[must_use]
    pub fn app_events(&self) -> Vec<AppEvent> {
        lock(&self.event_state().defined)
            .values()
            .cloned()
            .collect()
    }

    /// Raises `event`: the hooks on `app:<its name>` of every extension
    /// that may hear it run, with it as their detail. How many started.
    ///
    /// # Errors
    ///
    /// It was not defined ([`Self::define`]), or does not serialise.
    pub fn raise<E: TypedEvent>(&self, event: &E) -> Result<usize, String> {
        let detail = serde_json::to_value(event).map_err(|e| format!("{}: {e}", E::NAME))?;
        self.raise_app(E::NAME, &detail)
    }

    /// Raises app-defined event `name` with `detail` ([`Self::raise`]).
    ///
    /// # Errors
    ///
    /// No event `name` was defined.
    pub fn raise_app(&self, name: &str, detail: &Value) -> Result<usize, String> {
        if !lock(&self.event_state().defined).contains_key(name) {
            return Err(format!(
                "no app event {name} is defined (Host::define_event)"
            ));
        }
        Ok(self.emit_value(&format!("{}{name}", crate::events::APP_PREFIX), detail))
    }

    /// Raises app-defined event `name` with a detail made for each
    /// extension that hears it (`detail_for(<its id>)`), so what one
    /// extension hears need not be what another does: the playhead's
    /// `playhead.changed` gives a source's state to its own extension
    /// only (`architect_playhead::heard_by`). How many hooks started.
    ///
    /// # Errors
    ///
    /// No event `name` was defined.
    pub fn raise_app_each(
        &self,
        name: &str,
        detail_for: &dyn Fn(&str) -> Value,
    ) -> Result<usize, String> {
        if !lock(&self.event_state().defined).contains_key(name) {
            return Err(format!(
                "no app event {name} is defined (Host::define_event)"
            ));
        }
        Ok(self.emit_each(&format!("{}{name}", crate::events::APP_PREFIX), detail_for))
    }

    /// Raises extension `id`'s own event `<id>.<name>` with `detail`
    /// (`HostRequest::Raise`), defined the first time it is raised. How
    /// many hooks started.
    ///
    /// # Errors
    ///
    /// `name` is not `a-z 0-9 . _ -`.
    pub fn raise_own(&self, id: &str, name: &str, detail: &Value) -> Result<usize, String> {
        let full = format!("{id}.{name}");
        if !crate::events::valid_name(&full) {
            return Err(format!("{name:?} is not an event's name (a-z 0-9 . _ -)"));
        }
        if !lock(&self.event_state().defined).contains_key(&full) {
            self.define_event(AppEvent {
                name: full.clone(),
                title: format!("{id}: {name}"),
                ..AppEvent::default()
            })?;
        }
        self.raise_app(&full, detail)
    }

    // ── Document and app events ─────────────────────────────────────

    /// Raises document event `event` (`doc.saved`, …): the hooks on it run
    /// with `{path}` (`{from, to}`) as their detail. How many started.
    // How many is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn raise_doc(&self, event: &DocEvent) -> usize {
        self.emit_value(event.name(), &event.detail())
    }

    /// Raises `app.focused` (or, not `focused`, `app.blurred`), with the
    /// window when the app has more than one. How many started.
    // How many is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn focus(&self, focused: bool, window: Option<&str>) -> usize {
        let on = if focused { APP_FOCUSED } else { APP_BLURRED };
        self.emit_value(on, &json!({ "window": window }))
    }

    // ── Listeners ───────────────────────────────────────────────────

    /// Tells `heard` every event extension `owner` may hear as it is
    /// raised, as a hook of its would be told (a document's events only
    /// while it may read documents, an app event only while it may hear
    /// it): a web view's bridge (`crate::webview`), which passes on what
    /// its view asked for. Its key, for [`Self::unhear`].
    #[must_use]
    pub fn hear(&self, owner: &str, heard: Heard) -> u64 {
        let state = self.event_state();
        let key = state.next_heard.fetch_add(1, Ordering::Relaxed);
        lock(&state.heard).insert(key, (owner.to_owned(), heard));
        key
    }

    /// Stops listener `key` ([`Self::hear`]): whether it was there.
    #[allow(clippy::must_use_candidate)]
    pub fn unhear(&self, key: u64) -> bool {
        lock(&self.event_state().heard).remove(&key).is_some()
    }

    /// Tells the listeners event `on`, each with the detail for its owner,
    /// those of `only` alone when it names one.
    pub(crate) fn tell_heard(&self, only: Option<&str>, on: &str, detail_for: &dyn Fn(&str) -> Value) {
        let listeners: Vec<(String, Heard)> = lock(&self.event_state().heard)
            .values()
            .filter(|(owner, _)| only.is_none_or(|o| o == owner))
            .cloned()
            .collect();
        for (owner, heard) in listeners {
            let Some(may) = self.capabilities(&owner) else {
                continue;
            };
            if crate::events::app_event(on).is_some_and(|name| !may.may_hear(name)) {
                continue;
            }
            if crate::events::is_doc_event(on) && !may.may_read(crate::capabilities::DOCUMENTS) {
                continue;
            }
            heard(on, &detail_for(&owner));
        }
    }

    // ── Hooks made at run time ──────────────────────────────────────

    /// Makes hook `hook` for loaded extension `owner`, checked as its
    /// manifest's `[[hooks]]` are (its own action, a script of a Luau
    /// extension, an event or a schedule there is): its key, which
    /// [`Self::unsubscribe`] takes. A hook it already has is kept.
    ///
    /// # Errors
    ///
    /// It is not loaded, or the hook is refused.
    pub fn subscribe(&self, owner: &str, hook: HookDecl) -> Result<String, String> {
        let loaded = self
            .get(owner)
            .ok_or_else(|| format!("{owner} is not loaded"))?;
        let manifest = &loaded.manifest;
        let views = manifest.view_ids();
        Contribution {
            hooks: vec![hook.clone()],
            ..Contribution::default()
        }
        .check(&manifest.rules(&views))
        .map_err(|r| r.to_string())?;
        let key = hook.key();
        lock(&self.event_state().subscriptions)
            .entry(owner.to_owned())
            .or_default()
            .insert(key.clone(), hook);
        self.schedule(false);
        Ok(key)
    }

    /// Takes `owner`'s hook `key` (made with [`Self::subscribe`]) off:
    /// whether it had it.
    // Whether it had it is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn unsubscribe(&self, owner: &str, key: &str) -> bool {
        lock(&self.event_state().subscriptions)
            .get_mut(owner)
            .is_some_and(|hooks| hooks.remove(key).is_some())
    }

    /// The hooks `owner` made at run time.
    #[must_use]
    pub fn subscriptions(&self, owner: &str) -> Vec<HookDecl> {
        lock(&self.event_state().subscriptions)
            .get(owner)
            .map(|h| h.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The hooks made at run time on `on` (none on a schedule), with their
    /// owners.
    pub(crate) fn subscribed(&self, on: &str) -> Vec<(String, HookDecl)> {
        lock(&self.event_state().subscriptions)
            .iter()
            .flat_map(|(owner, hooks)| {
                hooks
                    .values()
                    .filter(|h| h.on == on && !h.scheduled())
                    .map(|h| (owner.clone(), h.clone()))
            })
            .collect()
    }

    /// What extension `owner` can hear (the host service's `events`): the
    /// app's events, the app-defined ones it may hear, the hooks it made.
    #[must_use]
    pub fn events_for(&self, owner: &str) -> Value {
        let may = self.capabilities(owner).unwrap_or_default();
        let app: Vec<Value> = self
            .app_events()
            .into_iter()
            .filter(|e| may.may_hear(&e.name))
            .map(|e| json!({ "name": format!("app:{}", e.name), "title": e.title, "description": e.description, "detail": e.detail }))
            .collect();
        json!({
            "events": crate::contrib::EVENTS,
            "app": app,
            "subscribed": self.subscriptions(owner).iter().map(HookDecl::key).collect::<Vec<_>>(),
        })
    }

    // ── The clock ────────────────────────────────────────────────────

    /// Runs what is due on its clock: each scheduled hook whose time has
    /// come (then schedules its next), and stops each background
    /// extension idle past its timeout.
    // What it did is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn tick(&self) -> Tick {
        let ran = self.schedule(true);
        let now = self.clock().now_ms();
        let cores: Vec<Arc<Core>> = lock(&self.event_state().background)
            .values()
            .cloned()
            .collect();
        let mut stopped = Vec::new();
        for core in &cores {
            if core.reap(now) {
                stopped.push(core.id.clone());
            }
        }
        Tick { ran, stopped }
    }

    /// Calls [`Self::tick`] every `every` on a thread of its own, for as
    /// long as the host lives (once: a second call does nothing).
    pub fn run_clock(&self, every: Duration) {
        if self.event_state().ticking.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = self.downgrade();
        let _ = std::thread::Builder::new()
            .name("ext-clock".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(every);
                    let Some(host) = weak.upgrade() else { return };
                    let _ = host.tick();
                }
            });
    }

    /// Every scheduled hook's next time: a new one's is set from now; when
    /// `run`, one that is due runs and its next is set. What ran.
    pub(crate) fn schedule(&self, run: bool) -> Vec<String> {
        let clock = self.clock();
        let (now, offset) = (clock.now_ms(), clock.utc_offset_min());
        let mut hooks: Vec<(String, HookDecl)> = self
            .contributions()
            .scheduled()
            .into_iter()
            .map(|by| (by.owner, by.item))
            .collect();
        hooks.extend(
            lock(&self.event_state().subscriptions)
                .iter()
                .flat_map(|(owner, hooks)| {
                    hooks
                        .values()
                        .filter(|h| h.scheduled())
                        .map(|h| (owner.clone(), h.clone()))
                }),
        );
        let mut due_now = Vec::new();
        {
            let mut due = lock(&self.event_state().due);
            let mut seen = BTreeSet::new();
            for (owner, hook) in hooks {
                let Some(Ok(schedule)) = Schedule::of(hook.every.as_deref(), hook.at.as_deref())
                else {
                    continue;
                };
                let key = (owner.clone(), hook.key());
                seen.insert(key.clone());
                match due.get(&key).copied() {
                    None => {
                        if let Some(next) = schedule.next_after(now, offset) {
                            due.insert(key, next);
                        }
                    }
                    Some(at) if run && at <= now => {
                        match schedule.next_after(now, offset) {
                            Some(next) => due.insert(key, next),
                            None => due.remove(&key),
                        };
                        due_now.push((owner, hook, at));
                    }
                    Some(_) => {}
                }
            }
            due.retain(|k, _| seen.contains(k));
        }
        let mut ran = Vec::new();
        for (owner, hook, at) in due_now {
            let detail = match (&hook.every, &hook.at) {
                (Some(e), _) => json!({ "due_ms": at, "every": e }),
                (None, a) => json!({ "due_ms": at, "at": a }),
            };
            if self.fire(&owner, &hook, SCHEDULE, &detail) {
                ran.push(format!("{owner} {}", hook.when()));
            }
        }
        ran
    }

    /// When `id`'s scheduled hooks next run (ms on the clock), by when.
    #[must_use]
    pub fn next_runs(&self, id: &str) -> Vec<(String, u64)> {
        lock(&self.event_state().due)
            .iter()
            .filter(|((owner, _), _)| owner == id)
            .map(|((_, key), at)| (key.clone(), *at))
            .collect()
    }

    // ── Background extensions ───────────────────────────────────────

    /// Each background extension now.
    #[must_use]
    pub fn background(&self) -> Vec<BackgroundStatus> {
        lock(&self.event_state().background)
            .values()
            .map(|c| c.status())
            .collect()
    }

    /// `id`'s instance as a background extension: started by `start` on
    /// its first event, stopped idle `idle` later. Shown in `APP_STATUS`
    /// from the first.
    pub(crate) fn lazy(
        &self,
        id: &str,
        idle: Duration,
        state: Option<Vec<u8>>,
        start: Start,
    ) -> Box<dyn Instance> {
        let core = Arc::new(Core {
            id: id.to_owned(),
            idle_ms: u64::try_from(idle.as_millis()).unwrap_or(u64::MAX),
            clock: self.clock(),
            logs: self.logs().clone(),
            start,
            st: Mutex::new(St {
                saved: state,
                last_used: self.clock().now_ms(),
                ..St::default()
            }),
        });
        // A reload's old record goes; the host stops its instance.
        lock(&self.event_state().background).insert(id.to_owned(), Arc::clone(&core));
        self.event_state().shown.store(true, Ordering::Release);
        self.show_background();
        Box::new(Lazy { core })
    }

    /// `APP_STATUS`'s `background` section, on its list and the app's
    /// (`Host::set_app_list`), once it has a background extension.
    pub(crate) fn show_background(&self) {
        if !self.event_state().shown.load(Ordering::Acquire) {
            return;
        }
        let lists = [Some(self.list().clone()), self.app_list()];
        for list in lists.into_iter().flatten() {
            let weak = self.downgrade();
            list.status().section("background", move || {
                weak.upgrade().map_or_else(
                    || "[]".to_owned(),
                    |host| serde_json::to_string(&host.background()).unwrap_or_default(),
                )
            });
        }
    }

    /// `id` was unloaded: its hooks made at run time, its schedule and its
    /// background record go.
    pub(crate) fn forget_events(&self, id: &str) {
        let state = self.event_state();
        lock(&state.subscriptions).remove(id);
        lock(&state.due).retain(|(owner, _), _| owner != id);
        lock(&state.background).remove(id);
    }
}

/// What starts a background extension's instance, from the state its last
/// one saved.
pub(crate) type Start =
    Box<dyn Fn(Option<Vec<u8>>) -> Result<Box<dyn Instance>, String> + Send + Sync>;

/// A background extension's record and its instance when it runs.
pub(crate) struct Core {
    id: String,
    idle_ms: u64,
    clock: SharedClock,
    logs: Logs,
    start: Start,
    st: Mutex<St>,
}

#[derive(Default)]
struct St {
    live: Option<Arc<dyn Instance>>,
    /// What its last instance saved as it stopped.
    saved: Option<Vec<u8>>,
    /// Its live instance's handlers, by action.
    handlers: BTreeMap<String, CallerAsyncHandler>,
    busy: usize,
    last_used: u64,
    starts: u64,
    started_by: Option<String>,
    error: Option<String>,
}

impl Core {
    /// Its instance, started for `by` when it is not running.
    fn ensure(&self, by: &str) -> Result<Arc<dyn Instance>, String> {
        let mut st = lock(&self.st);
        if let Some(live) = &st.live {
            return Ok(Arc::clone(live));
        }
        let span = tracing::info_span!(
            "ext.background_start",
            ext.id = %self.id,
            ext.started_by = %by,
            ext.outcome = tracing::field::Empty,
        );
        let _entered = span.enter();
        match (self.start)(st.saved.take()) {
            Ok(instance) => {
                let live: Arc<dyn Instance> = Arc::from(instance);
                st.live = Some(Arc::clone(&live));
                st.starts = st.starts.saturating_add(1);
                st.started_by = Some(by.to_owned());
                st.error = None;
                drop(st);
                span.record("ext.outcome", "started");
                self.logs.push(
                    &self.id,
                    Level::Info,
                    "background",
                    format!("started ({by})"),
                );
                Ok(live)
            }
            Err(why) => {
                st.error = Some(why.clone());
                drop(st);
                span.record("ext.outcome", "failed");
                self.logs.push(
                    &self.id,
                    Level::Error,
                    "background",
                    format!("did not start ({by}): {why}"),
                );
                Err(why)
            }
        }
    }

    /// Its live instance's handler of `decl`, started for it when not
    /// running.
    fn handler(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let live = self.ensure(&format!("action {}", decl.id))?;
        if let Some(h) = lock(&self.st).handlers.get(&decl.id) {
            return Ok(Arc::clone(h));
        }
        let h = live.action(decl)?;
        lock(&self.st)
            .handlers
            .insert(decl.id.clone(), Arc::clone(&h));
        Ok(h)
    }

    /// A call begins: it is not idle while one runs.
    fn enter(&self) {
        let mut st = lock(&self.st);
        st.busy = st.busy.saturating_add(1);
        st.last_used = self.clock.now_ms();
    }

    fn leave(&self) {
        let mut st = lock(&self.st);
        st.busy = st.busy.saturating_sub(1);
        st.last_used = self.clock.now_ms();
    }

    /// Stops it when it has been idle its timeout at `now`: whether it did.
    fn reap(&self, now: u64) -> bool {
        let mut st = lock(&self.st);
        let idle_until = st.last_used.saturating_add(self.idle_ms);
        if st.busy > 0 || now < idle_until {
            return false;
        }
        let Some(live) = st.live.take() else {
            return false;
        };
        st.handlers.clear();
        st.saved = live.save_state();
        drop(st);
        live.stop();
        tracing::info_span!("ext.background_stop", ext.id = %self.id, ext.idle_ms = self.idle_ms)
            .in_scope(|| {});
        self.logs.push(
            &self.id,
            Level::Info,
            "background",
            format!("stopped: idle for {} s", self.idle_ms / 1000),
        );
        true
    }

    fn status(&self) -> BackgroundStatus {
        let st = lock(&self.st);
        let state = if st.live.is_some() {
            BackgroundState::Running
        } else if st.error.is_some() {
            BackgroundState::Failed
        } else {
            BackgroundState::Stopped
        };
        BackgroundStatus {
            id: self.id.clone(),
            state,
            starts: st.starts,
            started_by: st.started_by.clone(),
            busy: st.busy,
            last_used_ms: st.last_used,
            idle_ms: self.idle_ms,
            stops_at_ms: (st.live.is_some() && st.busy == 0)
                .then(|| st.last_used.saturating_add(self.idle_ms)),
            error: st.error.clone(),
        }
    }
}

/// A background extension's instance: nothing runs until its first
/// action or hook.
struct Lazy {
    core: Arc<Core>,
}

impl Instance for Lazy {
    fn action(&self, decl: &ActionDecl) -> Result<CallerAsyncHandler, String> {
        let core = Arc::clone(&self.core);
        let decl = decl.clone();
        Ok(Arc::new(move |caller: &ActionCaller, args: &[u8]| {
            let core = Arc::clone(&core);
            let decl = decl.clone();
            let caller = caller.clone();
            let args = args.to_vec();
            let work: ActionFuture = Box::pin(async move {
                core.enter();
                let out = async {
                    let handler = core.handler(&decl).map_err(|message| ActionError::Failed {
                        id: decl.id.clone(),
                        message,
                    })?;
                    handler(&caller, &args)?.await
                }
                .await;
                core.leave();
                out
            });
            Ok(work)
        }))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        let st = lock(&self.core.st);
        st.live
            .as_ref()
            .map_or_else(|| st.saved.clone(), |live| live.save_state())
    }

    fn hook(&self, decl: &HookDecl, caller: &ActionCaller, args: Value) -> Option<ActionFuture> {
        let core = Arc::clone(&self.core);
        let decl = decl.clone();
        let caller = caller.clone();
        Some(Box::pin(async move {
            core.enter();
            let out = async {
                let live = core
                    .ensure(&format!("hook {}", decl.when()))
                    .map_err(|message| ActionError::Failed {
                        id: decl.when(),
                        message,
                    })?;
                match live.hook(&decl, &caller, args) {
                    Some(work) => work.await,
                    None => Ok(Vec::new()),
                }
            }
            .await;
            core.leave();
            out
        }))
    }

    fn stop(&self) {
        let live = {
            let mut st = lock(&self.core.st);
            st.handlers.clear();
            st.live.take()
        };
        if let Some(live) = live {
            live.stop();
        }
    }
}
