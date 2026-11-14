//! A WASI component extension's side (`docs/content/architecture/extensions.md`,
//! "WASI components"): what a Rust extension built for `wasm32-wasip2`
//! calls, against the `architect:extension` world
//! (`features/extensions/wit/extension.wit`).
//!
//! The same shape as a process extension's `Guest` (`architect-extensions`'
//! `guest`): views are Dioxus components, actions take and give JSON.
//!
//! ```ignore
//! fn build() -> architect_extension_wasi::Extension {
//!     architect_extension_wasi::Extension::new()
//!         .view("counter", Counter)
//!         .action("ACME_COUNTER_RESET", |_args| Ok(serde_json::json!(0)))
//! }
//!
//! architect_extension_wasi::export!(build);
//! ```
//!
//! Built as a `cdylib` with `cargo build --target wasm32-wasip2`, the
//! `.wasm` is a component the host's `wasi` runner loads. Each open view
//! is a `VirtualDom` of its own; its edits go to the host as the world's
//! `edit`s (the remote-view wire's), its events come back to its
//! handlers. The app is reached through [`host`] — `run`, `setting`,
//! `theme`, `log`, `service`, `call` — each call checked by the host against what
//! the extension may reach.
//!
//! There are no threads in a component, and no async yet (WASI 0.3): an
//! action runs to its end, a view's handlers run as their event comes. An
//! action that changes what a view shows calls [`refresh`]; the host asks
//! for the views' changes after every action and event.
//!
//! A view's timers ([`timers`]: `set_interval`, `set_timeout`, and the
//! hook [`timers::use_interval`]) run on the host's clock (`host::now`):
//! the host calls the component's `tick` when the next is due, each runs
//! inside its view's `VirtualDom` (so it writes its signals), and what they
//! changed goes to the host as the view's edits.
//!
//! ```ignore
//! fn Clock() -> Element {
//!     let mut seconds = use_signal(|| 0);
//!     architect_extension_wasi::timers::use_interval(1000, move || seconds += 1);
//!     rsx! { "{seconds} s" }
//! }
//! ```
//!
//! Heavy work goes to a worker ([`worker`]): another instance of the
//! component on a thread of the host's pool, spoken to by messages only.
//! A handler starts it ([`worker::spawn`]) and returns; the worker's
//! function ([`Extension::worker`]) reports its progress and posts what it
//! finds ([`Work`]); its owner's views redraw from its [`WorkState`] as
//! news comes. Dropping the [`Worker`] handle cancels it.
//!
//! ```ignore
//! Extension::new()
//!     .worker("count-primes", |work: &Work, args| {
//!         for i in 0..n { work.check()?; work.progress(i as f64, n as f64, ""); }
//!         Ok(json!(found))
//!     })
//! ```

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr as _;

use dioxus::core::{BorrowedAttributeValue, ElementId, Event, VirtualDom, WriteMutations};
use dioxus::html::geometry::{ClientPoint, Coordinates, ElementPoint, PagePoint, ScreenPoint};
use dioxus::html::input_data::{MouseButton, decode_mouse_button_set};
use dioxus::html::{
    Code, Key, Location, Modifiers, NamedKey, PlatformEventData, SerializedFocusData,
    SerializedFormData, SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
    set_event_converter,
};
use dioxus::prelude::{Element, ScopeId, Signal, WritableExt as _, try_consume_context};
use serde_json::Value as Json;

#[doc(hidden)]
// Generated: its unsafe ABI glue is wit-bindgen's, not ours to restyle.
#[allow(clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../wit",
        world: "extension",
        pub_export_macro: true,
        export_macro_name: "export_world",
        default_bindings_module: "architect_extension_wasi::bindings",
    });
}

use bindings::architect::extension::host as wit_host;
use bindings::architect::extension::types as wit;

type Action = Box<dyn Fn(Json, Json) -> Result<Json, String>>;
type WorkerFn = Box<dyn Fn(&Work, Json) -> Result<Json, String>>;

/// What a component extension adds: its views and actions, and its state
/// across a reload.
#[derive(Default)]
pub struct Extension {
    views: BTreeMap<String, fn() -> Element>,
    actions: BTreeMap<String, Action>,
    workers: BTreeMap<String, WorkerFn>,
    load: Option<Box<dyn Fn(&[u8])>>,
    save: Option<Box<dyn Fn() -> Vec<u8>>>,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("views", &self.views.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .field("workers", &self.workers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Extension {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Its view `id` (one its manifest declares): `root`, rendered by the
    /// host in a pane.
    #[must_use]
    pub fn view(mut self, id: impl Into<String>, root: fn() -> Element) -> Self {
        self.views.insert(id.into(), root);
        self
    }

    /// Its action `id`: JSON args in, JSON out.
    #[must_use]
    pub fn action(
        mut self,
        id: impl Into<String>,
        run: impl Fn(Json) -> Result<Json, String> + 'static,
    ) -> Self {
        self.actions
            .insert(id.into(), Box::new(move |args, _| run(args)));
        self
    }

    /// Its action `id`, told where it was invoked from (JSON: the palette,
    /// a menu and its target, a hook and its event).
    #[must_use]
    pub fn action_in(
        mut self,
        id: impl Into<String>,
        run: impl Fn(Json, Json) -> Result<Json, String> + 'static,
    ) -> Self {
        self.actions.insert(id.into(), Box::new(run));
        self
    }

    /// Its worker entry `entry` (a `[[workers]]` entry its manifest
    /// declares): what a worker instance runs, with the JSON args its
    /// owner spawned it with. What it returns is its result.
    #[must_use]
    pub fn worker(
        mut self,
        entry: impl Into<String>,
        run: impl Fn(&Work, Json) -> Result<Json, String> + 'static,
    ) -> Self {
        self.workers.insert(entry.into(), Box::new(run));
        self
    }

    /// Its state across a reload: `save` is asked for it before the copy
    /// it was is unloaded, `load` given it as the new one starts.
    #[must_use]
    pub fn state(
        mut self,
        load: impl Fn(&[u8]) + 'static,
        save: impl Fn() -> Vec<u8> + 'static,
    ) -> Self {
        self.load = Some(Box::new(load));
        self.save = Some(Box::new(save));
        self
    }
}

thread_local! {
    static EXTENSION: RefCell<Option<Rc<Extension>>> = const { RefCell::new(None) };
    static PANES: RefCell<BTreeMap<u64, VirtualDom>> = const { RefCell::new(BTreeMap::new()) };
    static REFRESH: Cell<bool> = const { Cell::new(false) };
    /// The pane whose code runs now (a render, an event, a timer): where a
    /// timer it sets runs.
    static CURRENT: Cell<Option<u64>> = const { Cell::new(None) };
    /// Its workers, as their owner hears them.
    static WORK: RefCell<BTreeMap<u64, WorkState>> = const { RefCell::new(BTreeMap::new()) };
}

/// Where a worker is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkStatus {
    /// None started.
    #[default]
    Idle,
    Running,
    Done,
    Failed,
    /// Cancelled before it finished.
    Cancelled,
}

/// How far a worker said it is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub done: f64,
    pub total: f64,
    pub note: String,
}

impl Progress {
    /// `done / total`, in 0..=1 (0 with no total).
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total > 0.0 {
            (self.done / self.total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A worker as its owner sees it: the same as a process's
/// (`architect_extensions::work::WorkState`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkState {
    pub status: WorkStatus,
    /// What it last reported.
    pub progress: Option<Progress>,
    /// What it posted, in order.
    pub messages: Vec<Json>,
    /// What it returned, or why it failed; `None` while it runs.
    pub result: Option<Result<Json, String>>,
}

impl WorkState {
    /// Its progress's fraction, 1 once done, 0 before any.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.status == WorkStatus::Done {
            return 1.0;
        }
        self.progress.as_ref().map_or(0.0, Progress::fraction)
    }

