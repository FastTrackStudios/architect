//! What crosses between a host and a process extension's views
//! (features `process` and `guest`; `docs/content/architecture/extensions.md`,
//! "Views: Dioxus, rendered remotely").
//!
//! The extension runs its own `VirtualDom`; what it renders crosses as
//! Dioxus's edit stream ([`Edit`], one per `WriteMutations` call): a stack
//! machine over the host's nodes, which names a node by its Dioxus element
//! id only when it pushes it ([`Edit::PushId`]) or registers it
//! ([`Edit::SetId`]). The host applies the edits into a Blitz sub-document
//! of its own. Events go back as [`ViewEvent`]s, to
//! the handlers of the element they name.
//!
//! These are architect's types, not Dioxus's serde shapes, so the wire is
//! versioned here ([`WIRE`]) and checked at hello, whatever Dioxus does.
//! The host also sends the app's theme as data ([`ThemeData`]), beside the
//! CSS it gives a view's document.

use facet::Facet;

/// The wire's version.
///
/// 3 added the theme as data ([`ThemeData`], `ProcessExtension::theme`);
/// 4 an action's invocation (`ProcessExtension::run_in`) and what a
/// process adds at run time ([`Update`], `ProcessExtension::updates`); 5
/// a frame for every event ([`ANSWERED_WIRE`]); 6 a closed view's place
/// given back ([`VIEWS_WIRE`]); 7 rich input: pointer, touch, wheel,
/// scroll and composition events ([`INPUT_WIRE`]).
pub const WIRE: u32 = 7;

/// The oldest wire a host still speaks: a process at 2 runs as before,
/// its views themed through CSS only (it is never sent [`ThemeData`]).
/// A host refuses one outside `WIRE_MIN..=WIRE`.
pub const WIRE_MIN: u32 = 2;

/// The first wire a process is sent the theme on.
pub const THEMED_WIRE: u32 = 3;

/// The first wire a process is sent its actions' invocations on, and
/// asked for its [`Update`]s.
pub const CONTRIB_WIRE: u32 = 4;

/// The first wire a process answers every event with a frame.
///
/// The frame is empty when the event changed nothing; its host times each
/// frame from the event that asked for it, and marks the view busy while
/// one runs past its budget. An older process's view is timed by its
/// frames alone (never busy).
pub const ANSWERED_WIRE: u32 = 5;

/// The first wire a process gives a closed view's place back.
///
/// Each open view is a request in flight on the process's stream lane
/// (`ProcessExtension::view`, held open while the view lives), and vox
/// caps those per lane; the host opens the lane with room for
/// [`MAX_VIEWS`]. A process at this wire ends a view's request when the
/// view closes. An older one holds it for as long as it runs: its host
/// opens at most [`MAX_VIEWS`] of its views over its connection's life,
/// closed ones counted.
pub const VIEWS_WIRE: u32 = 6;

/// The first wire a process is sent rich input on.
///
/// That is [`EventData::Pointer`], [`EventData::Touch`],
/// [`EventData::Wheel`], [`EventData::Scroll`] and
/// [`EventData::Composition`]. An older process is sent none of them
/// ([`EventData::since`]); it never had them, so nothing it does changes.
pub const INPUT_WIRE: u32 = 7;

/// The most views one process extension shows at once (each is a thread
/// in the process). One more is refused with an error that says so;
/// closing one makes room.
pub const MAX_VIEWS: usize = 256;

/// The event a host sends a view's root (element 0) when it draws it as
/// an inline embed, and again when the block changes.
///
/// Its data is a form value, the `contrib::EmbedInput` as JSON. A guest keeps it for
/// `linked::use_embed` instead of dispatching it; an older guest has no
/// handler on the root and ignores it, answering with an empty frame.
pub const EMBED_EVENT: &str = "architect-embed";

/// What a page that draws an extension's view (`web::Pane`, and the npm
/// package's `view.js`) sets on each link (`<a>`) itself: it opens apart
/// from the page, which it can neither reach nor send its address to.
pub const LINK_FORCED: [(&str, &str); 2] = [("rel", "noopener noreferrer"), ("target", "_blank")];

