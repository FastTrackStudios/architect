//! A built-in language, end to end: registered, found by fence name and
//! file type, highlighted by the editor's tree-sitter, each span named by
//! the theme token it is drawn in. And what the editor cannot highlight
//! yet says why.

#![cfg(feature = "syntax")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use architect_editor::languages::{Grammar, Language, Languages, builtin};
use architect_editor::syntax::{Unsupported, highlight, supported};

#[test]
fn rust_from_the_registry_is_highlighted_in_theme_tokens() {
    let languages = Languages::with_app(builtin());
    let source = "fn main() { let s = \"hi\"; } // done";
    for name in ["rust", "rs"] {
        let spans = highlight(&languages, name, source).unwrap();
        let of = |text: &str| {
            let at = source.find(text).unwrap();
            spans
                .iter()
                .find(|s| s.start <= at && at < s.end)
                .map(|s| s.token)
        };
        assert_eq!(of("fn"), Some("syntax.keyword"), "{name}");
        assert_eq!(of("\"hi\""), Some("syntax.string"), "{name}");
        assert_eq!(of("// done"), Some("syntax.comment"), "{name}");
        assert!(
            spans.windows(2).all(|w| w[0].end <= w[1].start),
            "in order, no overlap"
        );
        assert!(spans.iter().all(|s| s.token.starts_with("syntax.")));
    }
    // By file type: the language of `main.rs`.
    let id = languages.for_path("src/main.rs").unwrap().language.id;
    assert!(!highlight(&languages, &id, source).unwrap().is_empty());
}

#[test]
fn every_built_in_language_has_its_grammar_linked() {
    let languages = Languages::with_app(builtin());
    for registered in languages.all() {
        supported(&languages, &registered.language.id).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn what_the_editor_cannot_highlight_says_why() {
    let languages = Languages::with_app(builtin());
    assert_eq!(
        highlight(&languages, "kf", "x").unwrap_err(),
        Unsupported::Unknown("kf".to_owned())
    );
    let file = Language {
        grammar: Grammar::File("grammars/keyflow.wasm".to_owned()),
        ..Language::linked("keyflow", "Keyflow").fences(&["kf"])
    };
    let query = Language {
        highlights_text: Some("(comment) @comment".to_owned()),
        ..Language::linked("toml", "TOML")
    };
    languages.set("lab", vec![file, query, Language::linked("cobol", "COBOL")]);
    assert!(matches!(
        highlight(&languages, "kf", "x").unwrap_err(),
        Unsupported::GrammarFile { .. }
    ));
    assert!(matches!(
        highlight(&languages, "toml", "x").unwrap_err(),
        Unsupported::OwnQuery { .. }
    ));
    let none = highlight(&languages, "cobol", "x").unwrap_err();
    assert!(matches!(none, Unsupported::NotLinked { .. }));
    assert!(none.to_string().contains("cobol"));
}
