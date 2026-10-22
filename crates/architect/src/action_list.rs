//! The Action List: every command an app can perform, in one registry
//! that everything calls.
//!
//! A key, a menu, a command palette, a CLI, a script and a model talking
//! MCP (`architect-mcp`) do not each keep a table of what the app can do
//! and how — they all look it up here and run it here, so the same id
//! does the same thing whoever asks. What an action is (`ActionMeta`: id,
//! label, description, category, group, the shortcut it shows) comes from
//! `#[architect::actions]`; what it takes and gives back are facet types,
//! so the list can say exactly what to send ([`ActionInfo::params_schema`],
//! JSON Schema from [`crate::schema`]) and decode it before the handler
//! ever sees it.
//!
//! - **Running**: [`ActionList::run_json`] — the one door, JSON in and
//!   out (an action with no parameters takes `{}` or nothing);
//!   [`ActionList::run`] for a key or a menu, which has nothing to pass.
//! - **Registering**: a typed action with [`ActionList::register`]; a
//!   plain one through [`crate::action::ActionBackend`], which the
//!   `register_*_actions` functions `#[architect::actions]` emits already
//!   take — every existing action trait lands here unchanged.
//! - **Async actions**: [`ActionList::register_async`] takes a handler
//!   that answers with a future (a service call, a query, a peer), run
//!   with [`ActionList::run_json_async`]. Arguments are still decoded
//!   before the future exists, so a bad call is refused without running
//!   anything. `run_json_async` runs sync actions too, so a caller that can
//!   await (an MCP transport, a script) uses it for everything; the sync
//!   [`ActionList::run_json`] refuses an async action rather than block
//!   the thread it was called on. A `#[architect::service(actions)]` trait
//!   puts every method on the list with one call
//!   (`http::actions::register(backend, &list)`).
//! - **Live updates**: every run, and every [`ActionList::changed`] the
//!   app reports (a project opened, a track renamed), is an
//!   [`ActionEvent`] with a sequence number. [`ActionList::events_since`]
//!   reads them from where a watcher left off, waiting a while for the
//!   next if there are none yet — what a long-poll or an MCP `watch`
//!   tool is built on.
//! - **Guarding**: an action can be *disabled* with a reason
//!   ([`ActionList::set_enabled`]; it shows in [`ActionInfo`] and refuses
//!   to run with [`ActionError::Disabled`]); a list can carry a
//!   [`PermissionEngine`] ([`ActionList::set_authorizer`]) whose
//!   [`PermissionEngine::authorize`] decides who may see
//!   ([`ActionList::infos_for`]) and run ([`ActionList::run_json_as`])
//!   which action; a handler that panics is caught and answered as
//!   [`ActionError::Failed`], never a dead host (under `panic = "unwind"`,
//!   the default — a `panic = "abort"` build has nothing to catch).
//! - **Checking**: a second registration of an id is refused
//!   ([`ActionError::Duplicate`]) and the first stays.
//!   [`ActionList::validate`] reports what is wrong with a whole list
//!   (duplicate ids, ids that are not `AREA_VERB`, two actions on one
//!   shortcut, a schema that is not a JSON object) and
//!   [`ActionList::probe_hostile_args`] runs every action that takes
//!   arguments with `{}` and with malformed JSON ([`HOSTILE_ARGS`]),
//!   reporting any that panics or accepts garbage. An app's registry test
//!   is those two on its list:
//!
//! ```ignore
//! #[test]
//! fn the_action_list_is_sound() {
//!     let list = ActionList::new();
//!     my_app::actions::register(&list);
//!     list.validate().unwrap();
//!     list.probe_hostile_args().unwrap();
//! }
//! ```

use std::collections::{BTreeMap, VecDeque, btree_map};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::time::Duration;

use architect_permissions::{Decision, PermissionEngine, Principal};
use facet::{Facet, Shape};

use crate::action::{ActionBackend, ActionMeta};

/// What running an action does to the app — what a caller (a model,
/// above all) has to know before it runs one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
pub enum Effect {
    /// Reads only: safe to run any time, any number of times.
    ReadOnly,
    /// Changes the app, undoably.
    Mutating,
    /// Changes the app in a way an undo may not bring back — deletes,
    /// writes to disk, renders over a file.
    Destructive,
}

/// One action as the list describes it.
#[derive(Clone, Debug, Facet)]
pub struct ActionInfo {
    /// The stable id everything runs it by (`SESSION_GAIN_STAGE`).
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub category: String,
    pub group: String,
    /// How a person reaches it (`"Shift+G"`), for display; empty when
    /// it has no shortcut.
    pub shortcut: String,
    pub effect: Effect,
    /// The JSON Schema of what it takes; `None` takes nothing.
    pub params_schema: Option<String>,
    /// The JSON Schema of what it gives back; `None` gives nothing.
    pub result_schema: Option<String>,
    /// Whether it can run right now ([`ActionList::set_enabled`]).
    pub enabled: bool,
    /// Why not, when it cannot (`"no project is open"`).
    pub disabled_reason: Option<String>,
}

/// Why an action did not run, or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// No action has this id.
    Unknown(String),
    /// The arguments did not decode into what it takes — `expected` is
    /// its params schema, so the caller can correct itself.
    BadArgs {
        id: String,
        message: String,
        expected: Option<String>,
    },
    /// It ran and said no — or panicked (`message` starts `panicked: `).
    Failed { id: String, message: String },
    /// The list's [`PermissionEngine`] does not let this caller run it.
    Denied { id: String, reason: String },
    /// It cannot run right now, and says why.
    Disabled { id: String, reason: String },
    /// An action with this id is already registered; the first stays.
    Duplicate(String),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "no action {id}"),
            Self::BadArgs { id, message, .. } => write!(f, "{id}: bad arguments: {message}"),
            Self::Failed { id, message } => write!(f, "{id}: {message}"),
            Self::Denied { id, reason } => write!(f, "{id}: not allowed: {reason}"),
            Self::Disabled { id, reason } => write!(f, "{id}: disabled: {reason}"),
            Self::Duplicate(id) => write!(f, "{id}: already registered"),
        }
    }
}

impl std::error::Error for ActionError {}

/// Everything [`ActionList::validate`] or [`ActionList::probe_hostile_args`]
/// found wrong, one line each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub problems: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the Action List has {} problem(s):", self.problems.len())?;
        for problem in &self.problems {
            write!(f, "\n- {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

impl ValidationError {
    fn result(problems: Vec<String>) -> Result<(), Self> {
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Self { problems })
        }
    }
}

/// Something that happened, as a watcher sees it.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct ActionEvent {
    /// Its place in the list's history: a watcher asks for what came
    /// after the last one it saw.
    pub seq: u64,
    pub kind: EventKind,
}

/// What happened.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum EventKind {
    /// An action ran (`ok` false: it failed).
    Ran { id: String, ok: bool },
    /// The app's state changed: `topic` says what (`project`,
    /// `tracks`, `transport`), `detail` which (a GUID, a name).
    Changed { topic: String, detail: String },
    /// An action joined the list.
    Registered { id: String },
}

