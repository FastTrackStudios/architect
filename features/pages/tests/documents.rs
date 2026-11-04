//! A page's documents (#309): a TSX page and a Luau page, each its
//! extension's, create a document, edit it as one transaction and move it
//! through `documents` (the host service's document calls, as the
//! extension), on an app's own document set — a `Documents` the app
//! implements, here in memory.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionList, Principal};
use architect_extensions::documents::{DocEdit, DocError, DocInfo, Documents, is_under};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;

const ID: &str = "lab.page-docs";

const MANIFEST: &str = r#"
id = "lab.page-docs"
name = "Page documents"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
reads = ["documents"]
writes = ["documents"]
"#;

const TSX: &str = r#"import { h, useState, useEffect, documents } from "architect";

export default function Docs() {
  const [out, setOut] = useState<string>("");
  useEffect(() => {
    documents.create("notes/tsx.md", "one")
      .then(() => documents.edit("notes/tsx.md", [{ append: { text: " two" } }, { insert: { at: 0, text: "> " } }], "Page edit"))
      .then((text) => documents.move("notes/tsx.md", "archive").then((moved) =>
        documents.list().then((all) => setOut(text + "|" + moved.path + "|" + all.length))));
  }, []);
  return <span id="out">{out}</span>;
}
"#;

const LUAU: &str = r#"
documents.create("notes/luau.md", "one")
local text = documents.edit("notes/luau.md", {{ append = { text = " two" } }, { insert = { at = 0, text = "> " } }}, "Page edit")
local moved = documents.move("notes/luau.md", "archive")
local all = documents.list()
render(function()
  return span():attr("id", "out"):child(text .. "|" .. moved.path .. "|" .. tostring(#all))
end)
"#;

/// An app's documents: texts by path, and each step with who made it.
#[derive(Clone, Default)]
struct Memo {
    texts: Arc<Mutex<BTreeMap<String, String>>>,
    steps: Arc<Mutex<Vec<(String, String)>>>,
}

impl Documents for Memo {
    fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError> {
        Ok(self
            .texts
            .lock()
            .unwrap()
            .keys()
            .filter(|p| is_under(p, under))
            .map(|p| DocInfo::of(p))
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, DocError> {
        self.texts
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| DocError::NotFound(path.to_owned()))
    }

    fn create(&self, path: &str, text: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        self.texts
            .lock()
            .unwrap()
            .insert(path.to_owned(), text.to_owned());
        Ok(DocInfo::of(path))
    }

    fn edit(
        &self,
        path: &str,
        edits: &[DocEdit],
        step: &str,
        by: &Principal,
    ) -> Result<String, DocError> {
        let mut texts = self.texts.lock().unwrap();
        let text = texts
            .get_mut(path)
            .ok_or_else(|| DocError::NotFound(path.to_owned()))?;
        let mut chars: Vec<char> = text.chars().collect();
        for edit in edits {
            match edit {
                DocEdit::Insert { at, text } => {
                    let at = (*at).min(chars.len());
                    chars.splice(at..at, text.chars());
                }
                DocEdit::Append { text } => chars.extend(text.chars()),
                DocEdit::Delete { at, len } => {
                    chars.drain(*at..at + len);
                }
                DocEdit::Replace { text } => chars = text.chars().collect(),
            }
        }
        *text = chars.into_iter().collect();
        self.steps
            .lock()
            .unwrap()
            .push((step.to_owned(), by.describe()));
        Ok(text.clone())
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        let mut texts = self.texts.lock().unwrap();
        let text = texts
            .remove(path)
            .ok_or_else(|| DocError::NotFound(path.to_owned()))?;
        texts.insert(to.to_owned(), text);
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        self.texts.lock().unwrap().remove(path).map(|_| ()).ok_or_else(|| DocError::NotFound(path.to_owned()))
    }
}

fn extensions(docs: &Memo) -> (Extensions, Pages, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host =
        Host::new(list.clone(), SharedSections::new(Registry::new())).with_documents(docs.clone());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    // The page's calls to its extension's host (`EXT_PAGE_CALL`).
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-docs");
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

fn made_as_the_extension(docs: &Memo) {
    let steps = docs.steps.lock().unwrap().clone();
    assert_eq!(
        steps,
        [("Page edit".to_owned(), format!("extension:{ID}"))],
        "one step, as the extension"
    );
}

#[test]
fn a_tsx_page_creates_edits_and_moves_a_document() {
    let docs = Memo::default();
    let (ext, pages, _tmp) = extensions(&docs);
    assert_eq!(
        out_of(&ext, &pages, "docs.tsx", TSX),
        "> one two|archive/tsx.md|1"
    );
    assert_eq!(docs.read("archive/tsx.md").unwrap(), "> one two");
    made_as_the_extension(&docs);
}

#[test]
fn a_luau_page_creates_edits_and_moves_a_document() {
    let docs = Memo::default();
    let (ext, pages, _tmp) = extensions(&docs);
    assert_eq!(
        out_of(&ext, &pages, "docs.luau", LUAU),
        "> one two|archive/luau.md|1"
    );
    made_as_the_extension(&docs);
}
