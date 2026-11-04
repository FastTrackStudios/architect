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
//! `theme`, `log`, `call` — each call checked by the host against what
//! the extension may reach.
//!
//! There are no threads in a component, and no async yet (WASI 0.3): an
//! action runs to its end, a view's handlers run as their event comes. An
//! action that changes what a view shows calls [`refresh`]; the host asks
//! for the views' changes after every action and event.

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
use dioxus::prelude::Element;
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

/// What a component extension adds: its views and actions, and its state
/// across a reload.
#[derive(Default)]
pub struct Extension {
    views: BTreeMap<String, fn() -> Element>,
    actions: BTreeMap<String, Action>,
    load: Option<Box<dyn Fn(&[u8])>>,
    save: Option<Box<dyn Fn() -> Vec<u8>>>,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("views", &self.views.keys().collect::<Vec<_>>())
            .field("actions", &self.actions.keys().collect::<Vec<_>>())
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
    /// `failed`), as JSON.
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
        let mut writer = Writer::default();
        vdom.rebuild(&mut writer);
        PANES.with(|p| p.borrow_mut().insert(pane, vdom));
        Ok(writer.edits)
    }

    fn on_event(pane: u64, event: wit::ViewEvent) -> Result<Vec<wit::Edit>, String> {
        // Taken out while its handlers run: one that opens or closes a
        // view finds the others where they are.
        let mut vdom = PANES
            .with(|p| p.borrow_mut().remove(&pane))
            .ok_or_else(|| format!("no view open in pane {pane}"))?;
        dispatch(&vdom, event);
        let edits = settle(&mut vdom);
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
            let edits = settle(&mut vdom);
            PANES.with(|p| p.borrow_mut().insert(pane, vdom));
            if !edits.is_empty() {
                out.push(wit::PaneEdits { pane, edits });
            }
        }
        REFRESH.with(|r| r.set(false));
        out
    }

    fn close(pane: u64) {
        PANES.with(|p| p.borrow_mut().remove(&pane));
    }

    fn save_state() -> Option<Vec<u8>> {
        extension().ok()?.save.as_ref().map(|save| save())
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
    };
    vdom.runtime().handle_event(
        &event.name,
        Event::new(data, event.bubbles),
        ElementId::from_raw(element),
    );
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