/// How many events the list keeps for a watcher that falls behind.
pub const EVENT_HISTORY: usize = 4096;

/// What a handler is, once its types are erased: JSON in, JSON out.
pub type Handler = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, ActionError> + Send + Sync>;

/// The future an async action answers with: its result as JSON. `Send`
/// natively; on wasm, where nothing is, it need not be.
#[cfg(not(target_arch = "wasm32"))]
pub type ActionFuture = futures::future::BoxFuture<'static, Result<Vec<u8>, ActionError>>;
/// The future an async action answers with: its result as JSON. `Send`
/// natively; on wasm, where nothing is, it need not be.
#[cfg(target_arch = "wasm32")]
pub type ActionFuture = futures::future::LocalBoxFuture<'static, Result<Vec<u8>, ActionError>>;

/// An async handler, once its types are erased: the arguments are decoded
/// at once (`Err` refuses the call before anything runs) and the work is
/// the future it gives back.
pub type AsyncHandler = Arc<dyn Fn(&[u8]) -> Result<ActionFuture, ActionError> + Send + Sync>;

/// Whether an action can run right now: `Err` says why not.
pub type EnabledCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// The engine a list asks who may see and run what.
pub type Authorizer = Arc<dyn PermissionEngine + Send + Sync>;

/// Malformed JSON no action may accept: what
/// [`ActionList::probe_hostile_args`] sends (with two deeply nested
/// openings it builds itself).
pub const HOSTILE_ARGS: &[&[u8]] = &[
    b"{",
    b"}",
    b"[1,2",
    b"{\"a\":",
    b"{\"a\" 1}",
    b"nul",
    b"\"unterminated",
    b"{} trailing",
    b"\xff\xfe\xfd",
    b"{\"\xff\": 1}",
];

/// How deep the nested openings [`ActionList::probe_hostile_args`] sends go.
pub const HOSTILE_DEPTH: usize = 4096;

/// The deepest nesting of arrays and objects an action's arguments may
/// have; deeper is refused before any decoder recurses into it.
pub const MAX_ARGS_DEPTH: usize = 128;

#[derive(Clone)]
struct Entry {
    meta: &'static ActionMeta,
    effect: Effect,
    params: Option<&'static Shape>,
    result: Option<&'static Shape>,
    run: Run,
    enabled: Option<EnabledCheck>,
}

/// How an action does its work.
#[derive(Clone)]
enum Run {
    Sync(Handler),
    Async(AsyncHandler),
}

struct Events {
    next: u64,
    kept: VecDeque<ActionEvent>,
}

struct Inner {
    entries: RwLock<BTreeMap<String, Entry>>,
    /// Ids registered a second time, for [`ActionList::validate`].
    duplicates: Mutex<Vec<String>>,
    authorizer: RwLock<Option<Authorizer>>,
    events: Mutex<Events>,
    arrived: Condvar,
}

/// The registry. Cheap to clone: every clone is the same list.
#[derive(Clone)]
pub struct ActionList {
    inner: Arc<Inner>,
}

impl Default for ActionList {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ActionList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionList")
            .field("actions", &self.len())
            .finish()
    }
}

