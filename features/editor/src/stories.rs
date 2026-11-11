//! The editor's stories (feature `stories`).
//!
//! `CodeEditor` on a Rust buffer, highlighted in the theme's `syntax.*`
//! tokens, with one diagnostic. Linked into any binary that turns the feature on, as
//! `architect-ui`'s are; `tests/story.rs` keeps its golden.

use architect_story_runtime::story;
use dioxus::prelude::*;

use crate::state::EditorState;
use crate::theme::{NAMESPACE, TOKENS, css};
use crate::ui::{CodeEditor, EDITOR_CSS, language_named};
use crate::{Diagnostic, Severity};

/// The buffer the story edits.
pub const RUST: &str = r#"/// A greeting, said twice.
fn main() {
    let name = "Architect";
    for n in 0..2 {
        println!("hello {name} #{n}");
    }
    let unused = 42;
}
"#;

/// The `syntax.*` tokens resolved by the default theme, as the CSS
/// variables the editor's token classes read (what an app's theme layer
/// writes on its root).
fn syntax_vars() -> String {
    let store = architect_theme::ThemeStore::default();
    store.set(
        architect_theme::LayerKind::App,
        TOKENS
            .iter()
            .fold(architect_theme::Layer::default(), |layer, (name, value)| {
                layer.token(*name, *value)
            }),
    );
    store
        .current()
        .css_namespaced(&[NAMESPACE])
        .into_iter()
        .map(|(name, value)| format!("--{name}: {value};"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `CodeEditor` editing a Rust file: highlighted, numbered, one warning
/// under `unused`.
#[story(category = "Editor", name = "code-rust")]
pub fn code_editor_rust() -> Element {
    let state = use_signal(|| EditorState::new(RUST));
    let language = use_signal(|| language_named("src/main.rs"));
    let diagnostics = use_signal(|| {
        let at = RUST.find("unused").unwrap_or(0);
        vec![Diagnostic::new(
            at..at.saturating_add("unused".len()),
            Severity::Warning,
            "unused variable: `unused`",
        )]
    });
    let sheet = use_hook(|| {
        format!(
            "{EDITOR_CSS}\n{}\n.story-code {{ {} }}",
            css(),
            syntax_vars()
        )
    });
    rsx! {
        document::Style { {sheet} }
        div { class: "story-code", "data-story": "code-rust",
            CodeEditor { state, language, diagnostics }
        }
    }
}

/// Links the stories into a binary that names nothing else here.
pub const fn force_link() {}
