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
//! The applier trusts nothing: an edit naming a node it does not have, a
//! path that leads nowhere, a move that would make a cycle, or a frame
//! too large fails the view (the placeholder says so) instead of the app.
//! Not yet: pointer, touch, wheel and scroll events (Blitz scrolls the
//! sub-document itself), IME, `mounted` measurements, the shell's
//! components as named elements, the allowlist of elements and CSS.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use blitz_dom::{
    BaseDocument, DEFAULT_CSS, DocGuard, DocGuardMut, Document, DocumentConfig, EventDriver,
    EventHandler, LocalName, Namespace, Node, QualName,
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
    Edit, EventData, Frame, KeyData, Mods, Pointer, Value, ViewEvent, WireNode, WireTemplate,
};

/// At most this many templates per mount.
const MAX_TEMPLATES: usize = 4096;
/// At most this many nodes in one template.
const MAX_TEMPLATE_NODES: usize = 10_000;
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
    root: usize,
    stack: Vec<usize>,
    /// Dioxus element id → node, and back.
    ids: HashMap<u64, usize>,
    elements: HashMap<usize, u64>,
    /// Each template's roots, built once, cloned on each load.
    templates: HashMap<u32, Vec<usize>>,
    /// How many handlers there are for each event.
    listeners: HashMap<String, u32>,
    events: mpsc::UnboundedSender<ViewEvent>,
}