impl ActionList {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                entries: RwLock::new(BTreeMap::new()),
                duplicates: Mutex::new(Vec::new()),
                authorizer: RwLock::new(None),
                events: Mutex::new(Events {
                    next: 1,
                    kept: VecDeque::new(),
                }),
                arrived: Condvar::new(),
            }),
        }
    }

    /// Add an action that takes `A` and gives back `R` (each a facet
    /// type; `()` for nothing).
    ///
    /// # Errors
    ///
    /// [`ActionError::Duplicate`] when the id is already registered: the
    /// first registration stays, and [`Self::validate`] reports it too.
    pub fn register<A, R, F>(
        &self,
        meta: &'static ActionMeta,
        effect: Effect,
        run: F,
    ) -> Result<(), ActionError>
    where
        A: Facet<'static>,
        R: Facet<'static>,
        F: Fn(A) -> Result<R, String> + Send + Sync + 'static,
    {
        let id = meta.id;
        let takes = !is_unit(A::SHAPE);
        let handler: Handler = Arc::new(move |args: &[u8]| {
            let decoded: A = decode_args::<A>(id, takes, args)?;
            let out = run(decoded).map_err(|message| ActionError::Failed {
                id: id.to_owned(),
                message,
            })?;
            facet_json::to_vec(&out).map_err(|e| ActionError::Failed {
                id: id.to_owned(),
                message: format!("its result did not encode: {e}"),
            })
        });
        self.insert(Entry {
            meta,
            effect,
            params: takes.then_some(A::SHAPE),
            result: (!is_unit(R::SHAPE)).then_some(R::SHAPE),
            run: Run::Sync(handler),
            enabled: None,
        })
    }

    /// Add an action that takes `A` and answers, in time, with `R` — a
    /// service call, a query, anything that awaits. `A` is decoded when
    /// the action is called, before `run` is; the future `run` gives
    /// back is the work, run by [`Self::run_json_async`].
    ///
    /// # Errors
    ///
    /// As [`Self::register`].
    pub fn register_async<A, R, F, Fut>(
        &self,
        meta: &'static ActionMeta,
        effect: Effect,
        run: F,
    ) -> Result<(), ActionError>
    where
        A: Facet<'static>,
        R: Facet<'static>,
        F: Fn(A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, String>> + crate::MaybeSend + 'static,
    {
        let id = meta.id;
        let takes = !is_unit(A::SHAPE);
        let handler: AsyncHandler = Arc::new(move |args: &[u8]| {
            let decoded: A = decode_args::<A>(id, takes, args)?;
            let work = run(decoded);
            let future: ActionFuture = Box::pin(async move {
                let out = work.await.map_err(|message| ActionError::Failed {
                    id: id.to_owned(),
                    message,
                })?;
                facet_json::to_vec(&out).map_err(|e| ActionError::Failed {
                    id: id.to_owned(),
                    message: format!("its result did not encode: {e}"),
                })
            });
            Ok(future)
        });
        self.insert(Entry {
            meta,
            effect,
            params: takes.then_some(A::SHAPE),
            result: (!is_unit(R::SHAPE)).then_some(R::SHAPE),
            run: Run::Async(handler),
            enabled: None,
        })
    }

    /// Add an action whose handler is already JSON in, JSON out — one
    /// built at run time (an rpc method, a script). `params` and `result`
    /// say what it takes and gives, for its schema.
    ///
    /// # Errors
    ///
    /// As [`Self::register`].
    pub fn register_dynamic(
        &self,
        meta: &'static ActionMeta,
        effect: Effect,
        params: Option<&'static Shape>,
        result: Option<&'static Shape>,
        handler: Handler,
    ) -> Result<(), ActionError> {
        self.insert(Entry {
            meta,
            effect,
            params,
            result,
            run: Run::Sync(handler),
            enabled: None,
        })
    }

    /// [`Self::register_dynamic`] for a handler that answers with a
    /// future: what a projection built at run time registers.
    ///
    /// # Errors
    ///
    /// As [`Self::register`].
    pub fn register_dynamic_async(
        &self,
        meta: &'static ActionMeta,
        effect: Effect,
        params: Option<&'static Shape>,
        result: Option<&'static Shape>,
        handler: AsyncHandler,
    ) -> Result<(), ActionError> {
        self.insert(Entry {
            meta,
            effect,
            params,
            result,
            run: Run::Async(handler),
            enabled: None,
        })
    }

    fn insert(&self, entry: Entry) -> Result<(), ActionError> {
        let id = entry.meta.id.to_owned();
        let added = match self
            .inner
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.clone())
        {
            btree_map::Entry::Vacant(slot) => {
                slot.insert(entry);
                true
            }
            btree_map::Entry::Occupied(_) => false,
        };
        if !added {
            self.inner
                .duplicates
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(id.clone());
            return Err(ActionError::Duplicate(id));
        }
        self.publish(EventKind::Registered { id });
        Ok(())
    }

    /// Say when `id` can run: `check` answers `Err(why)` while it cannot
    /// (`"no project is open"`). Asked before every run and whenever the
    /// action is described; a check that panics counts as disabled.
    ///
    /// # Errors
    ///
    /// [`ActionError::Unknown`] when there is no such action.
    pub fn set_enabled<F>(&self, id: &str, check: F) -> Result<(), ActionError>
    where
        F: Fn() -> Result<(), String> + Send + Sync + 'static,
    {
        let mut entries = self
            .inner
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| ActionError::Unknown(id.to_owned()))?;
        entry.enabled = Some(Arc::new(check));
        drop(entries);
        Ok(())
    }

    /// Who may see and run what: from now on [`Self::infos_for`] shows a
    /// caller only what `engine` lets it run, and [`Self::run_json_as`]
    /// refuses the rest ([`PermissionEngine::authorize`]). Without one,
    /// every caller may run everything.
    pub fn set_authorizer(&self, engine: impl PermissionEngine + Send + Sync + 'static) {
        *self
            .inner
            .authorizer
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(engine));
    }

    /// May `who` run `id`? [`Decision::Allow`] when the list has no
    /// authorizer.
    #[must_use]
    pub fn authorize(&self, who: &Principal, id: &str) -> Decision {
        let engine = self
            .inner
            .authorizer
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        engine.map_or(Decision::Allow, |engine| engine.authorize(who, id))
    }

    /// The entries, copied out so no lock is held while an enabled check
    /// or a handler runs (either may use the list itself).
    fn snapshot(&self) -> Vec<Entry> {
        self.inner
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    fn entry(&self, id: &str) -> Option<Entry> {
        self.inner
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Every action, by id — the app's own, trusted view. A caller from
    /// outside sees [`Self::infos_for`].
    #[must_use]
    pub fn infos(&self) -> Vec<ActionInfo> {
        self.snapshot().iter().map(info).collect()
    }

    /// The actions `who` may run, by id.
    #[must_use]
    pub fn infos_for(&self, who: &Principal) -> Vec<ActionInfo> {
        self.snapshot()
            .iter()
            .filter(|e| self.authorize(who, e.meta.id).allowed())
            .map(info)
            .collect()
    }

    /// One action.
    #[must_use]
    pub fn info(&self, id: &str) -> Option<ActionInfo> {
        self.entry(id).as_ref().map(info)
    }

    /// One action, if `who` may run it.
    #[must_use]
    pub fn info_for(&self, who: &Principal, id: &str) -> Option<ActionInfo> {
        if !self.authorize(who, id).allowed() {
            return None;
        }
        self.info(id)
    }

    /// How many actions there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run `id` with `args` (JSON; empty for an action that takes
    /// nothing), giving back its result as JSON (`null` for nothing) —
    /// as the app itself (a key, a menu), so the authorizer is not asked.
    /// A caller from outside (a model, a peer) runs through
    /// [`Self::run_json_as`].
    ///
    /// # Errors
    ///
    /// No such action, it is disabled, arguments it cannot take, or it
    /// failed (or panicked).
    pub fn run_json(&self, id: &str, args: &[u8]) -> Result<Vec<u8>, ActionError> {
        self.run_checked(None, id, args)
    }

    /// Run `id` for `who`: as [`Self::run_json`], once the list's
    /// authorizer lets `who` run it.
    ///
    /// # Errors
    ///
    /// As [`Self::run_json`], and [`ActionError::Denied`].
    pub fn run_json_as(
        &self,
        who: &Principal,
        id: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ActionError> {
        self.run_checked(Some(who), id, args)
    }

    /// Run `id` with `args`, awaiting it when it is async — the door for
    /// a caller that can await (an MCP transport, a script). A sync
    /// action runs as [`Self::run_json`] would; the app's trusted door,
    /// so the authorizer is not asked.
    ///
    /// # Errors
    ///
    /// As [`Self::run_json`].
    pub async fn run_json_async(&self, id: &str, args: &[u8]) -> Result<Vec<u8>, ActionError> {
        self.run_async_checked(None, id, args).await
    }

    /// [`Self::run_json_async`] for `who`, once the authorizer lets it.
    ///
    /// # Errors
    ///
    /// As [`Self::run_json_as`].
    pub async fn run_json_async_as(
        &self,
        who: &Principal,
        id: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ActionError> {
        self.run_async_checked(Some(who), id, args).await
    }

    /// Whether `id` is an async action — one [`Self::run_json`] refuses.
    #[must_use]
    pub fn is_async(&self, id: &str) -> bool {
        self.entry(id)
            .is_some_and(|e| matches!(e.run, Run::Async(_)))
    }

    /// The entry for `id`, once `who` may run it.
    fn admitted(&self, who: Option<&Principal>, id: &str) -> Result<Entry, ActionError> {
        let Some(entry) = self.entry(id) else {
            return Err(ActionError::Unknown(id.to_owned()));
        };
        if let Some(who) = who
            && let Decision::Deny { reason } = self.authorize(who, id)
        {
            return Err(ActionError::Denied {
                id: id.to_owned(),
                reason,
            });
        }
        Ok(entry)
    }

    async fn run_async_checked(
        &self,
        who: Option<&Principal>,
        id: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ActionError> {
        use futures::FutureExt as _;
        use tracing::Instrument as _;
        let entry = self.admitted(who, id)?;
        let Run::Async(handler) = &entry.run else {
            return self.run_checked(who, id, args);
        };
        let args = normalized(&entry, args);
        let span = tracing::info_span!("action", action.id = id, action.ok = tracing::field::Empty);
        let started = span.in_scope(|| match enabled_state(entry.enabled.as_ref()) {
            Ok(()) => guarded_start(id, || {
                admit_args(&entry, args)?;
                handler(args)
            }),
            Err(reason) => Err(ActionError::Disabled {
                id: id.to_owned(),
                reason,
            }),
        });
        let out = match started {
            Ok(work) => std::panic::AssertUnwindSafe(work)
                .catch_unwind()
                .instrument(span.clone())
                .await
                .unwrap_or_else(|panic| Err(panicked(id, panic.as_ref()))),
            Err(error) => Err(error),
        };
        span.record("action.ok", out.is_ok());
        drop(span);
        self.publish(EventKind::Ran {
            id: id.to_owned(),
            ok: out.is_ok(),
        });
        out
    }

    fn run_checked(
        &self,
        who: Option<&Principal>,
        id: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ActionError> {
        let entry = self.admitted(who, id)?;
        let args = normalized(&entry, args);
        // One span per action run: the action's wide event. What it does
        // marks its stages on it (`architect_telemetry::timing`), so where
        // an action's time went is one record, whoever ran it — a key, a
        // menu, a model.
        let span = tracing::info_span!("action", action.id = id, action.ok = tracing::field::Empty);
        let out = span.in_scope(|| match enabled_state(entry.enabled.as_ref()) {
            Ok(()) => guarded(id, || call(&entry, args)),
            Err(reason) => Err(ActionError::Disabled {
                id: id.to_owned(),
                reason,
            }),
        });
        span.record("action.ok", out.is_ok());
        drop(span);
        self.publish(EventKind::Ran {
            id: id.to_owned(),
            ok: out.is_ok(),
        });
        out
    }

    /// Run `id`, which takes nothing — what a key or a menu does.
    ///
    /// # Errors
    ///
    /// As [`Self::run_json`].
    pub fn run(&self, id: &str) -> Result<(), ActionError> {
        self.run_json(id, b"").map(|_| ())
    }

    /// Run `id`, which takes nothing, for `who`.
    ///
    /// # Errors
    ///
    /// As [`Self::run_json_as`].
    pub fn run_as(&self, who: &Principal, id: &str) -> Result<(), ActionError> {
        self.run_json_as(who, id, b"").map(|_| ())
    }

    /// What is wrong with the list as a whole: an id registered twice, an
    /// id that is not `AREA_VERB` (`SCREAMING_SNAKE_CASE`), two actions
    /// on one shortcut, a params or result schema that is not a JSON
    /// object. Runs nothing.
    ///
    /// # Errors
    ///
    /// Every problem found, one line each.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut problems: Vec<String> = self
            .inner
            .duplicates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|id| format!("{id}: registered more than once (the first registration stays)"))
            .collect();
        let mut shortcuts: BTreeMap<String, &'static str> = BTreeMap::new();
        for entry in self.snapshot() {
            let id = entry.meta.id;
            if !is_action_id(id) {
                problems.push(format!(
                    "{id:?}: an id is SCREAMING_SNAKE_CASE, `AREA_VERB`"
                ));
            }
            let shortcut = entry.meta.shortcut.trim().to_lowercase();
            if !shortcut.is_empty()
                && let Some(other) = shortcuts.insert(shortcut, id)
            {
                problems.push(format!(
                    "{id}: its shortcut {:?} is also {other}'s",
                    entry.meta.shortcut
                ));
            }
            for (what, shape) in [("params", entry.params), ("result", entry.result)] {
                if let Some(problem) = shape.and_then(schema_problem) {
                    problems.push(format!("{id}: its {what} schema {problem}"));
                }
            }
        }
        ValidationError::result(problems)
    }

    /// Run every action that takes arguments with `{}` — which it may
    /// take or refuse, but must not panic on — and with each of
    /// [`HOSTILE_ARGS`] and two [`HOSTILE_DEPTH`]-deep nestings, which it
    /// must refuse without panicking. Actions that take nothing never see
    /// their arguments and are not run.
    ///
    /// This RUNS sync handlers (straight, past the enabled check and the
    /// authorizer): call it on a list built for a test, not on a live app.
    /// An async action is only decoded: the future its handler gives back
    /// is dropped unpolled, so nothing it would do happens.
    ///
    /// # Errors
    ///
    /// Every action that panicked or took malformed JSON, one line each.
    pub fn probe_hostile_args(&self) -> Result<(), ValidationError> {
        let deep_array = "[".repeat(HOSTILE_DEPTH);
        let deep_object = "{\"a\":".repeat(HOSTILE_DEPTH);
        let mut problems = Vec::new();
        for entry in self.snapshot() {
            if entry.params.is_none() {
                continue;
            }
            let id = entry.meta.id;
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| probe(&entry, b"{}"))) {
                problems.push(format!(
                    "{id}: panicked on {{}}: {}",
                    panic_message(panic.as_ref())
                ));
            }
            let hostile = HOSTILE_ARGS
                .iter()
                .copied()
                .chain([deep_array.as_bytes(), deep_object.as_bytes()]);
            for args in hostile {
                let shown = String::from_utf8_lossy(args.get(..24).unwrap_or(args)).into_owned();
                match catch_unwind(AssertUnwindSafe(|| probe(&entry, args))) {
                    Ok(Ok(_)) => problems.push(format!("{id}: took malformed JSON {shown:?}")),
                    Ok(Err(_)) => {}
                    Err(panic) => problems.push(format!(
                        "{id}: panicked on {shown:?}: {}",
                        panic_message(panic.as_ref())
                    )),
                }
            }
        }
        ValidationError::result(problems)
    }

    /// Say the app's state changed — `topic` what (`tracks`), `detail`
    /// which — so whoever watches hears it.
    pub fn changed(&self, topic: &str, detail: &str) {
        self.publish(EventKind::Changed {
            topic: topic.to_owned(),
            detail: detail.to_owned(),
        });
    }

    fn publish(&self, kind: EventKind) {
        let mut events = self
            .inner
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let seq = events.next;
        events.next = seq.saturating_add(1);
        events.kept.push_back(ActionEvent { seq, kind });
        while events.kept.len() > EVENT_HISTORY {
            events.kept.pop_front();
        }
        drop(events);
        self.inner.arrived.notify_all();
    }

    /// The events after `since` (0 for all it still keeps), at most
    /// `max`; when there are none yet, waits up to `wait` for one. Gives
    /// back the events and the `since` to ask with next.
    #[must_use]
    pub fn events_since(&self, since: u64, max: usize, wait: Duration) -> (Vec<ActionEvent>, u64) {
        let Ok(mut events) = self.inner.events.lock() else {
            return (Vec::new(), since);
        };
        let newer = |e: &Events| e.kept.back().is_some_and(|last| last.seq > since);
        if !newer(&events) && !wait.is_zero() {
            events = match self
                .inner
                .arrived
                .wait_timeout_while(events, wait, |e| !newer(e))
            {
                Ok((events, _)) => events,
                Err(_) => return (Vec::new(), since),
            };
        }
        let out: Vec<ActionEvent> = events
            .kept
            .iter()
            .filter(|e| e.seq > since)
            .take(max)
            .cloned()
            .collect();
        let next = out.last().map_or(since, |e| e.seq);
        (out, next)
    }

    /// The latest event's sequence number: where a new watcher starts.
    #[must_use]
    pub fn latest(&self) -> u64 {
        self.inner
            .events
            .lock()
            .map_or(0, |e| e.next.saturating_sub(1))
    }
}

/// Every `#[architect::actions]` trait registers through this, so its
/// actions — which take nothing and give nothing — land in the list as
/// they are. Marked [`Effect::Mutating`]: an action is a command. A
/// duplicate id is logged and kept out (the trait has no way to refuse);
/// [`ActionList::validate`] reports it.
impl ActionBackend for ActionList {
    fn register(
        &self,
        meta: &'static ActionMeta,
        handler: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) {
        let id = meta.id;
        let erased: Handler = Arc::new(move |_args: &[u8]| {
            handler().map_err(|message| ActionError::Failed {
                id: id.to_owned(),
                message,
            })?;
            Ok(b"null".to_vec())
        });
        let added = self.insert(Entry {
            meta,
            effect: Effect::Mutating,
            params: None,
            result: None,
            run: Run::Sync(erased),
            enabled: None,
        });
        // The trait cannot say no; `validate` will.
        if let Err(error) = added {
            tracing::error!(%error, "an action id was registered twice; the first stays");
        }
    }
}

fn info(entry: &Entry) -> ActionInfo {
    let m = entry.meta;
    let disabled_reason = enabled_state(entry.enabled.as_ref()).err();
    ActionInfo {
        id: m.id.to_owned(),
        display_name: m.display_name.to_owned(),
        description: m.description.to_owned(),
        category: m.category.to_owned(),
        group: m.group.to_owned(),
        shortcut: m.shortcut.to_owned(),
        effect: entry.effect,
        params_schema: entry.params.map(crate::schema::json_schema),
        result_schema: entry.result.map(crate::schema::json_schema),
        enabled: disabled_reason.is_none(),
        disabled_reason,
    }
}

/// Whether an action may run now: its enabled check, a panic in it
/// counted as a reason.
fn enabled_state(check: Option<&EnabledCheck>) -> Result<(), String> {
    let Some(check) = check else {
        return Ok(());
    };
    catch_unwind(AssertUnwindSafe(|| check())).unwrap_or_else(|panic| {
        Err(format!(
            "its enabled check panicked: {}",
            panic_message(panic.as_ref())
        ))
    })
}

/// Empty arguments, as the action expects them: `{}` for one that takes
/// some, `null` for one that takes none (whatever was sent).
fn normalized<'a>(entry: &Entry, args: &'a [u8]) -> &'a [u8] {
    let takes = entry.params.is_some();
    if !takes || args.iter().all(u8::is_ascii_whitespace) {
        if takes { b"{}" } else { b"null" }
    } else {
        args
    }
}

