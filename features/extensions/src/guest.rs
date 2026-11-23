//! A process extension's side (feature `guest`, desk only): what its
//! `main` calls (`docs/content/architecture/extensions.md`, "Processes
//! over vox").
//!
//! ```ignore
//! fn main() -> Result<(), GuestError> {
//!     Guest::new()
//!         .view("counter", Counter) // architect_extensions_dioxus::guest::GuestExt
//!         .action("acme.clicker.reset", |_args| async { Ok(serde_json::json!(0)) })
//!         .serve()
//! }
//! ```
//!
//! [`Guest::serve`] connects to the socket the host named in
//! [`SOCKET`](crate::proto::SOCKET), answers hello with what it adds, and
//! serves until the host goes (so it dies with its parent). Each view the
//! host opens runs on a thread of its own ([`ViewRunner`]; a Dioxus view's
//! `VirtualDom`, `architect_extensions_dioxus::guest`): its edits go to the
//! host as [`Frame`]s, the host's events come back to its handlers. A view whose component panics takes down its
//! thread only; the host sees its stream end.
//!
//! The host's theme reaches it two ways: a view's document has the app's
//! `:root` CSS variables (`var(--brand)`), and the theme comes as data
//! too (wire 3 on): [`Guest::theme`] reads it, [`Guest::on_theme`] is
//! told of every change, and a Dioxus view's `use_theme` gives it as a
//! signal ([`theme_changes`]), so a process that paints itself or works out its own colours
//! (`theme.color("clicker.face")`) follows the app's theme.
//!
//! A view's thread renders one event at a time: a handler that computes
//! for a second freezes its pane for that second (its host marks it
//! busy). Heavy work goes to [`spawn_work`] (`architect_extensions_dioxus::use_work` in a view): a
//! thread of its own, its progress and results back to the view as they
//! come, cancelled when the view closes (`crate::work`). Every event is
//! answered with a frame, empty when nothing changed (wire 5), so the
//! host times each one.
//!
//! What it adds to the app's interface at run time — tabs, status items,
//! openers, menu entries, hooks, slots ([`Contribution`]) — and its
//! runtime state (a status item's text) go to the host as they change:
//! [`contribute`], [`set_state`], [`clear_state`], from anywhere in the
//! process (wire 4 on). An action registered with [`Guest::action_in`]
//! is told where it was invoked from (the palette, a context menu and
//! its target, a hook and its event) as JSON.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use architect::{EventSink, Services, layers};
use architect_theme::Resolved;
use serde_json::Value as Json;
use tokio::sync::mpsc;

use crate::contrib::{Contribution, StateWrite};
use crate::manifest::API;
use crate::proto::{ProcessError, ProcessExtension, ProcessExtensionStreamSource, SOCKET};
use crate::wire::{Frame, Hello, Open, ThemeData, Update, ViewEvent, WIRE};
pub use crate::work::{
    Progress, Work, WorkHandle, WorkObserver, WorkState, WorkStatus, spawn_work,
};

/// What draws one view of the process for its host, on the view's own
/// thread: it reads the host's events from `events` and sends each
/// render's [`Frame`] to `sink`, until the host closes the pane (`events`
/// ends) or stops listening (a send fails). Every event is answered with a
/// frame, empty when nothing changed (wire 5).
pub type ViewRunner =
    Arc<dyn Fn(mpsc::UnboundedReceiver<ViewEvent>, &EventSink<Frame>) + Send + Sync>;

type ActionFuture = Pin<Box<dyn Future<Output = Result<Json, String>> + Send>>;
type Action = Arc<dyn Fn(Json, Json) -> ActionFuture + Send + Sync>;

/// What this process has to tell the host, waiting for it to listen.
struct Outbox {
    pending: Mutex<std::collections::VecDeque<Update>>,
    /// The contribution it last sent: sent again to a host that listens
    /// anew.
    last: Mutex<Option<String>>,
    arrived: tokio::sync::Notify,
}

fn outbox() -> &'static Outbox {
    static OUTBOX: OnceLock<Outbox> = OnceLock::new();
    OUTBOX.get_or_init(|| Outbox {
        pending: Mutex::new(std::collections::VecDeque::new()),
        last: Mutex::new(None),
        arrived: tokio::sync::Notify::new(),
    })
}

fn post(update: Update) {
    let out = outbox();
    out.pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(update);
    out.arrived.notify_one();
}