    /// As JSON: `{status, progress: {done, total, note}?, messages, result?, error?}`.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let status = match self.status {
            WorkStatus::Idle => "idle",
            WorkStatus::Running => "running",
            WorkStatus::Done => "done",
            WorkStatus::Failed => "failed",
            WorkStatus::Cancelled => "cancelled",
        };
        let mut out = serde_json::Map::new();
        out.insert("status".to_owned(), status.into());
        out.insert("messages".to_owned(), self.messages.clone().into());
        out.insert("fraction".to_owned(), self.fraction().into());
        if let Some(p) = &self.progress {
            out.insert(
                "progress".to_owned(),
                serde_json::json!({ "done": p.done, "total": p.total, "note": p.note }),
            );
        }
        match &self.result {
            Some(Ok(r)) => {
                out.insert("result".to_owned(), r.clone());
            }
            Some(Err(e)) => {
                out.insert("error".to_owned(), Json::String(e.clone()));
            }
            None => {}
        }
        Json::Object(out)
    }
}

/// A worker's side: report, post, receive, and look whether to stop. The
/// same as a process's work (`architect_extensions::work::Work`).
#[derive(Debug)]
pub struct Work {
    id: u64,
}

impl Work {
    /// Its handle, as its owner has it.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// How far it is: `done` of `total`, with a note (the latest wins;
    /// call it as often as you like).
    #[allow(clippy::unused_self)] // A worker's own call.
    pub fn progress(&self, done: f64, total: f64, note: &str) {
        bindings::architect::extension::worker::progress(done, total, note);
    }

    /// Something for its owner (a partial result): waits while its
    /// owner's queue is full.
    ///
    /// # Errors
    ///
    /// It was cancelled.
    #[allow(clippy::unused_self)]
    pub fn post(&self, message: &Json) -> Result<(), String> {
        bindings::architect::extension::worker::post(0, &message.to_string())
    }

    /// The next message from its owner, waiting at most `wait`.
    #[allow(clippy::unused_self)]
    #[must_use]
    pub fn receive(&self, wait: std::time::Duration) -> Option<Json> {
        let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        bindings::architect::extension::worker::receive(ms)
            .map(|m| serde_json::from_str(&m).unwrap_or(Json::String(m)))
    }

    /// Whether its owner cancelled it or went: stop soon (it is stopped
    /// at its next tick anyway).
    #[allow(clippy::unused_self)]
    #[must_use]
    pub fn cancelled(&self) -> bool {
        bindings::architect::extension::worker::cancelled()
    }

    /// `Err` once it is cancelled: `work.check()?` in a loop.
    ///
    /// # Errors
    ///
    /// It was cancelled.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err("cancelled".to_owned())
        } else {
            Ok(())
        }
    }
}

/// A worker, as its owner holds it. Dropping it cancels it.
#[derive(Debug)]
pub struct Worker {
    id: u64,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        WORK.with(|w| w.borrow_mut().remove(&self.id));
    }
}

impl Worker {
    /// Its handle.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// A message to it (it reads it with [`Work::receive`]).
    ///
    /// # Errors
    ///
    /// Its queue is full (it has not read what it was sent), or it ended.
    pub fn post(&self, message: &Json) -> Result<(), String> {
        bindings::architect::extension::worker::post(self.id, &message.to_string())
    }

    /// Stops it: its state says `cancelled` once the host says it ended.
    pub fn cancel(&self) {
        bindings::architect::extension::worker::cancel(self.id);
    }

    /// Where it is now.
    #[must_use]
    pub fn state(&self) -> WorkState {
        worker::state(self.id)
    }

    /// Whether it is running.
    #[must_use]
    pub fn running(&self) -> bool {
        self.state().status == WorkStatus::Running
    }
}

/// Workers: an owner's side ([`spawn`]) and the state its views read.
pub mod worker {
    use serde_json::Value as Json;

    use super::{WORK, WorkState, WorkStatus, Worker};

    /// Starts its worker `name` (one of its manifest's `[[workers]]`) with
    /// `args`, at once, in an instance of its own on the host's pool.
    ///
    /// # Errors
    ///
    /// It has no such worker, or as many running as it may.
    pub fn spawn(name: &str, args: &Json) -> Result<Worker, String> {
        let id = super::bindings::architect::extension::worker::spawn(name, &args.to_string())?;
        WORK.with(|w| {
            w.borrow_mut().insert(
                id,
                WorkState {
                    status: WorkStatus::Running,
                    ..WorkState::default()
                },
            );
        });
        Ok(Worker { id })
    }

    /// Worker `id`'s state, as its owner heard it (idle for none).
    #[must_use]
    pub fn state(id: u64) -> WorkState {
        WORK.with(|w| w.borrow().get(&id).cloned().unwrap_or_default())
    }
}

/// Worker `id`'s news into its state; every view redraws.
fn heard(id: u64, change: impl FnOnce(&mut WorkState)) {
    let known = WORK.with(|w| w.borrow_mut().get_mut(&id).map(change).is_some());
    if known {
        refresh();
    }
}

