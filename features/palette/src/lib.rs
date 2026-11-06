//! The palette and launcher engine (`docs/content/architecture/actions-everywhere.md`):
//! [`Source`]s give [`Item`]s, the [`Engine`] asks them all (or the one a
//! `kind:` prefix names) and puts what they gave in order, with [`fuzzy`]
//! matching, [`rank`]ing and [`Recents`]. No UI: the ⌘K Command Palette
//! (an extension, `features/palette/extension`), ⌘O open-anything
//! ([`Openers`]), an extension's contributed sources (`[[palette_sources]]`)
//! and the OS-wide launcher (`features/launcher`) all rank through it.
//!
//! MIT on its own (the rest of architect is MIT OR Apache-2.0), so any
//! launcher can take it.

mod fuzzy;
mod item;
mod open;
mod query;
mod rank;
mod recent;
mod source;

pub use fuzzy::{Match, fuzzy};
pub use item::{Effect, Item};
pub use open::{Found, OpenIn, OpenItem, Opener, Openers};
pub use query::{Query, Scope};
pub use rank::{Ranked, rank, rank_open, score_command, score_open};
pub use recent::Recents;
pub use source::{Engine, FnSource, Ranking, Source};

/// The palette's name for an [`Item`]: a command.
pub type Command = Item;
