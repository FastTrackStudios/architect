//! Architect's registry made the editor's (`syntax::drive`): a language
//! set in it, on a grammar the app linked in (`syntax::link_grammar`), is
//! highlighted in a markdown fence and found for a `CodeEditor` by its
//! file type; withdrawn, it is plain again; and a watcher hears each
//! change. One test: the editor's registry is process-wide.

#![cfg(feature = "syntax")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};

use architect_editor::languages::{Language, Languages, builtin};
use architect_editor::state::{DecorationKind, EditorState, Selection};
use architect_editor::syntax::{self, Lang, Unsupported};

/// The `md-tok-*` marks the markdown pass puts on `doc`, as
/// `(text, class)`.
fn token_marks(doc: &str) -> Vec<(String, String)> {
    let mut state = EditorState::new(doc);
    state.selection = Selection::caret(doc.len());
    architect_editor::state::markdown::live_preview(&state)
        .into_iter()
        .filter_map(|d| match d.kind {
            DecorationKind::Mark { class, .. } if class.starts_with("md-tok-") => {
                Some((doc.get(d.from..d.to).unwrap_or("").to_owned(), class))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_language_set_in_architects_registry_is_the_editors_until_withdrawn() {
    let heard = Arc::new(Mutex::new(Vec::<u64>::new()));
    let log = heard.clone();
    syntax::watch(move |r| {
        log.lock().unwrap().push(r);
        true
    });

    let languages = Languages::with_app(builtin());
    syntax::drive(&languages);
    // The app's own: Rust by path, for a CodeEditor.
    assert!(syntax::language_for_path("src/main.rs").is_some());
    assert!(syntax::problems().is_empty(), "{:?}", syntax::problems());

    // An extension's language on a grammar the app linked in, before the
    // grammar is there: left out, and said why.
    let doc = "# Song\n\n```chart\nfn x() { \"verse\" }\n```\n";
    let chart = Language::linked("chart", "Chart")
        .extensions(&["chart"])
        .fences(&["chart"])
        .host_grammar("chart");
    languages.set("lab.chart", vec![chart]);
    assert!(syntax::language_for_fence("chart").is_none());
    assert!(matches!(
        syntax::problems().as_slice(),
        [(id, Unsupported::NotLinked { .. })] if id == "chart"
    ));
    assert!(token_marks(doc).is_empty());

    // The app links it (a Rust grammar standing in, with its query): now
    // the fence is highlighted and a `.chart` file has a language.
    let before = syntax::languages_revision();
    syntax::link_grammar(
        "chart",
        Lang::Rust.grammar(),
        Some(Lang::Rust.highlights_query()),
    );
    assert!(syntax::languages_revision() > before);
    assert!(syntax::problems().is_empty());
    assert!(syntax::language_for_path("songs/a.chart").is_some());
    let marks = token_marks(doc);
    assert!(
        marks.contains(&("\"verse\"".to_owned(), "md-tok-s".to_owned())),
        "{marks:?}"
    );
    assert!(
        heard
            .lock()
            .unwrap()
            .contains(&syntax::languages_revision())
    );

    // The extension goes: plain again, and no language for the file.
    assert!(languages.withdraw("lab.chart"));
    assert!(syntax::language_by_id("chart").is_none());
    assert!(syntax::language_for_path("songs/a.chart").is_none());
    assert!(token_marks(doc).is_empty());
    // Back on: highlighted again.
    languages.set(
        "lab.chart",
        vec![
            Language::linked("chart", "Chart")
                .fences(&["chart"])
                .host_grammar("chart"),
        ],
    );
    assert!(!token_marks(doc).is_empty());
    // The grammar unlinked: withdrawn from the editor, a problem again.
    assert!(syntax::unlink_grammar("chart"));
    assert!(token_marks(doc).is_empty());
    assert_eq!(syntax::problems().len(), 1);
}