/// Whether attribute `name` of element `tag` is one the page forces
/// ([`LINK_FORCED`]): a view's edit neither changes nor removes it.
#[must_use]
pub fn forced_attribute(tag: &str, name: &str) -> bool {
    tag.eq_ignore_ascii_case("a")
        && LINK_FORCED
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(name))
}

/// What a process adds or changes at run time, as it happens (wire 4 on):
/// its contributions, in place of the ones it sent before, or its runtime
/// state.
///
/// JSON in the manifest's own shapes (`crate::contrib`), checked by the host
/// against its manifest as a manifest's tables are.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Update {
    /// A `Contribution` as JSON: everything it adds at run time now.
    Contribute { json: String },
    /// A `StateWrite` as JSON (its source is the extension's, whatever it
    /// says).
    State { json: String },
    /// Clears one key of its runtime state, or all of it (`None`).
    Clear { key: Option<String> },
}

/// The app's theme, as data: what a process that paints itself (on the
/// GPU, or working out colours) themes with. Sent as it connects and on
/// every change (wire 3 on).
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct ThemeData {
    /// The theme showing.
    pub id: String,
    /// `light` or `dark`.
    pub scheme: String,
    /// The app's theme revision: it moves on every change.
    pub revision: u64,
    /// Every resolved token (architect's and the app's dotted ones), its
    /// final CSS value.
    pub tokens: Vec<Token>,
}

/// One resolved token.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Token {
    pub name: String,
    pub value: String,
}

impl ThemeData {
    /// The theme `resolved`, to send.
    #[must_use]
    pub fn from_resolved(resolved: &architect_theme::Resolved) -> Self {
        Self {
            id: resolved.id().to_owned(),
            scheme: resolved.scheme().as_str().to_owned(),
            revision: resolved.revision(),
            tokens: resolved
                .tokens()
                .iter()
                .map(|(name, value)| Token {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
        }
    }

    /// The theme, received: `color`, `number`, `value` as the host reads
    /// them.
    #[must_use]
    pub fn to_resolved(&self) -> architect_theme::Resolved {
        let scheme = if self.scheme == "light" {
            architect_theme::Scheme::Light
        } else {
            architect_theme::Scheme::Dark
        };
        architect_theme::Resolved::new(
            &self.id,
            scheme,
            self.tokens
                .iter()
                .map(|t| (t.name.clone(), t.value.clone()))
                .collect(),
        )
        .at_revision(self.revision)
    }
}

/// What a process says when the host connects.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Hello {
    /// The extension API it was built against (`crate::API`).
    pub api: String,
    /// The remote-view wire it speaks ([`WIRE`]).
    pub wire: u32,
    /// The views it can render, by id.
    pub views: Vec<String>,
    /// The actions it runs, by id.
    pub actions: Vec<String>,
}

/// A view opened in a pane.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Open {
    /// The view's id, from the manifest.
    pub view: String,
    /// The host's number for this mount: what its events name.
    pub pane: u64,
}

/// What a view sends: its edits since the last frame, or why it stopped.
#[derive(Clone, Debug, PartialEq, Facet)]
#[repr(u8)]
pub enum Frame {
    /// The edits, in order.
    Edits { edits: Vec<Edit> },
    /// It cannot render (no such view, or its component failed).
    Failed(String),
}

/// An attribute's value.
#[derive(Clone, Debug, PartialEq, Facet)]
#[repr(u8)]
pub enum Value {
    /// Removes it.
    None,
    Text(String),
    Float(f64),
    Int(i64),
    Bool(bool),
}