impl std::fmt::Debug for RemoteDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteDocument")
            .field("root", &self.root)
            .field("templates", &self.templates.len())
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
        doc.add_user_agent_stylesheet(DEFAULT_CSS);
        let mut m = doc.mutate();
        let html = m.create_element(qual("html", None), Vec::new());
        let page = m.doc.root_node().id;
        m.append_children(page, &[html]);
        let head = m.create_element(qual("head", None), Vec::new());
        let body = m.create_element(qual("body", None), Vec::new());
        m.append_children(html, &[head, body]);
        let root = m.create_element(qual("main", None), Vec::new());
        m.append_children(body, &[root]);
        drop(m);
        Self {
            doc,
            root,
            stack: vec![root],
            ids: HashMap::from([(0, root)]),
            elements: HashMap::from([(root, 0)]),
            templates: HashMap::new(),
            listeners: HashMap::new(),
            events,
        }
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
        let (templates, edits) = match frame {
            Frame::Edits { templates, edits } => (templates, edits),
            Frame::Failed(why) => return Err(why),
        };
        if edits.len() > MAX_EDITS {
            return Err(format!("a frame of {} edits is too large", edits.len()));
        }
        for template in &templates {
            self.template(template)?;
        }
        for edit in edits {
            self.edit(edit)?;
        }
        Ok(())
    }

    fn template(&mut self, t: &WireTemplate) -> Result<(), String> {
        if self.templates.len() >= MAX_TEMPLATES {
            return Err("too many templates".to_owned());
        }
        if t.nodes.len() > MAX_TEMPLATE_NODES {
            return Err(format!("template {} is too large", t.id));
        }
        if self.templates.contains_key(&t.id) {
            return Err(format!("template {} was sent twice", t.id));
        }
        // A node's children come before it, each one child of one parent,
        // so the tree is built in order and cannot loop.
        let mut made = Vec::with_capacity(t.nodes.len());
        let mut taken = vec![false; t.nodes.len()];
        let mut m = self.doc.mutate();
        for (at, node) in t.nodes.iter().enumerate() {
            let id = match node {
                WireNode::Element {
                    tag,
                    namespace,
                    attrs,
                    children,
                } => {
                    let mut kids = Vec::with_capacity(children.len());
                    for &c in children {
                        let c = usize::try_from(c).map_err(|e| e.to_string())?;
                        if c >= at || taken.get(c).copied().unwrap_or(true) {
                            return Err(format!("template {} is not a tree", t.id));
                        }
                        if let Some(slot) = taken.get_mut(c) {
                            *slot = true;
                        }
                        kids.extend(made.get(c).copied());
                    }
                    let el = m.create_element(qual(tag, namespace.as_deref()), Vec::new());
                    for a in attrs {
                        set_attr(&mut m, el, &a.name, a.namespace.as_deref(), Some(&a.value))?;
                    }
                    m.append_children(el, &kids);
                    el
                }
                WireNode::Text(text) => m.create_text_node(text),
                WireNode::Dynamic => m.create_comment_node(),
            };
            made.push(id);
        }
        let mut roots = Vec::with_capacity(t.roots.len());
        for &r in &t.roots {
            let r = usize::try_from(r).map_err(|e| e.to_string())?;
            if taken.get(r).copied().unwrap_or(true) {
                return Err(format!("template {} has a bad root", t.id));
            }
            roots.extend(made.get(r).copied());
        }
        drop(m);
        self.templates.insert(t.id, roots);
        Ok(())
    }

    fn node(&self, id: u64) -> Result<usize, String> {
        self.ids
            .get(&id)
            .copied()
            .filter(|n| self.doc.get_node(*n).is_some())
            .ok_or_else(|| format!("no element {id}"))
    }

    fn pop(&mut self, m: u32) -> Result<Vec<usize>, String> {
        let m = usize::try_from(m).map_err(|e| e.to_string())?;
        // The root stays at the bottom of the stack.
        let keep = self
            .stack
            .len()
            .checked_sub(m)
            .filter(|k| *k >= 1)
            .ok_or("an edit took more nodes than there are")?;
        Ok(self.stack.split_off(keep))
    }

    fn walk(&self, path: &[u8]) -> Result<usize, String> {
        let mut at = *self.stack.last().ok_or("nothing to walk from")?;
        for &i in path {
            at = self
                .doc
                .get_node(at)
                .and_then(|n| n.children.get(usize::from(i)).copied())
                .ok_or("a path that leads nowhere")?;
        }
        Ok(at)
    }

    /// Whether `node` is `inside` or below it.
    fn within(&self, node: usize, inside: usize) -> bool {
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
    fn placeable(&self, parent: usize, kids: &[usize]) -> Result<(), String> {
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
    fn parent_of(&self, anchor: usize) -> Result<usize, String> {
        self.doc
            .get_node(anchor)
            .and_then(|n| n.parent)
            .filter(|_| anchor != self.root)
            .ok_or_else(|| "an edit reached outside the view".to_owned())
    }

    fn map(&mut self, node: usize, id: u64) {
        if let Some(old) = self.ids.insert(id, node) {
            self.elements.remove(&old);
        }
        self.elements.insert(node, id);
    }

    /// Forgets the ids of `node` and everything below it.
    fn forget(&mut self, node: usize) {
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

    fn replace(&mut self, anchor: usize, kids: &[usize]) -> Result<(), String> {
        let parent = self.parent_of(anchor)?;
        self.placeable(parent, kids)?;
        self.forget(anchor);
        let mut m = self.doc.mutate();
        m.insert_nodes_before(anchor, kids);
        m.remove_and_drop_node(anchor);
        Ok(())
    }

    fn edit(&mut self, edit: Edit) -> Result<(), String> {
        match edit {
            Edit::AppendChildren { id, m } => {
                let parent = self.node(id)?;
                let kids = self.pop(m)?;
                self.placeable(parent, &kids)?;
                self.doc.mutate().append_children(parent, &kids);
            }
            Edit::AssignNodeId { path, id } => {
                let node = self.walk(&path)?;
                if let Some(old) = self.ids.get(&id).copied()
                    && old != node
                    && !self.within(old, self.root)
                {
                    self.forget(old);
                    self.doc.mutate().remove_node_if_unparented(old);
                }
                self.map(node, id);
            }
            Edit::CreatePlaceholder { id } => {
                let node = self.doc.mutate().create_comment_node();
                self.map(node, id);
                self.stack.push(node);
            }
            Edit::CreateTextNode { value, id } => {
                let node = self.doc.mutate().create_text_node(&value);
                self.map(node, id);
                self.stack.push(node);
            }
            Edit::LoadTemplate {
                template,
                index,
                id,
            } => {
                let index = usize::try_from(index).map_err(|e| e.to_string())?;
                let root = self
                    .templates
                    .get(&template)
                    .and_then(|roots| roots.get(index))
                    .copied()
                    .ok_or_else(|| format!("no template {template} root {index}"))?;
                let node = self.doc.mutate().deep_clone_node(root);
                self.map(node, id);
                self.stack.push(node);
            }
            Edit::ReplaceNodeWith { id, m } => {
                let anchor = self.node(id)?;
                let kids = self.pop(m)?;
                self.replace(anchor, &kids)?;
            }
            Edit::ReplacePlaceholder { path, m } => {
                let kids = self.pop(m)?;
                let anchor = self.walk(&path)?;
                self.replace(anchor, &kids)?;
            }
            Edit::InsertAfter { id, m } | Edit::InsertBefore { id, m } => {
                let after = matches!(edit, Edit::InsertAfter { .. });
                let anchor = self.node(id)?;
                let kids = self.pop(m)?;
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
                id,
            } => {
                let node = self.node(id)?;
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
            Edit::SetText { value, id } => {
                let node = self.node(id)?;
                self.doc.mutate().set_node_text(node, &value);
            }
            Edit::Listen { name, id } => {
                let node = self.node(id)?;
                let mut m = self.doc.mutate();
                set_attr(&mut m, node, DIOXUS_ID, None, Some(&id.to_string()))?;
                drop(m);
                let n = self.listeners.entry(name).or_default();
                *n = n.saturating_add(1);
            }
            Edit::Unlisten { name, .. } => {
                if let Some(n) = self.listeners.get_mut(&name) {
                    *n = n.saturating_sub(1);
                }
            }
            Edit::Remove { id } => {
                let node = self.node(id)?;
                if node == self.root {
                    return Err("an edit reached outside the view".to_owned());
                }
                self.forget(node);
                self.doc.mutate().remove_and_drop_node(node);
            }
            Edit::PushRoot { id } => {
                let node = self.node(id)?;
                self.stack.push(node);
            }
        }
        Ok(())
    }
}

/// Sets (or, given no value, clears) an attribute, as Dioxus's own
/// renderer does: the `style` namespace is one CSS property.
fn set_attr(
    m: &mut blitz_dom::DocumentMutator<'_>,
    node: usize,
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
        chain: &[usize],
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
async fn with_doc<R>(node: &NodeHandle, f: impl FnOnce(&mut BaseDocument, usize) -> R) -> R {
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
fn remote(doc: &mut BaseDocument, node: usize) -> Option<&mut RemoteDocument> {
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

/// Mounts `view` and follows it: the frames into the document, until it
/// stops; then why, until it can be mounted again.
async fn follow(
    node: NodeHandle,
    processes: Processes,
    extension: String,
    view: String,
    mut shown: Signal<Shown>,
    mut retry: mpsc::UnboundedReceiver<()>,
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
        let events = session.events.clone();
        with_doc(&node, |doc, at| {
            doc.set_sub_document(at, Box::new(RemoteDocument::new(events)));
        })
        .await;
        let mut failed = None;
        while let Some(frame) = session.frames.next().await {
            let applied = with_doc(&node, |doc, at| {
                remote(doc, at).map_or_else(
                    || Err("the view was unmounted".to_owned()),
                    |r| r.apply(frame),
                )
            })
            .await;
            match applied {
                Ok(()) if *shown.peek() != Shown::Live => shown.set(Shown::Live),
                Ok(()) => {}
                Err(why) => {
                    failed = Some(why);
                    break;
                }
            }
        }
        drop(session);
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

/// A process extension's view `view`, filling its parent; a placeholder
/// saying why when it is not running.
#[component]
pub fn RemoteView(processes: Processes, extension: String, view: String) -> Element {
    let shown = use_signal(|| Shown::Waiting("Starting…".to_owned()));
    let (retry_to, retry) = use_hook(|| {
        let (to, from) = mpsc::unbounded::<()>();
        (to, std::rc::Rc::new(std::cell::RefCell::new(Some(from))))
    });
    let mut mounted = use_signal(|| false);
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
                        follow_with(node.clone());
                    }
                },
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

/// The nodes of `doc` that hold a [`RemoteDocument`]: the panes showing
/// a process extension's view.
#[must_use]
pub fn mounted_views(doc: &BaseDocument) -> Vec<usize> {
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