/// Every open view renders again (an action changed what they show): the
/// host takes their changes after the action.
pub fn refresh() {
    REFRESH.with(|r| r.set(true));
}

fn extension() -> Result<Rc<Extension>, String> {
    EXTENSION
        .with(|e| e.borrow().clone())
        .ok_or_else(|| "it was not started".to_owned())
}

/// The app, through the host service: every call checked against what
/// the extension may reach.
pub mod host {
    use serde_json::Value as Json;

    use super::wit_host;

    /// Why the host did not do what it was asked: `hostcall::HostError`
    /// (`refused`, `confirmation_required`, `not_loaded`, `not_found`,
    /// `failed`, `unsupported`), as JSON.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct HostError(pub Json);

    impl HostError {
        fn from_text(text: &str) -> Self {
            Self(
                serde_json::from_str(text)
                    .unwrap_or_else(|_| serde_json::json!({ "kind": "failed", "message": text })),
            )
        }

        /// Its kind (`refused`).
        #[must_use]
        pub fn kind(&self) -> &str {
            self.0
                .get("kind")
                .and_then(Json::as_str)
                .unwrap_or("failed")
        }

        /// Outside what the extension may reach (or a person granted).
        #[must_use]
        pub fn is_refused(&self) -> bool {
            self.kind() == "refused"
        }
    }

