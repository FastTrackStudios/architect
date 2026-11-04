//! A process extension's view in a pane (feature `remote`, desk only;
//! `docs/content/architecture/extensions.md`, "Views: Dioxus, rendered
//! remotely").
//!
//! [`RemoteView`] mounts a Blitz sub-document in its element, sized by
//! the pane it is in, and fills it with what the extension renders: a
//! [`RemoteDocument`] applies the extension's [`Frame`]s and turns the
//! pointer, keys and focus Blitz gives it into [`ViewEvent`]s for the
//! extension's handlers. When the process dies (or the view fails, or the
//! extension is unloaded) the pane shows a placeholder saying why, with a
//! button to start it again; the app runs on. When the process comes back
//! (the supervisor restarts it, or a reload loads a new one) the view
//! mounts again by itself.
//!
//! Each frame is timed from the event that asked for it (a process at
//! wire 5 answers every event, [`crate::wire::ANSWERED_WIRE`]) into the
//! view's `architect_workspace::ViewMeter` (`<extension>/<view>` in the
//! `Meters` in context; `EXT_VIEWS` reads it). While an event goes
//! unanswered past the budget ([`REMOTE_BUDGET`]) — the process's view
//! thread is busy with a long handler — the pane keeps its last frame
//! with a busy mark (`architect_shell::BusyBadge`); the app runs on.
//!
//! The applier trusts nothing: an edit naming a node it does not have, a
//! child that is not there, a move that would make a cycle, or a frame
//! too large fails the view (the placeholder says so) instead of the app.
//! Not yet: pointer, touch, wheel and scroll events (Blitz scrolls the
//! sub-document itself), IME, `mounted` measurements, the shell's
//! components as named elements, the allowlist of elements and CSS.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use architect_workspace::{Meters, ViewMeter};

use blitz_dom::{
    BaseDocument, DocGuard, DocGuardMut, Document, DocumentConfig, EventDriver, EventHandler,
    LocalName, Namespace, Node, NodeId, QualName, UserAgentStylesheet,
};
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, DomEvent, DomEventData, EventState, Location, Modifiers,
    MouseEventButton, UiEvent,
};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};

use crate::process::{Process, Processes, State};
use crate::wire::{
    ANSWERED_WIRE, Edit, EventData, Frame, KeyData, Mods, Pointer, Value, ViewEvent,
};

/// A process view's frame budget, from the event to its frame: two
/// frames at 60 Hz, the round trip over the socket included.
pub const REMOTE_BUDGET: Duration = Duration::from_millis(33);

/// At most this many edits in one frame.
const MAX_EDITS: usize = 200_000;

/// The attribute that names a node's Dioxus element id, as Dioxus's own
/// renderer marks it.
const DIOXUS_ID: &str = "data-dioxus-id";

/// A view's document in the host: Blitz's, filled by the extension's
/// edits, its events sent back.
pub struct RemoteDocument {
    doc: BaseDocument,
    /// Where the view's nodes go (Dioxus's element 0).
    root: NodeId,
    /// The edits' stack: each node, and the element id it was pushed or
    /// registered as. The root stays at the bottom.
    stack: Vec<(NodeId, Option<u64>)>,
    /// Dioxus element id → node, and back.
    ids: HashMap<u64, NodeId>,
    elements: HashMap<NodeId, u64>,
    /// How many handlers there are for each event.
    listeners: HashMap<String, u32>,
    events: mpsc::UnboundedSender<ViewEvent>,
    /// The host's theme, as a `<style>` in the head: its text node.
    theme: NodeId,
}

impl std::fmt::Debug for RemoteDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteDocument")
            .field("root", &self.root)
            .field("stack", &self.stack.len())
            .field("listeners", &self.listeners)
            .finish_non_exhaustive()
    }
}

fn qual(name: &str, namespace: Option<&str>) -> QualName {
    QualName::new(
        None,
        namespace.map_or(blitz_dom::ns!(html), Namespace::from),
        LocalName::from(name),
    )
}