/// One of Dioxus's `WriteMutations`, as data: a stack machine. Ids are
/// Dioxus's element ids; `m` counts nodes on the writer's stack, the
/// "top" is the last node pushed.
#[derive(Clone, Debug, PartialEq, Facet)]
#[repr(u8)]
pub enum Edit {
    /// Push the node registered as `id`.
    PushId { id: u64 },
    /// Register the top node as `id`, leaving it on the stack.
    SetId { id: u64 },
    /// Replace the top node with its `index`th child.
    Child { index: u32 },
    /// Drop the top node from the stack (not from the document).
    Pop,
    /// Make an element and push it.
    CreateElement {
        tag: String,
        namespace: Option<String>,
    },
    /// Make a text node and push it.
    CreateText { value: String },
    /// Replace the top node with a deep copy of itself.
    Clone,
    /// Append the top `m` nodes to the node below them, which stays.
    AppendChildren { m: u32 },
    /// Put the top `m` nodes where the node below them is, and drop it.
    ReplaceWith { m: u32 },
    /// Put the top `m` nodes after the node below them, which stays.
    InsertAfter { m: u32 },
    /// Put the top `m` nodes before the node below them, which stays.
    InsertBefore { m: u32 },
    /// Set (or, with [`Value::None`], clear) an attribute of the top node.
    SetAttribute {
        name: String,
        namespace: Option<String>,
        value: Value,
    },
    /// Set the top node's text.
    SetText { value: String },
    /// The top node has a handler for `name` events.
    Listen { name: String },
    /// The top node lost a handler for `name` events.
    Unlisten { name: String },
    /// Take the top node off the stack and out of the document.
    Remove,
}

/// An event on one of a view's elements, for its handlers.
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct ViewEvent {
    /// The Dioxus element id it happened on.
    pub element: u64,
    /// Its name as a handler is registered (`click`, `keydown`, `input`).
    pub name: String,
    pub bubbles: bool,
    pub data: EventData,
}

/// What an event carries.
#[derive(Clone, Debug, PartialEq, Facet)]
#[repr(u8)]
pub enum EventData {
    /// `click`, `dblclick`, `contextmenu`, `mouse*`.
    Mouse(Pointer),
    /// `keydown`, `keyup`, `keypress`.
    Key(KeyData),
    /// `input`: the field's value now.
    Form(String),
    /// `focus`, `blur`, `focusin`, `focusout`.
    Focus,
    /// `pointer*` (wire 7 on).
    Pointer(PointerData),
    /// `touchstart`, `touchmove`, `touchend`, `touchcancel` (wire 7 on).
    Touch(TouchData),
    /// `wheel` (wire 7 on).
    Wheel(WheelData),
    /// `scroll`: where the element it happened on is scrolled to now
    /// (wire 7 on).
    Scroll(ScrollData),
    /// `compositionstart`, `compositionupdate`, `compositionend`: the
    /// text being composed by an input method, or the text it committed
    /// (wire 7 on).
    Composition(String),
}

impl EventData {
    /// The first wire it is sent on.
    #[must_use]
    pub const fn since(&self) -> u32 {
        match self {
            Self::Mouse(_) | Self::Key(_) | Self::Form(_) | Self::Focus => WIRE_MIN,
            Self::Pointer(_)
            | Self::Touch(_)
            | Self::Wheel(_)
            | Self::Scroll(_)
            | Self::Composition(_) => INPUT_WIRE,
        }
    }
}

/// A pointer event: where, and which pointer (`PointerEvent`'s fields).
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct PointerData {
    pub at: Pointer,
    /// The pointer's id: the mouse is 1, a pen 2, a finger 10 and up.
    pub pointer_id: i32,
    /// `mouse`, `pen` or `touch`.
    pub pointer_type: String,
    pub is_primary: bool,
    /// 0 to 1; 0.5 for a mouse with a button held.
    pub pressure: f32,
    /// Degrees, -90 to 90.
    pub tilt_x: i32,
    pub tilt_y: i32,
    /// Degrees, 0 to 359.
    pub twist: i32,
}

/// A touch event: every finger down, and those that changed.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct TouchData {
    pub touches: Vec<TouchPoint>,
    pub changed: Vec<TouchPoint>,
    pub mods: Mods,
}

