//! The page API, defined once: what a page (a TSX/JS file on `QuickJS`, a
//! Luau element script, a process, the web build) sees of the app.
//!
//! Two `#[architect::rpc]` services over facet types:
//!
//! - [`PageHost`]: the host service's subset a page has today — the
//!   Action List, settings, the theme, shared state, notifications —
//!   with `#[subscribe]` streams for theme changes, live data and worker
//!   messages.
//! - [`PageView`]: a page's view — the element tree it renders
//!   ([`Element`], drawn as the remote-view edit stream,
//!   `architect_extensions::wire::Edit`), overlays, and the events that
//!   come back to its handlers.
//!
//! In process (`QuickJS`, Luau) a runner binds these calls directly, with
//! no wire; under feature `vox` they are vox services, and `codegen`
//! writes `architect.d.ts` and `architect.d.luau` from their descriptors
//! (`crate::codegen`), so every runner stays in step.

// The in-process host awaits nothing it does not have to; vox asks for
// `Send` futures itself.
#![allow(async_fn_in_trait)]
// The `unsafe` clippy sees is `Facet`'s derive (its reflection), not ours.
#![allow(clippy::unsafe_derive_deserialize)]

use facet::Facet;
use serde::{Deserialize, Serialize};

/// Why a page's call did not do what it asked.
#[architect::error]
#[derive(Eq)]
pub enum PageError {
    /// It may not: its capabilities do not reach that far.
    #[error("{0}")]
    Denied(String),
    /// No such action, setting or worker.
    #[error("{0}")]
    Unknown(String),
    /// It ran and failed.
    #[error("{0}")]
    Failed(String),
}

/// One node of a page's element tree: an element (`div`, `button`,
/// `input`…) or, with `tag` `#text`, a text node.
///
/// Style goes through the theme: a page writes `var(--foreground)` (or
/// `$foreground` in a style object, which its runtime turns into that),
/// so it follows the app's theme live. Handlers are numbers its runtime
/// keeps ([`Listener`]); an event comes back naming one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
#[serde(default)]
pub struct Element {
    /// `div`, `span`, `button`, `input`, `img`…; `#text` for text.
    pub tag: String,
    /// What keeps it itself among its siblings as a list changes.
    pub key: Option<String>,
    /// A text node's text.
    pub text: Option<String>,
    /// Its attributes: `style` (CSS text), `class`, `value`, `placeholder`…
    pub attrs: Vec<Attr>,
    /// Its handlers.
    pub on: Vec<Listener>,
    pub children: Vec<Self>,
}

/// An attribute.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

/// A handler on an element: its event (`click`, `input`, `keydown`) and
/// the number its runtime keeps the function under.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct Listener {
    pub event: String,
    pub handler: u32,
}

/// What an event brings a handler.
#[derive(Clone, Debug, Default, PartialEq, Facet, Serialize, Deserialize)]
#[serde(default)]
pub struct PageEvent {
    /// The handler it is for.
    pub handler: u32,
    /// `click`, `input`, `keydown`…
    pub event: String,
    /// An `input`'s value now.
    pub value: Option<String>,
    /// A key event's key (`Enter`, `a`).
    pub key: Option<String>,
    /// Where a pointer was, relative to the element.
    pub x: f64,
    pub y: f64,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub shift: bool,
}

/// An overlay a page opens over the app: a dialog, a popup.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    /// Its id: opening one with the same id replaces it.
    pub id: String,
    /// `dialog` (centred, the page dimmed) or `popup` (at its corner).
    pub kind: String,
    pub title: String,
    pub body: Element,
}

/// One action, as a page lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct ActionSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: String,
    /// `read_only`, `mutating` or `destructive`.
    pub effect: String,
    pub enabled: bool,
}

/// The app's theme, as a page reads it: a snapshot, made again only
/// when its revision moves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct ThemeSnapshot {
    pub id: String,
    /// `light` or `dark`.
    pub scheme: String,
    /// Moves on every change (it wraps: compare, never order).
    pub revision: u32,
    /// Every resolved token, its final CSS value.
    pub tokens: Vec<ThemeToken>,
}

/// One theme token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct ThemeToken {
    pub name: String,
    pub value: String,
}

/// A notification a page shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
#[serde(default)]
pub struct Notice {
    pub text: String,
    /// `info`, `success`, `warning` or `error`.
    pub kind: String,
}

/// News on a live-data topic: a shared-state key, or a watched action's
/// new answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct DataEvent {
    pub topic: String,
    /// JSON.
    pub value: String,
}

/// A message from one of a page's workers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet, Serialize, Deserialize)]
pub struct WorkerMessage {
    /// The worker's handle.
    pub worker: u32,
    /// `message`, `done` or `failed`.
    pub kind: String,
    /// JSON: what it posted, returned, or why it failed.
    pub value: String,
}

pub use host::PageHost;
pub use view::PageView;

/// The host service's subset (`#[architect::rpc]` emits a module's worth
/// of items per trait, so each has a module of its own).
pub mod host {
    use super::{ActionSummary, DataEvent, Notice, PageError, ThemeSnapshot, WorkerMessage};

    /// What a page may reach of the app: the host service's subset that
    /// exists today (the full host service is #178's).
    #[architect::rpc]
    pub trait PageHost {
        /// Runs action `id` with JSON `args`; its answer, as JSON.
        async fn run(&self, id: String, args: String) -> Result<String, PageError>;
        /// The actions it may run, filtered by text.
        async fn actions(&self, filter: String) -> Result<Vec<ActionSummary>, PageError>;
        /// A setting's value, as JSON (`null` when unset).
        async fn setting(&self, key: String) -> Result<String, PageError>;
        /// Sets a setting to JSON `value`.
        async fn set_setting(&self, key: String, value: String) -> Result<(), PageError>;
        /// The theme showing.
        async fn theme(&self) -> Result<ThemeSnapshot, PageError>;
        /// A shared-state key's value, as JSON (`null` when unset): shared by
        /// every page of the app.
        async fn shared(&self, key: String) -> Result<String, PageError>;
        /// Sets a shared-state key to JSON `value`; every page watching it
        /// hears ([`Self::data`]).
        async fn set_shared(&self, key: String, value: String) -> Result<(), PageError>;
        /// Shows a notification.
        async fn notify(&self, notice: Notice) -> Result<(), PageError>;
        /// The theme, on every change.
        #[subscribe]
        fn theme_changes(&self) -> ThemeSnapshot;
        /// Live data: shared state as it changes.
        #[subscribe]
        fn data(&self) -> DataEvent;
        /// Its workers' messages.
        #[subscribe]
        fn workers(&self) -> WorkerMessage;
    }
}

/// A page's view.
pub mod view {
    use super::{Element, Overlay, PageError, PageEvent};

    /// A page's view: what it renders, and the events that come back.
    #[architect::rpc]
    pub trait PageView {
        /// Its element tree now: drawn as edits from the last.
        async fn render(&self, tree: Element) -> Result<(), PageError>;
        /// Opens (or replaces) an overlay.
        async fn open_overlay(&self, overlay: Overlay) -> Result<(), PageError>;
        /// Closes the overlay `id`.
        async fn close_overlay(&self, id: String) -> Result<(), PageError>;
        /// The events on its elements, for its handlers.
        #[subscribe]
        fn events(&self) -> PageEvent;
    }
}
