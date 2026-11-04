//! A window's document parses `dangerous_inner_html`: the markup a
//! component hands it as HTML (an editor's painted caret, a widget's
//! body) becomes nodes, laid out and painted like the rest.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use architect_workspace::native::document_config;
use dioxus::prelude::*;
use dioxus_native::DioxusDocument;

#[component]
fn Page() -> Element {
    rsx! {
        div { class: "line",
            "C F"
            span { class: "widget", dangerous_inner_html: r#"<span class="caret"></span>"# }
            " G C"
        }
    }
}

#[test]
fn inner_html_becomes_nodes_in_a_window_document() {
    let mut doc = DioxusDocument::new(VirtualDom::new(Page), document_config(None, &[]));
    doc.initial_build();
    doc.inner.borrow_mut().resolve(0.0);
    let inner = doc.inner.borrow();
    let widget = inner.query_selector(".widget").unwrap();
    assert!(widget.is_some(), "the widget's own element is there");
    let caret = inner.query_selector(".widget > .caret").unwrap();
    assert!(
        caret.is_some(),
        "the widget's inner HTML is parsed into nodes"
    );
}
