//! What crosses between a host and a process extension's views
//! (features `process` and `guest`; `docs/content/architecture/extensions.md`,
//! "Views: Dioxus, rendered remotely").
//!
//! The extension runs its own `VirtualDom`; what it renders crosses as
//! Dioxus's edit stream ([`Edit`], one per `WriteMutations` call), each
//! template sent once ([`WireTemplate`], before the first edit that loads
//! it) and named by a number after that. The host applies the edits into
//! a Blitz sub-document of its own. Events go back as [`ViewEvent`]s, to
//! the handlers of the element they name.
//!
//! These are architect's types, not Dioxus's serde shapes, so the wire is
//! versioned here ([`WIRE`]) and checked at hello, whatever Dioxus does.

use facet::Facet;

/// The wire's version: a host refuses a process that speaks another.
pub const WIRE: u32 = 1;

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
    /// Templates first (each only once per mount), then the edits.
    Edits {
        templates: Vec<WireTemplate>,
        edits: Vec<Edit>,
    },
    /// It cannot render (no such view, or its component failed).
    Failed(String),
}

/// A template, its tree flattened: nodes refer to their children by
/// index into `nodes`.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct WireTemplate {
    /// The number edits load it by.
    pub id: u32,
    pub nodes: Vec<WireNode>,
    /// The template's roots, as indices into `nodes`.
    pub roots: Vec<u32>,
}

/// One node of a template.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum WireNode {
    Element {
        tag: String,
        namespace: Option<String>,
        /// Its static attributes; dynamic ones arrive as edits.
        attrs: Vec<WireAttr>,
        children: Vec<u32>,
    },
    Text(String),
    /// Filled in at run time: a placeholder until an edit replaces it.
    Dynamic,
}

/// A static attribute of a template's element.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct WireAttr {
    pub name: String,
    pub value: String,
    pub namespace: Option<String>,
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

/// One of Dioxus's `WriteMutations`, as data. Ids are Dioxus's element
/// ids; `m` counts nodes on the writer's stack.
#[derive(Clone, Debug, PartialEq, Facet)]
#[repr(u8)]
pub enum Edit {
    AppendChildren {
        id: u64,
        m: u32,
    },
    AssignNodeId {
        path: Vec<u8>,
        id: u64,
    },
    CreatePlaceholder {
        id: u64,
    },
    CreateTextNode {
        value: String,
        id: u64,
    },
    LoadTemplate {
        template: u32,
        index: u32,
        id: u64,
    },
    ReplaceNodeWith {
        id: u64,
        m: u32,
    },
    ReplacePlaceholder {
        path: Vec<u8>,
        m: u32,
    },
    InsertAfter {
        id: u64,
        m: u32,
    },
    InsertBefore {
        id: u64,
        m: u32,
    },
    SetAttribute {
        name: String,
        namespace: Option<String>,
        value: Value,
        id: u64,
    },
    SetText {
        value: String,
        id: u64,
    },
    Listen {
        name: String,
        id: u64,
    },
    Unlisten {
        name: String,
        id: u64,
    },
    Remove {
        id: u64,
    },
    PushRoot {
        id: u64,
    },
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
}

/// Where a pointer was, in CSS pixels, and its buttons.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
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