/// Puts what this process adds to the app's interface now (its tabs,
/// status items, openers, menu and context entries, hooks, slots), in
/// place of what it put before.
///
/// Checked by the host against its manifest, as the manifest's own
/// tables are.
pub fn contribute(contribution: &Contribution) {
    let mut json = serde_json::to_value(contribution).unwrap_or(Json::Null);
    // Its files go with it, under a key a host built before them reads
    // past (`Contribution` takes no `files`).
    let files = files_cell().get().filter(|f| !f.is_empty());
    if let (Some(files), Some(map)) = (files, json.as_object_mut()) {
        map.insert(
            crate::contrib::FILES_KEY.to_owned(),
            serde_json::json!(files),
        );
    }
    let json = json.to_string();
    *outbox().last.lock().unwrap_or_else(PoisonError::into_inner) = Some(json.clone());
    post(Update::Contribute { json });
}

fn files_cell() -> &'static OnceLock<Vec<(String, String)>> {
    static FILES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    &FILES
}

/// Keeps the files a linked extension's code holds, to send to its host.
///
/// `Extension::files`: its toolbars' images, read from its package linked
/// in. They go with what it contributes, so its process copy's host can
/// read them too. Set once, before it serves.
pub fn set_files(files: &[(&str, &str)]) {
    let _ = files_cell().set(
        files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect(),
    );
}

/// Writes one piece of its runtime state (a status item's text: its key
/// the item's id). Its source is always its own.
pub fn set_state(write: &StateWrite) {
    post(Update::State {
        json: serde_json::to_string(write).unwrap_or_default(),
    });
}

/// Clears one key of its runtime state, or all of it.
pub fn clear_state(key: Option<&str>) {
    post(Update::Clear {
        key: key.map(str::to_owned),
    });
}
type ThemeListener = Arc<dyn Fn(&Resolved) + Send + Sync>;

/// The theme the host last sent, for every view of this process.
fn theme_cell() -> &'static tokio::sync::watch::Sender<Option<Resolved>> {
    static THEME: OnceLock<tokio::sync::watch::Sender<Option<Resolved>>> = OnceLock::new();
    THEME.get_or_init(|| tokio::sync::watch::Sender::new(None))
}

/// The host's theme as it changes: `None` until the first arrives (from
/// a host that sends none, never), then every change. What a view's
/// theme hook watches.
#[must_use]
pub fn theme_changes() -> tokio::sync::watch::Receiver<Option<Resolved>> {
    theme_cell().subscribe()
}

/// Why a guest stopped serving.
#[derive(Debug, thiserror::Error)]
pub enum GuestError {
    /// It was not started by a host: no socket in its environment.
    #[error("not started by a host: {SOCKET} is not set")]
    NoSocket,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The host did not finish connecting.
    #[error("connecting to the host: {0}")]
    Connect(String),
}

/// A process extension's views and actions, served to its host.
#[derive(Clone, Default)]
pub struct Guest {
    views: BTreeMap<String, ViewRunner>,
    actions: BTreeMap<String, Action>,
    on_theme: Vec<ThemeListener>,
}

impl std::fmt::Debug for Guest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guest")
            .field("views", &self.views.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
            .field("on_theme", &self.on_theme.len())
            .finish()
    }
}

