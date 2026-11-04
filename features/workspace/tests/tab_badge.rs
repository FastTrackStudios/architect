//! A badged tab in a narrow tab bar (#314's leftover): its badge stays in
//! view and its name keeps a few letters, fading where it is cut, rather
//! than the badge pushing the name out. Laid out by Blitz.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_workspace::{Pane, TabBadges, Target, View, Views, Workspace, WorkspaceView};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};

/// Ten notes in one group, the one in front (opened last) badged.
#[component]
fn Narrow() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        for n in [0, 1, 2, 4, 5, 6, 7, 8, 9, 3] {
            let _ = w.open(
                Pane::new(format!("p{n}"), "note", format!("A long note title {n}")),
                Target::Tab,
            );
        }
        w
    });
    use_context_provider(|| {
        TabBadges::new(|_, pane| {
            (pane == "p3").then(|| {
                rsx! { span { "data-badge": "p3", style: "flex:0 0 auto; padding:0 4px; font-size:11px;", "12" } }
            })
        })
    });
    let views = Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many());
    rsx! {
        div { style: "position: absolute; inset: 0; display: flex;",
            WorkspaceView { workspace, views }
        }
    }
}

fn rect(doc: &DioxusDocument, selector: &str) -> (f64, f64, f64, f64) {
    let d = doc.inner.borrow();
    let id = d
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"));
    let r = d.get_client_bounding_rect(id).unwrap();
    (r.x, r.y, r.width, r.height)
}

#[test]
fn a_badge_stays_in_view_and_the_name_keeps_some_letters() {
    architect_shell::motion::set_reduced_motion(Some(true));
    let mut doc = DioxusDocument::new(
        VirtualDom::new(Narrow),
        DocumentConfig {
            viewport: Some(Viewport::new(720, 400, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
    let (tx, _, tw, _) = rect(&doc, "[data-tab=\"p3\"]");
    let (bx, _, bw, _) = rect(&doc, "[data-badge=\"p3\"]");
    let (_, _, name_w, _) = rect(&doc, "[data-tab-name=\"p3\"]");
    let (_, _, plain_w, _) = rect(&doc, "[data-tab=\"p5\"]");
    assert!(plain_w < 100.0, "the bar is narrow: an unbadged tab is {plain_w} wide");
    assert!(bw > 0.0 && bx + bw <= tx + tw + 0.5, "the badge ({bx}+{bw}) inside its tab ({tx}+{tw})");
    assert!(name_w >= 19.5, "the name keeps some letters: {name_w}");
    // Its close button too (the tab in front has one).
    let (cx, _, cw, _) = rect(&doc, "[data-tab=\"p3\"] [aria-label^=\"Close\"]");
    assert!(cx + cw <= tx + tw + 0.5, "its close button ({cx}+{cw}) inside it ({tx}+{tw})");
}