impl RemoteDocument {
    /// An empty view, sending its events to `events`.
    #[must_use]
    pub fn new(events: mpsc::UnboundedSender<ViewEvent>) -> Self {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        doc.add_user_agent_stylesheet(UserAgentStylesheet::default_sheet());
        let mut m = doc.mutate();
        let html = m.create_element(qual("html", None), Vec::new());
        let page = m.doc.root_node().id;
        m.append_children(page, &[html]);
        let head = m.create_element(qual("head", None), Vec::new());
        let body = m.create_element(qual("body", None), Vec::new());
        m.append_children(html, &[head, body]);
        let root = m.create_element(qual("main", None), Vec::new());
        m.append_children(body, &[root]);
        // The host's theme (`set_theme_css`): the view's own sheets come
        // after it, so a view still wins where it sets a colour itself.
        let style = m.create_element(qual("style", None), Vec::new());
        let theme = m.create_text_node("");
        m.append_children(style, &[theme]);
        m.append_children(head, &[style]);
        drop(m);
        Self {
            doc,
            root,
            stack: vec![(root, Some(0))],
            ids: HashMap::from([(0, root)]),
            elements: HashMap::from([(root, 0)]),
            listeners: HashMap::new(),
            events,
            theme,
        }
    }

    /// Sets the host's theme for the view: CSS (the host's `:root`
    /// custom properties, `architect_theme::RootCss`), so the view's
    /// `var(--…)` follow the app's theme live. A sub-document doesn't see
    /// the host's own sheets.
    pub fn set_theme_css(&mut self, css: &str) {
        self.doc.mutate().set_node_text(self.theme, css);
    }

    /// Its Blitz document, to read.
    #[must_use]
    pub const fn document(&self) -> &BaseDocument {
        &self.doc
    }

    /// Applies one frame.
    ///
    /// # Errors
    ///
    /// The view failed, or the frame is not one this document can apply
    /// (it is left as far as it got; the view should be dropped).
    pub fn apply(&mut self, frame: Frame) -> Result<(), String> {
        let edits = match frame {
            Frame::Edits { edits } => edits,
            Frame::Failed(why) => return Err(why),
        };
        if edits.len() > MAX_EDITS {
            return Err(format!("a frame of {} edits is too large", edits.len()));
        }
        for edit in edits {
            self.edit(edit)?;
        }
        Ok(())
    }

    fn node(&self, id: u64) -> Result<NodeId, String> {
        self.ids
            .get(&id)
            .copied()
            .filter(|n| self.doc.get_node(*n).is_some())
            .ok_or_else(|| format!("no element {id}"))
    }

    /// The top of the stack.
    fn top(&self) -> Result<(NodeId, Option<u64>), String> {
        self.stack
            .last()
            .copied()
            .filter(|(n, _)| self.doc.get_node(*n).is_some())
            .ok_or_else(|| "an edit on an empty stack".to_owned())
    }

    /// Takes the top entry off the stack; never the root.
    fn pop_one(&mut self) -> Result<(NodeId, Option<u64>), String> {
        if self.stack.len() <= 1 {
            return Err("an edit took more nodes than there are".to_owned());
        }
        self.stack
            .pop()
            .ok_or_else(|| "an edit took more nodes than there are".to_owned())
    }

    /// Takes the top `m` nodes off the stack, in stack order; never the
    /// root, and always with one node left below them.
    fn pop(&mut self, m: u32) -> Result<Vec<NodeId>, String> {
        let m = usize::try_from(m).map_err(|e| e.to_string())?;
        let keep = self
            .stack
            .len()
            .checked_sub(m)
            .filter(|k| *k >= 1)
            .ok_or("an edit took more nodes than there are")?;
        Ok(self
            .stack
            .split_off(keep)
            .into_iter()
            .map(|(n, _)| n)
            .collect())
    }

    /// Replaces the top of the stack with `node`, which has no id yet.
    fn set_top(&mut self, node: NodeId) -> Result<(), String> {
        if self.stack.len() <= 1 {
            return Err("an edit reached outside the view".to_owned());
        }
        if let Some(top) = self.stack.last_mut() {
            *top = (node, None);
        }
        Ok(())
    }

