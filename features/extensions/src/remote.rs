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
//! [`RemoteEmbed`] mounts the same view inline in a host's document, as
//! an embed (`crate::embeds`): its sub-document is laid out at the
//! host's width with its own height left to its content
//! ([`RemoteDocument::fit_content`]); the host measures that height
//! ([`RemoteDocument::content_height`], every [`MEASURE_EVERY`]) and sizes
//! the embed's box to it, up to its `max_height`, past which the
//! sub-document scrolls. The embed's input reaches the view as
//! [`crate::wire::EMBED_EVENT`], again whenever it changes.
//!
//! Input (#176): pointer, touch, wheel and scroll events, an input
//! method's composition, keys and focus all reach the view's handlers
//! ([`crate::wire::INPUT_WIRE`] on). A press holds the pointer: the
//! moves and the release that follow go to the element pressed, over it
//! or not ([`RemoteDocument`]'s implicit capture, as a touch is captured
//! on the web). Fast events are coalesced while the view is still
//! answering one ([`crate::wire::Coalescer`]). The view's document is
//! given the window's input method and clipboard, its caret area moved
//! to where the pane is ([`HostShell`]).
//!
//! Not yet: `mounted` measurements, the shell's components as named
//! elements, the allowlist of elements and CSS; a release outside the
//! pane reaches the view as a `pointercancel` when the pointer comes
//! back (Blitz's own capture holds only custom widgets).

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use architect_workspace::{Meters, ViewMeter};

use blitz_dom::{
    BaseDocument, DocGuard, DocGuardMut, Document, DocumentConfig, EventDriver, EventHandler,
    LocalName, Namespace, Node, NodeId, QualName, UserAgentStylesheet,
};
use blitz_traits::events::{
    BlitzFocusEvent, BlitzImeEvent, BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId,
    BlitzWheelDelta, BlitzWheelEvent, DomEvent, DomEventData, EventState, Location, Modifiers,
    MouseEventButton, UiEvent,
};
use blitz_traits::shell::{ClipboardError, ShellProvider};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};

use crate::process::{Process, Processes, State};
use crate::wire::{
    ANSWERED_WIRE, Coalescer, Edit, EventData, Frame, KeyData, Mods, Pointer, PointerData,
    ScrollData, TouchData, TouchPoint, Value, ViewEvent, WheelData,
};

/// A process view's frame budget, from the event to its frame: two
/// frames at 60 Hz, the round trip over the socket included.
pub const REMOTE_BUDGET: Duration = Duration::from_millis(33);

/// At most this many edits in one frame.
const MAX_EDITS: usize = 200_000;

/// How often a pane's place in the window, and the boxes of the views
/// it holds (`crate::embedded`), are looked at: an input method's caret
/// and a held view follow a pane that moves or a view that scrolls.
pub const PLACE_EVERY: Duration = Duration::from_millis(50);

/// How often an embed's content height is looked at.
pub const MEASURE_EVERY: Duration = Duration::from_millis(50);

/// What an embed's sub-document is styled with: its page as tall as its
/// content (not the viewport), margins kept inside it, so the host can
/// measure it.
const FIT_CSS: &str = "html, body { margin: 0 !important; height: auto !important; }                        main { min-height: 0 !important; height: auto !important; display: flow-root !important; }";

/// What a view's sub-document starts from: what a linked view inherits
/// from the pane it is in (`architect_workspace`'s host: no margin, the
/// pane's height, the theme's ink and font at 13px), in the theme's
/// variables (`set_theme_css`). Without it a view that sets no colour of
/// its own draws black in the user agent's serif on a dark theme.
const BASE_CSS: &str = "html, body { margin: 0; padding: 0; }                         html, body, main { height: 100%; }                         body { color: var(--ink, var(--foreground)); font-size: 13px;                         font-family: var(--font, var(--default-font-family, var(--font-sans, system-ui, sans-serif))); }";

