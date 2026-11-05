//! A window resized across a phone's width with a workspace on show: the
//! workspace draws again as a phone's, in its shell, without breaking the
//! rules of hooks (it asked for the shell only once it was a phone, and
//! panicked).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_shell::{Account, AppShell, Insets, Menu, Page, Screen, Tab};
use architect_workspace::{Pane, Target, View, Views, Workspace, WorkspaceView};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};
use lucide_dioxus::Inbox;

const DESK: Screen = Screen {
    width: 1280.0,
    height: 800.0,
    touch: false,
    insets: Insets {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
};

/// A desk's workspace that becomes a phone's after its first frame.
#[component]
fn Board() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        let _ = w.open(Pane::new("editor", "note", "Editor"), Target::Tab);
        w
    });
    let views = Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many());
    let mut phone = use_signal(|| false);
    use_effect(move || {
        if !*phone.peek() {
            phone.set(true);
        }
    });
    rsx! {
        div { "data-phone": "{phone}", style: "flex: 1 1 0; min-height: 0; display: flex; position: relative;",
            WorkspaceView { workspace, views, phone: phone(), in_shell: true }
        }
    }
}

fn app() -> Element {
    rsx! {
        div { style: "position: absolute; inset: 0;",
            AppShell {
                screen: DESK,
                tabs: vec![Tab::new("vault", "Vault", architect_shell::icon!(Inbox))],
                current: Some("vault"),
                on_tab: |_| (),
                account: Account::new("Cody Wright").menu(Menu::new()),
                left: rsx! { div { "the left side" } },
                Page { title: "Vault", Board {} }
            }
        }
    }
}

#[test]
fn a_workspace_becomes_a_phone_s_while_it_is_on_show() {
    architect_shell::motion::set_reduced_motion(Some(true));
    let mut doc = DioxusDocument::new(
        VirtualDom::new(app),
        DocumentConfig {
            viewport: Some(Viewport::new(1280, 800, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
    let d = doc.inner.borrow();
    assert!(
        d.query_selector("[data-phone=\"true\"]").unwrap().is_some(),
        "it became a phone's"
    );
}
