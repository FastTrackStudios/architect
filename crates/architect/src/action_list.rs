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
//! - **Live updates**: every run, and every [`ActionList::changed`] the
//!   app reports (a project opened, a track renamed), is an
//!   [`ActionEvent`] with a sequence number. [`ActionList::events_since`]
//!   reads them from where a watcher left off, waiting a while for the
//!   next if there are none yet — what a long-poll or an MCP `watch`
//!   tool is built on.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

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
    /// It ran and said no.
    Failed { id: String, message: String },
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "no action {id}"),
            Self::BadArgs { id, message, .. } => write!(f, "{id}: bad arguments: {message}"),
            Self::Failed { id, message } => write!(f, "{id}: {message}"),
        }
    }
}

impl std::error::Error for ActionError {}

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

struct Entry {
    meta: &'static ActionMeta,
    effect: Effect,
    params: Option<&'static Shape>,
    result: Option<&'static Shape>,
    handler: Handler,
}

struct Events {
    next: u64,
    kept: VecDeque<ActionEvent>,
}

struct Inner {
    entries: RwLock<BTreeMap<String, Entry>>,
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
        let count = self.inner.entries.read().map_or(0, |e| e.len());
        f.debug_struct("ActionList")
            .field("actions", &count)
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
                events: Mutex::new(Events {
                    next: 1,
                    kept: VecDeque::new(),
                }),
                arrived: Condvar::new(),
            }),
        }
    }

    /// Add an action that takes `A` and gives back `R` (each a facet
    /// type; `()` for nothing). A second registration of the same id
    /// replaces the first.
    pub fn register<A, R, F>(&self, meta: &'static ActionMeta, effect: Effect, run: F)
    where
        A: Facet<'static>,
        R: Facet<'static>,
        F: Fn(A) -> Result<R, String> + Send + Sync + 'static,
    {
        let id = meta.id;
        let takes = !is_unit(A::SHAPE);
        let handler: Handler = Arc::new(move |args: &[u8]| {
            let decoded: A = if takes {
                facet_json::from_slice::<A>(args).map_err(|e| ActionError::BadArgs {
                    id: id.to_owned(),
                    message: e.to_string(),
                    expected: Some(crate::schema::json_schema(A::SHAPE)),
                })?
            } else {
                facet_json::from_slice::<A>(b"null").map_err(|e| ActionError::BadArgs {
                    id: id.to_owned(),
                    message: e.to_string(),
                    expected: None,
                })?
            };
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
            handler,
        });
    }

    /// Add an action whose handler is already JSON in, JSON out — one
    /// built at run time (an rpc method, a script). `params` and `result`
    /// say what it takes and gives, for its schema.
    pub fn register_dynamic(
        &self,
        meta: &'static ActionMeta,
        effect: Effect,
        params: Option<&'static Shape>,
        result: Option<&'static Shape>,
        handler: Handler,
    ) {
        self.insert(Entry {
            meta,
            effect,
            params,
            result,
            handler,
        });
    }

    fn insert(&self, entry: Entry) {
        let id = entry.meta.id.to_owned();
        if let Ok(mut entries) = self.inner.entries.write() {
            entries.insert(id.clone(), entry);
        }
        self.publish(EventKind::Registered { id });
    }

    /// Every action, by id.
    #[must_use]
    pub fn infos(&self) -> Vec<ActionInfo> {
        self.inner
            .entries
            .read()
            .map(|entries| entries.values().map(info).collect())
            .unwrap_or_default()
    }

    /// One action.
    #[must_use]
    pub fn info(&self, id: &str) -> Option<ActionInfo> {
        self.inner.entries.read().ok()?.get(id).map(info)
    }

    /// How many actions there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.entries.read().map_or(0, |e| e.len())
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run `id` with `args` (JSON; empty for an action that takes
    /// nothing), giving back its result as JSON (`null` for nothing).
    ///
    /// # Errors
    ///
    /// No such action, arguments it cannot take, or it failed.
    pub fn run_json(&self, id: &str, args: &[u8]) -> Result<Vec<u8>, ActionError> {
        let handler = self.inner.entries.read().ok().and_then(|entries| {
            entries
                .get(id)
                .map(|e| (Arc::clone(&e.handler), e.params.is_some()))
        });
        let Some((handler, takes)) = handler else {
            return Err(ActionError::Unknown(id.to_owned()));
        };
        let args: &[u8] = if !takes || args.iter().all(u8::is_ascii_whitespace) {
            if takes { b"{}" } else { b"null" }
        } else {
            args
        };
        // One span per action run: the action's wide event. What it does
        // marks its stages on it (`architect_telemetry::timing`), so where
        // an action's time went is one record, whoever ran it — a key, a
        // menu, a model.
        let span = tracing::info_span!("action", action.id = id, action.ok = tracing::field::Empty);
        let out = span.in_scope(|| handler(args));
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

    /// Say the app's state changed — `topic` what (`tracks`), `detail`
    /// which — so whoever watches hears it.
    pub fn changed(&self, topic: &str, detail: &str) {
        self.publish(EventKind::Changed {
            topic: topic.to_owned(),
            detail: detail.to_owned(),
        });
    }

    fn publish(&self, kind: EventKind) {
        let Ok(mut events) = self.inner.events.lock() else {
            return;
        };
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
/// they are. Marked [`Effect::Mutating`]: an action is a command.
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
        self.insert(Entry {
            meta,
            effect: Effect::Mutating,
            params: None,
            result: None,
            handler: erased,
        });
    }
}

fn info(entry: &Entry) -> ActionInfo {
    let m = entry.meta;
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
    use facet::Facet;
    use std::sync::Arc;
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

    static HITS: ActionMeta = meta("DRUMS_HITS");
    static PLAY: ActionMeta = meta("PLAY");

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
        });
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

    /// A watcher reads what happened after where it left off, and waits
    /// for the next when there is nothing yet.
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