/// Decode an action's arguments into `A`: `null` when it takes none.
fn decode_args<A: Facet<'static>>(id: &str, takes: bool, args: &[u8]) -> Result<A, ActionError> {
    if takes {
        facet_json::from_slice::<A>(args).map_err(|e| ActionError::BadArgs {
            id: id.to_owned(),
            message: e.to_string(),
            expected: Some(crate::schema::json_schema(A::SHAPE)),
        })
    } else {
        facet_json::from_slice::<A>(b"null").map_err(|e| ActionError::BadArgs {
            id: id.to_owned(),
            message: e.to_string(),
            expected: None,
        })
    }
}

/// `args` may reach an action's handler when they are one well-formed
/// JSON value, no deeper than [`MAX_ARGS_DEPTH`], with nothing after it
/// (`facet_json` alone takes `{} junk`) — checked only when it takes any.
fn admit_args(entry: &Entry, args: &[u8]) -> Result<(), ActionError> {
    if let Some(params) = entry.params
        && let Err(message) = check_json(args)
    {
        return Err(ActionError::BadArgs {
            id: entry.meta.id.to_owned(),
            message,
            expected: Some(crate::schema::json_schema(params)),
        });
    }
    Ok(())
}

/// Hand admitted `args` to a sync action's handler; an async one is
/// refused, since running it here would block the caller's thread.
fn call(entry: &Entry, args: &[u8]) -> Result<Vec<u8>, ActionError> {
    admit_args(entry, args)?;
    match &entry.run {
        Run::Sync(handler) => handler(args),
        Run::Async(_) => Err(ActionError::Failed {
            id: entry.meta.id.to_owned(),
            message: "it is async: run it with run_json_async".to_owned(),
        }),
    }
}

