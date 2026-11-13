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
actions = ["LAB_VAULT"]
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
                    let end = at.saturating_add(*len).min(chars.len());
                    chars.drain((*at).min(end)..end);
                }
                DocEdit::Replace { text } => chars = text.chars().collect(),
            }
        }
        *text = chars.into_iter().collect();
        let after = text.clone();
        drop(texts);
        self.steps
            .lock()
            .unwrap()
            .push((step.to_owned(), by.describe()));
        Ok(after)
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        let mut texts = self.texts.lock().unwrap();
        let text = texts
            .remove(path)
            .ok_or_else(|| DocError::NotFound(path.to_owned()))?;
        texts.insert(to.to_owned(), text);
        drop(texts);
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        self.texts
            .lock()
            .unwrap()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| DocError::NotFound(path.to_owned()))
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

/// A page that reads the person's documents, through its extension's
/// host calls and through an action, and says what it got.
const READER: &str = r#"
local ok, got = pcall(function() return documents.read("private/diary.md") end)
local listed, all = pcall(function() return documents.list() end)
local ran, out = pcall(function() return actions.run("LAB_VAULT_READ") end)
local said = (ok and got or "no read") .. "|" .. (listed and tostring(#all) or "no list") .. "|" .. (ran and tostring(out) or "no run")
render(function()
  return span():attr("id", "out"):child(said)
end)
"#;

/// What `READER` says opened as its extension's embed, or as a guest's.
fn read_as(ext: &Extensions, pages: &Pages, guest: bool) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reader.luau");
    std::fs::write(&path, READER).unwrap();
    let manifest = ext.host().get(ID).unwrap().manifest;
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let input = architect_extensions::contrib::EmbedInput {
        claim: "note:setlist".to_owned(),
        source: "a shared note".to_owned(),
        props: serde_json::json!({ "guest": true }),
    };
    let mut session = if guest {
        pages.open_guest_embed(&page, ID, "page", input).unwrap()
    } else {
        pages
            .open_embed(&page, ID, "page", Some(gate(&manifest)), input)
            .unwrap()
    };
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

#[test]
fn an_extension_drawing_a_guests_content_cannot_read_the_persons_vault() {
    static VAULT_READ: architect::action::ActionMeta = architect::action::ActionMeta {
        id: "LAB_VAULT_READ",
        trait_name: "",
        method_name: "",
        display_name: "Read the vault",
        description: "The person's vault, as text.",
        category: "Lab",
        group: "Lab",
        shortcut: "",
        toggleable: false,
        undo: false,
    };
    let docs = Memo::default();
    docs.texts
        .lock()
        .unwrap()
        .insert("private/diary.md".to_owned(), "the person's own".to_owned());
    let (ext, pages, _tmp) = extensions(&docs);
    // The person's vault behind an action the extension's namespace holds.
    let vault = docs;
    let list = ext.host().list();
    list.register(
        &VAULT_READ,
        architect::action::Effect::ReadOnly,
        move |(): ()| vault.read("private/diary.md").map_err(|e| e.to_string()),
    )
    .unwrap();
    // As its own embed it reads what its capabilities grant.
    let own = read_as(&ext, &pages, false);
    assert_eq!(own, "the person's own|1|the person's own");
    // Drawing a guest's content, it reaches none of it.
    assert_eq!(read_as(&ext, &pages, true), "no read|no list|no run");
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
