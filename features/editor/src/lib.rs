//! The editor, offered by architect (#512).
//!
//! The editor lives in its own repo, FastTrackStudios/editor, and stays
//! there: it depends on nothing above it, so Task, Keyflow and Session can
//! all embed it. This crate is the one door to it from architect. An app
//! or an extension names `architect-editor` and gets the editor at the
//! revision architect pins (the workspace's `editor-*` entries, the only
//! pin of that repo), so nobody else pins it in lockstep.
//!
//! | Feature | What it gives |
//! |---|---|
//! | none | [`ext`] (the editor's extension surface as data), [`languages`] (the language registry) and [`theme`] (the `syntax.*` tokens). Wasm-clean, no tree-sitter, no Dioxus |
//! | `state` | [`state`]: the document model, transactions and the markdown pass |
//! | `syntax` | [`syntax`]: a registered language's highlighting, each span named by theme token |
//! | `ui` (+ `web` or `native`) | [`ui`]: the Dioxus component and its helpers |
//!
//! Nothing here is the editor's code: the modules are re-exports, the
//! registry and the tokens are architect's own. What the editor lacks is
//! built in its repo and arrives when the pin moves.

/// The editor's extension surface as data (`editor-ext`): snapshots, edit
/// commands, keymaps, decorations and slash entries, what sandboxed and
/// out-of-process runtimes send and receive.
pub use editor_ext as ext;

/// The document model (`editor-state`): [`state::EditorState`],
/// [`state::Transaction`], [`state::History`], the markdown pass.
#[cfg(feature = "state")]
pub use editor_state as state;

/// The Dioxus view (`editor-view`): [`ui::Editor`], [`ui::EDITOR_CSS`],
/// [`ui::dispatch_with_history`], the palette, hover and completion
/// helpers.
#[cfg(feature = "ui")]
pub use editor_view as ui;

pub mod languages;
#[cfg(feature = "syntax")]
pub mod syntax;
pub mod theme;

pub use languages::{Grammar, Language, Languages, Registered, Server};