    /// Whether `node` is `inside` or below it.
    fn within(&self, node: NodeId, inside: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(n) = at {
            if n == inside {
                return true;
            }
            at = self.doc.get_node(n).and_then(|n| n.parent);
        }
        false
    }

    /// `kids` may go under `parent`: none of them is it or above it, and
    /// none is (or holds) the root.
    fn placeable(&self, parent: NodeId, kids: &[NodeId]) -> Result<(), String> {
        if kids
            .iter()
            .any(|&k| self.within(parent, k) || self.within(self.root, k))
        {
            return Err("an edit would put a node inside itself".to_owned());
        }
        Ok(())
    }

    /// `anchor`'s parent, when it may be replaced or moved beside: not the
    /// root (whose parent is the host's), and in a tree (Dioxus builds
    /// subtrees apart, then puts them in).
    fn parent_of(&self, anchor: NodeId) -> Result<NodeId, String> {
        self.doc
            .get_node(anchor)
            .and_then(|n| n.parent)
            .filter(|_| anchor != self.root)
            .ok_or_else(|| "an edit reached outside the view".to_owned())
    }

    fn map(&mut self, node: NodeId, id: u64) {
        if let Some(old) = self.ids.insert(id, node) {
            self.elements.remove(&old);
        }
        if let Some(old) = self.elements.insert(node, id)
            && old != id
        {
            self.ids.remove(&old);
        }
    }

    /// Forgets the ids of `node` and everything below it.
    fn forget(&mut self, node: NodeId) {
        let mut pending = vec![node];
        while let Some(n) = pending.pop() {
            if let Some(node) = self.doc.get_node(n) {
                pending.extend(node.children.iter().copied());
            }
            if let Some(id) = self.elements.remove(&n) {
                self.ids.remove(&id);
            }
        }
    }

    /// Takes `node` (not the root) out of the document, its ids with it.
    fn drop_node(&mut self, node: NodeId) -> Result<(), String> {
        if self.within(self.root, node) {
            return Err("an edit reached outside the view".to_owned());
        }
        self.forget(node);
        self.doc.mutate().remove_and_drop_node(node);
        Ok(())
    }

