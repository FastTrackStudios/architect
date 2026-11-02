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
//! binding editor), and [`ConfirmSheet`], where a person approves an
//! agent's destructive call (the Action List's confirmation tickets),
//! with scoped actions a view registers while it is
//! mounted ([`use_command`]), keys run through `architect-input`.

#[cfg(feature = "ui")]
mod bridge;
#[cfg(feature = "ui")]
mod confirm;
mod custom;
#[cfg(any(test, all(feature = "native", target_os = "linux")))]
mod dbusmenu;
mod fuzzy;
#[cfg(all(feature = "native", target_os = "ios"))]
mod ios_menu;
mod keymap;
#[cfg(feature = "ui")]
mod keys_ui;
#[cfg(all(feature = "native", target_os = "linux"))]
mod linux_menu;
mod menu;
#[cfg(feature = "ui")]
mod menu_view;
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

#[cfg(feature = "ui")]
pub use bridge::{CommandInfo, CommandList, CommandsState, Ran, RunCommand, use_command_actions};
#[cfg(feature = "ui")]
pub use confirm::ConfirmSheet;
pub use custom::{Condition, CustomAction, Runner, Step, StepFailed};
pub use fuzzy::{Match, fuzzy};
#[cfg(all(feature = "native", target_os = "ios"))]
pub use ios_menu::NativeMenus;
pub use keymap::{
    Binding, Clash, Conflict, EVERYWHERE, Keymap, Layer, PersonKeymap, Rebind, display_chord,
    display_keys, normalize_keys, parse_glyph_chord, parse_shortcut,
};
#[cfg(feature = "ui")]
pub use keys_ui::{BindingEditor, KeymapTools, WhichKeyPanel, use_keymap, use_palette_opener};
#[cfg(all(feature = "native", target_os = "linux"))]
pub use linux_menu::{DbusMenu, NativeMenus, Shared as DbusMenuState};
pub use menu::{
    Entry, Item, Kind, Live, Menu, MenuBar, Pick, Resolved, Row, Section, Standard, Tree,
    without_platform,
};
#[cfg(feature = "ui")]
pub use menu_view::InWindowMenus;
#[cfg(feature = "native")]
pub use native::WindowKeys;
#[cfg(all(feature = "native", target_os = "macos"))]
pub use native_menu::NativeMenus;
pub use opener::{Found, OpenIn, OpenItem, Opener, Openers};
pub use palette::{Command, Effect, Query, Ranked, Scope, rank};
pub use recent::Recents;
#[cfg(feature = "ui")]
pub use ui::*;

/// The Action List action that runs any command by id, a view's own too
/// ([`use_command_actions`] registers it; feature `ui`).
pub const RUN_ACTION: &str = "COMMANDS_RUN";

/// The read-only Action List action that lists every command with its
/// effect, enabled or not and why ([`use_command_actions`]; feature `ui`).
pub const LIST_ACTION: &str = "COMMANDS_LIST";

/// The read-only Action List action that says which overlay is open and
/// what the palette's field says ([`use_command_actions`]; feature `ui`).
pub const STATE_ACTION: &str = "COMMANDS_STATE";

/// A guide scenario's target in the commands' UI, as a CSS selector:
/// `palette-row` and a command's id is that command's row in the ⌘K
/// palette (`architect-guide`'s resolvers). `None` for any other kind.
#[must_use]
pub fn target_selector(kind: &str, name: &str) -> Option<String> {
    match kind {
        "palette-row" => Some(format!("[data-command=\"{name}\"]")),
        _ => None,
    }
}
