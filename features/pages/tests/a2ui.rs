//! A2UI JSON pages, end to end: a `.a2ui.json` file opened through
//! `Pages`, drawn through the catalog as the same edits a TSX page's tree
//! is, bound to its extension's data and actions, and kept to its last
//! good self when a save breaks it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};

struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn open(pages: &Pages, path: &Path) -> Self {
        let page = PageFile::new(path, Origin::Override).unwrap();
        let session = pages.open(&page, "lab.test", "summary", None).unwrap();
        let (to, from) = futures::channel::mpsc::unbounded();
        Self {
            session,
            view: RemoteDocument::new(to),
            _events: from,
        }
    }

    fn pump(&mut self) {
        while let Ok(f) = self.session.frames.try_recv() {
            self.view.apply(f).unwrap();
        }
    }

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    fn html(&self) -> String {
        let doc = self.view.document();
        doc.query_selector("body")
            .ok()
            .flatten()
            .and_then(|n| doc.get_node(n))
            .map_or_else(String::new, |n| n.outer_html())
    }

    fn attr(&self, selector: &str, name: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        doc.get_node(node)?
            .attr(blitz_dom::LocalName::from(name))
            .map(str::to_owned)
    }

    /// Applies frames until `done`.
    fn wait_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        loop {
            self.pump();
            if done(self) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{what}; log: {:?}; document: {}",
                self.session.status().log.lines(),
                self.html()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn send(&mut self, selector: &str, name: &str, data: EventData) {
        let element = self
            .attr(selector, "data-dioxus-id")
            .unwrap_or_else(|| panic!("no {selector}"))
            .parse()
            .unwrap();
        self.session.event(ViewEvent {
            element,
            name: name.to_owned(),
            bubbles: true,
            data,
        });
    }
}

static STATE: ActionMeta = ActionMeta {
    id: "LAB_STATE",
    trait_name: "Lab",
    method_name: "state",
    display_name: "State",
    description: "the board",
    category: "Lab",
    group: "Lab",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static PIN: ActionMeta = ActionMeta {
    id: "LAB_PIN",
    trait_name: "Lab",
    method_name: "pin",
    display_name: "Pin",
    description: "pins a title",
    category: "Lab",
    group: "Lab",
    shortcut: "",
    toggleable: false,
    undo: false,
};

#[derive(facet::Facet)]
struct Pin {
    title: String,
}

#[derive(facet::Facet)]
struct Board {
    title: String,
    pins: Vec<String>,
}

/// A list with a board: `LAB_STATE` reads it, `LAB_PIN` adds to it.
fn board() -> (ActionList, Arc<Mutex<Vec<String>>>) {
    let list = ActionList::new();
    let pins = Arc::new(Mutex::new(vec!["first".to_owned()]));
    let theirs = Arc::clone(&pins);
    list.register_async(&STATE, Effect::ReadOnly, move |(): ()| {
        let pins = theirs.lock().unwrap().clone();
        async move {
            Ok::<_, String>(Board {
                title: "The board".to_owned(),
                pins,
            })
        }
    })
    .unwrap();
    let theirs = Arc::clone(&pins);
    list.register_async(&PIN, Effect::Mutating, move |pin: Pin| {
        let mut all = theirs.lock().unwrap();
        all.push(pin.title.clone());
        let n = all.len();
        async move { Ok::<_, String>(format!("{n} pinned")) }
    })
    .unwrap();
    (list, pins)
}

const PAGE: &str = r#"{
  "components": [
    { "id": "root", "component": "Column", "children": ["title", "draft", "pin", "said", "pins", "odd"] },
    { "id": "title", "component": "Text", "variant": "h2", "text": { "path": "/board/title" } },
    { "id": "draft", "component": "TextField", "label": "Title", "value": { "path": "/draft" } },
    { "id": "pin", "component": "Button", "child": "pin-label",
      "action": { "event": { "name": "LAB_PIN", "context": { "title": { "path": "/draft" } },
                             "wantResponse": true, "responsePath": "/said" } } },
    { "id": "pin-label", "component": "Text", "text": "Pin" },
    { "id": "said", "component": "Text", "text": { "path": "/said" } },
    { "id": "pins", "component": "List", "children": { "componentId": "pin-row", "path": "/board/pins" } },
    { "id": "pin-row", "component": "Text", "text": { "path": "" } },
    { "id": "odd", "component": "Marquee", "text": "not in the catalog" }
  ],
  "data": { "draft": "", "said": "-" },
  "bind": { "/board": { "action": "LAB_STATE" } }
}
"#;