/// One finger, in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Facet)]
pub struct TouchPoint {
    pub id: i32,
    pub client_x: f64,
    pub client_y: f64,
    pub page_x: f64,
    pub page_y: f64,
    pub screen_x: f64,
    pub screen_y: f64,
    /// 0 to 1.
    pub force: f64,
}

/// A wheel's turn, or a trackpad's swipe, signed as the web's: a
/// positive `delta_y` scrolls down.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct WheelData {
    pub at: Pointer,
    pub delta_x: f64,
    pub delta_y: f64,
    /// 0 pixels, 1 lines (the web's `deltaMode`; pages are sent as lines).
    pub delta_mode: u8,
}

/// Where an element is scrolled to, in CSS pixels.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct ScrollData {
    pub top: f64,
    pub left: f64,
    pub width: i32,
    pub height: i32,
    pub client_width: i32,
    pub client_height: i32,
}

/// Whether events named `name` come so fast that only the latest of a run
/// matters (the rest are [`Coalescer`]ed): moves, wheels, scrolls and a
/// composition's updates.
#[must_use]
pub fn coalesces(name: &str) -> bool {
    matches!(
        name,
        "pointermove" | "mousemove" | "touchmove" | "wheel" | "scroll" | "compositionupdate"
    )
}

/// Holds back the fast events ([`coalesces`]) while the view is still
/// answering one, so a view that draws slower than the pointer moves is
/// sent the latest move, not a queue of stale ones.
///
/// One event at a time is let through ([`Self::push`] while nothing is
/// in flight); what arrives meanwhile waits for [`Self::answered`]. A
/// run of fast events on one element under one name keeps only the last
/// (a wheel's deltas are summed); any other event (a press, a key) sends
/// what waits first, in order, so nothing is reordered or lost.
#[derive(Debug, Default)]
pub struct Coalescer {
    waiting: Vec<ViewEvent>,
    in_flight: usize,
    /// How many were dropped as stale, for a benchmark.
    pub merged: u64,
}

impl Coalescer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `event` arrived: what to send now (empty while it waits).
    // A wheel's deltas are summed as floats: no overflow to guard.
    #[allow(clippy::arithmetic_side_effects)]
    pub fn push(&mut self, event: ViewEvent) -> Vec<ViewEvent> {
        if !coalesces(&event.name) {
            let mut out = std::mem::take(&mut self.waiting);
            out.push(event);
            self.in_flight = self.in_flight.saturating_add(out.len());
            return out;
        }
        if self.in_flight == 0 && self.waiting.is_empty() {
            self.in_flight = self.in_flight.saturating_add(1);
            return vec![event];
        }
        match self.waiting.last_mut() {
            Some(last) if last.element == event.element && last.name == event.name => {
                let summed = match (&last.data, event.data) {
                    (EventData::Wheel(was), EventData::Wheel(mut now))
                        if was.delta_mode == now.delta_mode =>
                    {
                        now.delta_x += was.delta_x;
                        now.delta_y += was.delta_y;
                        EventData::Wheel(now)
                    }
                    (_, data) => data,
                };
                last.data = summed;
                last.bubbles = event.bubbles;
                self.merged = self.merged.saturating_add(1);
            }
            _ => self.waiting.push(event),
        }
        Vec::new()
    }

    /// The view answered one event: what to send now.
    pub fn answered(&mut self) -> Vec<ViewEvent> {
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.in_flight > 0 || self.waiting.is_empty() {
            return Vec::new();
        }
        let out = std::mem::take(&mut self.waiting);
        self.in_flight = self.in_flight.saturating_add(out.len());
        out
    }

    /// The view went: nothing is in flight now.
    pub fn reset(&mut self) {
        self.waiting.clear();
        self.in_flight = 0;
    }
}

