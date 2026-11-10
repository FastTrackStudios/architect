//! The Dioxus view (`editor-view`), re-exported whole.
//!
//! [`TextEditor`] (prose and markdown, what notes use), [`CodeEditor`] (a source file in
//! one language: line numbers, bracket matching, indentation, diagnostics),
//! [`Editor`], [`EDITOR_CSS`], [`dispatch_with_history`], the palette,
//! hover and completion helpers. Plus hooks that follow the language
//! registry, so an editor on screen draws again when a language comes or
//! goes.

pub use editor_view::*;

use dioxus::prelude::*;
use futures::StreamExt;

use crate::syntax::{self, Syntax};

/// The editor's language registry revision, as a signal.
///
/// It is [`syntax::languages_revision`], and moves when
/// [`syntax::drive`] registers or withdraws a language. Read it in a
/// component (or a memo) that draws highlighted text, and it draws again.
pub fn use_languages_revision() -> ReadSignal<u64> {
    let mut revision = use_signal(syntax::languages_revision);
    use_hook(move || {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<u64>();
        syntax::watch(move |r| tx.unbounded_send(r).is_ok());
        spawn(async move {
            while let Some(r) = rx.next().await {
                if *revision.peek() != r {
                    revision.set(r);
                }
            }
        });
    });
    revision.into()
}

/// The editor's language for `name` (a language id, a fence name, or a
/// file path or extension), kept up to date with the registry: what a
/// [`CodeEditor`]'s `language` prop takes.
///
/// ```ignore
/// let language = use_language("src/main.rs".to_owned());
/// rsx! { CodeEditor { state, language } }
/// ```
#[must_use]
pub fn use_language(name: String) -> Memo<Option<Syntax>> {
    let revision = use_languages_revision();
    use_memo(move || {
        revision();
        language_named(&name)
    })
}

/// The editor's language `name` means, now: an id, a fence name, a file
/// extension, or a path by its extension.
#[must_use]
pub fn language_named(name: &str) -> Option<Syntax> {
    syntax::language_by_id(name)
        .or_else(|| syntax::language_for_fence(name))
        .or_else(|| syntax::language_for_extension(name.trim_start_matches('.')))
        .or_else(|| syntax::language_for_path(name))
}

/// A `CodeEditor` wired to its language's server (features `ui` and
/// `lsp`, native): what [`use_code_document`] gives.
#[cfg(all(feature = "lsp", not(target_family = "wasm")))]
#[derive(Clone)]
pub struct CodeDocument {
    /// The server's diagnostics: `CodeEditor`'s `diagnostics`.
    pub diagnostics: ReadSignal<Vec<crate::Diagnostic>>,
    /// Sends each edit as `didChange` and keeps the caret and focus:
    /// `CodeEditor`'s `on_transaction`.
    pub on_transaction: Callback<TransactionEvent>,
    /// The open document, for hover, completion and the rest; `None`
    /// when no server was opened.
    pub document: Option<crate::lsp::DocRef>,
}

/// Opens the editor's file in its language's server, once, and keeps it
/// open while the component lives: `open` is called on mount (the
/// extension host's `Host::open_code`, which checks the language may run
/// its server, or `LanguageServers::open`). Diagnostics come back into
/// `diagnostics`; a format or a rename's edits are applied to `state` as
/// one transaction (through `history` when given), while the text is
/// still what the server answered for.
///
/// ```ignore
/// let code = use_code_document(|| host.open_code(&path, &root, &text).ok(), state, None);
/// rsx! { CodeEditor { state, language, diagnostics: code.diagnostics, on_transaction: code.on_transaction } }
/// ```
#[cfg(all(feature = "lsp", not(target_family = "wasm")))]
pub fn use_code_document(
    open: impl FnOnce() -> Option<crate::lsp::Document>,
    state: Signal<crate::state::EditorState>,
    history: Option<Signal<crate::state::History>>,
) -> CodeDocument {
    let document = use_hook(move || std::rc::Rc::new(open()));
    let mut diagnostics = use_signal(Vec::<crate::Diagnostic>::new);
    let held = document.clone();
    let on_transaction = use_callback(move |event: TransactionEvent| {
        let Some(doc) = held.as_ref() else {
            return;
        };
        if event.changes.iter().next().is_some() {
            doc.change(event.changes.clone(), event.doc_after.clone());
        }
        doc.set_caret(state.peek().selection.primary().head);
        doc.focus();
    });
    let handle = document.as_ref().as_ref().map(crate::lsp::Document::handle);
    let watched = handle.clone();
    use_hook(move || {
        let Some(doc) = watched else {
            return;
        };
        let mut heard = doc.watch_diagnostics();
        spawn(async move {
            while let Some(now) = heard.next().await {
                diagnostics.set(now);
            }
        });
        let mut edits = doc.watch_edits();
        spawn(async move {
            while let Some(batch) = edits.next().await {
                if state.peek().doc.to_string() != batch.against.to_string() {
                    continue;
                }
                let spec = crate::state::TransactionSpec::new()
                    .changes(batch.changes())
                    .user_event("lsp.edit");
                match history {
                    Some(history) => {
                        dispatch_with_history(state, history, spec, Some(on_transaction));
                    }
                    None => dispatch_spec(state, spec, Some(on_transaction)),
                }
            }
        });
    });
    CodeDocument {
        diagnostics: diagnostics.into(),
        on_transaction,
        document: handle,
    }
}