/// What [`ActionList::probe_hostile_args`] does to one action: run a sync
/// one; decode for an async one and drop its future unpolled.
fn probe(entry: &Entry, args: &[u8]) -> Result<(), ActionError> {
    admit_args(entry, args)?;
    match &entry.run {
        Run::Sync(handler) => handler(args).map(drop),
        Run::Async(handler) => handler(args).map(drop),
    }
}

/// A caught panic, as the failure it is answered with.
fn panicked(id: &str, panic: &(dyn std::any::Any + Send)) -> ActionError {
    let message = panic_message(panic);
    tracing::error!(action.id = id, %message, "an action panicked");
    ActionError::Failed {
        id: id.to_owned(),
        message: format!("panicked: {message}"),
    }
}

/// Run `f`, a panic in it answered as [`ActionError::Failed`].
fn guarded(
    id: &str,
    f: impl FnOnce() -> Result<Vec<u8>, ActionError>,
) -> Result<Vec<u8>, ActionError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| Err(panicked(id, panic.as_ref())))
}

/// Start an async action — decode and build its future — a panic in that
/// answered as [`ActionError::Failed`].
fn guarded_start(
    id: &str,
    f: impl FnOnce() -> Result<ActionFuture, ActionError>,
) -> Result<ActionFuture, ActionError> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| Err(panicked(id, panic.as_ref())))
}

/// A container the JSON check is inside.
enum Frame {
    Array,
    Object,
}

/// Bytes of JSON, read once front to back.
struct Scan<'a> {
    bytes: std::iter::Peekable<std::slice::Iter<'a, u8>>,
}

