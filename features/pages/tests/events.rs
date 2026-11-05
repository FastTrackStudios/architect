//! A page's hooks made at run time (#308): a TSX page and a Luau page,
//! each its extension's, subscribe one of its actions to a document's
//! save through `events.subscribe` (the host service's `subscribe`, as the
//! extension), and the save runs it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::events::DocEvent;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate, now_ms};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;

const ID: &str = "lab.page-events";

const MANIFEST: &str = r#"
id = "lab.page-events"
name = "Page events"
version = "0.1.0"
kind = "luau"
api = "0.1"

# Its pages hook doc.saved: a document's events are reading the documents.
[capabilities]
reads = ["documents"]

[[actions]]
id = "LAB_PAGE_EVENTS_NOTE"
name = "Note the save"
params = '{"type":"object"}'
script = "note.luau"
"#;

const NOTE: &str = r#"
ext.state{ key = "saved", text = args.detail.path }
"#;

const TSX: &str = r#"import { h, useState, useEffect, events } from "architect";

export default function Subscribe() {
  const [key, setKey] = useState<string>("");
  useEffect(() => {
    events.subscribe({ on: "doc.saved", action: "LAB_PAGE_EVENTS_NOTE" }).then(setKey);
  }, []);
  return <span id="out">{key}</span>;
}
"#;

const LUAU: &str = r#"
local key = events.subscribe({ on = "doc.saved", action = "LAB_PAGE_EVENTS_NOTE" })
local heard = events.list()
render(function()
  return span():attr("id", "out"):child(key .. "|" .. tostring(#heard.subscribed))
end)
"#;

fn extensions() -> (Extensions, Pages, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-events");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), MANIFEST).unwrap();
    std::fs::write(dir.join("note.luau"), NOTE).unwrap();
    let _ = ext.dev(&dir);
    assert!(ext.host().get(ID).is_some(), "{:?}", ext.problems());
    let pages = Pages::new(AppHost::new(list)).with_js().with_luau();
    (ext, pages, tmp)
}

fn out_of(ext: &Extensions, pages: &Pages, name: &str, text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    let manifest = ext.host().get(ID).unwrap().manifest;
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let mut session = pages
        .open(&page, ID, "page", Some(gate(&manifest)))
        .unwrap();
    let (to, _from) = futures::channel::mpsc::unbounded::<ViewEvent>();
    let mut view = RemoteDocument::new(to);
    let started = Instant::now();
    loop {
        if let Some(out) = text_of(&view, "#out").filter(|t| !t.is_empty()) {
            return out;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no answer; log: {:?}",
            session.status().log.lines()
        );
        apply(&mut session, &mut view);
    }
}

fn text_of(view: &RemoteDocument, selector: &str) -> Option<String> {
    let doc = view.document();
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

fn apply(session: &mut PageSession, view: &mut RemoteDocument) {
    match session.frames.try_recv() {
        Ok(frame) => view.apply(frame).unwrap(),
        Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
        Err(_) => std::thread::sleep(Duration::from_millis(5)),
    }
}

fn saved(ext: &Extensions) -> Option<String> {
    ext.host()
        .contributions()
        .state(now_ms())
        .into_iter()
        .find(|s| s.source == format!("ext:{ID}") && s.key == "saved")
        .map(|s| s.text)
}

fn save_runs_the_hook(ext: &Extensions, path: &str) {
    let n = ext.host().raise_doc(&DocEvent::Saved { path: path.into() });
    assert_eq!(n, 1);
    let started = Instant::now();
    while saved(ext).as_deref() != Some(path) {
        assert!(started.elapsed() < Duration::from_secs(5), "the hook ran");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_tsx_page_subscribes_its_extensions_action_to_a_save() {
    let (ext, pages, _tmp) = extensions();
    assert_eq!(
        out_of(&ext, &pages, "subscribe.tsx", TSX),
        "doc.saved/LAB_PAGE_EVENTS_NOTE"
    );
    save_runs_the_hook(&ext, "notes/tsx.md");
}

#[test]
fn a_luau_page_subscribes_its_extensions_action_to_a_save() {
    let (ext, pages, _tmp) = extensions();
    assert_eq!(
        out_of(&ext, &pages, "subscribe.luau", LUAU),
        "doc.saved/LAB_PAGE_EVENTS_NOTE|1"
    );
    save_runs_the_hook(&ext, "notes/luau.md");
}