/// Where a pointer was, in CSS pixels, and its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Facet)]
pub struct Pointer {
    pub client_x: f64,
    pub client_y: f64,
    pub page_x: f64,
    pub page_y: f64,
    pub screen_x: f64,
    pub screen_y: f64,
    /// Relative to the element it happened on.
    pub element_x: f64,
    pub element_y: f64,
    /// The button that changed (the web's `button`: 0 main, 1 middle, 2
    /// secondary).
    pub button: i16,
    /// The buttons held (the web's `buttons` bit set).
    pub buttons: u16,
    pub mods: Mods,
}

/// A key, as the web names it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet)]
pub struct KeyData {
    /// `a`, `Enter`, `ArrowUp`.
    pub key: String,
    /// The physical key: `KeyA`, `Enter`.
    pub code: String,
    /// 0 standard, 1 left, 2 right, 3 numpad.
    pub location: u8,
    pub repeat: bool,
    pub composing: bool,
    pub mods: Mods,
}

/// The modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Facet)]
pub struct Mods {
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

#[cfg(test)]
mod forced_tests {
    use super::{LINK_FORCED, forced_attribute};

    /// The third review of #539: a link in an extension's view must not
    /// navigate the page's own tab.
    #[test]
    fn a_links_rel_and_target_are_the_pages() {
        assert!(forced_attribute("a", "rel"));
        assert!(forced_attribute("A", "TARGET"));
        assert!(!forced_attribute("a", "href"));
        assert!(!forced_attribute("div", "target"));
        assert_eq!(
            LINK_FORCED,
            [("rel", "noopener noreferrer"), ("target", "_blank")]
        );
    }
}

#[cfg(test)]
mod coalescer_tests {
    use super::{Coalescer, EventData, Pointer, PointerData, ViewEvent, WheelData};

    fn ev(element: u64, name: &str, data: EventData) -> ViewEvent {
        ViewEvent {
            element,
            name: name.to_owned(),
            bubbles: true,
            data,
        }
    }

    fn mv(x: f64) -> ViewEvent {
        ev(
            1,
            "pointermove",
            EventData::Pointer(PointerData {
                at: Pointer {
                    client_x: x,
                    ..Pointer::default()
                },
                ..PointerData::default()
            }),
        )
    }

    fn x(e: &ViewEvent) -> f64 {
        match &e.data {
            EventData::Pointer(p) => p.at.client_x,
            _ => f64::NAN,
        }
    }

    #[test]
    fn moves_while_one_is_in_flight_wait_and_only_the_latest_is_sent() {
        let mut c = Coalescer::new();
        assert_eq!(c.push(mv(1.0)).len(), 1);
        for i in 2..=10 {
            assert!(c.push(mv(f64::from(i))).is_empty());
        }
        let sent = c.answered();
        assert_eq!(sent.len(), 1);
        assert!((x(&sent[0]) - 10.0).abs() < f64::EPSILON);
        assert_eq!(c.merged, 8);
        // Nothing left: the next answer sends nothing, the next move goes.
        assert!(c.answered().is_empty());
        assert_eq!(c.push(mv(11.0)).len(), 1);
    }

    #[test]
    fn a_press_sends_what_waits_first_in_order() {
        let mut c = Coalescer::new();
        let _ = c.push(mv(1.0));
        let _ = c.push(mv(2.0));
        let up = ev(1, "pointerup", EventData::Pointer(PointerData::default()));
        let sent = c.push(up);
        assert_eq!(
            sent.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["pointermove", "pointerup"]
        );
        assert!((x(&sent[0]) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_wheels_waiting_deltas_are_summed() {
        let wheel = |dy: f64| {
            ev(
                2,
                "wheel",
                EventData::Wheel(WheelData {
                    delta_y: dy,
                    ..WheelData::default()
                }),
            )
        };
        let mut c = Coalescer::new();
        let _ = c.push(wheel(10.0));
        let _ = c.push(wheel(10.0));
        let _ = c.push(wheel(5.0));
        let sent = c.answered();
        let EventData::Wheel(w) = &sent[0].data else {
            panic!("not a wheel")
        };
        assert!((w.delta_y - 15.0).abs() < f64::EPSILON);
    }
}
