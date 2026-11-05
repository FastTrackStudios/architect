//! The editor from a page (#312): a TSX page and a Luau page, each its
//! extension's, read the editor being typed in, indent its list item as
//! one undo step and decorate it, through `editor` (the host service's
//! editor calls, as the extension), on `editor::MemoryEditor`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::editor::MemoryEditor;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;

const ID: &str = "lab.page-editor";

const MANIFEST: &str = r#"
id = "lab.page-editor"
name = "Page editor"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
writes = ["editor"]
"#;

const TSX: &str = r#"import { h, useState, useEffect, editor } from "architect";

export default function Ed() {
  const [out, setOut] = useState<string>("");
  useEffect(() => {
    editor.state().then((ed) =>
      editor.edit({ changes: [{ from: ed.node.line_from, to: ed.node.line_from, insert: "  " }], name: "Indent", base: ed.version })
        .then((after) => editor.decorate([{ kind: "mark", from: 0, to: 1, class: "hot" }])
          .then(() => setOut(ed.node.kind + "|" + JSON.stringify(after.text)))));
  }, []);
  return <span id="out">{out}</span>;
}
"#;

const LUAU: &str = r#"
local ed = editor.state()
local after = editor.edit({ changes = {{ from = ed.node.line_from, to = ed.node.line_from, insert = "  " }}, name = "Indent", base = ed.version })
editor.decorate({{ kind = "mark", from = 0, to = 1, class = "hot" }})
render(function()
  return span():attr("id", "out"):child(ed.node.kind .. "|" .. after.text)
end)
"#;

fn extensions(editor: &Arc<MemoryEditor>) -> (Extensions, Pages, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_editors(editor.clone());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-editor");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), MANIFEST).unwrap();
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

fn edited_as_the_extension(editor: &MemoryEditor) {
    assert_eq!(editor.text(), "- a\n  - b");
    let steps = editor.steps();
    assert_eq!(steps.len(), 1, "one step");
    assert_eq!(
        (steps[0].name.as_str(), steps[0].by.as_str()),
        ("Indent", "extension:lab.page-editor")
    );
    assert!(editor.decorations().contains_key(ID));
}

#[test]
fn a_tsx_page_reads_edits_and_decorates_the_editor() {
    let editor = Arc::new(MemoryEditor::new("e", "note", "- a\n- b"));
    let (ext, pages, _tmp) = extensions(&editor);
    assert_eq!(
        out_of(&ext, &pages, "ed.tsx", TSX),
        "list-item|\"- a\\n  - b\""
    );
    edited_as_the_extension(&editor);
}

#[test]
fn a_luau_page_reads_edits_and_decorates_the_editor() {
    let editor = Arc::new(MemoryEditor::new("e", "note", "- a\n- b"));
    let (ext, pages, _tmp) = extensions(&editor);
    assert_eq!(
        out_of(&ext, &pages, "ed.luau", LUAU),
        "list-item|- a\n  - b"
    );
    edited_as_the_extension(&editor);
}
