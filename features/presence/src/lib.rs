//! App presence (`docs/content/architecture/app-presence.md`): how a desk
//! app lives when its window is not on screen — the Dock icon (a badge, a
//! menu, progress, a bounce), a menu-bar or tray item, background mode
//! (closing the window keeps it running, with no Dock icon), reopen on a
//! Dock click, opening at login, and one instance. Every part of it is an
//! Action List entry, so ⌘K, the menus, MCP and tours reach them.
//!
//! (Not people's presence in a document: that is `crdt::presence`.)
//!
//! The default build is the model: what an app declares ([`Presence`],
//! its Dock menu and [`Tray`] as Action List menus, the [`Badge`]), the
//! window's [`Lifecycle`], the [`LoginItem`]'s files and a second
//! launch's [`Handoff`] — pure data, wasm-clean. Feature `ui` binds it
//! ([`AppPresence`]); `native` does the platforms' part (`AppKit` on macOS,
//! `StatusNotifierItem` and XDG autostart on Linux, the Run key on Windows)
//! and claims the one instance ([`single_instance`]).

pub mod instance;
pub mod lifecycle;
pub mod login;
mod model;

#[cfg(all(feature = "native", target_os = "linux"))]
mod linux;
#[cfg(all(feature = "native", target_os = "macos"))]
mod mac;
#[cfg(feature = "native")]
mod native;
#[cfg(feature = "ui")]
mod platform;
#[cfg(feature = "ui")]
mod ui;

pub use instance::Handoff;
pub use lifecycle::{Effect, Event, Lifecycle, Place};
pub use login::{Applied, LoginItem, NotInstalled, Os, Plan};
pub use model::{
    Attention, BACKGROUND_ARG, Badge, HIDE_WINDOW, KEEP_RUNNING_SETTING, MenuTree,
    OPEN_AT_LOGIN_SETTING, Presence, QUIT, SHOW_WINDOW, STATE, TOGGLE_BACKGROUND, TOGGLE_LOGIN,
    Tray, TrayClick, TrayIcon, progress,
};
#[cfg(feature = "native")]
pub use native::{Launch, single_instance};
#[cfg(feature = "ui")]
pub use platform::{Ask, Call, Events, Heard, Platform, Recorder, TrayView, UsePlatform};
#[cfg(feature = "ui")]
pub use ui::{
    AppPresence, PresenceHandle, PresenceState, close_requested, launched_in_background,
    settings_section, try_use_presence,
};
