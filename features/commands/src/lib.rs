//! Commands over the Action List (`docs/content/architecture/actions-everywhere.md`):
//! the ⌘K palette, ⌘O open-anything, the action list window, and custom
//! actions.
//!
//! The default build is the model: [`fuzzy`] matching, the palette's
//! [`Query`] and [`rank`], [`Recents`], the [`Openers`] registry, and
//! [`CustomAction`]s — pure data, wasm-clean. Feature `ui` draws them in
//! the shell: [`CommandPalette`], [`QuickOpen`] and [`ActionListView`],
//! with scoped actions a view registers while it is mounted
//! ([`use_command`]).

mod custom;
mod fuzzy;
#[cfg(feature = "native")]
mod native;
mod opener;
mod palette;
mod recent;
#[cfg(feature = "ui")]
mod ui;

pub use custom::{Condition, CustomAction, Runner, Step, StepFailed};
pub use fuzzy::{Match, fuzzy};
#[cfg(feature = "native")]
pub use native::WindowKeys;
pub use opener::{Found, OpenIn, OpenItem, Opener, Openers};
pub use palette::{Command, Query, Ranked, Scope, rank};
pub use recent::Recents;
#[cfg(feature = "ui")]
pub use ui::*;
