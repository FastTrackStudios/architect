//! A film looks like the app: the document the runner mounts the lab in
//! gets the stylesheets the desk window's does, and what they place sits
//! where the window draws it.
//!
//! Films once drew without any of the app's `document::Style` sheets: the
//! runner's document had no head, so Dioxus dropped them, and every icon
//! button's hidden name (the shell's `.tip`) was drawn beside its icon,
//! pushing it off-centre (the rail, the capture button, the title row's
//! tabs).
//!
//! `tests/golden/head.txt` is the lab's head: each element the app puts
//! there, by name and its sheet's start. `ARCHITECT_GOLDEN=record`
//! writes it again (when a change adds or drops a stylesheet).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use architect_guide::ViewSize;
use architect_guide::run::{Desktop, Film, Headless, Pointer};
use blitz_dom::{BaseDocument, NodeId};

/// The lab, opened as its desk window opens it, settled.
fn lab() -> Headless {
    let mut lab = Headless::open(&architect_sandbox_media::app(), ViewSize::default());
    let start = Instant::now();
    let mut idle = 0_u32;
    while idle < 24 && start.elapsed() < Duration::from_secs(10) {
        if lab.pump() {
            idle = 0;
        } else {
            idle = idle.saturating_add(1);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    lab
}

fn name(doc: &BaseDocument, id: NodeId) -> Option<String> {
    doc.get_node(id)?
        .element_data()
        .map(|e| e.name.local.to_string())
}

/// The head's elements, one a line: its name, its attributes, and the
/// start of what is in it.
fn head(doc: &BaseDocument) -> String {
    let head = doc.query_selector("head").unwrap().expect("a head");
    let mut out = String::new();
    for &child in &doc.get_node(head).unwrap().children {
        let Some(tag) = name(doc, child) else {
            continue;
        };
        let node = doc.get_node(child).unwrap();
        let attrs: Vec<String> = node
            .element_data()
            .unwrap()
            .attrs()
            .iter()
            .map(|a| format!("{}={:?}", a.name.local, a.value))
            .collect();
        let text = node.text_content();
        let start: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let start: String = start.chars().take(72).collect();
        let _ = writeln!(out, "{tag} [{}] {start}", attrs.join(" "));
    }
    out
}

/// Both checks on the lab as it opens (the Inbox, its sidebar open), then
/// the icon check on the Vault (its sidebar's icon-only tabs). One at a
/// time: the lab's folder is the process's.
#[test]
fn the_film_draws_as_the_window_does() {
    {
        let lab = lab();
        let main = &lab.desktop().windows()[0];
        let doc = main.doc.inner.borrow();
        has_the_apps_stylesheets(&doc);
        let (checked, off) = icons_off_centre(&doc);
        assert!(checked >= 4, "too few icon buttons to check ({checked})");
        assert!(off.is_empty(), "icons off-centre:\n{}", off.join("\n"));
    }
    let scenario = architect_guide::load(VAULT).unwrap();
    let mut last = Last::default();
    architect_guide::run::run(&architect_sandbox_media::app(), &scenario, Some(&mut last)).unwrap();
    let (tabs, off) = last.0.expect("a frame");
    assert!(tabs >= 2, "the Vault's icon tabs were not found ({tabs})");
    assert!(
        off.is_empty(),
        "icons off-centre on the Vault:\n{}",
        off.join("\n")
    );
}

/// The lab's Vault page, opened from the rail.
const VAULT: &str = r#"id fidelity-vault
title "The Vault"
view {width 1280, height 760}
capture {fps 10, width 960}
steps (
    {
        id vault
        target @text"Vault"
        do @click{}
        done {on (@changed"lab"), check ({id LAB_STATE, has {tab vault}})}
        hold 0.5
    }
)
"#;

/// The last frame's icon check: how many icon-only tabs it found, and
/// every icon off-centre.
#[derive(Default)]
struct Last(Option<(usize, Vec<String>)>);

impl Film for Last {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        if let Some(main) = desktop.windows().first() {
            let doc = main.doc.inner.borrow();
            let tabs = doc
                .query_selector_all("[role=tab]")
                .unwrap()
                .into_iter()
                .filter(|&t| {
                    let mut found = Vec::new();
                    icons(&doc, t, &mut found);
                    found.len() == 1 && doc.get_node(t).unwrap().text_content().trim().is_empty()
                })
                .count();
            self.0 = Some((tabs, icons_off_centre(&doc).1));
        }
    }
}

fn has_the_apps_stylesheets(doc: &BaseDocument) {
    let head = head(doc);
    // The shell's own sheet, whose `.tip` hides an icon button's name.
    let sheets = doc
        .query_selector_all("head style")
        .unwrap()
        .into_iter()
        .map(|id| doc.get_node(id).unwrap().text_content());
    assert!(
        sheets
            .into_iter()
            .any(|s| s.contains(".tip-host:hover > .tip")),
        "the shell's sheet is not in the film's head:\n{head}"
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/head.txt");
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::write(&path, &head).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        head, want,
        "the film's head changed (ARCHITECT_GOLDEN=record if meant)"
    );
}

/// The `svg`s in `id`, outside its `.tip`.
fn icons(doc: &BaseDocument, id: NodeId, out: &mut Vec<NodeId>) {
    let node = doc.get_node(id).unwrap();
    for &child in &node.children {
        let Some(el) = doc.get_node(child).and_then(|n| n.element_data()) else {
            continue;
        };
        if el.attr(blitz_dom::local_name!("class")) == Some("tip") {
            continue;
        }
        if el.name.local == blitz_dom::local_name!("svg") {
            out.push(child);
        } else {
            icons(doc, child, out);
        }
    }
}

/// Every icon button (a `.tip-host`) and icon-only tab (a sidebar's) has
/// its icon in its middle, within a point.
fn icons_off_centre(doc: &BaseDocument) -> (usize, Vec<String>) {
    let hosts = doc.query_selector_all(".tip-host, [role=tab]").unwrap();
    let mut checked = 0_usize;
    let mut off = Vec::new();
    for host in hosts {
        let mut found = Vec::new();
        icons(doc, host, &mut found);
        // An icon button: one icon and no text of its own beside it.
        let [icon] = found[..] else { continue };
        let own_text = doc
            .get_node(host)
            .unwrap()
            .children
            .iter()
            .filter(|&&c| {
                doc.get_node(c).is_some_and(|n| {
                    n.element_data()
                        .is_none_or(|e| e.attr(blitz_dom::local_name!("class")) != Some("tip"))
                        && !n.text_content().trim().is_empty()
                })
            })
            .count();
        if own_text > 0 {
            continue;
        }
        let (Some(b), Some(i)) = (
            doc.get_client_bounding_rect(host),
            doc.get_client_bounding_rect(icon),
        ) else {
            continue;
        };
        if b.width <= 0.0 || i.width <= 0.0 {
            continue;
        }
        checked = checked.saturating_add(1);
        let dx = (b.x + b.width / 2.0) - (i.x + i.width / 2.0);
        let dy = (b.y + b.height / 2.0) - (i.y + i.height / 2.0);
        if dx.abs() > 1.0 || dy.abs() > 1.0 {
            let node = doc.get_node(host).unwrap();
            let label = node
                .element_data()
                .and_then(|e| e.attr(blitz_dom::local_name!("aria-label")))
                .map_or_else(|| node.text_content(), str::to_owned);
            off.push(format!(
                "{:?}: button {:.1},{:.1} {:.1}x{:.1}, icon off by ({dx:.1}, {dy:.1})",
                label.trim(),
                b.x,
                b.y,
                b.width,
                b.height
            ));
        }
    }
    (checked, off)
}