    fn edit(&mut self, edit: Edit) -> Result<(), String> {
        match edit {
            Edit::PushId { id } => {
                let node = self.node(id)?;
                self.stack.push((node, Some(id)));
            }
            Edit::SetId { id } => {
                let (node, _) = self.top()?;
                if node == self.root && id != 0 {
                    return Err("an edit reached outside the view".to_owned());
                }
                self.map(node, id);
                if let Some(top) = self.stack.last_mut() {
                    top.1 = Some(id);
                }
            }
            Edit::Child { index } => {
                let (node, _) = self.top()?;
                let child = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.doc.get_node(node)?.children.get(i).copied())
                    .ok_or("a child that is not there")?;
                self.set_top(child)?;
            }
            Edit::Pop => {
                self.pop_one()?;
            }
            Edit::CreateElement { tag, namespace } => {
                let node = self
                    .doc
                    .mutate()
                    .create_element(qual(&tag, namespace.as_deref()), Vec::new());
                self.stack.push((node, None));
            }
            Edit::CreateText { value } => {
                let node = self.doc.mutate().create_text_node(&value);
                self.stack.push((node, None));
            }
            Edit::Clone => {
                let (node, _) = self.top()?;
                if self.within(self.root, node) {
                    return Err("an edit reached outside the view".to_owned());
                }
                let copy = self.doc.mutate().deep_clone_node(node);
                self.set_top(copy)?;
            }
            Edit::AppendChildren { m } => {
                let kids = self.pop(m)?;
                let (parent, _) = self.top()?;
                self.placeable(parent, &kids)?;
                self.doc.mutate().append_children(parent, &kids);
            }
            Edit::ReplaceWith { m } => {
                let kids = self.pop(m)?;
                let (anchor, _) = self.pop_one()?;
                let parent = self.parent_of(anchor)?;
                self.placeable(parent, &kids)?;
                self.doc.mutate().insert_nodes_before(anchor, &kids);
                self.drop_node(anchor)?;
            }
            Edit::InsertAfter { m } | Edit::InsertBefore { m } => {
                let after = matches!(edit, Edit::InsertAfter { .. });
                let kids = self.pop(m)?;
                let (anchor, _) = self.top()?;
                let parent = self.parent_of(anchor)?;
                self.placeable(parent, &kids)?;
                let mut doc = self.doc.mutate();
                if after {
                    doc.insert_nodes_after(anchor, &kids);
                } else {
                    doc.insert_nodes_before(anchor, &kids);
                }
            }
            Edit::SetAttribute {
                name,
                namespace,
                value,
            } => {
                let (node, _) = self.top()?;
                let value = match value {
                    Value::None => None,
                    Value::Bool(false) if name == "checked" => None,
                    Value::Text(t) => Some(t),
                    Value::Float(f) => Some(f.to_string()),
                    Value::Int(i) => Some(i.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                };
                let mut m = self.doc.mutate();
                set_attr(&mut m, node, &name, namespace.as_deref(), value.as_deref())?;
            }
            Edit::SetText { value } => {
                let (node, _) = self.top()?;
                self.doc.mutate().set_node_text(node, &value);
            }
            Edit::Listen { name } => {
                let (node, id) = self.top()?;
                let id = id.ok_or("a handler on a node with no element id")?;
                let mut m = self.doc.mutate();
                set_attr(&mut m, node, DIOXUS_ID, None, Some(&id.to_string()))?;
                drop(m);
                let n = self.listeners.entry(name).or_default();
                *n = n.saturating_add(1);
            }
            Edit::Unlisten { name } => {
                if let Some(n) = self.listeners.get_mut(&name) {
                    *n = n.saturating_sub(1);
                }
            }
            Edit::Remove => {
                let (node, _) = self.pop_one()?;
                self.drop_node(node)?;
            }
        }
        Ok(())
    }
}

/// Sets (or, given no value, clears) an attribute, as Dioxus's own
/// renderer does: the `style` namespace is one CSS property.
fn set_attr(
    m: &mut blitz_dom::DocumentMutator<'_>,
    node: NodeId,
    name: &str,
    namespace: Option<&str>,
    value: Option<&str>,
) -> Result<(), String> {
    if m.doc.get_node(node).and_then(Node::element_data).is_none() {
        return Ok(());
    }
    if name == "dangerous_inner_html" {
        return Err("a view may not set raw HTML".to_owned());
    }
    if namespace == Some("style") {
        match value {
            Some(v) => m.set_style_property(node, name, v),
            None => m.remove_style_property(node, name),
        }
        return Ok(());
    }
    let name = qual(name, namespace);
    match value {
        Some(v) => m.set_attribute(node, name, v),
        None => m.clear_attribute(node, name),
    }
    Ok(())
}

impl Document for RemoteDocument {
    fn inner(&self) -> DocGuard<'_> {
        DocGuard::Ref(&self.doc)
    }

    fn inner_mut(&mut self) -> DocGuardMut<'_> {
        DocGuardMut::Ref(&mut self.doc)
    }

    fn handle_ui_event(&mut self, event: UiEvent) {
        let forward = Forward {
            listeners: &self.listeners,
            events: &self.events,
        };
        EventDriver::new(&mut self.doc, forward).handle_ui_event(event);
    }
}

/// Sends the events the view has handlers for to the extension.
struct Forward<'a> {
    listeners: &'a HashMap<String, u32>,
    events: &'a mpsc::UnboundedSender<ViewEvent>,
}

impl EventHandler for Forward<'_> {
    fn handle_event(
        &mut self,
        chain: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        _: &mut EventState,
    ) {
        let name = event.name();
        if self.listeners.get(name).copied().unwrap_or(0) == 0 {
            return;
        }
        let Some(data) = event_data(&event.data) else {
            return;
        };
        let doc = doc.inner();
        let Some(element) = chain
            .iter()
            .find_map(|n| doc.get_node(*n).and_then(dioxus_id))
        else {
            return;
        };
        // The extension bubbles it through its own tree.
        let _ = self.events.unbounded_send(ViewEvent {
            element,
            name: name.to_owned(),
            bubbles: event.bubbles,
            data,
        });
    }
}

