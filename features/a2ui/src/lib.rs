//! A2UI in architect: an agent shows the person a UI — a form, a list,
//! cards, an approval — drawn with architect-ui, in any architect host
//! (`docs/content/features/a2ui.md`).
//!
//! The protocol is `a2ui-base`'s (A2UI v1.0: the messages, JSON-Pointer
//! data binding, client functions, validation), pinned exactly; this crate
//! owns only what must be ours:
//!
//! - the **catalog** ([`catalog`]), the security boundary: only its
//!   components are drawn, each by an architect-ui component, and its JSON
//!   Schema and system prompt are generated from it;
//! - the **state**: each surface lives in a Loro document ([`Surfaces`]),
//!   so it persists, syncs and can be co-edited;
//! - the **command path**: a surface's events are Action List calls
//!   ([`events`]), made as the surface's author and asking the person
//!   before every write;
//! - the `A2UI_*` actions ([`actions`]) that render, patch, read and press
//!   a surface from MCP, the command line or a script.
//!
//! Feature `ui` draws a surface on Blitz ([`ui::A2uiSurface`]) and gives a
//! workspace view of the surfaces ([`ui::surfaces_view`]).

pub mod actions;
pub mod catalog;
pub mod events;
pub mod store;
#[cfg(feature = "ui")]
pub mod ui;

pub use catalog::{CATALOG, CATALOG_ID, ComponentDef};
pub use events::Outcome;
pub use store::{Surface, SurfaceError, Surfaces};

/// The protocol crate, as this one pins it.
pub use a2ui_base;