impl Scan<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.bytes.peek().copied().copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.bytes.next_if(|b| **b == byte).is_some()
    }

    fn skip_space(&mut self) {
        while self
            .bytes
            .next_if(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
            .is_some()
        {}
    }

    fn word(&mut self, rest: &[u8]) -> Result<(), String> {
        if rest.iter().all(|b| self.eat(*b)) {
            Ok(())
        } else {
            Err("a misspelt true, false or null".to_owned())
        }
    }

    fn digits(&mut self) -> usize {
        let mut n = 0_usize;
        while self.bytes.next_if(|b| b.is_ascii_digit()).is_some() {
            n = n.saturating_add(1);
        }
        n
    }

    fn number(&mut self) -> Result<(), String> {
        self.eat(b'-');
        if !self.eat(b'0') && self.digits() == 0 {
            return Err("a number without digits".to_owned());
        }
        if self.eat(b'.') && self.digits() == 0 {
            return Err("a number with nothing after its point".to_owned());
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if self.digits() == 0 {
                return Err("a number with an empty exponent".to_owned());
            }
        }
        Ok(())
    }

    /// A string, its opening quote already read.
    fn string(&mut self) -> Result<(), String> {
        loop {
            match self.bytes.next().copied() {
                None => return Err("a string that never ends".to_owned()),
                Some(b'"') => return Ok(()),
                Some(b'\\') => match self.bytes.next().copied() {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    Some(b'u') => {
                        for _ in 0..4 {
                            if self.bytes.next_if(|b| b.is_ascii_hexdigit()).is_none() {
                                return Err("a \\u escape without four hex digits".to_owned());
                            }
                        }
                    }
                    _ => return Err("an unknown escape in a string".to_owned()),
                },
                Some(b) if b < 0x20 => return Err("a control character in a string".to_owned()),
                Some(_) => {}
            }
        }
    }

    /// An object's key and its colon.
    fn key(&mut self) -> Result<(), String> {
        self.skip_space();
        if !self.eat(b'"') {
            return Err("an object key that is not a string".to_owned());
        }
        self.string()?;
        self.skip_space();
        if self.eat(b':') {
            Ok(())
        } else {
            Err("an object key without a colon".to_owned())
        }
    }

    fn scalar(&mut self) -> Result<(), String> {
        match self.peek() {
            Some(b'"') => {
                self.bytes.next();
                self.string()
            }
            Some(b't') => self.word(b"true"),
            Some(b'f') => self.word(b"false"),
            Some(b'n') => self.word(b"null"),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err("something that is not a JSON value".to_owned()),
            None => Err("nothing where a value should be".to_owned()),
        }
    }
}

/// Whether `input` is one well-formed JSON value, at most
/// [`MAX_ARGS_DEPTH`] deep, with only whitespace after it. Iterative: a
/// hostile depth costs a refusal, not the stack.
fn check_json(input: &[u8]) -> Result<(), String> {
    std::str::from_utf8(input).map_err(|e| format!("not UTF-8: {e}"))?;
    let mut scan = Scan {
        bytes: input.iter().peekable(),
    };
    let mut stack: Vec<Frame> = Vec::new();
    'value: loop {
        scan.skip_space();
        let opened = match scan.peek() {
            Some(b'[') => Some(Frame::Array),
            Some(b'{') => Some(Frame::Object),
            _ => None,
        };
        if let Some(frame) = opened {
            scan.bytes.next();
            if stack.len() >= MAX_ARGS_DEPTH {
                return Err(format!("nested deeper than {MAX_ARGS_DEPTH}"));
            }
            scan.skip_space();
            let empty = match frame {
                Frame::Array => scan.eat(b']'),
                Frame::Object => scan.eat(b'}'),
            };
            if !empty {
                if matches!(frame, Frame::Object) {
                    scan.key()?;
                }
                stack.push(frame);
                continue 'value;
            }
        } else {
            scan.scalar()?;
        }
        // A value ended: what may follow it depends on what holds it.
        loop {
            scan.skip_space();
            match stack.last() {
                None => {
                    return if scan.peek().is_none() {
                        Ok(())
                    } else {
                        Err("something after the value".to_owned())
                    };
                }
                Some(Frame::Array) => {
                    if scan.eat(b',') {
                        continue 'value;
                    }
                    if !scan.eat(b']') {
                        return Err("an array without , or ]".to_owned());
                    }
                }
                Some(Frame::Object) => {
                    if scan.eat(b',') {
                        scan.key()?;
                        continue 'value;
                    }
                    if !scan.eat(b'}') {
                        return Err("an object without , or }".to_owned());
                    }
                }
            }
            stack.pop();
        }
    }
}

/// What a panic said, when it said it as text.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_owned())
}

