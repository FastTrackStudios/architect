//! The app's interface for extensions, drawn (#306): a file-tree row an
//! extension badged draws its icon and badge (`tests/golden/badged-file-row.html`,
//! `ARCHITECT_GOLDEN=record` writes it again); the window's interface
//! shows a prompt and a menu over the window, and the person's answer is
//! the extension's call's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;
use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::host_ui_view::{Decorations, HostUiLayer, WindowUi};
use architect_extensions::hostcall::{Decoration, HostRequest, Target};
use architect_extensions::hostui::HostUi as _;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use dioxus::prelude::*;
use futures::executor::block_on;
use serde_json::json;

const ICONIZE: &str = r#"
id = "lab.iconize"
name = "Iconize"
version = "0.1.0"
kind = "data"

[capabilities]
ui = ["badges", "prompts", "menus", "notify"]

[[contribute."app.decorations"]]
target = { kind = "file", id = "notes/a.md" }
icon = "📌"
"#;

fn host() -> Host {
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let package = Package::embedded(ICONIZE, &[]).unwrap();
    host.load(&package, Origin::Builtin).unwrap();
    host
}

#[derive(Props, Clone, PartialEq)]
struct RowProps {
    host: Host,
}

/// A file tree's row, as an app draws one: the file's name, then what
/// extensions put on it.
#[allow(non_snake_case)]
fn FileRow(props: RowProps) -> Element {
    rsx! {
        div {
            class: "file-row",
            style: "display: flex; align-items: center; gap: 6px; padding: 2px 8px;",
            "data-path": "notes/a.md",
            span { style: "flex: 1;", "a.md" }
            Decorations { host: props.host, target: Target::file("notes/a.md") }
        }
    }
}

fn golden(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

#[test]
fn a_badged_file_tree_row_draws_its_icon_and_badge() {
    let host = host();
    // At run time, through the host service, as the extension.
    block_on(host.api("lab.iconize").call(HostRequest::Decorate {
        target: Target::file("notes/a.md"),
        decoration: Some(Decoration {
            icon: Some("star".to_owned()),
            badge: Some("3".to_owned()),
            color: Some("primary".to_owned()),
            tooltip: Some("3 open tasks".to_owned()),
        }),
    }))
    .unwrap();
    let mut dom = VirtualDom::new_with_props(FileRow, RowProps { host: host.clone() });
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    // Its manifest's pin first, then the badge set at run time.
    assert!(html.contains("📌"), "{html}");
    assert!(html.contains(r#"data-decoration-from="lab.iconize""#), "{html}");
    assert!(html.contains(">3</span>"), "{html}");
    assert!(html.contains("var(--primary)"), "{html}");
    let path = golden("badged-file-row.html");
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::write(&path, &html).unwrap();
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(kept, html, "{} changed: ARCHITECT_GOLDEN=record", path.display());

    // Unloaded, the row is bare again.
    assert!(host.unload("lab.iconize"));
    let mut dom = VirtualDom::new_with_props(FileRow, RowProps { host });
    dom.rebuild_in_place();
    assert!(!dioxus_ssr::render(&dom).contains("data-decorations"));
}

#[derive(Props, Clone, PartialEq)]
struct LayerProps {
    host: Host,
}

#[allow(non_snake_case)]
fn Layer(props: LayerProps) -> Element {
    rsx! { HostUiLayer { host: props.host } }
}

fn drawn(host: &Host) -> String {
    let mut dom = VirtualDom::new_with_props(Layer, LayerProps { host: host.clone() });
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

#[test]
fn the_window_shows_a_prompt_and_a_menu_and_the_answer_is_the_calls() {
    let host = host();
    let ui = WindowUi::lend(&host);
    let api = host.api("lab.iconize");
    let asking = std::thread::spawn({
        let api = api.clone();
        move || {
            block_on(api.call(
                serde_json::from_value(json!({
                    "op": "prompt", "kind": "text", "title": "Rename to?", "default": "a.md"
                }))
                .unwrap(),
            ))
        }
    });
    while ui.waiting().is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // Drawn over the window, the extension's name on it.
    let html = drawn(&host);
    assert!(html.contains(r#"data-ext-prompt="text""#), "{html}");
    assert!(html.contains("Rename to?") && html.contains("Iconize"), "{html}");
    let id = ui.waiting()[0].id;
    ui.answer(id, Some(json!("b.md")));
    assert_eq!(asking.join().unwrap(), Ok(json!("b.md")));

    let menu = std::thread::spawn(move || {
        block_on(api.call(
            serde_json::from_value(json!({
                "op": "menu", "x": 40, "y": 60,
                "items": [{"id": "pin", "label": "Pin", "icon": "pin"}, {"separator": true}, {"id": "clear", "label": "Clear icon"}]
            }))
            .unwrap(),
        ))
    });
    while ui.waiting().is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let html = drawn(&host);
    assert!(html.contains(r#"data-ext-menu="3""#), "{html}");
    assert!(html.contains("left: 40px; top: 60px;"), "{html}");
    // Dismissed: null.
    ui.answer(ui.waiting()[0].id, None);
    assert_eq!(menu.join().unwrap(), Ok(serde_json::Value::Null));

    // A notification is a toast.
    ui.notify(architect_extensions::hostui::Notification {
        from: architect_extensions::hostui::Asker {
            id: "lab.iconize".to_owned(),
            name: "Iconize".to_owned(),
        },
        text: "Icons applied".to_owned(),
        kind: "success".to_owned(),
        title: None,
    })
    .unwrap();
    let html = drawn(&host);
    assert!(html.contains(r#"data-ext-toast="success""#) && html.contains("Icons applied"), "{html}");
}

#[test]
fn the_overlays_lend_the_window_interface_unless_the_app_gave_one() {
    let one = host();
    let ui = WindowUi::lend(&one);
    assert_eq!(WindowUi::lend(&one), ui, "one a host");
    assert!(!one.offer_ui(Arc::new(WindowUi::new())), "the first stays");
    // An app's own interface stays its extensions'.
    let other = host();
    let fake = architect_extensions::hostui::FakeUi::new();
    other.set_ui(Arc::new(fake.clone()));
    let drawn_only = WindowUi::lend(&other);
    block_on(other.api("lab.iconize").call(HostRequest::Notify {
        text: "to the fake".to_owned(),
        kind: None,
        title: None,
    }))
    .unwrap();
    assert_eq!(fake.notices().len(), 1);
    assert!(drawn_only.toasts().is_empty());
}
