//! The Dioxus view (`editor-view`), re-exported whole: [`TextEditor`]
//! (prose and markdown, what notes use), [`CodeEditor`] (a source file in
//! one language: line numbers, bracket matching, indentation, diagnostics),
//! [`Editor`], [`EDITOR_CSS`], [`dispatch_with_history`], the palette,
//! hover and completion helpers. Plus hooks that follow the language
//! registry, so an editor on screen draws again when a language comes or
//! goes.

pub use editor_view::*;

use dioxus::prelude::*;
use futures::StreamExt;

use crate::syntax::{self, Syntax};

/// The editor's language registry revision
/// ([`syntax::languages_revision`]), as a signal that moves when
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