/// Another extension's view a remote view holds, as its placeholder says.
#[derive(Clone, Debug, Default)]
struct Held {
    key: u64,
    view: Option<(String, String)>,
    input: crate::contrib::EmbedInput,
}

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
    /// Whether it is sized to its content ([`Self::fit_content`]).
    fitted: bool,
    /// The element a press is holding the pointer on, until its release.
    held: Option<u64>,
    /// Whether an input method is composing (a `compositionstart` was
    /// sent and its end not yet).
    composing: bool,
    /// Its share of `architect_workspace::field_focused`: a field of its
    /// own has the focus (a pane's view keys leave letters to it).
    field: architect_workspace::FieldFocus,
    /// The scenes the view sent, by hash (#175).
    scenes: crate::scene::SceneCache,
    /// Scenes decoded off the UI thread before their frame is applied
    /// (`scene::predecode`), by hash.
    predecoded: HashMap<u64, Result<crate::scene::Commands, String>>,
    /// Where its scenes draw over the whole app (`overlay` in
    /// `capabilities.ui`, granted); clipped to their boxes when none.
    overlay: Option<crate::scene::OverlayLayer>,
    /// How many times its scenes told the view a new size.
    resizes: Arc<std::sync::atomic::AtomicU64>,
    /// Other extensions' views it holds (`crate::embedded`), by placeholder.
    holds: HashMap<NodeId, Held>,
    /// The next held view's key.
    next_held: u64,
    /// The elements it paints: their scene and transform now.
    painted: HashMap<NodeId, Arc<Mutex<crate::scene::Painted>>>,
    /// How many bytes of scenes and transforms it sent (for a benchmark).
    scene_bytes: u64,
    /// The frame's layers batches as bytes, by number, until the edit that
    /// names each takes it (`crate::wire::BYTES_WIRE`).
    batches: std::collections::HashMap<u64, Vec<u8>>,
    /// What its layers' animations read (`architect_scene::anim`): the
    /// app's clocks unless told.
    clocks: architect_scene::Clocks,
    /// Wakes its window at its layers' next step and when a clock jumps
    /// (`architect_scene::wake`), once it has a window.
    alarm: Option<Arc<architect_scene::wake::Alarm>>,
    /// The painted element the pointer is over or holds a region of.
    pointed: Option<NodeId>,
    /// The engine's values its scenes read, when it was granted them.
    engine: Option<Arc<architect_scene::engine::EngineTable>>,
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
        // What a linked view inherits from its pane (`BASE_CSS`), then
        // the host's theme (`set_theme_css`): the view's own sheets come
        // after both, so a view still wins where it sets a colour itself.
        let base = m.create_element(qual("style", None), Vec::new());
        let base_text = m.create_text_node(BASE_CSS);
        m.append_children(base, &[base_text]);
        m.append_children(head, &[base]);
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
            fitted: false,
            held: None,
            composing: false,
            field: architect_workspace::FieldFocus::new(),
            scenes: crate::scene::SceneCache::new(),
            predecoded: HashMap::new(),
            holds: HashMap::new(),
            next_held: 1,
            overlay: None,
            resizes: Arc::default(),
            painted: HashMap::new(),
            scene_bytes: 0,
            batches: std::collections::HashMap::new(),
            clocks: architect_scene::Clocks::app(),
            alarm: None,
            pointed: None,
            engine: None,
        }
    }

    /// Its scenes read the engine's values from `table` (its extension
    /// was granted `engine`): an animation bound to one is drawn at the
    /// level the engine last wrote, and its window wakes when the engine
    /// writes after a rest.
    pub fn set_engine(&mut self, table: Option<Arc<architect_scene::engine::EngineTable>>) {
        for painted in self.painted.values() {
            let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(layers) = p.layers.as_mut() {
                layers.grant(table.clone());
            }
            p.dirty = true;
        }
        self.engine = table;
        self.watch_engine();
    }

    fn watch_engine(&self) {
        if let (Some(table), Some(alarm)) = (&self.engine, &self.alarm) {
            architect_scene::engine::watch(table, alarm.waker());
        }
    }

    /// The painted element whose box holds `(x, y)` (its document's CSS
    /// pixels), and the point in it.
    fn painted_at(&self, x: f32, y: f32) -> Option<(NodeId, peniko::kurbo::Point)> {
        self.painted.keys().copied().find_map(|node| {
            let n = self.doc.get_node(node)?;
            let at = n.absolute_position(0.0, 0.0);
            let size = n.final_layout().size;
            let (dx, dy) = (x - at.x, y - at.y);
            ((0.0..size.width).contains(&dx) && (0.0..size.height).contains(&dy)).then(|| {
                (
                    node,
                    peniko::kurbo::Point::new(f64::from(dx), f64::from(dy)),
                )
            })
        })
    }

    /// A pointer event for its scenes' hit regions, before the view
    /// hears it: whether it was theirs (a press on a region, a drag), and
    /// so not forwarded. Hover, the cursor and a drag are drawn here,
    /// with no word to the view but what its regions heard.
    fn regions_event(&mut self, event: &UiEvent) -> (bool, bool) {
        use crate::scene::PointerKind;
        let (p, kind) = match event {
            UiEvent::PointerMove(p) => (p, PointerKind::Move),
            UiEvent::PointerDown(p) => (p, PointerKind::Down),
            UiEvent::PointerUp(p) => (p, PointerKind::Up),
            UiEvent::PointerLeave(p) | UiEvent::PointerCancel(p) => (p, PointerKind::Gone),
            _ => return (false, false),
        };
        if self.painted.values().all(|p| {
            p.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .layers
                .as_ref()
                .is_none_or(|l| l.layers().region_count() == 0)
        }) {
            return (false, false);
        }
        let (x, y) = (p.coords.client_x, p.coords.client_y);
        let holding = self.pointed.filter(|node| {
            self.painted.get(node).is_some_and(|p| {
                p.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .interaction
                    .holding()
            })
        });
        let under = self.painted_at(x, y);
        // Held: the element that holds it hears it wherever it goes.
        let target = match (holding, under) {
            (Some(node), _) => self.doc.get_node(node).map(|n| {
                let at = n.absolute_position(0.0, 0.0);
                (
                    node,
                    peniko::kurbo::Point::new(f64::from(x - at.x), f64::from(y - at.y)),
                )
            }),
            (None, under) => under,
        };
        let mut outcomes = Vec::new();
        // Off the element it was over: that one's hover ends.
        if let Some(was) = self
            .pointed
            .filter(|was| target.is_none_or(|(now, _)| now != *was))
            && let Some(painted) = self.painted.get(&was)
        {
            let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
            outcomes.push(p.pointer(PointerKind::Gone, peniko::kurbo::Point::ZERO));
        }
        self.pointed = target.map(|(node, _)| node);
        if let Some((node, at)) = target
            && let Some(painted) = self.painted.get(&node)
        {
            let outcome = painted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pointer(kind, at);
            outcomes.push(outcome);
            if kind == PointerKind::Gone {
                self.pointed = None;
            }
        }
        let shell = Arc::clone(&self.doc.shell_provider);
        let (mut consumed, mut cursor) = (false, false);
        for o in outcomes {
            consumed |= o.consumed;
            cursor |= o.cursor.is_some();
            if o.redraw {
                shell.request_redraw();
            }
        }
        if consumed && let Some(layer) = &self.overlay {
            layer.touch();
        }
        (consumed, cursor)
    }

    /// Whether a text field of its own has the focus, told to
    /// `architect_workspace::field_focused`.
    fn sync_field(&mut self) {
        let typing = self
            .doc
            .get_focussed_node_id()
            .and_then(|id| self.doc.get_node(id))
            .and_then(Node::element_data)
            .is_some_and(|el| el.text_input_data().is_some());
        self.field.set(typing);
    }

    /// Shows the cursor the region under the pointer asks for: after the
    /// view's own document set its element's, which it does when its
    /// hover changes.
    fn region_cursor(&self, changed: bool) {
        let cursor = self
            .pointed
            .and_then(|node| self.painted.get(&node))
            .map(|p| {
                p.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .interaction
                    .cursor()
            })
            .unwrap_or_default();
        if changed || cursor != architect_scene::region::Cursor::Default {
            self.doc
                .shell_provider
                .set_cursor(cursor.css().parse::<cursor_icon::CursorIcon>().ok());
        }
    }

    /// Its layers' animations read `clocks` from now on (a test's, or a
    /// host's own): every element it paints.
    pub fn set_clocks(&mut self, clocks: architect_scene::Clocks) {
        for painted in self.painted.values() {
            let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
            p.clocks = clocks.clone();
            p.dirty = true;
        }
        self.clocks = clocks;
    }

    /// Lets go of a held pointer the view will see no release for: a
    /// `pointercancel` to the element held, or, when the view doesn't
    /// listen for one, a `pointerup` (so a drag isn't left grabbed). An
    /// input method's composition ends too. The host calls it when the
    /// pane loses the focus or the window its pointer.
    pub fn let_go(&mut self, at: Option<&BlitzPointerEvent>) {
        if let Some(held) = self.held.take() {
            let listens = |n: &str| self.listeners.get(n).copied().unwrap_or(0) > 0;
            let name = if listens("pointercancel") {
                Some("pointercancel")
            } else if listens("pointerup") {
                Some("pointerup")
            } else {
                None
            };
            if let Some(name) = name {
                let data = at.map_or_else(PointerData::default, pointer_data);
                let _ = self.events.unbounded_send(ViewEvent {
                    element: held,
                    name: name.to_owned(),
                    bubbles: true,
                    data: EventData::Pointer(data),
                });
            }
        }
        self.end_composition(None);
    }

    /// Ends an input method's composition the view heard start, at
    /// `element` (or the root).
    fn end_composition(&mut self, element: Option<u64>) {
        if !self.composing {
            return;
        }
        self.composing = false;
        if self.listeners.get("compositionend").copied().unwrap_or(0) > 0 {
            let _ = self.events.unbounded_send(ViewEvent {
                element: element.unwrap_or(0),
                name: "compositionend".to_owned(),
                bubbles: true,
                data: EventData::Composition(String::new()),
            });
        }
    }

    /// The value `name` holds in its scenes' layers (`architect_scene`
    /// values: the view's, the host's, a drag's), if one holds it.
    #[must_use]
    pub fn scene_value(&self, name: &architect_scene::anim::Name) -> Option<f64> {
        self.painted.values().find_map(|p| {
            p.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .layers
                .as_ref()
                .and_then(|l| l.values().get(name))
        })
    }

    /// How many bytes of scene content and transforms the view has sent.
    #[must_use]
    pub const fn scene_bytes(&self) -> u64 {
        self.scene_bytes
    }

    /// A placeholder's held view or its input changed (`crate::embedded`).
    fn held_attr(&mut self, node: NodeId, name: &str, value: Option<&str>) -> Result<(), String> {
        use crate::embedded::{INPUT_ATTR, MAX_EMBEDDED, MAX_INPUT_BYTES, VIEW_ATTR, split_view};
        if !self.holds.contains_key(&node) {
            if value.is_none() {
                return Ok(());
            }
            if self.holds.len() >= MAX_EMBEDDED {
                return Err(format!("it holds more than {MAX_EMBEDDED} views"));
            }
            let key = self.next_held;
            self.next_held = self.next_held.saturating_add(1);
            self.holds.insert(
                node,
                Held {
                    key,
                    ..Held::default()
                },
            );
        }
        let Some(held) = self.holds.get_mut(&node) else {
            return Ok(());
        };
        if name == VIEW_ATTR {
            held.view = match value {
                Some(v) => Some(
                    split_view(v)
                        .map(|(e, w)| (e.to_owned(), w.to_owned()))
                        .ok_or_else(|| format!("{v:?} names no view (<extension>/<view>)"))?,
                ),
                None => None,
            };
        } else if name == INPUT_ATTR {
            held.input = match value {
                Some(v) if v.len() > MAX_INPUT_BYTES => {
                    return Err(format!(
                        "a held view's input of {} bytes: past its limit of {MAX_INPUT_BYTES}",
                        v.len()
                    ));
                }
                Some(v) => serde_json::from_str(v)
                    .map_err(|e| format!("a held view's input that isn't one: {e}"))?,
                None => crate::contrib::EmbedInput::default(),
            };
        }
        if held.view.is_none() && held.input == crate::contrib::EmbedInput::default() {
            self.holds.remove(&node);
        }
        Ok(())
    }

    /// The other extensions' views it holds now, and where: each
    /// placeholder's box in its view, scrolled (`crate::embedded`).
    #[must_use]
    pub fn embedded(&self) -> Vec<crate::embedded::Slot> {
        let scroll = self.doc.viewport_scroll();
        let mut out: Vec<crate::embedded::Slot> = self
            .holds
            .iter()
            .filter_map(|(node, held)| {
                let (extension, view) = held.view.clone()?;
                let n = self.doc.get_node(*node)?;
                let at = n.absolute_position(0.0, 0.0);
                let size = n.final_layout().size;
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                Some(crate::embedded::Slot {
                    key: held.key,
                    extension,
                    view,
                    input: held.input.clone(),
                    x: at.x - scroll.x as f32,
                    y: at.y - scroll.y as f32,
                    width: size.width,
                    height: size.height,
                })
            })
            .collect();
        out.sort_by_key(|s| s.key);
        out
    }

    /// Lets its scenes draw over the whole app (an extension granted
    /// `overlay`), through `layer`, whose widget fills the window; with
    /// none they are clipped to their boxes.
    pub fn set_overlay(&mut self, layer: Option<crate::scene::OverlayLayer>) {
        self.overlay = layer;
    }

    /// Puts its scenes where their elements are in the window, the pane
    /// being at `pane` (CSS pixels): where an overlay draws them.
    pub fn place_scenes(&mut self, pane: (f32, f32)) {
        let Some(layer) = &self.overlay else { return };
        let mut moved = false;
        for (node, painted) in &self.painted {
            let Some(n) = self.doc.get_node(*node) else {
                continue;
            };
            let at = n.absolute_position(0.0, 0.0);
            let origin = (f64::from(pane.0 + at.x), f64::from(pane.1 + at.y));
            let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
            if p.origin != origin {
                p.origin = origin;
                moved = true;
            }
            drop(p);
        }
        if moved {
            layer.touch();
        }
    }

    /// How many times its scenes have told the view a new size.
    #[must_use]
    pub fn scene_resizes(&self) -> u64 {
        self.resizes.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Scenes of the next frame, decoded already (off the UI thread,
    /// `scene::predecode`).
    pub fn predecoded(&mut self, decoded: Vec<(u64, Result<crate::scene::Commands, String>)>) {
        self.predecoded.extend(decoded);
    }

    /// An element's layers (`scene::LAYERS_ATTR`) changed: the batch is
    /// applied to them, and the element is drawn from them as a widget.
    fn layers_attr(&mut self, node: NodeId, value: Option<&str>) -> Result<(), String> {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return Ok(());
        };
        let batch = match crate::scene::layers_named(value) {
            // Its bytes came beside the edits: moved, not decoded.
            Some(n) => self
                .batches
                .remove(&n)
                .ok_or_else(|| format!("layers batch {n} that its frame did not carry"))?,
            None => crate::scene::layers_batch(value)?,
        };
        self.scene_bytes = self
            .scene_bytes
            .saturating_add(u64::try_from(batch.len()).unwrap_or(u64::MAX));
        let engine = self.engine.clone();
        if !self.painted.contains_key(&node) {
            // The widget, as a scene's.
            self.paint_attr(node, crate::scene::TRANSFORM_ATTR, None)?;
        }
        let Some(painted) = self.painted.get(&node) else {
            return Ok(());
        };
        let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
        let applied = p
            .layers
            .get_or_insert_with(|| {
                let mut receiver = architect_scene::ops::Receiver::new();
                receiver.grant(engine.clone());
                receiver
            })
            // Given whole: a large font or image is drawn from the batch's
            // own bytes, not a copy of them.
            .apply_owned(batch)
            .map_err(|e| format!("its layers: {e}"));
        p.dirty = true;
        drop(p);
        if let Some(layer) = &self.overlay {
            layer.touch();
        }
        applied
    }

    /// An element's scene (`scene::SCENE_ATTR`) or transform
    /// (`scene::TRANSFORM_ATTR`) changed: it is drawn as a widget, its
    /// state updated in place.
    fn paint_attr(&mut self, node: NodeId, name: &str, value: Option<&str>) -> Result<(), String> {
        use crate::scene::{Affine, SCENE_ATTR, parse_transform};
        let len = value.map_or(0, str::len);
        self.scene_bytes = self
            .scene_bytes
            .saturating_add(u64::try_from(len).unwrap_or(u64::MAX));
        if name == SCENE_ATTR && value.is_none() {
            if let Some(painted) = self.painted.remove(&node) {
                if let Some(layer) = &self.overlay {
                    layer.remove(&painted);
                }
                self.doc.remove_custom_widget(node);
            }
            return Ok(());
        }
        let commands = match (name == SCENE_ATTR, value) {
            (true, Some(v)) => {
                let predecoded = &mut self.predecoded;
                Some(self.scenes.receive_with(v, |json| {
                    let hash = crate::scene::split(v).map_or(0, |(h, _)| h);
                    predecoded
                        .remove(&hash)
                        .unwrap_or_else(|| crate::scene::decode(json))
                })?)
            }
            _ => None,
        };
        let transform =
            (name != SCENE_ATTR).then(|| value.map_or(Affine::IDENTITY, parse_transform));
        if let Some(painted) = self.painted.get(&node) {
            let mut p = painted.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(c) = commands {
                p.commands = c;
            }
            if let Some(t) = transform {
                p.transform = t;
            }
            p.dirty = true;
            drop(p);
            if let Some(layer) = &self.overlay {
                layer.touch();
            }
            return Ok(());
        }
        let element = self.elements.get(&node).copied().unwrap_or(0);
        let painted = Arc::new(Mutex::new(crate::scene::Painted {
            commands: commands.unwrap_or_default(),
            layers: None,
            clocks: self.clocks.clone(),
            drawn: architect_scene::Readings::NONE,
            alarm: self.alarm.clone(),
            interaction: architect_scene::region::Interaction::new(),
            drawn_lit: None,
            drawn_values: 0,
            transform: transform.unwrap_or(Affine::IDENTITY),
            element,
            dirty: true,
            origin: (0.0, 0.0),
        }));
        self.painted.insert(node, Arc::clone(&painted));
        if let Some(layer) = &self.overlay {
            layer.add(Arc::clone(&painted));
        }
        self.doc.set_custom_widget(
            node,
            Box::new(crate::scene::SceneWidget {
                painted,
                overlay: self.overlay.is_some(),
                events: self.events.clone(),
                resizes: Arc::clone(&self.resizes),
                size: None,
            }),
        );
        Ok(())
    }

    /// Gives the view the host window's input method, clipboard and
    /// redraws ([`HostShell`]): a field in it can be composed into.
    pub fn set_host_shell(&mut self, shell: HostShell) {
        let window = Arc::clone(&shell.window);
        let alarm = Arc::new(architect_scene::wake::Alarm::new(Arc::new(move || {
            window.request_redraw();
        })));
        for painted in self.painted.values() {
            painted.lock().unwrap_or_else(PoisonError::into_inner).alarm = Some(Arc::clone(&alarm));
        }
        self.alarm = Some(alarm);
        self.watch_engine();
        self.doc.set_shell_provider(Arc::new(shell));
    }

    /// The element a press holds the pointer on now, if any.
    #[must_use]
    pub const fn held(&self) -> Option<u64> {
        self.held
    }

    /// Sizes it to its content, as an inline embed: the page as tall as
    /// what is in it rather than the viewport, so
    /// [`Self::content_height`] says how tall to make its box. Once is
    /// enough.
    pub fn fit_content(&mut self) {
        if self.fitted {
            return;
        }
        let head = self
            .doc
            .get_node(self.theme)
            .and_then(|text| text.parent)
            .and_then(|style| self.doc.get_node(style))
            .and_then(|style| style.parent);
        let Some(head) = head else { return };
        let mut m = self.doc.mutate();
        let style = m.create_element(qual("style", None), Vec::new());
        let text = m.create_text_node(FIT_CSS);
        m.append_children(style, &[text]);
        m.append_children(head, &[style]);
        drop(m);
        self.fitted = true;
    }

    /// How tall its content is as last laid out, points: its page's
    /// height. Sized to its content ([`Self::fit_content`]), that is the
    /// height to give its box.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.doc.root_element().final_layout().size.height
    }

    /// Sets the host's theme for the view: CSS (the host's `:root`
    /// custom properties, `architect_theme::RootCss`), so the view's
    /// `var(--…)` follow the app's theme live. A sub-document doesn't see
    /// the host's own sheets.
    pub fn set_theme_css(&mut self, css: &str) {
        self.doc.mutate().set_node_text(self.theme, css);
    }

    /// A press on `node` focuses the nearest element at or above it that
    /// takes the focus, the old one hearing `blur` and the new `focus`, as
    /// Blitz does for a press it sees. A press a hit region takes never
    /// reaches Blitz, so without this the view's keys went to whatever had
    /// the focus before (or nowhere). Whether the focus moved.
    pub fn focus_pressed(&mut self, node: NodeId) -> bool {
        let mut at = Some(node);
        let target = loop {
            let Some(id) = at else { return false };
            let Some(n) = self.doc.get_node(id) else {
                return false;
            };
            if n.is_focussable() {
                break id;
            }
            at = n.parent;
        };
        let old = self.doc.get_focussed_node_id();
        if old == Some(target) || !self.doc.set_focus_to(target) {
            return false;
        }
        let forward = Forward {
            listeners: &self.listeners,
            events: &self.events,
            held: &mut self.held,
            composing: &mut self.composing,
        };
        let mut driver = EventDriver::new(&mut self.doc, forward);
        if let Some(old) = old {
            driver.handle_dom_event(DomEvent::new(old, DomEventData::Blur(BlitzFocusEvent)));
            driver.handle_dom_event(DomEvent::new(old, DomEventData::FocusOut(BlitzFocusEvent)));
        }
        driver.handle_dom_event(DomEvent::new(target, DomEventData::Focus(BlitzFocusEvent)));
        driver.handle_dom_event(DomEvent::new(
            target,
            DomEventData::FocusIn(BlitzFocusEvent),
        ));
        self.sync_field();
        true
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
            Frame::Batches { edits, batches } => {
                for batch in batches {
                    if self.batches.insert(batch.n, batch.bytes).is_some() {
                        return Err(format!("layers batch {} twice in a frame", batch.n));
                    }
                }
                edits
            }
            Frame::Failed(why) => return Err(why),
        };
        if edits.len() > MAX_EDITS {
            return Err(format!("a frame of {} edits is too large", edits.len()));
        }
        for edit in edits {
            self.edit(edit)?;
        }
        // Every batch is named by an edit of its frame, where it applies.
        if !self.batches.is_empty() {
            let mut left: Vec<u64> = self.batches.drain().map(|(n, _)| n).collect();
            left.sort_unstable();
            return Err(format!(
                "layers batches {left:?} that no edit of their frame named"
            ));
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
        // A scene set before its element had an id: its size goes there now.
        if let Some(painted) = self.painted.get(&node) {
            painted
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .element = id;
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
            self.holds.remove(&n);
            if let (Some(painted), Some(layer)) = (self.painted.remove(&n), &self.overlay) {
                layer.remove(&painted);
            }
        }
    }

    /// Whether `node` is a `<style>` element.
    fn is_style(&self, node: NodeId) -> bool {
        self.doc
            .get_node(node)
            .and_then(Node::element_data)
            .is_some_and(|e| &*e.name.local == "style")
    }

    /// A `<style>`'s raw CSS (`dangerous_inner_html`, as a view writes
    /// `style { dangerous_inner_html: CSS }`): its one text child, as
    /// `style { "{CSS}" }` would make it. It is the view's own sheet in its
    /// own document, never markup.
    fn set_style_text(&mut self, node: NodeId, css: &str) {
        let kids = self
            .doc
            .get_node(node)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        for kid in kids {
            self.forget(kid);
        }
        let mut m = self.doc.mutate();
        m.remove_and_drop_all_children(node);
        let text = m.create_text_node(css);
        m.append_children(node, &[text]);
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
                if name == crate::embedded::VIEW_ATTR || name == crate::embedded::INPUT_ATTR {
                    let text = match &value {
                        Value::Text(t) => Some(t.as_str()),
                        _ => None,
                    };
                    return self.held_attr(node, &name, text);
                }
                if name == crate::scene::LAYERS_ATTR {
                    let text = match &value {
                        Value::Text(t) => Some(t.as_str()),
                        _ => None,
                    };
                    return self.layers_attr(node, text);
                }
                if name == crate::scene::SCENE_ATTR || name == crate::scene::TRANSFORM_ATTR {
                    let text = match &value {
                        Value::Text(t) => Some(t.as_str()),
                        _ => None,
                    };
                    return self.paint_attr(node, &name, text);
                }
                let value = match value {
                    Value::None => None,
                    Value::Bool(false) if name == "checked" => None,
                    Value::Text(t) => Some(t),
                    Value::Float(f) => Some(f.to_string()),
                    Value::Int(i) => Some(i.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                };
                if name == "dangerous_inner_html" && self.is_style(node) {
                    self.set_style_text(node, value.as_deref().unwrap_or_default());
                    return Ok(());
                }
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
        // A wheel's points in the view's first: its hit regions and its
        // handlers both read them there.
        let event = localised_wheel(event);
        // Its scenes' hit regions first: a press or a drag on one is the
        // host's, and the view hears what its regions heard instead.
        let (consumed, cursor) = self.regions_event(&event);
        if consumed {
            // A press a region took still focuses the view it landed on, as
            // a press anywhere else in it does: its keys come next.
            if let UiEvent::PointerDown(p) = &event
                && let Some((node, _)) = self.painted_at(p.coords.client_x, p.coords.client_y)
            {
                let _ = self.focus_pressed(node);
            }
            self.region_cursor(cursor);
            return;
        }
        // A pointer that comes back, or leaves, with no button held lost
        // its release where the view couldn't see it: what it held is
        // let go.
        if let UiEvent::PointerMove(p) | UiEvent::PointerLeave(p) = &event
            && p.buttons.is_empty()
            && self.held.is_some()
        {
            let p = p.clone();
            self.let_go(Some(&p));
        }
        let releases = matches!(event, UiEvent::PointerUp(_) | UiEvent::PointerCancel(_));
        let forward = Forward {
            listeners: &self.listeners,
            events: &self.events,
            held: &mut self.held,
            composing: &mut self.composing,
        };
        let pointer = matches!(
            event,
            UiEvent::PointerMove(_) | UiEvent::PointerUp(_) | UiEvent::PointerLeave(_)
        );
        EventDriver::new(&mut self.doc, forward).handle_ui_event(event);
        self.sync_field();
        if releases {
            self.held = None;
        }
        if pointer {
            self.region_cursor(cursor);
        }
    }
}

