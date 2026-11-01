//! Commands over the Action List (`docs/content/architecture/actions-everywhere.md`):
//! the ⌘K palette, ⌘O open-anything, the action list window, and custom
//! actions.
//!
//! The default build is the model: [`fuzzy`] matching, the palette's
//! [`Query`] and [`rank`], [`Recents`], the [`Openers`] registry,
//! [`CustomAction`]s, the [`Keymap`] (the app's bindings with a person's
//! [`PersonKeymap`] over them) and an action's [`params`] asked step by
//! step — pure data, wasm-clean. Feature `ui` draws them in the shell:
//! [`CommandPalette`], [`QuickOpen`] and [`ActionListView`] (with its
//! binding editor), with scoped actions a view registers while it is
//! mounted ([`use_command`]), keys run through `architect-input`.

mod custom;
mod fuzzy;
mod keymap;
#[cfg(feature = "ui")]
mod keys_ui;
mod menu;
#[cfg(feature = "native")]
mod native;
#[cfg(all(feature = "native", target_os = "macos"))]
mod native_menu;
mod opener;
mod palette;
pub mod params;
mod recent;
#[cfg(feature = "ui")]
mod ui;

pub use custom::{Condition, CustomAction, Runner, Step, StepFailed};
pub use fuzzy::{Match, fuzzy};
pub use keymap::{
    Binding, Clash, Conflict, EVERYWHERE, Keymap, Layer, PersonKeymap, Rebind, display_chord,
    display_keys, normalize_keys, parse_glyph_chord, parse_shortcut,
};
#[cfg(feature = "ui")]
pub use keys_ui::{BindingEditor, KeymapTools, WhichKeyPanel, use_keymap, use_palette_opener};
pub use menu::{Entry, Item, Menu, MenuBar, Resolved, Standard};
#[cfg(feature = "native")]
pub use native::WindowKeys;
#[cfg(all(feature = "native", target_os = "macos"))]
pub use native_menu::NativeMenus;
pub use opener::{Found, OpenIn, OpenItem, Opener, Openers};
pub use palette::{Command, Query, Ranked, Scope, rank};
pub use recent::Recents;
#[cfg(feature = "ui")]
pub use ui::*;
