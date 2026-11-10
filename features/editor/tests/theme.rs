//! The syntax tokens in a real theme store: each resolves to a colour in
//! every built-in theme, a theme overrides one, and they come out as the
//! CSS variables the editor's sheet reads.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use architect_editor::theme::{NAMESPACE, TOKENS, css, css_var};
use architect_theme::{Layer, LayerKind, ThemeStore};

fn app_layer() -> Layer {
    TOKENS
        .iter()
        .fold(Layer::default(), |layer, (name, value)| {
            layer.token(*name, *value)
        })
}

#[test]
fn every_syntax_token_resolves_and_reaches_css() {
    let store = ThemeStore::default();
    store.set(LayerKind::App, app_layer());
    let resolved = store.current();
    assert!(resolved.problems().is_empty(), "{:?}", resolved.problems());
    let vars = resolved.css_namespaced(&[NAMESPACE]);
    let sheet = css();
    for (name, _) in TOKENS {
        let value = resolved
            .get(name)
            .unwrap_or_else(|| panic!("{name} did not resolve"));
        assert!(
            !value.contains('$') && !value.contains("mix("),
            "{name} = {value}"
        );
        let var = css_var(name);
        assert!(
            vars.iter().any(|(css, _)| format!("--{css}") == var),
            "{var} is not written"
        );
        assert!(sheet.contains(&format!("var({var}, ")), "{var} is not read");
    }
}

#[test]
fn a_person_restyles_a_token() {
    let store = ThemeStore::default();
    store.set(LayerKind::App, app_layer());
    let before = store.current().value("syntax.keyword").to_owned();
    store.set(
        LayerKind::Person,
        Layer::default().token("syntax.keyword", "#ff0000"),
    );
    let after = store.current().value("syntax.keyword").to_owned();
    assert_ne!(before, after);
    assert_eq!(after.to_ascii_lowercase(), "#ff0000");
}