/// A wheel's points in the view's document: Blitz moves a pointer
/// event's client and page points into a sub-document's space but not a
/// wheel's, whose `element` point (the host's, in the view's box) is
/// where it is in the view.
fn localised_wheel(event: UiEvent) -> UiEvent {
    match event {
        UiEvent::Wheel(mut w) => {
            let (dx, dy) = (
                w.coords.client_x - w.element.x,
                w.coords.client_y - w.element.y,
            );
            w.coords.client_x -= dx;
            w.coords.client_y -= dy;
            w.coords.page_x -= dx;
            w.coords.page_y -= dy;
            UiEvent::Wheel(w)
        }
        other => other,
    }
}

/// Sends the events the view has handlers for to the extension.
struct Forward<'a> {
    listeners: &'a HashMap<String, u32>,
    events: &'a mpsc::UnboundedSender<ViewEvent>,
    held: &'a mut Option<u64>,
    composing: &'a mut bool,
}

impl Forward<'_> {
    fn listens(&self, name: &str) -> bool {
        self.listeners.get(name).copied().unwrap_or(0) > 0
    }

    fn send(&self, element: u64, name: &str, bubbles: bool, data: EventData) {
        if !self.listens(name) {
            return;
        }
        // The extension bubbles it through its own tree.
        let _ = self.events.unbounded_send(ViewEvent {
            element,
            name: name.to_owned(),
            bubbles,
            data,
        });
    }
}

