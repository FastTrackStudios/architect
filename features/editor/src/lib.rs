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
//! | `syntax` | [`syntax`]: the registry made the editor's ([`syntax::drive`]), grammars linked by the app ([`syntax::link_grammar`]), a registered language's highlighting |
//! | `ui` (+ `web` or `native`) | [`ui`]: the Dioxus components (`TextEditor`, `CodeEditor`), their helpers, and hooks that follow the registry |
//! | `stories` | [`stories`]: `CodeEditor` in the story gallery |
//! | `lsp` | [`lsp`]: the editor's LSP client, and [`lsp::LanguageServers`], which runs a language's server per root (native only; nothing on wasm) |
//! | `lsp-stub` | `lsp::stub`: a stub language server for tests |
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

/// Diagnostics as the editor draws them: a byte range, a severity, a
/// message ([`Diagnostic`], [`Severity`]); what `CodeEditor`'s
/// `diagnostics` prop takes and what a language server's are made into.
#[cfg(feature = "state")]
pub use editor_state::diagnostic;
#[cfg(feature = "state")]
pub use editor_state::diagnostic::{Diagnostic, Severity};

#[cfg(all(feature = "lsp", not(target_family = "wasm")))]
pub mod lsp;

pub mod languages;
#[cfg(feature = "stories")]
pub mod stories;
#[cfg(feature = "syntax")]
pub mod syntax;
pub mod theme;
#[cfg(feature = "ui")]
pub mod ui;

pub use languages::{Grammar, Language, Languages, Registered, Server};
