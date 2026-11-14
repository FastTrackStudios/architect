//! A process extension's side (feature `guest`, desk only): what its
//! `main` calls (`docs/content/architecture/extensions.md`, "Processes
//! over vox").
//!
//! ```ignore
//! fn main() -> Result<(), GuestError> {
//!     Guest::new()
//!         .view("counter", Counter)
//!         .action("acme.clicker.reset", |_args| async { Ok(serde_json::json!(0)) })
//!         .serve()
//! }
//! ```
//!
//! [`Guest::serve`] connects to the socket the host named in
//! [`SOCKET`](crate::proto::SOCKET), answers hello with what it adds, and
//! serves until the host goes (so it dies with its parent). Each view the
//! host opens gets its own `VirtualDom` on a thread of its own: its edits
//! go to the host as [`Frame`]s ([`WireWriter`]), the host's events come
//! back to its handlers. A view whose component panics takes down its
//! thread only; the host sees its stream end.
//!
//! The host's theme reaches it two ways: a view's document has the app's
//! `:root` CSS variables (`var(--brand)`), and the theme comes as data
//! too (wire 3 on): [`Guest::theme`] reads it, [`Guest::on_theme`] is
//! told of every change, and [`use_theme`] gives it to a view as a
//! signal, so a process that paints itself or works out its own colours
//! (`theme.color("clicker.face")`) follows the app's theme.
//!
//! A view's thread renders one event at a time: a handler that computes
//! for a second freezes its pane for that second (its host marks it
//! busy). Heavy work goes to [`spawn_work`] ([`use_work`] in a view): a
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

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use architect::{EventSink, Services, layers};
use architect_theme::Resolved;
use dioxus::core::{BorrowedAttributeValue, ElementId, Event, VirtualDom, WriteMutations};
use dioxus::html::geometry::{ClientPoint, Coordinates, ElementPoint, PagePoint, ScreenPoint};
use dioxus::html::input_data::{MouseButton, decode_mouse_button_set};
use dioxus::html::{
    Code, Key, Location, Modifiers, NamedKey, PlatformEventData, SerializedFocusData,
    SerializedFormData, SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
    set_event_converter,
};
use dioxus::prelude::Element;
use serde_json::Value as Json;
use tokio::sync::mpsc;

use crate::contrib::{Contribution, StateWrite};
use crate::manifest::API;
use crate::proto::{ProcessError, ProcessExtension, ProcessExtensionStreamSource, SOCKET};
use crate::wire::{
    Edit, EventData, Frame, Hello, KeyData, Mods, Open, Pointer, ThemeData, Update, Value,
    ViewEvent, WIRE,
};
pub use crate::work::{
    Progress, UseWork, Work, WorkHandle, WorkState, WorkStatus, spawn_work, use_work,
};

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
    let json = serde_json::to_string(contribution).unwrap_or_default();
    *outbox().last.lock().unwrap_or_else(PoisonError::into_inner) = Some(json.clone());
    post(Update::Contribute { json });
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

