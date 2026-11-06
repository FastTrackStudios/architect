//! Bases: a vault's `.base` files drawn as views over its notes, as
//! Obsidian's Bases are (`docs/content/features/bases.md`).
//!
//! - [`model`]: a base as data, the edits made to it (filters, sort,
//!   group-by, properties, formulas, views, a type's options), and the
//!   view types there are ([`model::ViewTypes`]).
//! - `actions` (feature `actions`): the `BASES_*` actions over an open
//!   vault.
//! - `ui` (feature `ui`): a base as a workspace view (`ui::view`) with its
//!   toolbar and view picker; Bases' own table, cards and list; a type an
//!   extension adds (`[[base_views]]`, `linked::Builder::base_view`) drawn
//!   by it; and [`ui::Bases`], the linked extension that draws a note's
//!   ```` ```base ```` fence.
//!
//! The grouping a view's `groupBy` does is `vault::bases`' over the shared
//! `vault_proto::grouping` (re-exported as `vault_live::grouping`), which the navigator's list pane uses too.

pub mod model;

#[cfg(feature = "actions")]
pub mod actions;

#[cfg(feature = "ui")]
pub mod ui;
