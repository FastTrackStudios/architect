//! `CodeEditor`'s story (`stories::code_editor_rust`), played headlessly
//! on Blitz: a Rust buffer highlighted in the theme's `syntax.*` tokens,
//! numbered, one warning underlined. Its drawn HTML matches
//! `tests/golden/code-rust.html` (`ARCHITECT_GOLDEN=record` writes it).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::redundant_closure_for_method_calls
)]

use std::path::Path;

use architect_editor::stories;
use architect_guide::run::{Desktop, Film, Pointer};

/// The story, as HTML, at the last frame shown.
#[derive(Default)]
struct StoryHtml(String);

impl Film for StoryHtml {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        let Ok(found) = doc.query_selector_all("[data-story=\"code-rust\"]") else {
            return;
        };
        self.0 = found
            .iter()
            .filter_map(|id| doc.get_node(*id))
            .map(|n| n.outer_html_pretty())
            .collect::<Vec<_>>()
            .join("\n");
    }
}

const SCENARIO: &str = r#"id code-rust
title "CodeEditor on Rust"
view {width 960, height 600}
capture {fps 10, width 640}
steps (
    {id drawn, target @css".editor-code", done {timeout 10}}
    {id end, hold 0.3}
)
"#;

#[test]
fn the_code_editor_story_draws_rust_in_theme_tokens() {
    stories::force_link();
    let scenario = architect_guide::load(SCENARIO).unwrap();
    let mut film = StoryHtml::default();
    architect_guide::run::run(
        &architect_guide::story::app(&stories::CODE_EDITOR_RUST_STORY),
        &scenario,
        Some(&mut film),
    )
    .unwrap_or_else(|e| panic!("{e}\n{}", film.0));
    let html = film.0;

    // Highlighted: a keyword, a string and a comment, each in a token
    // class the theme's `syntax.*` variables colour.
    for class in ["md-tok-k", "md-tok-s", "md-tok-c"] {
        assert!(html.contains(class), "{class} in {html}");
    }
    // Numbered, and the warning under `unused`.
    assert!(html.contains("code-ln"), "line numbers: {html}");
    assert!(html.contains("cm-lsp-warning"), "the warning: {html}");

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/code-rust.html");
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &html).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (ARCHITECT_GOLDEN=record)", path.display()));
    if want != html {
        let got = path.with_extension("got.html");
        std::fs::write(&got, &html).unwrap();
        panic!(
            "code-rust does not look the same (diff {} {})",
            path.display(),
            got.display()
        );
    }
}