impl Guest {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Its view `id` (a manifest's `[[views]]`), drawn by `run`: on a
    /// thread of its own for each pane the host opens it in, until the
    /// host closes it. A Dioxus view is drawn by
    /// `architect_extensions_dioxus::guest` (`GuestExt::view`).
    #[must_use]
    pub fn view_with(mut self, id: impl Into<String>, run: ViewRunner) -> Self {
        self.views.insert(id.into(), run);
        self
    }

    /// Its action `id` (a manifest's `[[actions]]` with no preset): JSON
    /// arguments in (`null` for none), JSON out, or why it failed.
    #[must_use]
    pub fn action<F, Fut>(mut self, id: impl Into<String>, run: F) -> Self
    where
        F: Fn(Json) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Json, String>> + Send + 'static,
    {
        let run: Action = Arc::new(move |args, _| Box::pin(run(args)));
        self.actions.insert(id.into(), run);
        self
    }

    /// Its action `id`, told where it was invoked from: JSON arguments and
    /// the invocation (`{source, target?, id?, pane?, view?, url?, path?,
    /// event?, …}`; `{source: "app"}` from a host that says nothing).
    #[must_use]
    pub fn action_in<F, Fut>(mut self, id: impl Into<String>, run: F) -> Self
    where
        F: Fn(Json, Json) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Json, String>> + Send + 'static,
    {
        let run: Action = Arc::new(move |args, cx| Box::pin(run(args, cx)));
        self.actions.insert(id.into(), run);
        self
    }

    /// What it adds to the interface from the start ([`contribute`]).
    #[must_use]
    pub fn contribution(self, contribution: &Contribution) -> Self {
        contribute(contribution);
        self
    }

    /// Calls `listener` with the host's theme each time it arrives: as
    /// the host connects, and on every change (on the thread that serves
    /// the host: hand heavy work on).
    #[must_use]
    pub fn on_theme(mut self, listener: impl Fn(&Resolved) + Send + Sync + 'static) -> Self {
        self.on_theme.push(Arc::new(listener));
        self
    }

    /// The host's theme as it last sent it (`None` before the first, or
    /// from a host that sends none): `color`, `number`, `value`, `scheme`,
    /// `revision` as the host reads them.
    #[must_use]
    pub fn theme() -> Option<Resolved> {
        theme_cell().borrow().clone()
    }

    /// Runs `work` on a thread of its own, off every view's thread
    /// ([`spawn_work`]): its progress and results go to its
    /// [`WorkHandle`]; dropping that cancels it.
    pub fn spawn_work<F>(work: F) -> WorkHandle
    where
        F: FnOnce(&Work) -> Result<Json, String> + Send + 'static,
    {
        spawn_work(work)
    }

    /// Serves the host that started this process, until it goes.
    ///
    /// # Errors
    ///
    /// No host started it, or the connection could not be made.
    pub fn serve(self) -> Result<(), GuestError> {
        let socket = std::env::var_os(SOCKET).ok_or(GuestError::NoSocket)?;
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(self.serve_at(Path::new(&socket)))
    }

    /// Serves the host listening at `socket`, until it goes.
    ///
    /// # Errors
    ///
    /// The connection could not be made.
    pub async fn serve_at(self, socket: &Path) -> Result<(), GuestError> {
        let stream = vox_stream::connect(socket).await?;
        let router = Served {
            guest: Arc::new(self),
            panes: Arc::default(),
        }
        .into_router();
        let connection = vox_core::acceptor_on(vox_stream::StreamLink::unix(stream))
            .on_lane(router.acceptor())
            .establish_connection()
            .await
            .map_err(|e| GuestError::Connect(format!("{e:?}")))?;
        // The host service, the other way (a host from before it serves
        // none: `host` says so).
        if let Ok(client) = connection
            .open_lane::<crate::host_proto::ExtensionHostClient>()
            .await
        {
            *host_slot().lock().unwrap_or_else(PoisonError::into_inner) = Some(client);
        }
        // Live documents, on the same connection (`crate::live`).
        #[cfg(feature = "doc-sync")]
        if let Ok(client) = connection.open_lane::<crdt::sync::DocSyncClient>().await {
            *docs_slot().lock().unwrap_or_else(PoisonError::into_inner) = Some(client);
        }
        connection.closed().await;
        host_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        #[cfg(feature = "doc-sync")]
        docs_slot()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Ok(())
    }
}

fn host_slot() -> &'static Mutex<Option<crate::host_proto::ExtensionHostClient>> {
    static HOST: OnceLock<Mutex<Option<crate::host_proto::ExtensionHostClient>>> = OnceLock::new();
    HOST.get_or_init(|| Mutex::new(None))
}

/// Asks the host service (`crate::hostcall`), from anywhere in the
/// process once it is connected.
///
/// It runs an app action, reads the selection or a setting, keeps data —
/// checked by the host against what this extension may reach.
///
/// # Errors
///
/// The host refused or failed, or there is no host to ask (not connected
/// yet, or a host that serves no host service).
pub async fn host(
    request: crate::hostcall::HostRequest,
) -> Result<Json, crate::hostcall::HostError> {
    let client = host_slot()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| crate::hostcall::HostError::Failed {
            message: "no host service: not connected, or the host serves none".to_owned(),
        })?;
    let text = serde_json::to_string(&request).map_err(|e| crate::hostcall::HostError::Failed {
        message: e.to_string(),
    })?;
    let answer = client
        .call(text)
        .await
        .map_err(architect::vox_into_app::<ProcessError>)
        .map_err(|e| crate::hostcall::HostError::Failed {
            message: format!("the host service: {e}"),
        })?;
    crate::hostcall::HostAnswer::from_json(&answer).into_result()
}