/// Whether events named `name` go to the element a press holds while it
/// is held, over it or not.
fn captured(name: &str) -> bool {
    matches!(
        name,
        "pointermove"
            | "pointerup"
            | "pointercancel"
            | "mousemove"
            | "mouseup"
            | "touchmove"
            | "touchend"
            | "touchcancel"
    )
}

impl EventHandler for Forward<'_> {
    fn handle_event(
        &mut self,
        chain: &[NodeId],
        event: &mut DomEvent,
        doc: &mut dyn Document,
        state: &mut EventState,
    ) {
        let name = event.name();
        // A view that drags with pointer events owns its held moves: the
        // document's own default (selecting the text under them) would
        // fight it. Only when it hears no mouse or touch moves, which a
        // cancelled pointer move would not send.
        if name == "pointermove"
            && self.held.is_some()
            && self.listens("pointermove")
            && !self.listens("mousemove")
            && !self.listens("touchmove")
        {
            state.prevent_default();
        }
        let target = {
            let doc = doc.inner();
            chain
                .iter()
                .find_map(|n| doc.get_node(*n).and_then(dioxus_id))
        };
        // A press holds the pointer on the element it landed on.
        if name == "pointerdown" {
            *self.held = target;
        }
        let target = match *self.held {
            Some(held) if captured(name) => Some(held),
            _ => target,
        };
        let Some(element) = target else {
            return;
        };
        if let DomEventData::Ime(ime) = &event.data {
            self.composition(element, ime);
            return;
        }
        // Focus leaving mid-composition ends it: the field won't hear the
        // rest.
        if matches!(name, "blur" | "focusout") && *self.composing {
            *self.composing = false;
            self.send(
                element,
                "compositionend",
                true,
                EventData::Composition(String::new()),
            );
        }
        if !self.listens(name) {
            return;
        }
        let Some(data) = event_data(&event.data) else {
            return;
        };
        self.send(element, name, event.bubbles, data);
    }
}