/// `AREA_VERB`: upper-case ASCII letters, digits and `_`, starting with a
/// letter.
fn is_action_id(id: &str) -> bool {
    id.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && id
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// What is wrong with the JSON Schema of `shape`, if anything.
fn schema_problem(shape: &'static Shape) -> Option<String> {
    let schema = match catch_unwind(|| crate::schema::json_schema(shape)) {
        Ok(schema) => schema,
        Err(panic) => {
            return Some(format!(
                "could not be written: {}",
                panic_message(panic.as_ref())
            ));
        }
    };
    match facet_json::from_str::<facet_value::Value>(&schema) {
        Ok(value) if value.is_object() => None,
        Ok(_) => Some(format!("is not a JSON object: {schema}")),
        Err(e) => Some(format!("is not JSON ({e}): {schema}")),
    }
}

/// Whether `shape` is `()` — nothing to take or give.
fn is_unit(shape: &'static Shape) -> bool {
    shape.id == <() as Facet<'static>>::SHAPE.id
}

#[cfg(test)]
mod tests {
    use super::{ActionError, ActionList, Effect, EventKind};
    use crate::action::{ActionBackend, ActionMeta};
    use architect_permissions::{
        Action, Decision, PermissionEngine, Principal, Resource, Rule, ScopeEngine,
    };
    use facet::Facet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const fn meta(id: &'static str) -> ActionMeta {
        ActionMeta {
            id,
            trait_name: "Test",
            method_name: id,
            display_name: id,
            description: "a test action",
            category: "Test",
            group: "Test",
            shortcut: "",
            toggleable: false,
            undo: false,
        }
    }

    const fn keyed(id: &'static str, shortcut: &'static str) -> ActionMeta {
        ActionMeta {
            shortcut,
            ..meta(id)
        }
    }

    static HITS: ActionMeta = meta("DRUMS_HITS");
    static PLAY: ActionMeta = meta("PLAY");
    static SAVE: ActionMeta = keyed("PROJECT_SAVE", "Ctrl+S");
    static SAVE_AS: ActionMeta = keyed("PROJECT_SAVE_AS", "ctrl+s");
    static DELETE: ActionMeta = meta("PROJECT_DELETE");
    static BOOM: ActionMeta = meta("DRUMS_BOOM");
    static LOWER: ActionMeta = meta("drums.hits");

    #[derive(Facet)]
    struct Query {
        track: String,
        limit: Option<u32>,
    }

    #[derive(Facet, Debug, PartialEq)]
    struct Hits {
        times: Vec<f64>,
    }

    /// A typed action runs from JSON and answers in JSON; arguments it
    /// cannot take are refused with what it does take; a plain action
    /// registered the old way runs too; an unknown id is said so.
    #[test]
    fn everything_runs_through_one_door() {
        let list = ActionList::new();
        list.register(&HITS, Effect::ReadOnly, |q: Query| {
            let n = q.limit.unwrap_or(2);
            Ok(Hits {
                times: (0..n).map(f64::from).collect(),
            })
        })
        .expect("registered");
        let played = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&played);
        ActionBackend::register(
            &list,
            &PLAY,
            Arc::new(move || {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }),
        );

        let out = list
            .run_json("DRUMS_HITS", br#"{"track":"t","limit":3}"#)
            .expect("ran");
        assert_eq!(
            String::from_utf8(out).expect("utf8"),
            r#"{"times":[0,1,2]}"#
        );
        match list.run_json("DRUMS_HITS", br#"{"limit":3}"#) {
            Err(ActionError::BadArgs { expected, .. }) => {
                assert!(expected.is_some_and(|s| s.contains("\"track\"")));
            }
            other => panic!("refused, with what it takes: {other:?}"),
        }
        list.run("PLAY").expect("a key runs it");
        assert!(played.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(list.run("NOPE"), Err(ActionError::Unknown("NOPE".into())));

        let hits = list.info("DRUMS_HITS").expect("listed");
        assert_eq!(hits.effect, Effect::ReadOnly);
        assert!(
            hits.params_schema
                .is_some_and(|s| s.contains("\"required\":[\"track\"]"))
        );
        assert!(list.info("PLAY").is_some_and(|p| p.params_schema.is_none()));
    }

    fn hits(list: &ActionList) {
        list.register(&HITS, Effect::ReadOnly, |q: Query| {
            if q.track == "none" {
                return Err("no such track".to_owned());
            }
            Ok(Hits { times: vec![0.5] })
        })
        .expect("registered");
    }

    fn unit(list: &ActionList, meta: &'static ActionMeta) {
        list.register(meta, Effect::Mutating, |(): ()| Ok(()))
            .expect("registered");
    }

    /// (a) A disabled action says why — in its info and when run — and
    /// runs again once its check says yes; a check that panics counts as
    /// disabled; there is no check to set on an unknown id.
    #[test]
    fn a_disabled_action_says_why() {
        let list = ActionList::new();
        unit(&list, &SAVE);
        let open = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&open);
        list.set_enabled("PROJECT_SAVE", move || {
            if flag.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("no project is open".to_owned())
            }
        })
        .expect("set");

        let info = list.info("PROJECT_SAVE").expect("listed");
        assert!(!info.enabled);
        assert_eq!(info.disabled_reason.as_deref(), Some("no project is open"));
        assert_eq!(
            list.run("PROJECT_SAVE"),
            Err(ActionError::Disabled {
                id: "PROJECT_SAVE".into(),
                reason: "no project is open".into()
            })
        );

        open.store(true, Ordering::SeqCst);
        assert!(list.info("PROJECT_SAVE").is_some_and(|i| i.enabled));
        list.run("PROJECT_SAVE").expect("enabled now");

        list.set_enabled("PROJECT_SAVE", || panic!("check blew up"))
            .expect("set");
        let info = list.info("PROJECT_SAVE").expect("listed");
        assert!(
            info.disabled_reason
                .is_some_and(|r| r.contains("check blew up"))
        );
        assert_eq!(
            list.set_enabled("NOPE", || Ok(())),
            Err(ActionError::Unknown("NOPE".into()))
        );
    }

    /// The test engine: users may run read-only drum actions only; a
    /// service may run anything.
    struct DrumsOnly;
    impl PermissionEngine for DrumsOnly {
        fn check(&self, _: &Principal, _: &Resource, _: &Action) -> Decision {
            Decision::deny("ask authorize")
        }
        fn authorize(&self, who: &Principal, id: &str) -> Decision {
            match who {
                Principal::Service { .. } => Decision::Allow,
                _ if id.starts_with("DRUMS_") => Decision::Allow,
                _ => Decision::deny(format!("{} may not run {id}", who.describe())),
            }
        }
    }

    /// (b) With an authorizer, a caller runs only what it is allowed and
    /// is told why not; the app's own `run_json` is not asked; the
    /// default `authorize` goes through `check` on `action/<id>`.
    #[test]
    fn the_authorizer_decides_who_runs_what() {
        let list = ActionList::new();
        hits(&list);
        unit(&list, &DELETE);
        let model = Principal::User {
            user_id: "model".into(),
        };
        assert!(
            list.authorize(&model, "PROJECT_DELETE").allowed(),
            "no engine: allowed"
        );

        list.set_authorizer(DrumsOnly);
        list.run_json_as(&model, "DRUMS_HITS", br#"{"track":"a"}"#)
            .expect("allowed");
        assert_eq!(
            list.run_as(&model, "PROJECT_DELETE"),
            Err(ActionError::Denied {
                id: "PROJECT_DELETE".into(),
                reason: "user:model may not run PROJECT_DELETE".into()
            })
        );
        list.run_as(&Principal::Service { name: "app".into() }, "PROJECT_DELETE")
            .expect("a service may");
        list.run("PROJECT_DELETE")
            .expect("the app itself is not asked");

        list.set_authorizer(ScopeEngine::new(vec![Rule::new(
            "action/PROJECT_DELETE",
            &[Action::RUN],
        )]));
        list.run_as(&model, "PROJECT_DELETE").expect("rule allows");
        assert!(matches!(
            list.run_json_as(&model, "DRUMS_HITS", br#"{"track":"a"}"#),
            Err(ActionError::Denied { .. })
        ));
    }

    /// (g) A caller sees only the actions it may run.
    #[test]
    fn a_caller_sees_only_what_it_may_run() {
        let list = ActionList::new();
        hits(&list);
        unit(&list, &DELETE);
        let model = Principal::User {
            user_id: "model".into(),
        };
        assert_eq!(list.infos_for(&model).len(), 2, "no engine: everything");
        list.set_authorizer(DrumsOnly);
        let seen: Vec<String> = list.infos_for(&model).into_iter().map(|i| i.id).collect();
        assert_eq!(seen, vec!["DRUMS_HITS".to_owned()]);
        assert!(list.info_for(&model, "PROJECT_DELETE").is_none());
        assert!(list.info_for(&model, "DRUMS_HITS").is_some());
        assert_eq!(list.infos().len(), 2, "the app sees everything");
    }

    /// (c) A handler that panics is a `Failed`, not a dead host, and the
    /// list goes on working.
    #[test]
    fn a_panicking_handler_is_a_failure() {
        let list = ActionList::new();
        list.register(&BOOM, Effect::Mutating, |(): ()| -> Result<(), String> {
            panic!("boom")
        })
        .expect("registered");
        hits(&list);
        match list.run("DRUMS_BOOM") {
            Err(ActionError::Failed { id, message }) => {
                assert_eq!(id, "DRUMS_BOOM");
                assert_eq!(message, "panicked: boom");
            }
            other => panic!("a failure: {other:?}"),
        }
        let start = list.latest();
        list.run_json("DRUMS_HITS", br#"{"track":"a"}"#)
            .expect("still works");
        let (events, _) = list.events_since(start, 10, Duration::ZERO);
        assert_eq!(events.len(), 1);
    }

    /// (d) A second registration of an id is refused and the first stays;
    /// through the old trait path it is kept out and `validate` says so.
    #[test]
    fn a_duplicate_id_is_an_error() {
        let list = ActionList::new();
        hits(&list);
        let again = list.register(&HITS, Effect::Destructive, |(): ()| Ok(()));
        assert_eq!(again, Err(ActionError::Duplicate("DRUMS_HITS".into())));
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.info("DRUMS_HITS").map(|i| i.effect),
            Some(Effect::ReadOnly),
            "the first stays"
        );
        ActionBackend::register(&list, &HITS, Arc::new(|| Ok(())));
        let problems = list.validate().expect_err("duplicates").problems;
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(
            problems
                .iter()
                .all(|p| p.starts_with("DRUMS_HITS: registered more than once"))
        );
    }

    /// (e) `validate` finds a bad id and two actions on one shortcut, and
    /// passes a sound list; every schema it reads back is a JSON object.
    #[test]
    fn validate_reports_what_is_wrong() {
        let sound = ActionList::new();
        hits(&sound);
        unit(&sound, &SAVE);
        unit(&sound, &PLAY);
        sound.validate().expect("sound");

        let list = ActionList::new();
        unit(&list, &SAVE);
        unit(&list, &SAVE_AS);
        unit(&list, &LOWER);
        let problems = list.validate().expect_err("problems").problems;
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("\"drums.hits\"")));
        assert!(problems.iter().any(|p| p.contains("is also PROJECT_SAVE")));
        let shown = list.validate().expect_err("problems").to_string();
        assert!(shown.starts_with("the Action List has 2 problem(s):"));
    }

    /// (e) Every action that takes arguments survives `{}` and refuses
    /// malformed JSON — and one that panics on `{}` is named.
    #[test]
    fn hostile_arguments_are_refused() {
        #[derive(Facet)]
        struct Anything {
            n: Option<u32>,
        }

        let list = ActionList::new();
        hits(&list);
        unit(&list, &PLAY);
        list.probe_hostile_args().expect("sound");
        for junk in [
            &br#"{"track":"a"} trailing"#[..],
            br#"{"track":"a",}"#,
            "[".repeat(200).as_bytes(),
        ] {
            assert!(
                matches!(
                    list.run_json("DRUMS_HITS", junk),
                    Err(ActionError::BadArgs { .. })
                ),
                "{}",
                String::from_utf8_lossy(junk)
            );
        }
        list.run_json("DRUMS_HITS", br#" {"track":"a\u00e9\n","limit":null} "#)
            .expect("well-formed, with an escape and spaces");
        list.register(
            &BOOM,
            Effect::Mutating,
            |a: Anything| -> Result<(), String> { panic!("no n: {:?}", a.n) },
        )
        .expect("registered");
        let problems = list.probe_hostile_args().expect_err("boom").problems;
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems
                .iter()
                .all(|p| p.starts_with("DRUMS_BOOM: panicked on {}"))
        );
    }

    /// A watcher reads what happened after where it left off, and waits
    /// for the next when there is nothing yet.
    static ASK: ActionMeta = meta("PEER_ASK");

    fn explode() -> Result<(), String> {
        panic!("kaboom")
    }
    static SLOW_BOOM: ActionMeta = meta("PEER_BOOM");

    #[derive(Facet)]
    struct Ask {
        name: String,
    }

    /// An async action is decoded at once and awaited by
    /// `run_json_async`; the sync door refuses it rather than block; a
    /// sync action runs through the async door too; the run is an event.
    #[test]
    fn an_async_action_is_awaited() {
        let list = ActionList::new();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        list.register_async(&ASK, Effect::ReadOnly, move |a: Ask| {
            let flag = Arc::clone(&flag);
            async move {
                flag.store(true, Ordering::SeqCst);
                if a.name.is_empty() {
                    Err("nobody".to_owned())
                } else {
                    Ok(format!("hello {}", a.name))
                }
            }
        })
        .expect("registered");
        hits(&list);
        assert!(list.is_async("PEER_ASK"));
        assert!(!list.is_async("DRUMS_HITS"));

        let out = futures::executor::block_on(list.run_json_async("PEER_ASK", br#"{"name":"ada"}"#))
            .expect("ran");
        assert_eq!(out, br#""hello ada""#);
        assert!(ran.load(Ordering::SeqCst));

        let failed = futures::executor::block_on(list.run_json_async("PEER_ASK", br#"{"name":""}"#));
        assert_eq!(
            failed,
            Err(ActionError::Failed {
                id: "PEER_ASK".into(),
                message: "nobody".into()
            })
        );
        let bad = futures::executor::block_on(list.run_json_async("PEER_ASK", br#"{"nom":1}"#));
        assert!(matches!(bad, Err(ActionError::BadArgs { expected: Some(_), .. })), "{bad:?}");

        let refused = list.run_json("PEER_ASK", br#"{"name":"ada"}"#);
        assert!(
            matches!(&refused, Err(ActionError::Failed { message, .. }) if message.contains("run_json_async")),
            "{refused:?}"
        );

        let sync = futures::executor::block_on(list.run_json_async("DRUMS_HITS", br#"{"track":"t","limit":1}"#))
            .expect("a sync action runs through the async door");
        assert_eq!(sync, br#"{"times":[0.5]}"#);

        let (events, _) = list.events_since(0, 100, Duration::ZERO);
        assert!(events.iter().any(|e| e.kind
            == EventKind::Ran {
                id: "PEER_ASK".into(),
                ok: true
            }));
    }

    /// A panic inside an async action's future is a failure; the
    /// authorizer and the enabled check guard the async door as the sync.
    #[test]
    fn the_async_door_is_guarded_like_the_sync_one() {
        let list = ActionList::new();
        list.register_async(&SLOW_BOOM, Effect::Destructive, |(): ()| async { explode() })
        .expect("registered");
        let out = futures::executor::block_on(list.run_json_async("PEER_BOOM", b""));
        assert!(
            matches!(&out, Err(ActionError::Failed { message, .. }) if message.starts_with("panicked: ")),
            "{out:?}"
        );
        list.set_enabled("PEER_BOOM", || Err("not now".into()))
            .expect("known");
        let out = futures::executor::block_on(list.run_json_async("PEER_BOOM", b""));
        assert_eq!(
            out,
            Err(ActionError::Disabled {
                id: "PEER_BOOM".into(),
                reason: "not now".into()
            })
        );
        let unknown = futures::executor::block_on(list.run_json_async("NOPE", b""));
        assert_eq!(unknown, Err(ActionError::Unknown("NOPE".into())));
    }

    /// The probe decodes an async action's arguments but never polls its
    /// future, so nothing it would do happens.
    #[test]
    fn the_probe_never_runs_an_async_action() {
        let list = ActionList::new();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        list.register_async(&ASK, Effect::Mutating, move |_: Ask| {
            let flag = Arc::clone(&flag);
            async move {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }
        })
        .expect("registered");
        list.validate().expect("sound");
        list.probe_hostile_args().expect("refuses garbage");
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn a_watcher_hears_what_happens() {
        let list = ActionList::new();
        let start = list.latest();
        list.changed("tracks", "{A}");
        let (events, next) = list.events_since(start, 10, Duration::ZERO);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0].kind, EventKind::Changed { topic, .. } if topic == "tracks"));
        let (none, same) = list.events_since(next, 10, Duration::from_millis(20));
        assert!(none.is_empty());
        assert_eq!(same, next);

        let later = list.clone();
        let waiter =
            std::thread::spawn(move || later.events_since(next, 10, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(30));
        list.changed("transport", "play");
        let (woken, _) = waiter.join().expect("joined");
        assert_eq!(woken.len(), 1, "woken by the next event, not the timeout");
    }
}