/// The host's theme in a view, as a signal: `None` until the first
/// arrives (from a host that sends none, never), then every change.
/// Reading it subscribes the component.
#[must_use]
pub fn use_theme() -> Option<Resolved> {
    use dioxus::prelude::{ReadableExt as _, WritableExt as _, use_future, use_signal};
    let mut theme = use_signal(Guest::theme);
    use_future(move || async move {
        let mut changes = theme_cell().subscribe();
        loop {
            let now = changes.borrow_and_update().clone();
            if *theme.peek() != now {
                theme.set(now);
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    });
    theme.read().clone()
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
    views: BTreeMap<String, fn() -> Element>,
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

    /// Its view `id` (a manifest's `[[views]]`), drawn by `root`.
    #[must_use]
    pub fn view(mut self, id: impl Into<String>, root: fn() -> Element) -> Self {
        self.views.insert(id.into(), root);
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
        connection.closed().await;
        host_slot()
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
        let root = self.guest.views.get(&open.view).copied();
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
                match root {
                    Some(root) => render(root, events, &sink),
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

/// Runs one view: renders it, then its events and its own work, sending
/// each render's edits, until the host closes it or stops listening.
fn render(
    root: fn() -> Element,
    mut events: mpsc::UnboundedReceiver<ViewEvent>,
    sink: &EventSink<Frame>,
) {
    let Some(rt) = runtime() else { return };
    rt.block_on(async move {
        // Events arrive as the wire's data, made into dioxus-html's
        // serialised kind; this process renders nothing else.
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        let mut vdom = VirtualDom::new(root);
        // An inline embed's input (`linked::use_embed`): empty in a pane,
        // filled by the host's `EMBED_EVENT` when it mounts one.
        let embed = vdom.in_scope(dioxus::prelude::ScopeId::ROOT, || {
            dioxus::prelude::Signal::new(crate::contrib::EmbedInput::default())
        });
        vdom.provide_root_context(crate::linked::EmbedContext(embed));
        let mut writer = WireWriter::default();
        vdom.rebuild(&mut writer);
        if sink.send(writer.take()).await.is_err() {
            return;
        }
        loop {
            let event = tokio::select! {
                event = events.recv() => match event {
                    Some(event) => Some(event),
                    None => return,
                },
                () = vdom.wait_for_work() => None,
            };
            // An event is always answered (wire 5): its host times the
            // frame from it, empty or not.
            let answers = event.is_some();
            if let Some(event) = event {
                if let Some(input) = embed_input(&event) {
                    use dioxus::prelude::WritableExt as _;
                    let mut embed = embed;
                    vdom.in_runtime(|| embed.set(input));
                } else {
                    dispatch(&vdom, event);
                }
            }
            vdom.render_immediate(&mut writer);
            if (answers || !writer.is_empty()) && sink.send(writer.take()).await.is_err() {
                return;
            }
        }
    });
}

/// The host's input for an inline embed, when `event` is one
/// ([`crate::wire::EMBED_EVENT`] on the root).
fn embed_input(event: &ViewEvent) -> Option<crate::contrib::EmbedInput> {
    if event.element != 0 || event.name != crate::wire::EMBED_EVENT {
        return None;
    }
    match &event.data {
        EventData::Form(json) => serde_json::from_str(json).ok(),
        _ => None,
    }
}

/// Hands `event` to its element's handlers.
fn dispatch(vdom: &VirtualDom, event: ViewEvent) {
    let Ok(element) = usize::try_from(event.element) else {
        return;
    };
    let data: Rc<dyn Any> = match event.data {
        EventData::Mouse(p) => Rc::new(PlatformEventData::new(Box::new(mouse(&p)))),
        EventData::Key(k) => Rc::new(PlatformEventData::new(Box::new(keyboard(&k)))),
        EventData::Form(value) => Rc::new(PlatformEventData::new(Box::new(
            SerializedFormData::new(value, Vec::new()),
        ))),
        EventData::Focus => Rc::new(PlatformEventData::new(Box::new(
            SerializedFocusData::default(),
        ))),
        EventData::Pointer(p) => Rc::new(PlatformEventData::new(Box::new(rich::pointer(&p)))),
        EventData::Touch(t) => Rc::new(PlatformEventData::new(Box::new(rich::touch(&t)))),
        EventData::Wheel(w) => Rc::new(PlatformEventData::new(Box::new(rich::wheel(&w)))),
        EventData::Scroll(s) => Rc::new(PlatformEventData::new(Box::new(rich::scroll(&s)))),
        EventData::Composition(text) => {
            Rc::new(PlatformEventData::new(Box::new(rich::composition(&text))))
        }
    };
    vdom.runtime().handle_event(
        &event.name,
        Event::new(data, event.bubbles),
        ElementId::from_raw(element),
    );
}

/// The rich input kinds (wire 7) as dioxus-html's serialised data. Their
/// fields are private to dioxus-html, so each is built from its serde
/// shape.
mod rich {
    use dioxus::html::{
        SerializedCompositionData, SerializedPointerData, SerializedScrollData,
        SerializedTouchData, SerializedWheelData,
    };
    use serde_json::{Value as Json, json};

    use crate::wire::{Pointer, PointerData, ScrollData, TouchData, TouchPoint, WheelData};

    fn point(p: &Pointer) -> Json {
        json!({
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

    /// # Panics
    ///
    /// Never: the shape is dioxus-html's own (checked by its tests).
    #[allow(clippy::expect_used)]
    pub(super) fn pointer(p: &PointerData) -> SerializedPointerData {
        let value = merged(
            point(&p.at),
            json!({
                "pointer_id": p.pointer_id, "width": 1.0, "height": 1.0,
                "pressure": p.pressure, "tangential_pressure": 0.0,
                "tilt_x": p.tilt_x, "tilt_y": p.tilt_y, "twist": p.twist,
                "pointer_type": p.pointer_type, "is_primary": p.is_primary,
            }),
        );
        serde_json::from_value(value).expect("dioxus-html's pointer shape")
    }

    fn touch_point(t: &TouchPoint) -> Json {
        json!({
            "identifier": t.id,
            "client_x": t.client_x, "client_y": t.client_y,
            "page_x": t.page_x, "page_y": t.page_y,
            "screen_x": t.screen_x, "screen_y": t.screen_y,
            "force": t.force, "radius_x": 1.0, "radius_y": 1.0, "rotation_angle": 0.0,
        })
    }

    #[allow(clippy::expect_used)]
    pub(super) fn touch(t: &TouchData) -> SerializedTouchData {
        let all: Vec<Json> = t.touches.iter().map(touch_point).collect();
        let changed: Vec<Json> = t.changed.iter().map(touch_point).collect();
        serde_json::from_value(json!({
            "alt_key": t.mods.alt, "ctrl_key": t.mods.ctrl,
            "meta_key": t.mods.meta, "shift_key": t.mods.shift,
            "touches": &all, "changed_touches": changed, "target_touches": &all,
        }))
        .expect("dioxus-html's touch shape")
    }

    #[allow(clippy::expect_used)]
    pub(super) fn wheel(w: &WheelData) -> SerializedWheelData {
        let value = merged(
            point(&w.at),
            json!({
                "delta_mode": w.delta_mode, "delta_x": w.delta_x,
                "delta_y": w.delta_y, "delta_z": 0.0,
            }),
        );
        serde_json::from_value(value).expect("dioxus-html's wheel shape")
    }

    pub(super) const fn scroll(s: &ScrollData) -> SerializedScrollData {
        SerializedScrollData {
            scroll_top: s.top,
            scroll_left: s.left,
            scroll_width: s.width,
            scroll_height: s.height,
            client_width: s.client_width,
            client_height: s.client_height,
        }
    }

    #[allow(clippy::expect_used)]
    pub(super) fn composition(data: &str) -> SerializedCompositionData {
        serde_json::from_value(json!({ "data": data })).expect("dioxus-html's composition shape")
    }

    #[cfg(test)]
    mod tests {
        use dioxus::html::{HasPointerData, HasTouchData, HasWheelData};

        use super::*;

        #[test]
        fn each_rich_kind_builds_from_its_shape() {
            let at = Pointer {
                element_x: 3.0,
                element_y: 4.0,
                buttons: 1,
                ..Pointer::default()
            };
            let p = pointer(&PointerData {
                at,
                pointer_id: 11,
                pointer_type: "touch".to_owned(),
                is_primary: true,
                pressure: 0.5,
                ..PointerData::default()
            });
            assert_eq!(p.pointer_id(), 11);
            assert_eq!(p.pointer_type(), "touch");
            let t = touch(&TouchData {
                touches: vec![TouchPoint { id: 1, ..TouchPoint::default() }],
                changed: vec![TouchPoint { id: 1, ..TouchPoint::default() }],
                ..TouchData::default()
            });
            assert_eq!(t.touches().len(), 1);
            let w = wheel(&WheelData { at, delta_y: 120.0, ..WheelData::default() });
            assert!(matches!(w.delta(), dioxus::html::geometry::WheelDelta::Pixels(v) if (v.y - 120.0).abs() < f64::EPSILON));
            let _ = composition("日本");
        }
    }
}

fn modifiers(m: Mods) -> Modifiers {
    let mut out = Modifiers::empty();
    out.set(Modifiers::ALT, m.alt);
    out.set(Modifiers::CONTROL, m.ctrl);
    out.set(Modifiers::META, m.meta);
    out.set(Modifiers::SHIFT, m.shift);
    out
}

fn mouse(p: &Pointer) -> SerializedMouseData {
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

fn keyboard(k: &KeyData) -> SerializedKeyboardData {
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

/// Writes a `VirtualDom`'s mutations as the wire's [`Edit`]s.
#[derive(Debug, Default)]
pub struct WireWriter {
    edits: Vec<Edit>,
}

impl WireWriter {
    /// Nothing written since the last [`take`](Self::take).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// What was written since the last take, as one frame.
    pub fn take(&mut self) -> Frame {
        Frame::Edits {
            edits: std::mem::take(&mut self.edits),
        }
    }
}

fn wire_id(id: ElementId) -> u64 {
    u64::try_from(id.raw()).unwrap_or(u64::MAX)
}

fn count(m: usize) -> u32 {
    u32::try_from(m).unwrap_or(u32::MAX)
}

impl WriteMutations for WireWriter {
    fn push_id(&mut self, id: ElementId) {
        self.edits.push(Edit::PushId { id: wire_id(id) });
    }

    fn set_id(&mut self, id: ElementId) {
        self.edits.push(Edit::SetId { id: wire_id(id) });
    }

    fn child(&mut self, index: usize) {
        self.edits.push(Edit::Child {
            index: count(index),
        });
    }

    fn pop(&mut self) {
        self.edits.push(Edit::Pop);
    }

    fn create_element(&mut self, tag: &str, ns: Option<&str>) {
        self.edits.push(Edit::CreateElement {
            tag: tag.to_owned(),
            namespace: ns.map(str::to_owned),
        });
    }

    fn create_text(&mut self, value: &str) {
        self.edits.push(Edit::CreateText {
            value: value.to_owned(),
        });
    }

    fn clone(&mut self) {
        self.edits.push(Edit::Clone);
    }

    fn append_children(&mut self, m: usize) {
        self.edits.push(Edit::AppendChildren { m: count(m) });
    }

    fn replace_with(&mut self, m: usize) {
        self.edits.push(Edit::ReplaceWith { m: count(m) });
    }

    fn insert_after(&mut self, m: usize) {
        self.edits.push(Edit::InsertAfter { m: count(m) });
    }

    fn insert_before(&mut self, m: usize) {
        self.edits.push(Edit::InsertBefore { m: count(m) });
    }

    fn set_attribute(&mut self, name: &str, ns: Option<&str>, value: BorrowedAttributeValue<'_>) {
        let value = match value {
            BorrowedAttributeValue::Text(t) => Value::Text(t.to_owned()),
            BorrowedAttributeValue::Float(f) => Value::Float(f),
            BorrowedAttributeValue::Int(i) => Value::Int(i),
            BorrowedAttributeValue::Bool(b) => Value::Bool(b),
            BorrowedAttributeValue::None => Value::None,
            // Values of any other type cannot cross.
            BorrowedAttributeValue::Any(_) => return,
        };
        self.edits.push(Edit::SetAttribute {
            name: name.to_owned(),
            namespace: ns.map(str::to_owned),
            value,
        });
    }

    fn set_text(&mut self, value: &str) {
        self.edits.push(Edit::SetText {
            value: value.to_owned(),
        });
    }

    fn add_event_listener(&mut self, name: &str) {
        // A mounted element is measured in the host, not here.
        if name == "mounted" {
            return;
        }
        self.edits.push(Edit::Listen {
            name: name.to_owned(),
        });
    }

    fn remove_event_listener(&mut self, name: &str) {
        if name == "mounted" {
            return;
        }
        self.edits.push(Edit::Unlisten {
            name: name.to_owned(),
        });
    }

    fn remove(&mut self) {
        self.edits.push(Edit::Remove);
    }
}