impl Forward<'_> {
    /// An input method's event, as the web's composition events.
    fn composition(&mut self, element: u64, ime: &BlitzImeEvent) {
        match ime {
            BlitzImeEvent::Preedit(text, _) if !text.is_empty() => {
                if !*self.composing {
                    *self.composing = true;
                    self.send(
                        element,
                        "compositionstart",
                        true,
                        EventData::Composition(String::new()),
                    );
                }
                self.send(
                    element,
                    "compositionupdate",
                    true,
                    EventData::Composition(text.clone()),
                );
            }
            BlitzImeEvent::Commit(text) => {
                if !*self.composing {
                    self.send(
                        element,
                        "compositionstart",
                        true,
                        EventData::Composition(String::new()),
                    );
                }
                *self.composing = false;
                self.send(
                    element,
                    "compositionend",
                    true,
                    EventData::Composition(text.clone()),
                );
            }
            BlitzImeEvent::Disabled if *self.composing => {
                *self.composing = false;
                self.send(
                    element,
                    "compositionend",
                    true,
                    EventData::Composition(String::new()),
                );
            }
            _ => {}
        }
    }
}

/// The host window's shell, as a view's document sees it: its input
/// method, clipboard and redraws, the caret area moved from the view's
/// coordinates to the window's by where the pane is.
#[derive(Clone)]
pub struct HostShell {
    window: Arc<dyn ShellProvider>,
    /// The view's frames, asked of the window at most once a refresh
    /// (#840): a batch applied to its document asks for one, and so does
    /// the document itself after each change it makes.
    pacer: architect_workspace::pacing::Pacer,
    /// Where the pane is in the window, CSS pixels, and the view's last
    /// caret area while its input method is on (in its own pixels).
    state: Arc<Mutex<ShellState>>,
}

#[derive(Default)]
struct ShellState {
    at: (f32, f32),
    caret: Option<(f32, f32, f32, f32)>,
}

impl HostShell {
    /// The window's `shell`, for a pane at `x`, `y`.
    #[must_use]
    pub fn new(window: Arc<dyn ShellProvider>, x: f32, y: f32) -> Self {
        let redraw = Arc::clone(&window);
        Self {
            pacer: architect_workspace::pacing::Pacer::new(Arc::new(move || {
                redraw.request_redraw();
            })),
            window,
            state: Arc::new(Mutex::new(ShellState {
                at: (x, y),
                caret: None,
            })),
        }
    }

    /// The pane is at `x`, `y` now: a caret the input method shows moves
    /// with it.
    pub fn moved(&self, x: f32, y: f32) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.at == (x, y) {
            return;
        }
        state.at = (x, y);
        if let Some((cx, cy, w, h)) = state.caret {
            self.window.set_ime_cursor_area(cx + x, cy + y, w, h);
        }
    }
}

impl HostShell {
    /// What asks the window for the view's frames, paced to the display.
    #[must_use]
    pub const fn pacer(&self) -> &architect_workspace::pacing::Pacer {
        &self.pacer
    }
}

impl ShellProvider for HostShell {
    fn request_redraw(&self) {
        self.pacer.request();
    }

    fn set_cursor(&self, icon: Option<cursor_icon::CursorIcon>) {
        self.window.set_cursor(icon);
    }

    fn set_ime_enabled(&self, is_enabled: bool) {
        if !is_enabled {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .caret = None;
        }
        self.window.set_ime_enabled(is_enabled);
    }

