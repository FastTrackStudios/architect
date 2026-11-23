//! A process extension's Dioxus views, served to its host
//! (`architect_extensions::guest`, feature `guest`).
//!
//! ```ignore
//! use architect_extensions::guest::{Guest, GuestError};
//! use architect_extensions_dioxus::guest::GuestExt as _;
//!
//! fn main() -> Result<(), GuestError> {
//!     Guest::new()
//!         .view("counter", Counter)
//!         .action("acme.clicker.reset", |_args| async { Ok(serde_json::json!(0)) })
//!         .serve()
//! }
//! ```
//!
//! Each view the host opens gets its own `VirtualDom` on a thread of its
//! own ([`runner`]): its edits go to the host as frames ([`WireWriter`]),
//! the host's events come back to its handlers. A view whose component
//! panics takes down its thread only; the host sees its stream end.
//! [`use_theme`] gives a view the host's theme as a signal.

use std::any::Any;
use std::rc::Rc;
use std::str::FromStr as _;
use std::sync::Arc;

use architect::EventSink;
use architect_extensions::guest::{Guest, ViewRunner, theme_changes};
use architect_extensions::linked::{Extension, ViewRoot as AnyRoot};
use architect_extensions::wire::{
    Edit, EventData, Frame, KeyData, Mods, Pointer, Value, ViewEvent,
};
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
use tokio::sync::mpsc;

/// Serving Dioxus views from a process.
pub trait GuestExt: Sized {
    /// Its view `id` (a manifest's `[[views]]`), drawn by `root`.
    #[must_use]
    fn view(self, id: impl Into<String>, root: fn() -> Element) -> Self;

    /// `extension` served as a process: its views, its actions (told
    /// where they were invoked from) and what it adds past its manifest,
    /// sent as the host connects.
    #[must_use]
    fn extension(extension: &dyn Extension) -> Self;
}

impl GuestExt for Guest {
    fn view(self, id: impl Into<String>, root: fn() -> Element) -> Self {
        self.view_with(id, runner(root))
    }

    fn extension(extension: &dyn Extension) -> Self {
        Guest::extension_with(extension, &|root: &AnyRoot| {
            root.downcast_ref::<fn() -> Element>().copied().map(runner)
        })
    }
}

/// Serves `extension` as a process to the host that started it, until
/// it goes ([`GuestExt::extension`], then [`Guest::serve`]).
///
/// # Errors
///
/// No host started it, or the connection could not be made.
pub fn serve(extension: &dyn Extension) -> Result<(), architect_extensions::guest::GuestError> {
    <Guest as GuestExt>::extension(extension).serve()
}

/// What draws `root` for the host, in a `VirtualDom` of its own.
#[must_use]
pub fn runner(root: fn() -> Element) -> ViewRunner {
    Arc::new(move |events, sink: &EventSink<Frame>| render(root, events, sink))
}

/// A linked extension's root, as `architect_extensions::linked::Builder` keeps it, if it is a Dioxus one.
#[must_use]
pub fn component(root: &AnyRoot) -> Option<fn() -> Element> {
    root.downcast_ref::<fn() -> Element>().copied()
}

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
}

