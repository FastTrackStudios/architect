//! `ThemeStoreProvider`: a theme store's switch reaches the CSS variables
//! architect-ui's root sets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;

use architect_theme::{use_theme_store, Layer, LayerKind, ThemeChoice, ThemeStore};
use architect_ui::theme::{use_theme, ThemeStoreProvider};
use dioxus::prelude::*;

thread_local! {
    static STORE: RefCell<Option<ThemeStore>> = const { RefCell::new(None) };
    static CSS: RefCell<String> = const { RefCell::new(String::new()) };
}

fn app() -> Element {
    use_theme_store(|| STORE.with(|s| s.borrow().clone().unwrap()));
    rsx! { ThemeStoreProvider { Probe {} } }
}

#[component]
fn Probe() -> Element {
    let state = use_theme().state;
    let css = state.read().active_style().css_variables();
    CSS.with(|c| *c.borrow_mut() = format!("{}|{css}", state.read().mode.as_str()));
    rsx! {}
}

fn css() -> String {
    CSS.with(|c| c.borrow().clone())
}

#[test]
fn switching_the_store_changes_the_css_variables() {
    let store = ThemeStore::default();
    STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
    let mut dom = VirtualDom::new(app);
    dom.rebuild_in_place();
    assert!(css().starts_with("dark|"), "{}", css());
    assert!(css().contains("--primary: #818cf8;"), "{}", css());

    store.set(
        LayerKind::Pick,
        Layer::default().choice(ThemeChoice::One("catppuccin-light".into())),
    );
    futures::executor::block_on(dom.wait_for_work());
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    assert!(css().starts_with("light|"), "{}", css());
    assert!(css().contains("--primary: #8839ef;"), "{}", css());
    assert!(css().contains("--background: #eff1f5;"), "{}", css());

    // A token the person overrides shows too.
    store.set_token(LayerKind::Person, "radius", "3px");
    futures::executor::block_on(dom.wait_for_work());
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    assert!(css().contains("--radius: 3px;"), "{}", css());

    // An app's namespaced token stays off the root unless opted in.
    store.set_token(LayerKind::App, "arrange.item.edge", "$border");
    futures::executor::block_on(dom.wait_for_work());
    dom.render_immediate(&mut dioxus::core::NoOpMutations);
    assert!(!css().contains("arrange"), "{}", css());
}

#[test]
fn an_apps_namespaces_are_css_variables_only_when_asked_for() {
    use architect_ui::theme::ThemeStyle;
    let store = ThemeStore::default();
    store.set(
        LayerKind::App,
        Layer::default()
            .token("arrange.item.edge", "#123456")
            .token("mixer.strip", "#654321"),
    );
    let r = store.current();
    let plain = ThemeStyle::from_resolved_with(&r, &[]).css_variables();
    assert!(
        !plain.contains("arrange") && !plain.contains("mixer"),
        "{plain}"
    );
    let opted = ThemeStyle::from_resolved_with(&r, &["arrange"]).css_variables();
    assert!(opted.contains("--arrange-item-edge: #123456;"), "{opted}");
    assert!(!opted.contains("mixer"), "{opted}");
}