    impl std::fmt::Display for HostError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let say = |k: &str| self.0.get(k).and_then(Json::as_str);
            match (self.kind(), say("why"), say("message")) {
                ("refused", Some(why), _) => write!(f, "refused: {why}"),
                (_, _, Some(message)) => f.write_str(message),
                (kind, _, _) => f.write_str(kind),
            }
        }
    }

    impl std::error::Error for HostError {}

    fn json(text: &str) -> Json {
        serde_json::from_str(text).unwrap_or(Json::Null)
    }

    /// Runs app action `action` with `args`, as this extension acting for
    /// whoever ran it.
    ///
    /// # Errors
    ///
    /// Refused, or the action failed.
    pub fn run(action: &str, args: &Json) -> Result<Json, HostError> {
        wit_host::run_action(action, &args.to_string())
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    /// One of its own settings' values.
    ///
    /// # Errors
    ///
    /// Not one of its own.
    pub fn setting(key: &str) -> Result<Json, HostError> {
        wit_host::setting(key)
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Sets one of its own settings.
    ///
    /// # Errors
    ///
    /// Not one of its own, or the store refused it.
    pub fn set_setting(key: &str, value: &Json) -> Result<(), HostError> {
        wit_host::set_setting(key, &value.to_string()).map_err(|e| HostError::from_text(&e))
    }

    /// The app's theme now: its id, `light` or `dark`, and every resolved
    /// token's CSS value.
    #[must_use]
    pub fn theme() -> Theme {
        let t = wit_host::theme();
        Theme {
            id: t.id,
            scheme: t.scheme,
            revision: t.revision,
            tokens: t.tokens.into_iter().map(|t| (t.name, t.value)).collect(),
        }
    }

    /// The app's theme, as data.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Theme {
        pub id: String,
        pub scheme: String,
        pub revision: u64,
        pub tokens: Vec<(String, String)>,
    }

    impl Theme {
        /// Token `name`'s value.
        #[must_use]
        pub fn token(&self, name: &str) -> Option<&str> {
            self.tokens
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// How loud a log line is.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Level {
        Debug,
        Info,
        Warn,
        Error,
    }

    /// A line in its log (`EXT_LOGS`). Its stdout and stderr go there too.
    pub fn log(level: Level, text: &str) {
        let level = match level {
            Level::Debug => wit_host::Level::Debug,
            Level::Info => wit_host::Level::Info,
            Level::Warn => wit_host::Level::Warn,
            Level::Error => wit_host::Level::Error,
        };
        wit_host::log(level, text);
    }

    /// Calls method `method` of service `service` (one its manifest
    /// `[requires]`) with `args`: its provider's answer.
    ///
    /// # Errors
    ///
    /// It does not require it, no provider meets its requirement, or the
    /// provider's action refused or failed.
    pub fn service(service: &str, method: &str, args: &Json) -> Result<Json, HostError> {
        wit_host::call_service(service, method, &args.to_string())
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    // ── The app's interface (`capabilities.ui`) ─────────────────────

    /// A notification: `kind` is `info`, `success`, `warning` or `error`.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["notify"]`).
    pub fn notify(text: &str, kind: &str) -> Result<(), HostError> {
        wit_host::notify(text, kind).map_err(|e| HostError::from_text(&e))
    }

    /// The clipboard's text; none when it holds none.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["clipboard"]`), or the app has no clipboard.
    pub fn clipboard_read() -> Result<Option<String>, HostError> {
        wit_host::clipboard_read().map_err(|e| HostError::from_text(&e))
    }

    /// Puts `text` on the clipboard.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["clipboard"]`), or the app has no clipboard.
    pub fn clipboard_write(text: &str) -> Result<(), HostError> {
        wit_host::clipboard_write(text).map_err(|e| HostError::from_text(&e))
    }

    /// A file or folder picker, or a save dialog (`request`: `{kind,
    /// title?, filters?, folder?, name?}`): the path picked (`open_many`:
    /// a list), `null` when the person cancelled. It waits for them.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["pickers"]`), or the app has no pickers.
    pub fn pick(request: &Json) -> Result<Json, HostError> {
        wit_host::pick(&request.to_string())
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    /// A question (`request`: `{kind: text | confirm | pick_one, title,
    /// message?, default?, placeholder?, choices?}`): the answer, `null`
    /// when cancelled. It waits for the person.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["prompts"]`), or the app cannot ask.
    pub fn prompt(request: &Json) -> Result<Json, HostError> {
        wit_host::prompt(&request.to_string())
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    /// A context menu of its own items at a point (`request`: `{items:
    /// [{id, label, icon?, shortcut?, disabled?, separator?}], x?, y?}`):
    /// the id chosen, `null` when dismissed.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["menus"]`), or the app has no menus.
    pub fn menu(request: &Json) -> Result<Json, HostError> {
        wit_host::menu(&request.to_string())
            .map(|t| json(&t))
            .map_err(|e| HostError::from_text(&e))
    }

    /// An icon or badge (`decoration`: `{icon?, badge?, color?,
    /// tooltip?}`) on a file-tree row (`kind` `file`, `id` its path) or a
    /// tab (`tab`); `None` takes it off.
    ///
    /// # Errors
    ///
    /// Refused (`ui = ["badges"]`).
    pub fn decorate(kind: &str, id: &str, decoration: Option<&Json>) -> Result<(), HostError> {
        let decoration = decoration.map(Json::to_string);
        wit_host::decorate(kind, id, decoration.as_deref()).map_err(|e| HostError::from_text(&e))
    }

    /// The time on the host's clock, ms since the epoch: what timers run
    /// on.
    #[must_use]
    pub fn now() -> u64 {
        wit_host::now()
    }

    /// A hook made at run time (`contrib::HookDecl`: `{"on": "doc.saved",
    /// "action": "ACME_LINT"}`, `{"every": "5m", "action": …}`): one of its
    /// actions on an event or a schedule. Its key, for [`unsubscribe`].
    ///
    /// # Errors
    ///
    /// Refused (an `app:` event it may not hear, an action not its own).
    pub fn subscribe(hook: &Json) -> Result<String, HostError> {
        wit_host::subscribe(&hook.to_string()).map_err(|e| HostError::from_text(&e))
    }

    /// Takes the hook it made with key `key` off: whether it had it.
    ///
    /// # Errors
    ///
    /// The host failed.
    pub fn unsubscribe(key: &str) -> Result<bool, HostError> {
        wit_host::unsubscribe(key).map_err(|e| HostError::from_text(&e))
    }

    /// The app's documents under folder `under` (`""`: every one), as
    /// JSON (`[{path, name, space}]`). Needs `reads = ["documents"]`.
    ///
    /// # Errors
    ///
    /// Refused, or the app has no documents for extensions.
    pub fn list_documents(under: &str) -> Result<Json, HostError> {
        wit_host::list_documents(under)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Document `path`'s text. Needs `reads = ["documents"]`.
    ///
    /// # Errors
    ///
    /// Refused, or no document there.
    pub fn read_document(path: &str) -> Result<String, HostError> {
        wit_host::read_document(path).map_err(|e| HostError::from_text(&e))
    }

    /// A new document at `path` holding `text`: its info, as JSON. Needs
    /// `writes = ["documents"]`, as every change does.
    ///
    /// # Errors
    ///
    /// Refused, or one is there already.
    pub fn create_document(path: &str, text: &str) -> Result<Json, HostError> {
        wit_host::create_document(path, text)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// `edits` (`[{"insert": {"at", "text"}}, {"delete": {"at", "len"}},
    /// {"append": {"text"}}, {"replace": {"text"}}]`) as one transaction,
    /// one undo step named `name`: the text after.
    ///
    /// # Errors
    ///
    /// Refused, no document, or an edit outside its text.
    pub fn edit_document(
        path: &str,
        edits: &Json,
        name: Option<&str>,
    ) -> Result<String, HostError> {
        wit_host::edit_document(path, &edits.to_string(), name)
            .map_err(|e| HostError::from_text(&e))
    }

    /// Document `path` at `to`, its history kept: its info, as JSON.
    ///
    /// # Errors
    ///
    /// Refused, no document, or one at `to` already.
    pub fn rename_document(path: &str, to: &str) -> Result<Json, HostError> {
        wit_host::rename_document(path, to)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Document `path` into folder `folder`, its name kept: its info.
    ///
    /// # Errors
    ///
    /// Refused, no document, or one there already.
    pub fn move_document(path: &str, folder: &str) -> Result<Json, HostError> {
        wit_host::move_document(path, folder)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Deletes document `path`.
    ///
    /// # Errors
    ///
    /// Refused, or no document there.
    pub fn delete_document(path: &str) -> Result<(), HostError> {
        wit_host::delete_document(path).map_err(|e| HostError::from_text(&e))
    }

    /// One of its actions on every document event (`{"action": "ACME_REINDEX"}`):
    /// the hooks' keys, as JSON.
    ///
    /// # Errors
    ///
    /// Refused (`reads = ["documents"]`), or not its action.
    pub fn watch_documents(hook: &Json) -> Result<Json, HostError> {
        wit_host::watch_documents(&hook.to_string())
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// The editor being typed in, as JSON.
    ///
    /// `{editor, context, text, selection, primary, cursor, node,
    /// version}`; `node` is the block at
    /// the cursor: `{kind, line_from, line_to, indent, marker?,
    /// content_from, cell?, cells?}`). Needs `reads` or `writes =
    /// ["editor"]`.
    ///
    /// # Errors
    ///
    /// Refused, or no editor has the focus.
    pub fn editor_state() -> Result<Json, HostError> {
        wit_host::editor_state()
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// `command` as one transaction, one undo step: the editor after, as
    /// JSON.
    ///
    /// `{"changes": [{"from", "to", "insert"}], "selection"?: [{"anchor",
    /// "head"}], "name"?, "base"?}`, byte offsets into the text it read.
    /// Needs `writes = ["editor"]`.
    ///
    /// # Errors
    ///
    /// Refused, no editor, or a change that does not fit its text (stale,
    /// out of bounds, overlapping).
    pub fn edit_editor(command: &Json) -> Result<Json, HostError> {
        wit_host::edit_editor(&command.to_string())
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Its decorations in the editor (`[{"kind": "mark", "from", "to",
    /// "class"}, {"kind": "line" | "gutter" | "widget", …}]`), in place of
    /// the ones it set before. Needs `writes = ["editor"]`.
    ///
    /// # Errors
    ///
    /// Refused, no editor, or one that does not fit its text.
    pub fn decorate_editor(decorations: &Json) -> Result<(), HostError> {
        wit_host::decorate_editor(&decorations.to_string()).map_err(|e| HostError::from_text(&e))
    }

    /// An HTTP request through the host.
    ///
    /// `request` is `{"url": …, "method"?, "headers"?, "body"?,
    /// "timeout_ms"?, "max_bytes"?, "stream"?, "binary"?}`; the answer
    /// `{status, url, headers, body}`, or with `stream` `{status, url,
    /// headers, stream}` (read it with [`read_body`]). Only to the hosts
    /// in `capabilities.hosts`, every redirect checked. A
    /// header's value may name one of its `[[secrets]]`
    /// (`"Bearer {{secret:api_key}}"`): the host puts it in.
    ///
    /// # Errors
    ///
    /// Refused (a host, or a redirect to one, it may not reach), a time or
    /// size limit passed, or the network failed.
    pub fn fetch(request: &Json) -> Result<Json, HostError> {
        wit_host::fetch(&request.to_string())
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// The next part of streamed response `stream`'s body, at most `max`
    /// bytes (0: the host's default): `{chunk, done}`.
    ///
    /// # Errors
    ///
    /// No such stream, a time or size limit passed, or the network failed.
    pub fn read_body(stream: &str, max: u32) -> Result<Json, HostError> {
        wit_host::read_body(stream, max)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Stops reading streamed response `stream`: whether it was open.
    #[must_use]
    pub fn close_body(stream: &str) -> bool {
        wit_host::close_body(stream)
    }

    /// Whether the person has set its secret `name` (never its value).
    ///
    /// # Errors
    ///
    /// Not one of its `[[secrets]]`.
    pub fn secret_set(name: &str) -> Result<bool, HostError> {
        wit_host::secret_set(name).map_err(|e| HostError::from_text(&e))
    }

    /// Asks the person for its secret `name`: whether they gave one.
    ///
    /// # Errors
    ///
    /// Not one of its `[[secrets]]`, or the app cannot ask.
    pub fn ask_secret(name: &str) -> Result<bool, HostError> {
        wit_host::ask_secret(name).map_err(|e| HostError::from_text(&e))
    }

    /// The ports the app has, of `kind` (`"midi"`, `"osc"`; `""` every
    /// one): `[{kind, name, direction}]`.
    ///
    /// # Errors
    ///
    /// It may open no port (`capabilities.ports`), or the app has none.
    pub fn ports(kind: &str) -> Result<Json, HostError> {
        wit_host::ports(kind)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Opens a port, its answer `{port, kind, name}`.
    ///
    /// `request` is `{"kind": "midi", "name": …}` or `{"kind": "osc",
    /// "name": "host:port", "listen"?: …}`. What it hears is a `port.message` event to this extension,
    /// or waits for [`port_receive`].
    ///
    /// # Errors
    ///
    /// Refused (`capabilities.ports` does not name it), or it would not
    /// open.
    pub fn open_port(request: &Json) -> Result<Json, HostError> {
        wit_host::open_port(&request.to_string())
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Sends `message` (`{bytes}` or `{type, channel, number, value}` to a
    /// MIDI port, `{address, args}` to an OSC one) out of its port `port`.
    ///
    /// # Errors
    ///
    /// Not its port, not a message of its kind, or the device is gone.
    pub fn port_send(port: &str, message: &Json) -> Result<(), HostError> {
        wit_host::port_send(port, &message.to_string()).map_err(|e| HostError::from_text(&e))
    }

    /// What its port `port` heard that no hook took, at most `max` (0:
    /// all).
    ///
    /// # Errors
    ///
    /// Not its port.
    pub fn port_receive(port: &str, max: u32) -> Result<Json, HostError> {
        wit_host::port_receive(port, max)
            .map(|t| serde_json::from_str(&t).unwrap_or(Json::Null))
            .map_err(|e| HostError::from_text(&e))
    }

    /// Closes its port `port`: whether it was open.
    #[must_use]
    pub fn close_port(port: &str) -> bool {
        wit_host::close_port(port)
    }

    /// Any host-service call (`hostcall::HostRequest`: `{"op": "selection"}`,
    /// `{"op": "read_file", "path": …}`), its answer.
    ///
    /// # Errors
    ///
    /// Refused, or it failed.
    pub fn call(request: &Json) -> Result<Json, HostError> {
        let answer = json(&wit_host::call(&request.to_string()));
        match (answer.get("ok"), answer.get("error")) {
            (Some(ok), _) => Ok(ok.clone()),
            (_, Some(e)) => Err(HostError(e.clone())),
            _ => Err(HostError(serde_json::json!({
                "kind": "failed",
                "message": "the host's answer is not one",
            }))),
        }
    }
}

/// A view's timers, on the host's clock (`host::now`).
///
/// The same as a page's `setTimeout` / `setInterval` (#302). One set while a view
/// renders or handles an event is that view's: it runs inside its
/// `VirtualDom` and goes when the view closes. One set by an action is
/// the component's.
pub mod timers {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use super::{CURRENT, PANES, refresh};
    use crate::bindings::architect::extension::host as wit_host;

    struct Timer {
        pane: Option<u64>,
        at: u64,
        /// 0: once.
        every: u64,
        run: Box<dyn FnMut()>,
    }

    thread_local! {
        static TIMERS: RefCell<BTreeMap<u64, Timer>> = const { RefCell::new(BTreeMap::new()) };
        static NEXT: Cell<u64> = const { Cell::new(1) };
        /// The timer running now, and whether it cleared itself.
        static RUNNING: Cell<Option<u64>> = const { Cell::new(None) };
        static CLEARED: Cell<bool> = const { Cell::new(false) };
    }

    fn add(ms: u32, every: bool, run: Box<dyn FnMut()>) -> u64 {
        let id = NEXT.with(|n| {
            let id = n.get();
            n.set(id.saturating_add(1));
            id
        });
        let wait = u64::from(ms);
        let timer = Timer {
            pane: CURRENT.with(Cell::get),
            at: wit_host::now().saturating_add(wait),
            every: if every { wait.max(1) } else { 0 },
            run,
        };
        TIMERS.with(|t| t.borrow_mut().insert(id, timer));
        id
    }

    /// Runs `run` once, `ms` from now: its id.
    pub fn set_timeout(ms: u32, run: impl FnOnce() + 'static) -> u64 {
        let mut once = Some(run);
        add(
            ms,
            false,
            Box::new(move || {
                if let Some(run) = once.take() {
                    run();
                }
            }),
        )
    }

    /// Runs `run` every `ms` (at least 1), the first `ms` from now: its id.
    pub fn set_interval(ms: u32, run: impl FnMut() + 'static) -> u64 {
        add(ms, true, Box::new(run))
    }

    /// Stops timer `id` (one that ended, or none: nothing).
    pub fn clear(id: u64) {
        if RUNNING.with(Cell::get) == Some(id) {
            CLEARED.with(|c| c.set(true));
        }
        TIMERS.with(|t| t.borrow_mut().remove(&id));
    }

    /// A component's interval for as long as it is mounted: `run` every
    /// `ms`, set on its first render and cleared when it goes.
    pub fn use_interval(ms: u32, run: impl FnMut() + 'static) {
        let id = dioxus::prelude::use_hook(move || set_interval(ms, run));
        dioxus::prelude::use_drop(move || clear(id));
    }

    /// Pane `pane` closed: its timers go.
    pub(super) fn closed(pane: u64) {
        TIMERS.with(|t| t.borrow_mut().retain(|_, timer| timer.pane != Some(pane)));
    }

    /// Runs the timers due now; how many ms until the next (none: none).
    pub(super) fn tick() -> Option<u32> {
        let now = wit_host::now();
        let mut due: Vec<(u64, u64)> = TIMERS.with(|t| {
            t.borrow()
                .iter()
                .filter(|(_, timer)| timer.at <= now)
                .map(|(id, timer)| (timer.at, *id))
                .collect()
        });
        due.sort_unstable();
        for (_, id) in due {
            // Out while it runs: it may set and clear timers.
            let Some(mut timer) = TIMERS.with(|t| t.borrow_mut().remove(&id)) else {
                continue;
            };
            RUNNING.with(|r| r.set(Some(id)));
            CLEARED.with(|c| c.set(false));
            let before = CURRENT.with(|c| c.replace(timer.pane));
            let open = timer.pane.and_then(|pane| {
                PANES
                    .with(|p| p.borrow_mut().remove(&pane))
                    .map(|v| (pane, v))
            });
            if let Some((pane, vdom)) = open {
                vdom.in_runtime(|| (timer.run)());
                PANES.with(|p| p.borrow_mut().insert(pane, vdom));
            } else {
                // A component's own (or its view went): it changes what
                // every view shows, as an action does.
                (timer.run)();
                refresh();
            }
            CURRENT.with(|c| c.set(before));
            RUNNING.with(|r| r.set(None));
            if timer.every > 0 && !CLEARED.with(Cell::get) {
                timer.at = now.saturating_add(timer.every);
                TIMERS.with(|t| t.borrow_mut().insert(id, timer));
            }
        }
        let next = TIMERS.with(|t| t.borrow().values().map(|timer| timer.at).min())?;
        Some(u32::try_from(next.saturating_sub(wit_host::now())).unwrap_or(u32::MAX))
    }
}

/// Builds an extension: what [`export!`] names.
#[doc(hidden)]
pub trait Setup {
    fn build() -> Extension;
}

/// The world's exports, over what `S` builds.
#[doc(hidden)]
pub struct Component<S>(std::marker::PhantomData<S>);

impl<S: Setup> bindings::exports::architect::extension::guest::Guest for Component<S> {
    fn init(state: Option<Vec<u8>>) -> Result<(), String> {
        // Events arrive as data, made into dioxus-html's serialised kind.
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        let ext = Rc::new(S::build());
        if let (Some(state), Some(load)) = (state, &ext.load) {
            load(&state);
        }
        EXTENSION.with(|e| *e.borrow_mut() = Some(ext));
        Ok(())
    }

    fn actions() -> Vec<String> {
        extension()
            .map(|e| e.actions.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn views() -> Vec<String> {
        extension()
            .map(|e| e.views.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn invoke(action: String, args: String, invocation: String) -> Result<String, String> {
        let ext = extension()?;
        let run = ext
            .actions
            .get(&action)
            .ok_or_else(|| format!("no action {action}"))?;
        let parse = |t: &str| serde_json::from_str(t).unwrap_or(Json::Null);
        run(parse(&args), parse(&invocation)).map(|v| v.to_string())
    }

    fn render(pane: u64, view: String) -> Result<Vec<wit::Edit>, String> {
        let ext = extension()?;
        let root = *ext
            .views
            .get(&view)
            .ok_or_else(|| format!("no view {view}"))?;
        let mut vdom = VirtualDom::new(root);
        // An inline embed's input ([`embed`]): none in a pane, the host's
        // `architect-embed` event fills it when it mounts the view as one.
        let input = vdom.in_scope(ScopeId::ROOT, || Signal::new(None::<Embed>));
        vdom.provide_root_context(EmbedSlot(input));
        let mut writer = Writer::default();
        let before = CURRENT.with(|c| c.replace(Some(pane)));
        vdom.rebuild(&mut writer);
        CURRENT.with(|c| c.set(before));
        PANES.with(|p| p.borrow_mut().insert(pane, vdom));
        Ok(writer.edits)
    }

    fn on_event(pane: u64, event: wit::ViewEvent) -> Result<Vec<wit::Edit>, String> {
        // Taken out while its handlers run: one that opens or closes a
        // view finds the others where they are.
        let mut vdom = PANES
            .with(|p| p.borrow_mut().remove(&pane))
            .ok_or_else(|| format!("no view open in pane {pane}"))?;
        let before = CURRENT.with(|c| c.replace(Some(pane)));
        if let Some(input) = embed_input(&event) {
            if let Some(EmbedSlot(mut slot)) =
                vdom.in_scope(ScopeId::ROOT, try_consume_context::<EmbedSlot>)
            {
                vdom.in_runtime(|| slot.set(Some(input)));
            }
        } else {
            dispatch(&vdom, event);
        }
        let edits = settle(&mut vdom);
        CURRENT.with(|c| c.set(before));
        PANES.with(|p| p.borrow_mut().insert(pane, vdom));
        Ok(edits)
    }

    fn pending() -> Vec<wit::PaneEdits> {
        let panes: Vec<u64> = PANES.with(|p| p.borrow().keys().copied().collect());
        let mut out = Vec::new();
        for pane in panes {
            let Some(mut vdom) = PANES.with(|p| p.borrow_mut().remove(&pane)) else {
                continue;
            };
            let before = CURRENT.with(|c| c.replace(Some(pane)));
            let edits = settle(&mut vdom);
            CURRENT.with(|c| c.set(before));
            PANES.with(|p| p.borrow_mut().insert(pane, vdom));
            if !edits.is_empty() {
                out.push(wit::PaneEdits { pane, edits });
            }
        }
        REFRESH.with(|r| r.set(false));
        out
    }

    fn close(pane: u64) {
        // Its hooks' drops (a `use_interval`'s clear) run as it goes.
        let vdom = PANES.with(|p| p.borrow_mut().remove(&pane));
        drop(vdom);
        timers::closed(pane);
    }

    fn save_state() -> Option<Vec<u8>> {
        extension().ok()?.save.as_ref().map(|save| save())
    }

    fn tick() -> Option<u32> {
        timers::tick()
    }
}

impl<S: Setup> bindings::exports::architect::extension::work::Guest for Component<S> {
    fn run(entry: String, worker: u64, args: String) -> Result<String, String> {
        let ext = extension()?;
        let run = ext
            .workers
            .get(&entry)
            .ok_or_else(|| format!("no worker entry {entry}"))?;
        let args = serde_json::from_str(&args).unwrap_or(Json::Null);
        run(&Work { id: worker }, args).map(|v| v.to_string())
    }

    fn on_message(worker: u64, message: String) {
        let message = serde_json::from_str(&message).unwrap_or(Json::String(message));
        heard(worker, |s| s.messages.push(message));
    }

    fn on_progress(worker: u64, progress: bindings::architect::extension::worker::ProgressReport) {
        heard(worker, |s| {
            s.progress = Some(Progress {
                done: progress.done,
                total: progress.total,
                note: progress.note,
            });
        });
    }

    fn on_done(worker: u64, outcome: bindings::exports::architect::extension::work::Outcome) {
        use bindings::exports::architect::extension::work::Outcome;
        heard(worker, |s| match outcome {
            Outcome::Done(r) => {
                s.status = WorkStatus::Done;
                s.result = Some(Ok(serde_json::from_str(&r).unwrap_or(Json::String(r))));
            }
            Outcome::Failed(why) => {
                s.status = WorkStatus::Failed;
                s.result = Some(Err(why));
            }
            Outcome::Cancelled => {
                s.status = WorkStatus::Cancelled;
                s.result = Some(Err("cancelled".to_owned()));
            }
        });
    }
}

/// Its queued work done and rendered: the edits.
fn settle(vdom: &mut VirtualDom) -> Vec<wit::Edit> {
    vdom.process_events();
    // An action or a handler said every view shows something new; the
    // flag is cleared once every view has rendered (`pending`).
    if REFRESH.with(Cell::get) {
        vdom.mark_all_dirty();
    }
    let mut writer = Writer::default();
    vdom.render_immediate(&mut writer);
    writer.edits
}

/// Exports the extension `build` makes as the component's world.
///
/// ```ignore
/// architect_extension_wasi::export!(build);
/// ```
#[macro_export]
macro_rules! export {
    ($build:path) => {
        #[doc(hidden)]
        struct __ArchitectSetup;
        impl $crate::Setup for __ArchitectSetup {
            fn build() -> $crate::Extension {
                $build()
            }
        }
        #[doc(hidden)]
        type __ArchitectComponent = $crate::Component<__ArchitectSetup>;
        $crate::bindings::export_world!(__ArchitectComponent with_types_in $crate::bindings);
    };
}

/// Hands `event` to its element's handlers.
/// The event the host sends a view it mounts as an inline embed, on its
/// root (`architect_extensions::wire::EMBED_EVENT`), its data the input as
/// JSON.
const EMBED_EVENT: &str = "architect-embed";

/// What the host gave a view it mounts as an inline embed: the block it
/// draws ([`embed`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Embed {
    /// What claims it (`fence:kf`).
    pub claim: String,
    /// The block's text: a fence's body, a note's.
    pub source: String,
    /// The host's properties for it (a site's: `info`, `page`); `null`
    /// when none.
    pub props: Json,
}

#[derive(Clone, Copy)]
struct EmbedSlot(Signal<Option<Embed>>);

/// The block this view draws as an inline embed; `None` in a pane.
///
/// The host mounts an `[[embeds]] view` for a claimed fence in a note, on
/// a share link or a Site built by the node. Read in a component, it
/// re-renders when the input comes or changes.
///
/// ```ignore
/// fn Chart() -> Element {
///     let source = embed().map(|e| e.source).unwrap_or_default();
///     rsx! { svg { "data-follow-playhead": "", /* … */ } }
/// }
/// ```
#[must_use]
pub fn embed() -> Option<Embed> {
    try_consume_context::<EmbedSlot>().and_then(|EmbedSlot(slot)| slot())
}

/// The input of `event`, when it is the host's [`EMBED_EVENT`].
fn embed_input(event: &wit::ViewEvent) -> Option<Embed> {
    if event.element != 0 || event.name != EMBED_EVENT {
        return None;
    }
    let wit::EventData::Form(json) = &event.data else {
        return None;
    };
    let value: Json = serde_json::from_str(json).ok()?;
    let text = |k: &str| value.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
    Some(Embed {
        claim: text("claim"),
        source: text("source"),
        props: value.get("props").cloned().unwrap_or(Json::Null),
    })
}

/// A dioxus-html serialised kind, from its serde shape (its fields are
/// private); an event whose shape is wrong carries nothing.
macro_rules! serialised {
    ($kind:ty, $value:expr) => {{
        let data: Rc<dyn std::any::Any> = match serde_json::from_value::<$kind>($value) {
            Ok(data) => Rc::new(PlatformEventData::new(Box::new(data))),
            Err(_) => Rc::new(()),
        };
        data
    }};
}

fn dispatch(vdom: &VirtualDom, event: wit::ViewEvent) {
    let Ok(element) = usize::try_from(event.element) else {
        return;
    };
    let data: Rc<dyn std::any::Any> = match event.data {
        wit::EventData::Mouse(p) => Rc::new(PlatformEventData::new(Box::new(mouse(&p)))),
        wit::EventData::Key(k) => Rc::new(PlatformEventData::new(Box::new(keyboard(&k)))),
        wit::EventData::Form(value) => Rc::new(PlatformEventData::new(Box::new(
            SerializedFormData::new(value, Vec::new()),
        ))),
        wit::EventData::Focus => Rc::new(PlatformEventData::new(Box::new(
            SerializedFocusData::default(),
        ))),
        wit::EventData::Pointer(p) => serialised!(dioxus::html::SerializedPointerData, merged(
            point(&p.at),
            serde_json::json!({
                "pointer_id": p.pointer_id, "width": 1.0, "height": 1.0,
                "pressure": p.pressure, "tangential_pressure": 0.0,
                "tilt_x": p.tilt_x, "tilt_y": p.tilt_y, "twist": p.twist,
                "pointer_type": p.pointer_type, "is_primary": p.is_primary,
            }),
        )),
        wit::EventData::Touch(t) => {
            let all: Vec<Json> = t.touches.iter().map(touch_point).collect();
            let changed: Vec<Json> = t.changed.iter().map(touch_point).collect();
            serialised!(dioxus::html::SerializedTouchData, serde_json::json!({
                "alt_key": t.mods.alt, "ctrl_key": t.mods.ctrl,
                "meta_key": t.mods.meta, "shift_key": t.mods.shift,
                "touches": all.clone(), "changed_touches": changed, "target_touches": all,
            }))
        }
        wit::EventData::Wheel(w) => serialised!(dioxus::html::SerializedWheelData, merged(
            point(&w.at),
            serde_json::json!({
                "delta_mode": w.delta_mode, "delta_x": w.delta_x,
                "delta_y": w.delta_y, "delta_z": 0.0,
            }),
        )),
        wit::EventData::Scroll(s) => Rc::new(PlatformEventData::new(Box::new(
            dioxus::html::SerializedScrollData {
                scroll_top: s.top,
                scroll_left: s.left,
                scroll_width: s.width,
                scroll_height: s.height,
                client_width: s.client_width,
                client_height: s.client_height,
            },
        ))),
        wit::EventData::Composition(text) => serialised!(dioxus::html::SerializedCompositionData, serde_json::json!({ "data": text })),
    };
    vdom.runtime().handle_event(
        &event.name,
        Event::new(data, event.bubbles),
        ElementId::from_raw(element),
    );
}


fn point(p: &wit::Pointer) -> Json {
    serde_json::json!({
        "alt_key": p.mods.alt, "ctrl_key": p.mods.ctrl,
        "meta_key": p.mods.meta, "shift_key": p.mods.shift,
        "button": p.button, "buttons": p.buttons,
        "client_x": p.client_x, "client_y": p.client_y,
        "offset_x": p.element_x, "offset_y": p.element_y,
        "page_x": p.page_x, "page_y": p.page_y,
        "screen_x": p.screen_x, "screen_y": p.screen_y,
    })
}

fn merged(mut base: Json, more: Json) -> Json {
    if let (Some(b), Json::Object(m)) = (base.as_object_mut(), more) {
        b.extend(m);
    }
    base
}

fn touch_point(t: &wit::TouchPoint) -> Json {
    serde_json::json!({
        "identifier": t.id,
        "client_x": t.client_x, "client_y": t.client_y,
        "page_x": t.page_x, "page_y": t.page_y,
        "screen_x": t.screen_x, "screen_y": t.screen_y,
        "force": t.force, "radius_x": 1.0, "radius_y": 1.0, "rotation_angle": 0.0,
    })
}

fn modifiers(m: wit::Mods) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::ALT, m.alt);
    out.set(Modifiers::CONTROL, m.ctrl);
    out.set(Modifiers::META, m.meta);
    out.set(Modifiers::SHIFT, m.shift);
    out
}

fn mouse(p: &wit::Pointer) -> SerializedMouseData {
    SerializedMouseData::new(
        Some(MouseButton::from_web_code(p.button)),
        decode_mouse_button_set(p.buttons),
        Coordinates::new(
            ScreenPoint::new(p.screen_x, p.screen_y),
            ClientPoint::new(p.client_x, p.client_y),
            ElementPoint::new(p.element_x, p.element_y),
            PagePoint::new(p.page_x, p.page_y),
        ),
        modifiers(p.mods),
    )
}

fn keyboard(k: &wit::KeyData) -> SerializedKeyboardData {
    let location = match k.location {
        1 => Location::Left,
        2 => Location::Right,
        3 => Location::Numpad,
        _ => Location::Standard,
    };
    SerializedKeyboardData::new(
        Key::from_str(&k.key).unwrap_or(Key::Named(NamedKey::Unidentified)),
        Code::from_str(&k.code).unwrap_or(Code::Unidentified),
        location,
        k.repeat,
        modifiers(k.mods),
        k.composing,
    )
}

/// A `VirtualDom`'s mutations as the world's edits.
#[derive(Default)]
struct Writer {
    edits: Vec<wit::Edit>,
}

fn id(e: ElementId) -> u64 {
    u64::try_from(e.raw()).unwrap_or(u64::MAX)
}

fn count(m: usize) -> u32 {
    u32::try_from(m).unwrap_or(u32::MAX)
}

impl WriteMutations for Writer {
    fn push_id(&mut self, e: ElementId) {
        self.edits.push(wit::Edit::PushId(id(e)));
    }

    fn set_id(&mut self, e: ElementId) {
        self.edits.push(wit::Edit::SetId(id(e)));
    }

    fn child(&mut self, index: usize) {
        self.edits.push(wit::Edit::Child(count(index)));
    }

    fn pop(&mut self) {
        self.edits.push(wit::Edit::Pop);
    }

    fn create_element(&mut self, tag: &str, ns: Option<&str>) {
        self.edits
            .push(wit::Edit::CreateElement(wit::CreateElement {
                tag: tag.to_owned(),
                namespace: ns.map(str::to_owned),
            }));
    }

    fn create_text(&mut self, value: &str) {
        self.edits.push(wit::Edit::CreateText(value.to_owned()));
    }

    fn clone(&mut self) {
        self.edits.push(wit::Edit::Clone);
    }

    fn append_children(&mut self, m: usize) {
        self.edits.push(wit::Edit::AppendChildren(count(m)));
    }

    fn replace_with(&mut self, m: usize) {
        self.edits.push(wit::Edit::ReplaceWith(count(m)));
    }

    fn insert_after(&mut self, m: usize) {
        self.edits.push(wit::Edit::InsertAfter(count(m)));
    }

    fn insert_before(&mut self, m: usize) {
        self.edits.push(wit::Edit::InsertBefore(count(m)));
    }

    fn set_attribute(&mut self, name: &str, ns: Option<&str>, value: BorrowedAttributeValue<'_>) {
        let value = match value {
            BorrowedAttributeValue::Text(t) => wit::Value::Text(t.to_owned()),
            BorrowedAttributeValue::Float(f) => wit::Value::Float(f),
            BorrowedAttributeValue::Int(i) => wit::Value::Int(i),
            BorrowedAttributeValue::Bool(b) => wit::Value::Bool(b),
            BorrowedAttributeValue::None => wit::Value::None,
            // Values of any other type cannot cross.
            BorrowedAttributeValue::Any(_) => return,
        };
        self.edits.push(wit::Edit::SetAttribute(wit::SetAttribute {
            name: name.to_owned(),
            namespace: ns.map(str::to_owned),
            value,
        }));
    }

    fn set_text(&mut self, value: &str) {
        self.edits.push(wit::Edit::SetText(value.to_owned()));
    }

    fn add_event_listener(&mut self, name: &str) {
        // A mounted element is measured in the host, not here.
        if name != "mounted" {
            self.edits.push(wit::Edit::Listen(name.to_owned()));
        }
    }

    fn remove_event_listener(&mut self, name: &str) {
        if name != "mounted" {
            self.edits.push(wit::Edit::Unlisten(name.to_owned()));
        }
    }

    fn remove(&mut self) {
        self.edits.push(wit::Edit::Remove);
    }
}
