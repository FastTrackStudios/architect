//! A page's way to the app's interface (#306): a TSX page on `QuickJS`
//! and a Luau page, each its extension's, notify, write and read the
//! clipboard, open a picker, a prompt and a menu and get the answers back,
//! and badge a file-tree row, through `ui` and `clipboard` (`EXT_HOST_CALL`,
//! as the extension); a call outside its capabilities is refused. The
//! host's interface is a `FakeUi`, which answers at once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::hostcall::Target;
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;
use serde_json::{Value, json};

const ID: &str = "lab.page-ask";

const MANIFEST: &str = r#"
id = "lab.page-ask"
name = "Page ask"
version = "0.1.0"
kind = "data"
api = "0.1"

[capabilities]
ui = ["notify", "clipboard", "pickers", "prompts", "menus", "badges"]
"#;

const TSX: &str = r#"import { h, useState, useEffect, ui, clipboard, notify } from "architect";

export default function Ask() {
  const [out, setOut] = useState<string>("");
  useEffect(() => {
    (async () => {
      notify("hello from a page", "success");
      await clipboard.write("copied by a page");
      const clip = await clipboard.read();
      const paths = await ui.pick({ kind: "open_many" });
      const name = await ui.input("Name?", { default: "untitled" });
      const sure = await ui.confirm("Sure?");
      const chosen = await ui.menu([{ id: "go", label: "Go" }], { x: 1, y: 2 });
      await ui.decorate({ kind: "file", id: "a.md" }, { badge: "P" });
      let refused = "";
      try {
        await ui.call({ op: "selection" });
      } catch (e) {
        refused = String((e as Error).message);
      }
      setOut(JSON.stringify({ clip, paths, name, sure, chosen, refused }));
    })();
  }, []);
  return <span id="out">{out}</span>;
}
"#;

const LUAU: &str = r#"
clipboard.write("copied by a Luau page")
local clip = clipboard.read()
local name = ui.prompt({ kind = "text", title = "Name?" })
local chosen = ui.menu({ { id = "a", label = "A" } }, { x = 3, y = 4 })
local ok, why = pcall(function() return ui.call({ op = "org" }) end)
render(function()
  return span():attr("id", "out"):child(clip .. "|" .. name .. "|" .. chosen .. "|" .. tostring(ok))
end)
"#;

struct App {
    pages: Pages,
    ui: FakeUi,
    ext: Extensions,
    _tmp: tempfile::TempDir,
}

fn app(ui: FakeUi) -> App {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(ui.clone());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-ask");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), MANIFEST).unwrap();
    let _ = ext.dev(&dir);
    assert!(ext.host().get(ID).is_some(), "{:?}", ext.problems());
    let pages = Pages::new(AppHost::new(list)).with_js().with_luau();
    App {
        pages,
        ui,
        ext,
        _tmp: tmp,
    }
}

/// Opens `name` (with `text`) as the extension's page, its gate its
/// capabilities: its `#out` once it says something.
fn out_of(app: &App, name: &str, text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    let manifest = app.ext.host().get(ID).unwrap().manifest;
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let mut session = app
        .pages
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

fn apply(session: &mut PageSession, view: &mut RemoteDocument) {
    match session.frames.try_recv() {
        Ok(frame) => view.apply(frame).unwrap(),
        Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
        Err(_) => std::thread::sleep(Duration::from_millis(5)),
    }
}

fn text_of(view: &RemoteDocument, selector: &str) -> Option<String> {
    let doc = view.document();
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

fn wait(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(5), "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_tsx_page_notifies_copies_asks_and_gets_the_answers_back() {
    let app = app(FakeUi::new().answer(json!(["/a.csv", "/b.csv"])));
    let out: Value = serde_json::from_str(&out_of(&app, "ask.tsx", TSX)).unwrap();
    let refused = out["refused"].as_str().unwrap().to_owned();
    assert_eq!(
        out,
        json!({
            "clip": "copied by a page",
            "paths": ["/a.csv", "/b.csv"],
            "name": "untitled",
            "sure": true,
            "chosen": "go",
            "refused": refused,
        })
    );
    // Its extension may not read the selection.
    assert!(refused.contains("refused (reads.selection)"), "{refused}");
    wait("the page's notification", || !app.ui.notices().is_empty());
    let n = &app.ui.notices()[0];
    assert_eq!((n.text.as_str(), n.kind.as_str(), n.from.id.as_str()), ("hello from a page", "success", ID));
    let badge = app.ext.host().decorations_of(&Target::file("a.md"));
    assert_eq!((badge[0].from.as_str(), badge[0].decoration.badge.as_deref()), (ID, Some("P")));
    assert!(app.ui.asked().iter().all(|q| q.from == ID));
}

#[test]
fn a_luau_page_copies_asks_and_gets_the_answers_back() {
    let app = app(FakeUi::new().answer(json!("Ada")));
    assert_eq!(
        out_of(&app, "ask.luau", LUAU),
        "copied by a Luau page|Ada|a|false"
    );
    assert_eq!(app.ui.clipboard().as_deref(), Some("copied by a Luau page"));
}
