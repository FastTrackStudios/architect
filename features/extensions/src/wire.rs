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
/// a frame for every event ([`ANSWERED_WIRE`]).
pub const WIRE: u32 = 5;

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