fn dioxus_id(node: &Node) -> Option<u64> {
    node.attr(LocalName::from(DIOXUS_ID))
        .and_then(|v| v.parse().ok())
}

const fn mods(m: Modifiers) -> Mods {
    Mods {
        alt: m.contains(Modifiers::ALT),
        ctrl: m.contains(Modifiers::CONTROL),
        meta: m.contains(Modifiers::META),
        shift: m.contains(Modifiers::SHIFT),
    }
}

fn pointer(p: &BlitzPointerEvent) -> Pointer {
    Pointer {
        client_x: p.coords.client_x.into(),
        client_y: p.coords.client_y.into(),
        page_x: p.coords.page_x.into(),
        page_y: p.coords.page_y.into(),
        screen_x: p.coords.screen_x.into(),
        screen_y: p.coords.screen_y.into(),
        element_x: p.element.x.into(),
        element_y: p.element.y.into(),
        button: match p.button {
            MouseEventButton::Main => 0,
            MouseEventButton::Auxiliary => 1,
            MouseEventButton::Secondary => 2,
            MouseEventButton::Fourth => 3,
            MouseEventButton::Fifth => 4,
        },
        buttons: p.buttons.bits().into(),
        mods: mods(p.mods),
    }
}

fn key(k: &BlitzKeyEvent) -> KeyData {
    KeyData {
        key: k.key.to_string(),
        code: k.code.to_string(),
        location: match k.location {
            Location::Left => 1,
            Location::Right => 2,
            Location::Numpad => 3,
            Location::Standard => 0,
        },
        repeat: k.is_auto_repeating,
        composing: k.is_composing,
        mods: mods(k.modifiers),
    }
}

fn event_data(data: &DomEventData) -> Option<EventData> {
    match data {
        DomEventData::Click(p)
        | DomEventData::ContextMenu(p)
        | DomEventData::DoubleClick(p)
        | DomEventData::MouseMove(p)
        | DomEventData::MouseDown(p)
        | DomEventData::MouseUp(p)
        | DomEventData::MouseEnter(p)
        | DomEventData::MouseLeave(p)
        | DomEventData::MouseOver(p)
        | DomEventData::MouseOut(p) => Some(EventData::Mouse(pointer(p))),
        DomEventData::KeyPress(k) | DomEventData::KeyDown(k) | DomEventData::KeyUp(k) => {
            Some(EventData::Key(key(k)))
        }
        DomEventData::Input(i) => Some(EventData::Form(i.value.clone())),
        DomEventData::Focus(_)
        | DomEventData::Blur(_)
        | DomEventData::FocusIn(_)
        | DomEventData::FocusOut(_) => Some(EventData::Focus),
        _ => None,
    }
}

/// What the pane shows besides the view.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Shown {
    /// Nothing to show yet, and why.
    Waiting(String),
    /// The view.
    Live,
    /// It stopped, and why: the placeholder, with a button.
    Stopped(String),
}

/// Runs `f` on the host's document, when it is free.
async fn with_doc<R>(node: &NodeHandle, f: impl FnOnce(&mut BaseDocument, NodeId) -> R) -> R {
    loop {
        // A task runs while nothing else holds the document; this checks
        // anyway, and waits a frame if something does.
        if node.try_doc().is_some() {
            let mut doc = node.doc_mut();
            return f(&mut doc, node.node_id());
        }
        futures_timer::Delay::new(Duration::from_millis(8)).await;
    }
}

/// The view's document in `node`, to apply a frame to.
fn remote(doc: &mut BaseDocument, node: NodeId) -> Option<&mut RemoteDocument> {
    let sub: &mut dyn Any = doc.subdoc_mut(node)?;
    sub.downcast_mut::<RemoteDocument>()
}