/// The app's actions as this process's views see them.
///
/// `ViewCtx::actions`, the same as a linked view's: the ones it may run,
/// listed by the host (`HostRequest::Actions`) and filled in the first
/// time a view asks, each run through the host (`run`), as the extension.
/// Shared by every view of the process. Ask for it in a view.
#[must_use]
pub fn app_actions() -> architect::action::ActionList {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LIST: OnceLock<architect::action::ActionList> = OnceLock::new();
    static FILLING: AtomicBool = AtomicBool::new(false);
    let list = LIST.get_or_init(architect::action::ActionList::new).clone();
    if !FILLING.swap(true, Ordering::AcqRel) {
        let filled = list.clone();
        // On the asking view's runtime (each view thread runs one).
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(async move {
                    if !fill_actions(&filled).await {
                        // Not connected yet: the next view asks again.
                        FILLING.store(false, Ordering::Release);
                    }
                });
            }
            Err(_) => FILLING.store(false, Ordering::Release),
        }
    }
    list
}

/// Registers what the host lists, each forwarded to it: whether it could.
async fn fill_actions(list: &architect::action::ActionList) -> bool {
    use architect::action::{ActionError, ActionInfo, Effect};
    let Ok(Json::Array(all)) = host(crate::hostcall::HostRequest::Actions).await else {
        return false;
    };
    for a in all {
        let text = |k: &str| {
            a.get(k)
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let schema = |k: &str| a.get(k).and_then(Json::as_str).map(str::to_owned);
        let id = text("id");
        if id.is_empty() || list.info(&id).is_some() {
            continue;
        }
        let info = ActionInfo {
            id: id.clone(),
            display_name: text("display_name"),
            description: text("description"),
            category: text("category"),
            group: text("group"),
            shortcut: text("shortcut"),
            effect: match text("effect").as_str() {
                "read_only" => Effect::ReadOnly,
                "destructive" => Effect::Destructive,
                _ => Effect::Mutating,
            },
            params_schema: schema("params_schema"),
            result_schema: schema("result_schema"),
            enabled: a.get("enabled").and_then(Json::as_bool).unwrap_or(true),
            disabled_reason: None,
            checked: a.get("checked").and_then(Json::as_bool),
        };
        let run_id = id.clone();
        let _ = list.register_forwarded(
            &info,
            Arc::new(move |args: &[u8]| {
                let (id, args) = (
                    run_id.clone(),
                    serde_json::from_slice(args).unwrap_or(Json::Null),
                );
                let run: architect::action::ActionFuture = Box::pin(async move {
                    let request = crate::hostcall::HostRequest::Run {
                        action: id.clone(),
                        args,
                        ticket: None,
                    };
                    let out = host(request).await.map_err(|e| ActionError::Failed {
                        id: id.clone(),
                        message: e.to_string(),
                    })?;
                    serde_json::to_vec(&out).map_err(|e| ActionError::Failed {
                        id,
                        message: e.to_string(),
                    })
                });
                Ok(run)
            }),
        );
    }
    true
}

#[cfg(feature = "doc-sync")]
fn docs_slot() -> &'static Mutex<Option<crdt::sync::DocSyncClient>> {
    static DOCS: OnceLock<Mutex<Option<crdt::sync::DocSyncClient>>> = OnceLock::new();
    DOCS.get_or_init(|| Mutex::new(None))
}

/// The `DocSync` lane to the host, on this process's connection
/// (`crate::live`): what a [`crate::live::DocLane`] syncs over.
///
/// # Errors
///
/// Not connected yet.
#[cfg(feature = "doc-sync")]
pub fn doc_sync() -> Result<crate::live::SyncClient, crate::hostcall::HostError> {
    docs_slot()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .map(crate::live::SyncClient::new)
        .ok_or_else(|| crate::hostcall::HostError::Failed {
            message: "no documents lane: not connected to the host".to_owned(),
        })
}

/// The vault file `path`, a document of a kind this extension declares, as
/// a live document of the node's: a lane to sync a replica of it over
/// (`crate::live`).
///
/// # Errors
///
/// The host refused it (not its kind, no `reads = ["documents"]`, no such
/// document) or has no live documents.
#[cfg(feature = "doc-sync")]
pub async fn document(path: &str) -> Result<crate::live::DocLane, crate::hostcall::HostError> {
    let answer = host(crate::hostcall::HostRequest::LiveDocument {
        path: path.to_owned(),
    })
    .await?;
    crate::live::DocLane::from_answer(answer, doc_sync()?)
}