/// The host's theme in a view, as a signal: `None` until the first
/// arrives (from a host that sends none, never), then every change.
/// Reading it subscribes the component.
#[must_use]
pub fn use_theme() -> Option<Resolved> {
    use dioxus::prelude::{ReadableExt as _, WritableExt as _, use_future, use_signal};
    let mut theme = use_signal(Guest::theme);
    use_future(move || async move {
        let mut changes = theme_changes();
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
        // Under a root of its own that gives it what an app gives a
        // linked view: the theme store, and a document for its head.
        let mut vdom = VirtualDom::new(GuestRoot);
        vdom.provide_root_context(ViewRoot(root));
        // An inline embed's input (`linked::use_embed`): empty in a pane,
        // filled by the host's `EMBED_EVENT` when it mounts one.
        let embed = vdom.in_scope(dioxus::prelude::ScopeId::ROOT, || {
            dioxus::prelude::Signal::new(architect_extensions::contrib::EmbedInput::default())
        });
        vdom.provide_root_context(architect_extensions::linked::EmbedContext(embed));
        vdom.provide_root_context(architect_extensions::linked::InProcess);
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

/// The view a [`GuestRoot`] draws.
#[derive(Clone, Copy)]
struct ViewRoot(fn() -> Element);

/// What every view a process serves is drawn under: what the app gives a
/// linked view, so the same code works in both.
///
/// - The app's theme store (`architect_theme::try_use_theme`,
///   `use_resolved`), its theme the one the host last sent (`use_theme`).
/// - A document whose head is the view's own: a `document::Style` is a
///   `style` element drawn first, which its host applies as it applies any.
#[allow(non_snake_case)]
fn GuestRoot() -> Element {
    use dioxus::prelude::*;
    let handle = architect_theme::use_theme_store(|| {
        architect_theme::ThemeStore::new(architect_theme::Registry::builtin())
    });
    let mut resolved = handle.resolved();
    use_future(move || async move {
        let mut changes = theme_changes();
        loop {
            let now = changes.borrow_and_update().clone();
            if let Some(now) = now
                && *resolved.peek() != now
            {
                resolved.set(now);
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    });
    let styles = use_signal(Vec::<String>::new);
    use_context_provider(|| {
        let head: Rc<dyn dioxus::document::Document> = Rc::new(Head { styles });
        head
    });
    rsx! {
        for css in styles() {
            style { "{css}" }
        }
        Root {}
    }
}

/// The view itself, as a component of its own.
#[allow(non_snake_case)]
fn Root() -> Element {
    (dioxus::prelude::use_context::<ViewRoot>().0)()
}

/// A process view's document: its head's styles, drawn in it.
struct Head {
    styles: dioxus::prelude::Signal<Vec<String>>,
}

impl dioxus::document::Document for Head {
    fn eval(&self, js: String) -> dioxus::document::Eval {
        dioxus::document::NoOpDocument.eval(js)
    }

    fn create_head_element(
        &self,
        name: &str,
        _attributes: &[(&str, String)],
        contents: Option<String>,
    ) {
        use dioxus::prelude::{ReadableExt as _, WritableExt as _};
        // A stylesheet by `href` has nothing here to fetch it with.
        if name != "style" {
            return;
        }
        let Some(css) = contents.filter(|c| !c.trim().is_empty()) else {
            return;
        };
        let mut styles = self.styles;
        if !styles.peek().contains(&css) {
            styles.write().push(css);
        }
    }
}

/// The host's input for an inline embed, when `event` is one
/// ([`architect_extensions::wire::EMBED_EVENT`] on the root).
fn embed_input(event: &ViewEvent) -> Option<architect_extensions::contrib::EmbedInput> {
    if event.element != 0 || event.name != architect_extensions::wire::EMBED_EVENT {
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
        // A handler named for the mouse needs mouse data, whatever a
        // host sent (a browser's `click` is a `PointerEvent`): a pointer's
        // data there would panic dioxus-html's conversion.
        EventData::Pointer(p)
            if architect_extensions::wire::kind_of(&event.name)
                == architect_extensions::wire::DataKind::Mouse =>
        {
            Rc::new(PlatformEventData::new(Box::new(mouse(&p.at))))
        }
        EventData::Pointer(p) => Rc::new(PlatformEventData::new(Box::new(rich::pointer(&p)))),
        EventData::Touch(t) => Rc::new(PlatformEventData::new(Box::new(rich::touch(&t)))),
        EventData::Wheel(w) => Rc::new(PlatformEventData::new(Box::new(rich::wheel(&w)))),
        EventData::Scroll(s) => Rc::new(PlatformEventData::new(Box::new(rich::scroll(&s)))),
        EventData::Composition(text) => {
            Rc::new(PlatformEventData::new(Box::new(rich::composition(&text))))
        }
    };
    // A scene's size, and what its regions heard, from its host, are its
    // element's `input`.
    let name = if event.name == architect_extensions::scene::RESIZE_EVENT
        || event.name == architect_extensions::scene::REGION_EVENT
    {
        "input"
    } else {
        event.name.as_str()
    };
    vdom.runtime().handle_event(
        name,
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

    use architect_extensions::wire::{
        Pointer, PointerData, ScrollData, TouchData, TouchPoint, WheelData,
    };

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
                touches: vec![TouchPoint {
                    id: 1,
                    ..TouchPoint::default()
                }],
                changed: vec![TouchPoint {
                    id: 1,
                    ..TouchPoint::default()
                }],
                ..TouchData::default()
            });
            assert_eq!(t.touches().len(), 1);
            let w = wheel(&WheelData {
                at,
                delta_y: 120.0,
                ..WheelData::default()
            });
            assert!(
                matches!(w.delta(), dioxus::html::geometry::WheelDelta::Pixels(v) if (v.y - 120.0).abs() < f64::EPSILON)
            );
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

    /// What was written since the last take, as one frame: with the
    /// layers batches its edits name, as bytes, in a view that sends
    /// them so (`architect_extensions::wire::BYTES_WIRE`).
    pub fn take(&mut self) -> Frame {
        let edits = std::mem::take(&mut self.edits);
        let batches = architect_extensions::scene::outbox::take();
        if batches.is_empty() {
            return Frame::Edits { edits };
        }
        Frame::Batches {
            edits,
            batches: batches
                .into_iter()
                .map(|(n, bytes)| architect_extensions::wire::Batch { n, bytes })
                .collect(),
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

#[cfg(test)]
mod click_tests {
    use dioxus::html::{SerializedHtmlEventConverter, set_event_converter};
    use dioxus::prelude::*;

    use architect_extensions::wire::{
        Edit, EventData, Frame, PointerData as WirePointer, ViewEvent,
    };

    /// #720 H4: a browser sends `click` as a `PointerEvent`; a host
    /// that passed it on as pointer data must not panic `onclick`.
    #[test]
    fn a_click_carrying_pointer_data_reaches_onclick() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CLICKS: AtomicU32 = AtomicU32::new(0);
        fn app() -> Element {
            rsx! { button { onclick: move |e| {
                let _ = e.client_coordinates();
                CLICKS.fetch_add(1, Ordering::Relaxed);
            }, "b" } }
        }
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        let mut vdom = VirtualDom::new(app);
        let mut writer = super::WireWriter::default();
        vdom.rebuild(&mut writer);
        let id = match writer.take() {
            // The node a handler was put on: the `PushId` before `Listen`.
            Frame::Edits { edits } | Frame::Batches { edits, .. } => edits
                .windows(2)
                .find_map(|w| match w {
                    [Edit::PushId { id }, Edit::Listen { .. }] => Some(*id),
                    _ => None,
                })
                .expect("the button's id"),
            Frame::Failed(why) => panic!("{why}"),
        };
        super::dispatch(
            &vdom,
            ViewEvent {
                element: id,
                name: "click".to_owned(),
                bubbles: true,
                data: EventData::Pointer(WirePointer {
                    pointer_type: "mouse".to_owned(),
                    ..WirePointer::default()
                }),
            },
        );
        assert_eq!(CLICKS.load(Ordering::Relaxed), 1);
    }
}