/// Waits for `process` to move, the loaded processes to change, or the
/// placeholder's button.
async fn something_changes(
    process: Option<&Process>,
    processes: &Processes,
    retry: &mut mpsc::UnboundedReceiver<()>,
) {
    let mut watch = process.map(Process::watch);
    let moved = async {
        match watch.as_mut() {
            Some(w) => {
                let _ = w.changed().await;
            }
            None => std::future::pending().await,
        }
    };
    futures::select! {
        () = Box::pin(moved).fuse() => {},
        () = Box::pin(processes.changed()).fuse() => {},
        _ = retry.next() => {},
    }
}

/// How a pane times its view's frames: its meter, and whether it is
/// busy now (shown).
#[derive(Clone)]
struct Timing {
    meter: ViewMeter,
    busy: Signal<bool>,
}

/// Mounts `view` and follows it: the frames into the document, until it
/// stops; then why, until it can be mounted again.
#[allow(clippy::too_many_arguments)]
async fn follow(
    node: NodeHandle,
    processes: Processes,
    extension: String,
    view: String,
    mut shown: Signal<Shown>,
    mut retry: mpsc::UnboundedReceiver<()>,
    theme: Option<Signal<String>>,
    mut timing: Timing,
) {
    loop {
        let Some(process) = processes.get(&extension) else {
            shown.set(Shown::Waiting(format!("{extension} is not loaded")));
            something_changes(None, &processes, &mut retry).await;
            continue;
        };
        let generation = match process.state() {
            State::Running { generation, .. } => generation,
            State::Starting => {
                shown.set(Shown::Waiting(format!("Starting {}…", process.name())));
                something_changes(Some(&process), &processes, &mut retry).await;
                continue;
            }
            State::Exited { why, restarting } => {
                let then = if restarting {
                    " — starting it again"
                } else {
                    ""
                };
                shown.set(Shown::Stopped(format!(
                    "{} stopped: {why}{then}",
                    process.name()
                )));
                something_changes(Some(&process), &processes, &mut retry).await;
                continue;
            }
            State::Stopped => {
                shown.set(Shown::Waiting(format!("{} was unloaded", process.name())));
                something_changes(None, &processes, &mut retry).await;
                continue;
            }
        };
        let mut session = match process.open(&view) {
            Ok(session) => session,
            Err(why) => {
                shown.set(Shown::Stopped(why));
                something_changes(Some(&process), &processes, &mut retry).await;
                continue;
            }
        };
        // The pane's events go through here, each one's time kept until
        // the frame that answers it.
        let answered = process.hello().is_some_and(|h| h.wire >= ANSWERED_WIRE);
        let asked: Rc<RefCell<VecDeque<Instant>>> = Rc::default();
        let (events, mut proxied) = mpsc::unbounded::<ViewEvent>();
        // Each event asked, to the frame loop's watch for an overdue one.
        let (asking, mut was_asked) = mpsc::unbounded::<()>();
        let forwarding = {
            let (to, asked) = (session.events.clone(), Rc::clone(&asked));
            spawn(async move {
                while let Some(event) = proxied.next().await {
                    if answered {
                        asked.borrow_mut().push_back(Instant::now());
                        let _ = asking.unbounded_send(());
                    }
                    if to.unbounded_send(event).is_err() {
                        return;
                    }
                }
            })
        };
        with_doc(&node, |doc, at| {
            let mut remote = RemoteDocument::new(events);
            if let Some(css) = theme {
                remote.set_theme_css(&css.peek());
            }
            doc.set_sub_document(at, Box::new(remote));
        })
        .await;
        let mut failed = None;
        while let Some(frame) =
            next_frame(&mut session.frames, &asked, &mut was_asked, &mut timing).await
        {
            let from = asked.borrow_mut().pop_front();
            let applying = Instant::now();
            let applied = with_doc(&node, |doc, at| {
                let applied = remote(doc, at).map_or_else(
                    || Err("the view was unmounted".to_owned()),
                    |r| r.apply(frame),
                );
                // A frame no input asked for (a save, live data, a worker's
                // news) still shows: the window is asked to draw again.
                doc.shell_provider.request_redraw();
                applied
            })
            .await;
            // From the event that asked; an older process's, its apply.
            match (answered, from) {
                (true, Some(asked_at)) => {
                    timing.meter.record(asked_at.elapsed());
                }
                (false, _) => {
                    timing.meter.record(applying.elapsed());
                }
                (true, None) => timing.meter.set_busy(false),
            }
            if *timing.busy.peek() {
                timing.busy.set(false);
            }
            match applied {
                Ok(()) if *shown.peek() != Shown::Live => shown.set(Shown::Live),
                Ok(()) => {}
                Err(why) => {
                    failed = Some(why);
                    break;
                }
            }
        }
        forwarding.cancel();
        drop(session);
        if *timing.busy.peek() {
            timing.busy.set(false);
        }
        timing.meter.set_busy(false);
        with_doc(&node, BaseDocument::remove_sub_document).await;
        // The frames end a moment before the supervisor sees the exit.
        if matches!(process.state(), State::Running { generation: g, .. } if g == generation) {
            let mut watch = process.watch();
            futures::select! {
                _ = Box::pin(watch.changed()).fuse() => {},
                () = futures_timer::Delay::new(Duration::from_millis(500)).fuse() => {},
            }
        }
        let why = match (failed, process.state()) {
            (Some(why), _) => format!("{}'s view failed: {why}", process.name()),
            (None, State::Exited { why, restarting }) => {
                let then = if restarting {
                    " — starting it again"
                } else {
                    ""
                };
                format!("{} stopped: {why}{then}", process.name())
            }
            (None, State::Stopped) => format!("{} was unloaded", process.name()),
            (None, _) => format!("{}'s view closed", process.name()),
        };
        shown.set(Shown::Stopped(why));
        // The process went: follow it back. The view went on its own:
        // wait for the button.
        if matches!(process.state(), State::Running { generation: g, .. } if g == generation) {
            let _ = retry.next().await;
        } else {
            something_changes(Some(&process), &processes, &mut retry).await;
        }
    }
}