/// The views open now, by pane: where their events go.
type Panes = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<ViewEvent>>>>;

#[derive(Clone)]
struct Served {
    guest: Arc<Guest>,
    panes: Panes,
}

impl Services for Served {
    fn layers() -> impl architect::Layer<Self> {
        layers![crate::proto::Service, crate::proto::StreamService]
    }
}

impl ProcessExtension for Served {
    async fn hello(&self) -> Result<Hello, ProcessError> {
        Ok(Hello {
            api: API.to_owned(),
            wire: WIRE,
            views: self.guest.views.keys().cloned().collect(),
            actions: self.guest.actions.keys().cloned().collect(),
        })
    }

    async fn run(&self, action: String, args: String) -> Result<String, ProcessError> {
        self.run_in(action, args, String::new()).await
    }

    async fn run_in(
        &self,
        action: String,
        args: String,
        invocation: String,
    ) -> Result<String, ProcessError> {
        let cx = serde_json::from_str(&invocation)
            .unwrap_or_else(|_| serde_json::json!({ "source": "app" }));
        let run = self
            .guest
            .actions
            .get(&action)
            .cloned()
            .ok_or(ProcessError::NoAction(action))?;
        let args = if args.trim().is_empty() {
            Json::Null
        } else {
            serde_json::from_str(&args).map_err(|e| ProcessError::Failed(e.to_string()))?
        };
        run(args, cx)
            .await
            .map(|out| out.to_string())
            .map_err(ProcessError::Failed)
    }

    async fn event(&self, pane: u64, event: ViewEvent) -> Result<(), ProcessError> {
        let to = self
            .panes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&pane)
            .cloned()
            .ok_or(ProcessError::NoPane(pane))?;
        to.send(event).map_err(|_| ProcessError::NoPane(pane))
    }

    async fn theme(&self, theme: ThemeData) -> Result<(), ProcessError> {
        let resolved = theme.to_resolved();
        theme_cell().send_replace(Some(resolved.clone()));
        for listener in &self.guest.on_theme {
            listener(&resolved);
        }
        Ok(())
    }

    async fn close(&self, pane: u64) -> Result<(), ProcessError> {
        // Its sender gone, the view's loop ends.
        self.panes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&pane);
        Ok(())
    }
}

impl ProcessExtensionStreamSource for Served {
    fn updates_attach(&self, _since: u64, sink: EventSink<Update>) {
        tokio::spawn(async move {
            let out = outbox();
            // A host that listens anew hears what is added now first.
            let last = out
                .last
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(json) = last {
                if sink.send(Update::Contribute { json }).await.is_err() {
                    return;
                }
            }
            loop {
                let next = out
                    .pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front();
                match next {
                    Some(update) => {
                        if sink.send(update).await.is_err() {
                            return;
                        }
                    }
                    None => out.arrived.notified().await,
                }
            }
        });
    }

    fn view_attach(&self, open: Open, sink: EventSink<Frame>) {
        self.attach(open, sink, false);
    }

    fn scene_view_attach(&self, open: Open, sink: EventSink<Frame>) {
        self.attach(open, sink, true);
    }
}

impl Served {
    /// Mounts `open`'s view on a thread of its own; `bytes`: its scene
    /// batches cross as bytes beside its edits (`crate::wire::BYTES_WIRE`).
    fn attach(&self, open: Open, sink: EventSink<Frame>, bytes: bool) {
        let root = self.guest.views.get(&open.view).cloned();
        let (to, events) = mpsc::unbounded_channel();
        self.panes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(open.pane, to);
        let panes = Arc::clone(&self.panes);
        let spawned = std::thread::Builder::new()
            .name(format!("view {}", open.view))
            .spawn(move || {
                let span =
                    tracing::info_span!("ext.view", ext.view = %open.view, ext.pane = open.pane);
                let _entered = span.enter();
                if bytes {
                    crate::scene::outbox::enable();
                }
                match root {
                    Some(run) => run(events, &sink),
                    None => refuse(&sink, format!("no view {}", open.view)),
                }
                panes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&open.pane);
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "a view's thread did not start");
        }
    }
}

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
}

fn refuse(sink: &EventSink<Frame>, why: String) {
    if let Some(rt) = runtime() {
        rt.block_on(async {
            let _ = sink.send(Frame::Failed(why)).await;
        });
    }
}
