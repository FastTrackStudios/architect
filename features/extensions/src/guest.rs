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

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};

use architect::{EventSink, Services, layers};
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

use crate::manifest::API;
use crate::proto::{ProcessError, ProcessExtension, ProcessExtensionStreamSource, SOCKET};
use crate::wire::{
    Edit, EventData, Frame, Hello, KeyData, Mods, Open, Pointer, Value, ViewEvent, WIRE,
};

type ActionFuture = Pin<Box<dyn Future<Output = Result<Json, String>> + Send>>;
type Action = Arc<dyn Fn(Json) -> ActionFuture + Send + Sync>;

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
}

impl std::fmt::Debug for Guest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guest")
            .field("views", &self.views.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
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
        let run: Action = Arc::new(move |args| Box::pin(run(args)));
        self.actions.insert(id.into(), run);
        self
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
        connection.closed().await;
        Ok(())
    }
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
        run(args)
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
            if let Some(event) = event {
                dispatch(&vdom, event);
            }
            vdom.render_immediate(&mut writer);
            if !writer.is_empty() && sink.send(writer.take()).await.is_err() {
                return;
            }
        }
    });
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
    };
    vdom.runtime().handle_event(
        &event.name,
        Event::new(data, event.bubbles),
        ElementId::from_raw(element),
    );
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
