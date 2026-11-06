//! A page's own DOM (`architect/dom`, `js/dom.js`) on `QuickJS`: a
//! framework renders to it in the page's runtime, and what it did reaches
//! the pane as edits — Preact itself (the lab Board's vendored copy,
//! `apps/sandbox/board/pages/vendor/`), and the DOM by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{Edit, EventData, Frame, Pointer, Value, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};

const PREACT: &str = include_str!("../../../apps/sandbox/board/pages/vendor/preact.js");
const HOOKS: &str = include_str!("../../../apps/sandbox/board/pages/vendor/hooks.js");

struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn open(dir: &Path, name: &str) -> Self {
        let pages = Pages::new(AppHost::new(ActionList::new())).with_js();
        let page = PageFile::new(dir.join(name), Origin::Override).unwrap();
        let session = pages.open(&page, "lab.test", "page", None).unwrap();
        let (to, from) = futures::channel::mpsc::unbounded();
        Self {
            session,
            view: RemoteDocument::new(to),
            _events: from,
        }
    }

    /// The next frame's edits, applied; panics on none in time.
    fn edits(&mut self) -> Vec<Edit> {
        let until = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        while Instant::now() < until {
            match self.session.frames.try_recv() {
                Ok(frame) => {
                    self.view.apply(frame.clone()).unwrap();
                    match frame {
                        Frame::Edits { edits } => return edits,
                        Frame::Failed(why) => panic!("the page failed: {why}"),
                    }
                }
                Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        panic!(
            "no frame; log: {:?}, error: {:?}",
            self.session.status().log.lines(),
            self.session.status().error()
        );
    }

    /// Frames until one has edits (a page's first may come after its
    /// theme's).
    fn drawn(&mut self) -> Vec<Edit> {
        loop {
            let edits = self.edits();
            if edits
                .iter()
                .any(|e| matches!(e, Edit::CreateElement { .. }))
            {
                return edits;
            }
        }
    }

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    fn attr(&self, selector: &str, name: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        doc.get_node(node)?
            .attr(blitz_dom::LocalName::from(name))
            .map(ToOwned::to_owned)
    }

    fn count(&self, selector: &str) -> usize {
        self.view
            .document()
            .query_selector_all(selector)
            .map_or(0, |v| v.len())
    }

    /// The document's nodes `selector` finds: the same node for as long
    /// as it is the same element.
    fn nodes(&self, selector: &str) -> Vec<String> {
        self.view
            .document()
            .query_selector_all(selector)
            .map(|v| v.iter().map(|n| format!("{n:?}")).collect())
            .unwrap_or_default()
    }

    fn id(&self, selector: &str) -> u64 {
        self.attr(selector, "data-dioxus-id")
            .unwrap_or_else(|| panic!("no {selector}"))
            .parse()
            .unwrap()
    }

    /// `name` on `selector` as the pane sends it; the frame it answers.
    fn event(&mut self, selector: &str, name: &str, data: EventData) -> Vec<Edit> {
        let element = self.id(selector);
        self.session.event(ViewEvent {
            element,
            name: name.to_owned(),
            bubbles: true,
            data,
        });
        self.edits()
    }

    fn click(&mut self, selector: &str) -> Vec<Edit> {
        self.event(selector, "click", EventData::Mouse(Pointer::default()))
    }
}

fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn created(edits: &[Edit]) -> Vec<&str> {
    edits
        .iter()
        .filter_map(|e| match e {
            Edit::CreateElement { tag, .. } => Some(tag.as_str()),
            _ => None,
        })
        .collect()
}

fn texts(edits: &[Edit]) -> Vec<&str> {
    edits
        .iter()
        .filter_map(|e| match e {
            Edit::CreateText { value } | Edit::SetText { value } => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

const TALLY: &str = r#"import { h, render } from "./vendor/preact.js";
import { useState } from "./vendor/hooks.js";
import { document } from "architect/dom";

function Tally() {
  const [n, setN] = useState(0);
  const [items, setItems] = useState<string[]>(["a", "b"]);
  const [draft, setDraft] = useState("");
  return (
    <div id="tally" style={{ padding: "8px", color: "var(--foreground)" }}>
      <button id="inc" onClick={() => setN(n + 1)}>+1</button>
      <span id="n">{n}</span>
      <input id="draft" value={draft} onInput={(e: any) => setDraft(e.currentTarget.value)} />
      <button id="add" onClick={() => setItems([draft, ...items])}>Add</button>
      <ul>{items.map((t) => <li key={t}>{t}</li>)}</ul>
    </div>
  );
}

render(<Tally />, document.body);
export default document.body;
"#;

#[test]
fn a_preact_tree_renders_through_the_pages_dom_as_edits() {
    let dir = folder(&[
        ("tally.tsx", TALLY),
        ("vendor/preact.js", PREACT),
        ("vendor/hooks.js", HOOKS),
    ]);
    let mut pane = Pane::open(dir.path(), "tally.tsx");

    // Preact built it in the page's DOM; the pane got it as edits.
    let first = pane.drawn();
    assert_eq!(
        created(&first),
        ["div", "button", "span", "input", "button", "ul", "li", "li"],
        "{first:?}"
    );
    assert_eq!(texts(&first), ["+1", "0", "Add", "a", "b"], "{first:?}");
    assert!(
        first.iter().any(
            |e| matches!(e, Edit::SetAttribute { name, value: Value::Text(v), .. }
            if name == "style" && v == "padding: 8px; color: var(--foreground);")
        ),
        "its style, from Preact's style object: {first:?}"
    );
    assert!(
        first
            .iter()
            .any(|e| matches!(e, Edit::Listen { name } if name == "click")),
        "its click listeners: {first:?}"
    );
    assert!(
        first
            .iter()
            .any(|e| matches!(e, Edit::Listen { name } if name == "input")),
        "its input's: {first:?}"
    );
    assert_eq!(pane.text("#n").as_deref(), Some("0"));

    // A click reaches Preact's listener; its state changes one text.
    let click = pane.click("#inc");
    assert!(created(&click).is_empty(), "{click:?}");
    assert_eq!(texts(&click), ["1"], "{click:?}");
    assert!(
        !click.iter().any(|e| matches!(e, Edit::Remove)),
        "{click:?}"
    );
    assert_eq!(pane.text("#n").as_deref(), Some("1"));
    pane.click("#inc");
    assert_eq!(pane.text("#n").as_deref(), Some("2"));

    // Typed in: the input's value goes to the listener, then to its
    // attribute.
    pane.event("#draft", "input", EventData::Form("z".to_owned()));
    assert_eq!(pane.attr("#draft", "value").as_deref(), Some("z"));

    // A keyed item added at the top: one new `li`, the others kept.
    let before = pane.nodes("li");
    let add = pane.click("#add");
    assert_eq!(created(&add), ["li"], "{add:?}");
    assert_eq!(pane.count("li"), 3);
    assert_eq!(pane.text("li").as_deref(), Some("z"));
    assert_eq!(
        pane.nodes("li")[1..],
        before[..],
        "the old items are the same elements"
    );
    assert_eq!(pane.session.status().error(), None);
}

const BY_HAND: &str = r#"import { document } from "architect/dom";

const log: string[] = [];
const out = document.createElement("p");
out.id = "log";
const list = document.createElement("ol");
for (const t of ["one", "two", "three"]) {
  const li = document.createElement("li");
  li.textContent = t;
  list.appendChild(li);
}
const outer = document.createElement("div");
outer.className = "outer";
outer.addEventListener("click", (e: any) => log.push("outer:" + e.eventPhase), true);
outer.addEventListener("click", (e: any) => { log.push("outer-bubble"); out.textContent = log.join(" "); });
const inner = document.createElement("button");
inner.id = "rotate";
inner.style.backgroundColor = "red";
inner.style.setProperty("--x", "1");
inner.addEventListener("click", (e: any) => {
  log.push("inner:" + e.target.id);
  // Last to first: a move, many mutations, one frame.
  list.insertBefore(list.lastChild, list.firstChild);
  inner.style.backgroundColor = "";
  inner.setAttribute("data-rotated", "yes");
});
outer.append(inner, "text");
document.body.append(outer, list, out);
export default document.body;
"#;

#[test]
fn the_dom_by_hand_events_capture_and_bubble_moves_keep_their_nodes() {
    let dir = folder(&[("hand.tsx", BY_HAND)]);
    let mut pane = Pane::open(dir.path(), "hand.tsx");
    let first = pane.drawn();
    assert_eq!(
        created(&first),
        ["div", "button", "ol", "li", "li", "li", "p"]
    );
    assert_eq!(
        pane.attr("#rotate", "style").as_deref(),
        Some("background-color: red; --x: 1;")
    );
    let three = pane.nodes("li")[2].clone();

    let click = pane.click("#rotate");
    assert!(
        created(&click).is_empty(),
        "a move makes nothing: {click:?}"
    );
    assert_eq!(
        pane.text("#log").as_deref(),
        Some("outer:1 inner:rotate outer-bubble")
    );
    assert_eq!(pane.text("li").as_deref(), Some("three"));
    assert_eq!(
        pane.nodes("li")[0],
        three,
        "the moved item is the same element"
    );
    assert_eq!(pane.attr("#rotate", "style").as_deref(), Some("--x: 1;"));
    assert_eq!(pane.attr("#rotate", "data-rotated").as_deref(), Some("yes"));
}

#[test]
fn a_dom_page_without_its_default_export_says_what_to_export() {
    let dir = folder(&[(
        "none.tsx",
        "import { document } from \"architect/dom\";\ndocument.body.textContent = \"x\";\n",
    )]);
    let pane = Pane::open(dir.path(), "none.tsx");
    let started = Instant::now();
    let error = loop {
        if let Some(e) = pane.session.status().error() {
            break e;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no error");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("architect/dom's document.body"), "{error}");
}