fn pages(list: ActionList) -> Pages {
    Pages::new(AppHost::new(list)).with_a2ui()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_a2ui_page_is_bound_to_its_extensions_data_and_actions() {
    let (list, pins) = board();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("summary.a2ui.json");
    std::fs::write(&path, PAGE).unwrap();
    let pages = pages(list);
    tokio::task::block_in_place(|| {
        let mut pane = Pane::open(&pages, &path);
        // Its binding read LAB_STATE; its template drew each pin.
        pane.wait_until("the board's title", |p| {
            p.text("h2").as_deref() == Some("The board")
        });
        pane.wait_until("the pins", |p| {
            p.text(".a2ui-List").as_deref() == Some("first")
        });
        // Outside the catalog: the labelled placeholder, never drawn.
        let odd = pane.text(".a2ui-placeholder").unwrap();
        assert!(odd.contains("Marquee"), "{odd}");
        // Typed into its bound field, then pressed: the action gets the
        // context read through the data model, its answer goes to
        // `responsePath`, and the binding reads the board again.
        pane.send(
            ".a2ui-TextField input",
            "input",
            EventData::Form("second".to_owned()),
        );
        pane.wait_until("the typed draft", |p| {
            p.attr(".a2ui-TextField input", "value").as_deref() == Some("second")
        });
        pane.send(
            ".a2ui-Button button",
            "click",
            EventData::Mouse(Pointer::default()),
        );
        pane.wait_until("the action's answer, and the board read again", |p| {
            p.text(".a2ui-Column")
                .is_some_and(|t| t.contains("2 pinned"))
                && p.text(".a2ui-List").as_deref() == Some("firstsecond")
        });
        assert_eq!(*pins.lock().unwrap(), ["first", "second"]);
    });
}

#[test]
fn a_broken_a2ui_save_keeps_the_last_good_page_and_says_where() {
    let (list, _) = board();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("summary.a2ui.json");
    std::fs::write(&path, PAGE).unwrap();
    let pages = pages(list);
    let mut pane = Pane::open(&pages, &path);
    pane.wait_until("the page", |p| p.text(".a2ui-Button").as_deref() == Some("Pin"));
    // A trailing comma on line 3: not JSON.
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(
        &path,
        "{\n  \"components\": [\n    { \"id\": \"root\", \"component\": \"Text\", \"text\": \"x\" },\n  ]\n}\n",
    )
    .unwrap();
    pane.wait_until("the banner", |p| p.text(".page-error").is_some());
    let error = std::fs::read_to_string(dir.path().join("summary.a2ui.json.error")).unwrap();
    assert!(error.starts_with("summary.a2ui.json:4:"), "{error}");
    assert_eq!(pane.text(".a2ui-Button").as_deref(), Some("Pin"));
    let problems = pages.status_problems();
    assert!(
        problems
            .iter()
            .any(|p| p.message.contains("summary.a2ui.json:4:")),
        "{problems:?}"
    );
    // No root: said so.
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(
        &path,
        r#"{ "components": [{ "id": "top", "component": "Text", "text": "x" }] }"#,
    )
    .unwrap();
    pane.wait_until("the missing root", |p| {
        p.text(".page-error")
            .is_some_and(|t| t.contains("no component \"root\""))
    });
    // Fixed: the page, and both gone.
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(
        &path,
        r#"{ "components": [{ "id": "root", "component": "Text", "text": "fixed" }] }"#,
    )
    .unwrap();
    pane.wait_until("the fixed page", |p| {
        p.text(".a2ui-Text").as_deref() == Some("fixed") && p.text(".page-error").is_none()
    });
    assert!(!dir.path().join("summary.a2ui.json.error").exists());
}

#[test]
fn an_a2ui_page_is_checked_once_and_takes_the_message_form() {
    let pages = pages(ActionList::new());
    let dir = tempfile::tempdir().unwrap();
    // The messages an agent sends (A2UI_RENDER's), as a page.
    let messages = dir.path().join("hello.a2ui.json");
    std::fs::write(
        &messages,
        r#"[
  { "createSurface": { "surfaceId": "s", "catalogId": "https://architect.fasttrackstudio.dev/a2ui/catalog/v1" } },
  { "updateComponents": { "surfaceId": "s", "components": [
    { "id": "root", "component": "Card", "child": "t" },
    { "id": "t", "component": "Text", "text": { "path": "/name" } }
  ] } },
  { "updateDataModel": { "surfaceId": "s", "path": "/name", "value": "Ada" } }
]"#,
    )
    .unwrap();
    let checked = pages.check_file(&messages).unwrap();
    assert!(checked.nodes >= 3, "{checked:?}");
    let bad = dir.path().join("bad.a2ui.json");
    std::fs::write(&bad, r#"{ "components": [], "extra": 1 }"#).unwrap();
    let why = pages.check_file(&bad).unwrap_err();
    assert!(why.contains("no field \"extra\""), "{why}");
}
