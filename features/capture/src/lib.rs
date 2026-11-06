//! Capture: the one door every app captures through (#352).
//!
//! A line typed into the composer, the global capture window, ⌘K or
//! `CAPTURE_ADD` is read by the quick-add parser ([`parse`]), lands in the
//! vault's inbox as a note ([`inbox`]), and comes back once to be
//! processed: filed as a note, made into a target's thing (a task, …),
//! put off, or discarded. Extensions add what a capture can become with
//! `[[capture_targets]]` ([`target`]).
//!
//! The default set is the model — the parser, the inbox's notes and the
//! targets — pure data, wasm-clean (an extension's process links it, as
//! Tasks' does for `TASKS_ADD`). `actions` puts `CAPTURE_*` and `INBOX_*`
//! on an Action List over the vault's `VAULT_*` ([`actions`]); `ui` adds
//! the composer, the inbox deck and the capture window ([`ui`]);
//! `native` the system-wide key that opens the window from any app, and
//! the window raised over every app (desk only).
//!
//! `docs/content/features/capture.md`.

pub mod inbox;
pub mod parse;
pub mod target;

#[cfg(feature = "actions")]
pub mod actions;

#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "native")]
pub mod shortcut;

pub use parse::{Draft, Repeat, TokenRule, merge, parse, parse_with};
pub use target::{Capture, Target};