    fn set_ime_cursor_area(&self, x: f32, y: f32, width: f32, height: f32) {
        let (dx, dy) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.caret = Some((x, y, width, height));
            state.at
        };
        self.window
            .set_ime_cursor_area(x + dx, y + dy, width, height);
    }

    fn request_clipboard_text(&self) -> Result<(), ClipboardError> {
        self.window.request_clipboard_text()
    }

    fn set_clipboard_text(&self, text: String) -> Result<(), ClipboardError> {
        self.window.set_clipboard_text(text)
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

fn pointer_data(p: &BlitzPointerEvent) -> PointerData {
    let (pointer_id, pointer_type) = match p.id {
        BlitzPointerId::Mouse => (1, "mouse"),
        BlitzPointerId::Pen => (2, "pen"),
        BlitzPointerId::Finger(n) => (
            i32::try_from(n).unwrap_or(i32::MAX - 10).saturating_add(10),
            "touch",
        ),
    };
    PointerData {
        at: pointer(p),
        pointer_id,
        pointer_type: pointer_type.to_owned(),
        is_primary: p.is_primary,
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        pressure: p.details.pressure as f32,
        tilt_x: p.details.tilt_x.into(),
        tilt_y: p.details.tilt_y.into(),
        twist: p.details.twist.into(),
    }
}

fn touch_point(p: &BlitzPointerEvent) -> TouchPoint {
    let id = match p.id {
        BlitzPointerId::Finger(n) => i32::try_from(n).unwrap_or(i32::MAX),
        BlitzPointerId::Mouse | BlitzPointerId::Pen => 0,
    };
    TouchPoint {
        id,
        client_x: p.coords.client_x.into(),
        client_y: p.coords.client_y.into(),
        page_x: p.coords.page_x.into(),
        page_y: p.coords.page_y.into(),
        screen_x: p.coords.screen_x.into(),
        screen_y: p.coords.screen_y.into(),
        force: p.details.pressure,
    }
}

fn touch_data(p: &BlitzPointerEvent, ended: bool) -> TouchData {
    let mut touches: Vec<TouchPoint> = p
        .active_pointers
        .borrow()
        .iter()
        .filter(|a| a.is_finger() && (!ended || a.id != p.id))
        .map(touch_point)
        .collect();
    if !ended && !touches.iter().any(|t| t.id == touch_point(p).id) {
        touches.push(touch_point(p));
    }
    TouchData {
        touches,
        changed: vec![touch_point(p)],
        mods: mods(p.mods),
    }
}

fn wheel_data(w: &BlitzWheelEvent) -> WheelData {
    // Blitz carries winit's sign (positive scrolls up); the wire, the
    // web's (positive scrolls down), as a browser sends it.
    let (delta_x, delta_y, delta_mode) = match w.delta {
        BlitzWheelDelta::Pixels(x, y) => (-x, -y, 0),
        BlitzWheelDelta::Lines(x, y) => (-x, -y, 1),
    };
    WheelData {
        at: Pointer {
            client_x: w.coords.client_x.into(),
            client_y: w.coords.client_y.into(),
            page_x: w.coords.page_x.into(),
            page_y: w.coords.page_y.into(),
            screen_x: w.coords.screen_x.into(),
            screen_y: w.coords.screen_y.into(),
            element_x: w.element.x.into(),
            element_y: w.element.y.into(),
            button: 0,
            buttons: w.buttons.bits().into(),
            mods: mods(w.mods),
        },
        delta_x,
        delta_y,
        delta_mode,
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
        DomEventData::PointerMove(p)
        | DomEventData::PointerDown(p)
        | DomEventData::PointerUp(p)
        | DomEventData::PointerCancel(p)
        | DomEventData::PointerEnter(p)
        | DomEventData::PointerLeave(p)
        | DomEventData::PointerOver(p)
        | DomEventData::PointerOut(p) => Some(EventData::Pointer(pointer_data(p))),
        DomEventData::TouchStart(p) | DomEventData::TouchMove(p) => {
            Some(EventData::Touch(touch_data(p, false)))
        }
        DomEventData::TouchEnd(p) | DomEventData::TouchCancel(p) => {
            Some(EventData::Touch(touch_data(p, true)))
        }
        DomEventData::Wheel(w) => Some(EventData::Wheel(wheel_data(w))),
        DomEventData::Scroll(s) => Some(EventData::Scroll(ScrollData {
            top: s.scroll_top,
            left: s.scroll_left,
            width: s.scroll_width,
            height: s.scroll_height,
            client_width: s.client_width,
            client_height: s.client_height,
        })),
        DomEventData::KeyPress(k) | DomEventData::KeyDown(k) | DomEventData::KeyUp(k) => {
            Some(EventData::Key(key(k)))
        }
        DomEventData::Input(i) => Some(EventData::Form(i.value.clone())),
        DomEventData::Focus(_)
        | DomEventData::Blur(_)
        | DomEventData::FocusIn(_)
        | DomEventData::FocusOut(_) => Some(EventData::Focus),
        DomEventData::Ime(_) | DomEventData::AppleStandardKeybinding(_) => None,
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

/// Where `node` is in its window, CSS pixels.
fn pane_at(doc: &BaseDocument, node: NodeId) -> (f32, f32) {
    doc.get_node(node).map_or((0.0, 0.0), |n| {
        let p = n.absolute_position(0.0, 0.0);
        (p.x, p.y)
    })
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
/// Whether two engine tables are the same one (or both none).
fn same_table(
    a: Option<&Arc<architect_scene::engine::EngineTable>>,
    b: Option<&Arc<architect_scene::engine::EngineTable>>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

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
    fit: Option<Fit>,
    overlay: Option<crate::scene::OverlayLayer>,
    engine: bool,
    mut slots: Signal<Vec<crate::embedded::Slot>>,
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
                if !replaced(&processes, &extension, &process) {
                    something_changes(None, &processes, &mut retry).await;
                }
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
        let wire = process.hello().map_or(crate::wire::WIRE_MIN, |h| h.wire);
        let answered = wire >= ANSWERED_WIRE;
        let asked: Rc<RefCell<VecDeque<Instant>>> = Rc::default();
        let (events, mut proxied) = mpsc::unbounded::<ViewEvent>();
        // Each event asked, to the frame loop's watch for an overdue one.
        let (asking, mut was_asked) = mpsc::unbounded::<()>();
        // Each frame, to the coalescer: the view answered one.
        let (answer, mut answers) = mpsc::unbounded::<()>();
        let forwarding = {
            let (to, asked) = (session.events.clone(), Rc::clone(&asked));
            spawn(async move {
                let mut coalescer = Coalescer::new();
                loop {
                    let ready = futures::select! {
                        event = proxied.next() => match event {
                            // What its wire can't carry, it never had.
                            Some(event) if event.data.since() > wire => continue,
                            // A view that answers nothing is sent all.
                            Some(event) if !answered => vec![event],
                            Some(event) => coalescer.push(event),
                            None => return,
                        },
                        _ = answers.next() => coalescer.answered(),
                    };
                    for event in ready {
                        if answered {
                            asked.borrow_mut().push_back(Instant::now());
                            let _ = asking.unbounded_send(());
                        }
                        if to.unbounded_send(event).is_err() {
                            return;
                        }
                    }
                }
            })
        };
        let to_view = events.clone();
        let shell = with_doc(&node, |doc, at| {
            let mut remote = RemoteDocument::new(events);
            remote.set_overlay(overlay.clone());
            // Granted the engine's values: its scenes read the table its
            // engine published (`architect_scene::engine`).
            if engine {
                remote.set_engine(architect_scene::engine::published(&extension));
            }
            if let Some(css) = theme {
                remote.set_theme_css(&css.peek());
            }
            if fit.as_ref().is_some_and(|f| f.sized) {
                remote.fit_content();
            }
            let (x, y) = pane_at(doc, at);
            let shell = HostShell::new(doc.shell_provider.clone(), x, y);
            remote.set_host_shell(shell.clone());
            doc.set_sub_document(at, Box::new(remote));
            shell
        })
        .await;
        // Its frames asked of the window at most once a refresh, when the
        // image they render into comes free (#840).
        let pacer = shell.pacer().clone();
        // Where the pane is, now and then: an input method's caret moves
        // with the pane, not only when a frame comes.
        let placing = {
            let (node, shell, extension) = (node.clone(), shell.clone(), extension.clone());
            spawn(async move {
                loop {
                    futures_timer::Delay::new(PLACE_EVERY).await;
                    // The engine's table, looked for again: an engine in a
                    // process of its own may start after the pane, or
                    // restart with a new table.
                    let table = engine
                        .then(|| architect_scene::engine::published(&extension))
                        .flatten();
                    let (pane, held) = with_doc(&node, |doc, at| {
                        let pane = pane_at(doc, at);
                        let held = remote(doc, at).map(|r| {
                            if engine && !same_table(r.engine.as_ref(), table.as_ref()) {
                                r.set_engine(table);
                            }
                            r.place_scenes(pane);
                            r.embedded()
                        });
                        (pane, held)
                    })
                    .await;
                    shell.moved(pane.0, pane.1);
                    if let Some(held) = held
                        && *slots.peek() != held
                    {
                        slots.set(held);
                    }
                }
            })
        };
        // An embed's input: now, and whenever it changes.
        if let Some(fit) = &fit {
            let _ = to_view.unbounded_send(embed_event(&fit.latest.borrow()));
            *fit.live.borrow_mut() = Some(to_view);
        }
        let mut failed = None;
        while let Some(frame) =
            next_frame(&mut session.frames, &asked, &mut was_asked, &mut timing).await
        {
            let from = asked.borrow_mut().pop_front();
            let applying = Instant::now();
            let _ = answer.unbounded_send(());
            // An event is always answered (wire 5), with no edits when the
            // view did not change: that answer is timed, not drawn.
            let changes = !matches!(&frame, Frame::Edits { edits } if edits.is_empty());
            // Its scenes decoded off the UI thread first.
            let (frame, decoded) = decode_scenes(frame).await;
            let applied = with_doc(&node, |doc, at| {
                let (x, y) = pane_at(doc, at);
                shell.moved(x, y);
                let applied = remote(doc, at).map_or_else(
                    || Err("the view was unmounted".to_owned()),
                    |r| {
                        r.predecoded(decoded);
                        let applied = r.apply(frame);
                        r.place_scenes((x, y));
                        applied
                    },
                );
                // The views it holds now (their boxes settle by the next
                // look, `PLACE_EVERY`).
                let held = remote(doc, at).map(|r| r.embedded());
                // A frame no input asked for (a save, live data, a worker's
                // news) still shows: the window is asked to draw again. An
                // empty one changes nothing on screen; a frame for it would
                // take the next present's place, and the frame that does
                // change it waits a refresh behind (#840).
                if changes {
                    pacer.request();
                }
                (applied, held)
            })
            .await;
            let (applied, held) = applied;
            if let Some(held) = held
                && *slots.peek() != held
            {
                slots.set(held);
            }
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
        placing.cancel();
        if !slots.peek().is_empty() {
            slots.set(Vec::new());
        }
        if let Some(fit) = &fit {
            fit.live.borrow_mut().take();
        }
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
        } else if !replaced(&processes, &extension, &process) {
            something_changes(Some(&process), &processes, &mut retry).await;
        }
    }
}

/// `frame`'s scenes decoded (`crate::scene::predecode`) on a thread of
/// their own, so a large one can't stall the window; a frame with none
/// is passed straight on.
async fn decode_scenes(
    frame: Frame,
) -> (Frame, Vec<(u64, Result<crate::scene::Commands, String>)>) {
    let has = matches!(&frame, Frame::Edits { edits } | Frame::Batches { edits, .. } if edits.iter().any(|e| matches!(
        e,
        Edit::SetAttribute { name, value: Value::Text(v), .. }
            if name == crate::scene::SCENE_ATTR && v.contains('\n')
    )));
    if !has {
        return (frame, Vec::new());
    }
    let (to, from) = futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("scene decode".to_owned())
        .spawn(move || {
            let decoded = match &frame {
                Frame::Edits { edits } | Frame::Batches { edits, .. } => {
                    crate::scene::predecode(edits)
                }
                Frame::Failed(_) => Vec::new(),
            };
            let _ = to.send((frame, decoded));
        });
    if spawned.is_err() {
        return (
            Frame::Failed("its scenes could not be decoded".to_owned()),
            Vec::new(),
        );
    }
    from.await.unwrap_or_else(|_| {
        (
            Frame::Failed("its scenes could not be decoded".to_owned()),
            Vec::new(),
        )
    })
}

/// Whether another process of `extension` has taken `process`'s place (a
/// reload starts the new one before the old one stops): follow it now,
/// as the change it made was before anything here listened.
fn replaced(processes: &Processes, extension: &str, process: &Process) -> bool {
    processes.get(extension).is_some_and(|p| p != *process)
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
///
/// Its scenes (`crate::scene`) are clipped to their boxes unless
/// `overlay`: the extension was granted `overlay` (`capabilities.ui`) and
/// may draw over the whole app. `engine`: it was granted `engine`, and
/// its scenes read the values its engine writes lock-free.
///
/// `input`, when given, reaches the view as its `linked::use_embed` does
/// an embed's, now and on every change: a pane's is its state
/// (`linked::PANE_CLAIM`, read with `linked::use_pane_state`).
#[component]
pub fn RemoteView(
    processes: Processes,
    extension: String,
    view: String,
    #[props(default)] overlay: bool,
    #[props(default)] input: Option<Signal<crate::contrib::EmbedInput>>,
    #[props(default)] engine: bool,
) -> Element {
    // Keyed by what it shows (a one-item list: a key on a lone node does
    // not remount it): a pane that goes on to another process view (one
    // tab replaced by the next in the same group) mounts that view, not
    // the session the first one opened.
    let key = format!("{extension}/{view}");
    rsx! {
        for key in [key] {
            Mount { key: "{key}", processes: processes.clone(), extension: extension.clone(), view: view.clone(), input, max_height: None, overlay, engine }
        }
    }
}

/// [`RemoteView`] in a workspace pane: told the pane's id and state
/// (`linked::PANE_CLAIM`), now and whenever the state changes, so a view
/// opened on a file (an opener's) knows which.
#[component]
pub fn RemotePaneView(
    processes: Processes,
    extension: String,
    view: String,
    #[props(default)] overlay: bool,
    #[props(default)] engine: bool,
) -> Element {
    let pane = architect_workspace::try_use_pane();
    let now = crate::linked::pane_input(
        pane.as_ref().map(|p| p.id.0.as_str()).unwrap_or_default(),
        &pane
            .as_ref()
            .map(architect_workspace::PaneHandle::state)
            .unwrap_or_default(),
    );
    let mut input = use_signal(|| now.clone());
    if *input.peek() != now {
        input.set(now);
    }
    rsx! { RemoteView { processes, extension, view, overlay, engine, input: Some(input) } }
}

/// A process extension's view `view` inline in a host's document, as an
/// embed (`crate::embeds::EmbedView` mounts it): as wide as its parent,
/// as tall as its content up to `max_height` points (then it scrolls),
/// given `input` (its `linked::use_embed`) now and on every change.
#[component]
pub fn RemoteEmbed(
    processes: Processes,
    extension: String,
    view: String,
    input: Signal<crate::contrib::EmbedInput>,
    #[props(default = crate::embeds::DEFAULT_MAX_HEIGHT)] max_height: u32,
) -> Element {
    let key = format!("{extension}/{view}");
    rsx! {
        for key in [key] {
            Mount { key: "{key}", processes: processes.clone(), extension: extension.clone(), view: view.clone(), input: Some(input), max_height: Some(max_height), overlay: false, engine: false }
        }
    }
}

/// Extension `extension`'s view `view` as an overlay layer over another
/// view (`[[overlays]] draws`, `crate::overlay`): filling its parent (the
/// view's box), clipped to it, given `spaces` (the view's coordinates) as
/// its input whenever they change. Mount it in a box that passes the
/// pointer through (`pointer-events: none`) over the view.
#[component]
pub fn RemoteOverlay(
    processes: Processes,
    extension: String,
    view: String,
    /// The kind of view it draws over.
    over: String,
    spaces: crate::overlay::OverlaySpaces,
) -> Element {
    let input = crate::contrib::EmbedInput {
        claim: crate::overlay::claim(&over),
        source: String::new(),
        props: serde_json::to_value(&spaces).unwrap_or_default(),
    };
    let mut signal = use_signal(|| input.clone());
    if *signal.peek() != input {
        signal.set(input);
    }
    let key = format!("{extension}/{view}");
    rsx! {
        for key in [key] {
            Mount { key: "{key}", processes: processes.clone(), extension: extension.clone(), view: view.clone(), input: Some(signal), max_height: None, overlay: false, engine: false }
        }
    }
}

/// Another extension's view held in a remote view (`crate::embedded`):
/// linked in, drawn in the app's document; a process's or a component's,
/// as a remote view of its own, filling the box, given its input.
#[component]
fn HeldView(processes: Processes, slot: crate::embedded::Slot) -> Element {
    let crate::embedded::Slot {
        extension,
        view,
        input,
        ..
    } = slot;
    if let Some(ui) = try_use_context::<crate::interface::ExtensionUi>()
        && let Some(root) = ui.host().linked_view(&extension, &view)
    {
        return rsx! { crate::interface::LinkedOverlay { root, input } };
    }
    let mut signal = use_signal(|| input.clone());
    if *signal.peek() != input {
        signal.set(input);
    }
    let key = format!("{extension}/{view}");
    rsx! {
        for key in [key] {
            Mount { key: "{key}", processes: processes.clone(), extension: extension.clone(), view: view.clone(), input: Some(signal), max_height: None, overlay: false, engine: false }
        }
    }
}

/// An embed's input, shared between its component and its view's
/// session: the latest, and where to send it while a session is open.
#[derive(Clone)]
struct Fit {
    latest: Rc<RefCell<crate::contrib::EmbedInput>>,
    live: Rc<RefCell<Option<mpsc::UnboundedSender<ViewEvent>>>>,
    /// Sized to its content (an inline embed); an overlay fills its box.
    sized: bool,
}

/// The [`crate::wire::EMBED_EVENT`] that carries `input`.
fn embed_event(input: &crate::contrib::EmbedInput) -> ViewEvent {
    ViewEvent {
        element: 0,
        name: crate::wire::EMBED_EVENT.to_owned(),
        bubbles: false,
        data: EventData::Form(serde_json::to_string(input).unwrap_or_default()),
    }
}

/// The height an embed's box shows: its content's, at least
/// [`crate::embeds::MIN_HEIGHT`], at most `max`.
#[must_use]
pub fn embed_height(content: f32, max: u32) -> f32 {
    let min = crate::embeds::MIN_HEIGHT;
    let max = f32::from(u16::try_from(max).unwrap_or(u16::MAX));
    content.clamp(min, max.max(min))
}

/// Looks at the content height of the sub-document in `node` every
/// [`MEASURE_EVERY`], into `height` when it moves.
async fn measure(node: NodeHandle, mut height: Signal<f32>) {
    loop {
        let now = with_doc(&node, |doc, at| remote(doc, at).map(|r| r.content_height())).await;
        if let Some(h) = now
            && (h - *height.peek()).abs() > 0.5
        {
            height.set(h);
        }
        futures_timer::Delay::new(MEASURE_EVERY).await;
    }
}

/// A process view in a pane (filling it) or, given an input, inline as
/// an embed (content-sized).
#[component]
fn Mount(
    processes: Processes,
    extension: String,
    view: String,
    input: Option<Signal<crate::contrib::EmbedInput>>,
    max_height: Option<u32>,
    overlay: bool,
    engine: bool,
) -> Element {
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
    // An embed's input, to its session as it changes; its height, as
    // measured.
    let fit = use_hook(|| {
        input.map(|i| Fit {
            sized: max_height.is_some(),
            latest: Rc::new(RefCell::new(i.peek().clone())),
            live: Rc::default(),
        })
    });
    {
        let fit = fit.clone();
        use_effect(move || {
            let (Some(fit), Some(input)) = (fit.as_ref(), input) else {
                return;
            };
            let now = input();
            if *fit.latest.borrow() == now {
                return;
            }
            if let Some(to) = fit.live.borrow().as_ref() {
                let _ = to.unbounded_send(embed_event(&now));
            }
            *fit.latest.borrow_mut() = now;
        });
    }
    let height = use_signal(|| 0.0_f32);
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
    // Other extensions' views it holds, where their placeholders are.
    let slots = use_signal(Vec::<crate::embedded::Slot>::new);
    // A view that may draw over the whole app: its scenes' layer, drawn by
    // a widget filling the window (`scene::OverlayLayer`).
    let layer = use_hook(|| overlay.then(crate::scene::OverlayLayer::new));
    let over = use_hook(|| {
        layer
            .as_ref()
            .map(|l| dioxus_native_dom::CustomWidgetAttr::new(l.widget()))
    });
    let follow_with = {
        let (processes, extension) = (processes.clone(), extension.clone());
        move |node: NodeHandle| {
            if let Some(retry) = retry.borrow_mut().take() {
                spawn(follow(
                    node.clone(),
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
                    fit.clone(),
                    layer.clone(),
                    engine,
                    slots,
                ));
                if fit.as_ref().is_some_and(|f| f.sized) {
                    spawn(measure(node, height));
                }
            }
        }
    };
    let holding = processes.clone();
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
    // An overlay layer (`RemoteOverlay`) says nothing over the view it
    // draws on while its process starts or is gone: it is simply absent.
    let layer_only = input.is_some() && max_height.is_none();
    let placeholder = match shown() {
        _ if layer_only => None,
        Shown::Live => None,
        Shown::Waiting(why) => Some((why, false)),
        Shown::Stopped(why) => Some((why, true)),
    };
    // A pane fills its parent; an embed is as tall as its content (a
    // placeholder's line while it has none).
    let tall = match (max_height, &placeholder) {
        (None, _) => "100%".to_owned(),
        (Some(_), Some(_)) => "72px".to_owned(),
        (Some(max), None) => format!("{}px", embed_height(height(), max)),
    };
    // A view that may draw over the whole app is not clipped by its pane.
    let clip = if overlay { "visible" } else { "hidden" };
    rsx! {
        div { style: "position: relative; width: 100%; height: {tall}; overflow: {clip};",
            div {
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
                // The pane lost the focus (another pane, the window):
                // whatever the view held is let go.
                onblur: move |_| {
                    if let Some(node) = node_at.peek().clone() {
                        spawn(async move {
                            with_doc(&node, |doc, at| {
                                if let Some(r) = remote(doc, at) {
                                    r.let_go(None);
                                    // Its field's keys are not the app's
                                    // to leave alone any more.
                                    r.field.set(false);
                                }
                            })
                            .await;
                        });
                    }
                },
                onmounted: move |e: MountedEvent| {
                    if !mounted() && let Some(node) = e.data().downcast::<NodeHandle>() {
                        mounted.set(true);
                        node_at.set(Some(node.clone()));
                        follow_with(node.clone());
                    }
                },
            }
            // The views it holds, each over its placeholder's box, in front
            // of the view so they take their own input.
            for slot in slots() {
                div {
                    key: "{slot.key}",
                    class: "remote-view-held",
                    style: "position: absolute; left: {slot.x}px; top: {slot.y}px; \
                            width: {slot.width}px; height: {slot.height}px; overflow: hidden;",
                    HeldView { processes: holding.clone(), slot: slot.clone() }
                }
            }
            if let Some(over) = over {
                // Over the whole window, above the panes; no input.
                object {
                    class: "remote-view-overlay",
                    style: "position: fixed; left: 0; top: 0; width: 100vw; height: 100vh; \
                            pointer-events: none; z-index: 2147483647;",
                    data: over,
                }
            }
            if busy() && placeholder.is_none() && !layer_only {
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

#[cfg(test)]
mod pacing_tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use super::{Arc, HostShell, ShellProvider};

    struct Counting(Arc<AtomicU32>);

    impl ShellProvider for Counting {
        fn request_redraw(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// #840: a view's document asks its window for a frame after every
    /// change (Blitz's mutator does, and so does each batch applied). Asked
    /// right after a frame, they wait for the display's next refresh and
    /// share one frame, instead of each starting one that then waits for a
    /// free swapchain image with the window's thread blocked.
    #[test]
    fn a_views_redraws_right_after_a_frame_share_one_at_the_next_refresh() {
        if !architect_workspace::pacing::enabled() {
            return;
        }
        let asked = Arc::new(AtomicU32::new(0));
        let shell = HostShell::new(Arc::new(Counting(Arc::clone(&asked))), 0.0, 0.0);
        architect_workspace::pacing::set_refresh(Duration::from_micros(8_333));
        // A frame that waited for its image: it ended at a refresh.
        architect_workspace::pacing::frame_ended(Instant::now(), Duration::from_millis(7));
        for _ in 0..10 {
            shell.request_redraw();
        }
        assert_eq!(
            asked.load(Ordering::SeqCst),
            0,
            "a frame asked for before its refresh"
        );
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(asked.load(Ordering::SeqCst), 1, "ten changes, one frame");
    }
}