/// The next frame; meanwhile, once the oldest unanswered event is past
/// the budget, the view is busy.
async fn next_frame(
    frames: &mut mpsc::UnboundedReceiver<Frame>,
    asked: &Rc<RefCell<VecDeque<Instant>>>,
    was_asked: &mut mpsc::UnboundedReceiver<()>,
    timing: &mut Timing,
) -> Option<Frame> {
    loop {
        let oldest = asked.borrow().front().copied();
        let budget = timing.meter.budget();
        let overdue = async {
            match oldest {
                Some(at) if !*timing.busy.peek() => {
                    futures_timer::Delay::new(budget.saturating_sub(at.elapsed())).await;
                }
                _ => std::future::pending().await,
            }
        };
        futures::select! {
            frame = frames.next() => return frame,
            // A new event: look again at the oldest.
            again = was_asked.next() => {
                // Its events go nowhere now: only frames are left.
                if again.is_none() {
                    return frames.next().await;
                }
            }
            () = Box::pin(overdue).fuse() => {
                timing.meter.set_busy(true);
                timing.busy.set(true);
            }
        }
    }
}

/// A process extension's view `view`, filling its parent; a placeholder
/// saying why when it is not running.
///
/// Its frames are timed into `<extension>/<view>`'s meter in the
/// `architect_workspace::Meters` in context (when there are); while one
/// runs past [`REMOTE_BUDGET`] the pane keeps its last frame, marked
/// busy.
#[component]
pub fn RemoteView(processes: Processes, extension: String, view: String) -> Element {
    let shown = use_signal(|| Shown::Waiting("Starting…".to_owned()));
    let busy = use_signal(|| false);
    let meters = try_use_context::<Meters>();
    let meter = use_hook(|| {
        let name = format!("{extension}/{view}");
        meters.map_or_else(
            || ViewMeter::new(&name, REMOTE_BUDGET),
            |m| m.meter(&name, REMOTE_BUDGET),
        )
    });
    {
        // Unmounted mid-frame: no longer busy.
        let meter = meter.clone();
        use_drop(move || meter.set_busy(false));
    }
    let (retry_to, retry) = use_hook(|| {
        let (to, from) = mpsc::unbounded::<()>();
        (to, std::rc::Rc::new(std::cell::RefCell::new(Some(from))))
    });
    let mut mounted = use_signal(|| false);
    // The app's theme, into the view's document now and on every change.
    let theme = architect_theme::try_use_root_css();
    let mut node_at = use_signal(|| None::<NodeHandle>);
    use_effect(move || {
        let Some(css) = theme.map(|t| t()) else {
            return;
        };
        let Some(node) = node_at.peek().clone() else {
            return;
        };
        spawn(async move {
            with_doc(&node, |doc, at| {
                if let Some(r) = remote(doc, at) {
                    r.set_theme_css(&css);
                }
            })
            .await;
        });
    });
    let follow_with = {
        let (processes, extension) = (processes.clone(), extension.clone());
        move |node: NodeHandle| {
            if let Some(retry) = retry.borrow_mut().take() {
                spawn(follow(
                    node,
                    processes.clone(),
                    extension.clone(),
                    view.clone(),
                    shown,
                    retry,
                    theme,
                    Timing {
                        meter: meter.clone(),
                        busy,
                    },
                ));
            }
        }
    };
    let restart = {
        move |_| {
            if let Some(p) = processes.get(&extension)
                && matches!(
                    p.state(),
                    State::Exited {
                        restarting: false,
                        ..
                    }
                )
            {
                p.restart();
            }
            let _ = retry_to.unbounded_send(());
        }
    };
    let placeholder = match shown() {
        Shown::Live => None,
        Shown::Waiting(why) => Some((why, false)),
        Shown::Stopped(why) => Some((why, true)),
    };
    rsx! {
        div { style: "position: relative; width: 100%; height: 100%; overflow: hidden;",
            div {
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
                onmounted: move |e: MountedEvent| {
                    if !mounted() && let Some(node) = e.data().downcast::<NodeHandle>() {
                        mounted.set(true);
                        node_at.set(Some(node.clone()));
                        follow_with(node.clone());
                    }
                },
            }
            if busy() && placeholder.is_none() {
                architect_shell::BusyBadge {}
            }
            if let Some((why, stopped)) = placeholder {
                div {
                    style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; \
                            flex-direction: column; align-items: center; justify-content: center; gap: 10px; \
                            padding: 24px; font-family: -apple-system, system-ui, sans-serif; font-size: 13px; \
                            color: #a1a1aa; text-align: center;",
                    span { class: "remote-view-why", "{why}" }
                    if stopped {
                        button {
                            class: "remote-view-restart",
                            style: "padding: 5px 12px; border-radius: 6px; border: 1px solid #3f3f46; \
                                    background: #27272a; color: #e4e4e7; font-size: 13px;",
                            onclick: restart,
                            "Start again"
                        }
                    }
                }
            }
        }
    }
}

/// How many elements a view's document holds, its root's own aside:
/// what `EXT_CHECK` says a first frame drew.
#[must_use]
pub fn element_count(doc: &BaseDocument) -> u32 {
    let mut count = 0_u32;
    let mut next = vec![doc.root_node().id];
    while let Some(id) = next.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node.is_element() {
            count = count.saturating_add(1);
        }
        next.extend(node.children.iter().copied());
    }
    // `html`, `head`, `style`, `body` and `main` are the document's own.
    count.saturating_sub(5)
}

/// The nodes of `doc` that hold a [`RemoteDocument`]: the panes showing
/// a process extension's view.
#[must_use]
pub fn mounted_views(doc: &BaseDocument) -> Vec<NodeId> {
    doc.sub_document_node_ids()
        .into_iter()
        .filter(|n| {
            doc.subdoc(*n).is_some_and(|d| {
                let d: &dyn Any = d;
                d.is::<RemoteDocument>()
            })
        })
        .collect()
}
