//! A page's tree, drawn as edits: applied to the `RemoteDocument` a
//! process view uses, the document is the tree, render after render.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{Edit, Frame};
use architect_pages::api::{Attr, Element, Listener};
use architect_pages::diff::{Mirror, TEXT};

fn el(tag: &str, children: Vec<Element>) -> Element {
    Element {
        tag: tag.to_owned(),
        children,
        ..Element::default()
    }
}

fn text(t: &str) -> Element {
    Element {
        tag: TEXT.to_owned(),
        text: Some(t.to_owned()),
        ..Element::default()
    }
}

fn keyed(key: &str, t: &str) -> Element {
    Element {
        key: Some(key.to_owned()),
        ..el("li", vec![text(t)])
    }
}

fn attr(mut e: Element, name: &str, value: &str) -> Element {
    e.attrs.push(Attr {
        name: name.to_owned(),
        value: value.to_owned(),
    });
    e
}

/// The tree as markup, the way [`markup`] reads the document.
fn expect(tree: &[Element]) -> String {
    tree.iter().map(one).collect()
}

fn one(e: &Element) -> String {
    if e.tag == TEXT {
        return e.text.clone().unwrap_or_default();
    }
    let mut attrs: Vec<String> = e
        .attrs
        .iter()
        .map(|a| format!(" {}={:?}", a.name, a.value))
        .collect();
    attrs.sort();
    format!(
        "<{}{}>{}</{}>",
        e.tag,
        attrs.concat(),
        expect(&e.children),
        e.tag
    )
}

/// The view's `main` element's content, as markup (Dioxus's id marks
/// left out).
fn markup(view: &RemoteDocument) -> String {
    let doc = view.document();
    let main = doc.query_selector("main").unwrap().unwrap();
    let node = doc.get_node(main).unwrap();
    node.children.iter().map(|c| read(view, *c)).collect()
}

fn read(view: &RemoteDocument, id: blitz_dom::NodeId) -> String {
    let node = view.document().get_node(id).unwrap();
    if let Some(t) = node.text_data() {
        return t.content.clone();
    }
    let Some(data) = node.element_data() else {
        return String::new();
    };
    let tag = data.name.local.to_string();
    let mut attrs: Vec<String> = data
        .attrs()
        .iter()
        .filter(|a| &*a.name.local != "data-dioxus-id")
        .map(|a| format!(" {}={:?}", &*a.name.local, a.value))
        .collect();
    attrs.sort();
    let kids: String = node.children.iter().map(|c| read(view, *c)).collect();
    format!("<{tag}{}>{kids}</{tag}>", attrs.concat())
}

fn draw(view: &mut RemoteDocument, mirror: &mut Mirror, tree: &[Element]) -> Vec<Edit> {
    let edits = mirror.update(tree);
    view.apply(Frame::Edits {
        edits: edits.clone(),
    })
    .unwrap_or_else(|e| panic!("{e}: {edits:#?}"));
    assert_eq!(markup(view), expect(tree));
    edits
}

#[test]
fn the_document_follows_the_tree_render_after_render() {
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut view = RemoteDocument::new(events);
    let mut mirror = Mirror::new();
    let list = |keys: &[&str]| {
        vec![
            attr(
                el("h1", vec![text("Board")]),
                "style",
                "color: var(--foreground)",
            ),
            el(
                "ul",
                keys.iter().map(|k| keyed(k, &format!("pin {k}"))).collect(),
            ),
        ]
    };
    draw(&mut view, &mut mirror, &list(&["a", "b", "c"]));
    // Nothing changed: nothing to send.
    assert!(draw(&mut view, &mut mirror, &list(&["a", "b", "c"])).is_empty());
    // Reordered, one gone, one new.
    draw(&mut view, &mut mirror, &list(&["c", "a", "d"]));
    draw(&mut view, &mut mirror, &list(&["d", "c", "b", "a", "e"]));
    draw(&mut view, &mut mirror, &list(&[]));
    draw(&mut view, &mut mirror, &list(&["x"]));
    // A node of another kind in its place; an attribute gone; text edited.
    let other = vec![
        el("h2", vec![text("Pins")]),
        el("ul", vec![keyed("x", "pin x, edited")]),
        text("trailing"),
    ];
    draw(&mut view, &mut mirror, &other);
    draw(&mut view, &mut mirror, &list(&["x", "y"]));
}

#[test]
fn keyed_children_keep_their_nodes_as_they_move() {
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut view = RemoteDocument::new(events);
    let mut mirror = Mirror::new();
    let tree = |keys: &[&str]| vec![el("ul", keys.iter().map(|k| keyed(k, k)).collect())];
    draw(&mut view, &mut mirror, &tree(&["a", "b", "c"]));
    let edits = draw(&mut view, &mut mirror, &tree(&["c", "b", "a"]));
    // Moves only: nothing made, nothing removed.
    assert!(
        edits.iter().all(|e| !matches!(
            e,
            Edit::CreateElement { .. } | Edit::CreateText { .. } | Edit::Remove
        )),
        "{edits:?}"
    );
}

#[test]
fn an_event_finds_its_handler_on_the_element_or_above() {
    let mut mirror = Mirror::new();
    let mut button = el("button", vec![el("span", vec![text("Pin")])]);
    button.on.push(Listener {
        event: "click".to_owned(),
        handler: 7,
    });
    let edits = mirror.update(&[el("div", vec![button.clone()])]);
    let listened: Vec<_> = edits
        .iter()
        .filter(|e| matches!(e, Edit::Listen { .. }))
        .collect();
    assert_eq!(listened.len(), 1);
    // div 1, button 2, span 3, text 4: a click on the span is the button's.
    assert_eq!(mirror.handler(3, "click"), Some(7));
    assert_eq!(mirror.handler(2, "click"), Some(7));
    assert_eq!(mirror.handler(1, "click"), None);
    assert_eq!(mirror.handler(3, "input"), None);
    // A new function for the same event: no edit, the new number.
    button.on[0].handler = 9;
    let edits = mirror.update(&[el("div", vec![button])]);
    assert!(edits.is_empty(), "{edits:?}");
    assert_eq!(mirror.handler(3, "click"), Some(9));
}
