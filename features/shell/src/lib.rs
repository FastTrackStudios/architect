//! The app shell (`docs/content/architecture/app-shell.md`): a screen's
//! title and main action at the top, the app's tabs and one separate
//! button at the bottom, a sidebar on each side — laid out for a phone, a
//! tablet or a desk from one declaration.
//!
//! The app mounts [`AppShell`] once, at its root, and tells it the tabs,
//! which is current, the separate button and what goes in each side. A
//! page is a [`Page`]: its title, its main action and its ⋯ [`Menu`], over
//! its body; the shell draws the bar where this size puts it.
//!
//! ```rust,ignore
//! use architect_shell::{icon, Action, Menu, Page};
//!
//! rsx! {
//!     Page {
//!         title: "Tasks",
//!         action: Action::new("New task", icon!(Plus), move |_| new_task()),
//!         menu: Menu::new()
//!             .item("Filter", icon!(Funnel), move |_| filter.open()).note("2 on")
//!             .item("Sort", icon!(ArrowUpDown), move |_| sort.open())
//!             .divider()
//!             .check("Show done", show_done(), move |_| show_done.toggle()),
//!         TaskList {}
//!     }
//! }
//! ```
//!
//! Everything is laid out with inline styles so it renders the same in
//! Blitz (no `position: fixed`, no `env()`, no external sheets) as in a
//! browser: the bars and drawers are positioned against the shell's root,
//! which fills its parent, and the safe-area insets come from the host
//! through [`Screen`].
//!
//! The default feature is the contract ([`Screen`], [`FormFactor`],
//! [`Insets`], [`ShellStyle`]): pure data, wasm-clean. Feature `ui` adds
//! the components.

// `Tab`, `Action` and the menu compare their icons' fn pointers: a false
// "differs" only redraws a bar.
#![cfg_attr(feature = "ui", allow(unpredictable_function_pointer_comparisons))]

mod screen;
pub use screen::{FormFactor, Insets, Screen, ShellStyle};

#[cfg(feature = "ui")]
mod shell;
#[cfg(feature = "ui")]
pub use shell::*;
