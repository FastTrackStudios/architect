//! The extension showcase's examples (#710, `examples/showcase`,
//! `docs/content/guide/showcase.md`), each loaded from its own folder as a
//! development copy, as `just showcase` loads it, and run through the
//! actions a person runs:
//!
//! - the Notify sampler (H7, H8, H9, H10): a toast, the clipboard, a file
//!   picker, prompts and a menu at a point, answered by a `FakeUi`;
//! - Templater-lite (R2, R3, C1, C7, C11, H1, H10): its templates listed
//!   with their fields, a note made from one through `VAULT_CREATE` with the
//!   fields asked in prompts, today's date at the cursor of the live
//!   editor, and its page and slash row declared.
//!
//! The vault here is a small stand-in for the app's `VAULT_*` actions
//! (`features/vault/vault/src/actions.rs`): what they take and answer, on
//! notes in memory.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use architect_extensions::editor::MemoryEditor;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostui::FakeUi;
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use facet::Facet;
use futures::executor::block_on;
use serde_json::{Value, json};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(name)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, ActionError> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
}

/// A host on `list` with `ui` as its interface, the example loaded as a
/// development copy (it runs with what its manifest asks).
fn load(list: &ActionList, ui: &FakeUi, name: &str) -> Host {
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(ui.clone());
    let package = Package::read(&example(name)).unwrap();
    host.load(&package, Origin::Dev).unwrap();
    host
}

// ---- the vault's actions, in memory -------------------------------------

/// Notes by vault path: their properties and body.
#[derive(Clone, Default)]
struct Notes(Arc<Mutex<BTreeMap<String, (Value, String)>>>);

impl Notes {
    fn put(&self, path: &str, properties: Value, body: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(path.to_owned(), (properties, body.to_owned()));
    }

    fn get(&self, path: &str) -> Option<(Value, String)> {
        self.0.lock().unwrap().get(path).cloned()
    }
}

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: id,
        display_name: id,
        description: "",
        category: "Vault",
        group: "Vault",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta("VAULT_LIST");
static READ: ActionMeta = meta("VAULT_READ");
static CREATE: ActionMeta = meta("VAULT_CREATE");

/// `VAULT_LIST`'s arguments, as the app's take them.
#[derive(Facet)]
struct ListArgs {
    #[facet(default)]
    folder: Option<String>,
}

/// `VAULT_READ`'s.
#[derive(Facet)]
struct PathArgs {
    path: String,
}

/// `VAULT_CREATE`'s.
#[derive(Facet)]
struct CreateArgs {
    path: String,
    #[facet(default)]
    body: Option<String>,
    #[facet(default)]
    properties: Option<String>,
}

fn raw(v: &Value) -> RawJson {
    RawJson::from(v.to_string())
}

/// `VAULT_LIST`, `VAULT_READ` and `VAULT_CREATE` over `notes`, taking
/// and answering what the app's do.
fn vault(list: &ActionList, notes: &Notes) {
    let n = notes.clone();
    list.register_with(
        &LIST,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: ListArgs| {
            let folder = a.folder.unwrap_or_default();
            let paths: Vec<String> = n.0.lock().unwrap().keys().cloned().collect();
            let out: Vec<Value> = paths
                .iter()
                .filter(|p| folder.is_empty() || p.starts_with(&format!("{folder}/")))
                .map(|p| {
                    let title = p.rsplit('/').next().unwrap().trim_end_matches(".md");
                    let parent = p.rsplit_once('/').map_or("", |(f, _)| f);
                    json!({ "path": p, "title": title, "folder": parent })
                })
                .collect();
            Ok::<_, String>(raw(&Value::Array(out)))
        },
    )
    .unwrap();
    let n = notes.clone();
    list.register_with(
        &READ,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: PathArgs| {
            let (properties, body) = n
                .get(&a.path)
                .ok_or_else(|| format!("{}: no such note", a.path))?;
            Ok::<_, String>(raw(&json!({
                "path": a.path, "properties": properties, "body": body,
                "tags": [], "links": [], "backlinks": [],
            })))
        },
    )
    .unwrap();
    let n = notes.clone();
    list.register_with(
        &CREATE,
        Effect::Mutating,
        move |_: &ActionCaller, a: CreateArgs| {
            if n.get(&a.path).is_some() {
                return Err(format!("{}: already there", a.path));
            }
            let properties = match a.properties.as_deref() {
                Some(text) => serde_json::from_str(text).map_err(|e| format!("properties: {e}"))?,
                None => json!({}),
            };
            n.put(&a.path, properties, a.body.as_deref().unwrap_or(""));
            Ok(raw(&json!({ "path": a.path })))
        },
    )
    .unwrap();
}

/// The showcase vault's templates (`apps/ui/showcase-vault/Templates`).
fn templates(notes: &Notes) {
    notes.put(
        "Templates/Meeting.md",
        json!({ "title": "{{title}}", "tags": ["meeting"], "date": "{{date}}" }),
        "# {{title}}\n\nMet on {{date}} with {{who}}.\n",
    );
    notes.put(
        "Templates/Daily.md",
        json!({ "title": "{{date}}", "tags": ["daily"] }),
        "# {{date}}\n\n## Plan\n",
    );
    notes.put("Notes/Other.md", json!({}), "Not a template.\n");
}

// ---- the Notify sampler -------------------------------------------------

#[test]
fn the_notify_sampler_toasts_copies_picks_asks_and_opens_a_menu() {
    let list = ActionList::new();
    let picked = "/tmp/showcase/picked.md";
    let ui = FakeUi::new()
        // pick
        .answer(json!(picked))
        // ask: a name, a colour, a yes
        .answer(json!("Grace"))
        .answer(json!("amber"))
        .answer(json!(true));
    let _host = load(&list, &ui, "notify-sampler");

    // H7: a toast, said as the extension.
    let out = run(
        &list,
        "SHOWCASE_NOTIFY_TOAST",
        &json!({ "text": "Hi", "kind": "success" }),
    )
    .unwrap();
    assert_eq!(out, "Hi");
    let n = ui.notices();
    assert_eq!((n[0].text.as_str(), n[0].kind.as_str()), ("Hi", "success"));
    assert_eq!(n[0].from.name, "Notify sampler");

    // H8: the clipboard, written and read back.
    let out = run(&list, "SHOWCASE_NOTIFY_COPY", &json!({ "text": "a link" })).unwrap();
    assert_eq!(out, "a link");
    assert_eq!(ui.clipboard().as_deref(), Some("a link"));

    // H9: the picker, with its filter.
    let out = run(&list, "SHOWCASE_NOTIFY_PICK", &Value::Null).unwrap();
    assert_eq!(out, picked);

    // H10: a text prompt, a pick-one and a confirm.
    let out = run(&list, "SHOWCASE_NOTIFY_ASK", &Value::Null).unwrap();
    assert_eq!(
        out,
        json!({ "name": "Grace", "colour": "amber", "sure": true })
    );
    assert!(
        ui.notices().iter().any(|n| n.text == "Grace likes amber"),
        "{:?}",
        ui.notices()
    );

    // C3: a menu at a point; the fake chooses the first row it may.
    let out = run(&list, "SHOWCASE_NOTIFY_MENU", &json!({ "x": 40, "y": 60 })).unwrap();
    assert_eq!(out, "copy");

    let asked: Vec<&str> = ui.asked().iter().map(|q| q.request.op()).collect();
    assert_eq!(asked, ["pick", "prompt", "prompt", "prompt", "menu"]);
}

#[test]
fn the_notify_sampler_asks_for_nothing_past_the_interface() {
    let package = Package::read(&example("notify-sampler")).unwrap();
    let caps = &package.manifest.capabilities;
    assert!(caps.actions.is_empty() && caps.hosts.is_empty() && caps.writes.is_empty());
    assert_eq!(
        caps.ui,
        ["notify", "clipboard", "pickers", "prompts", "menus"]
    );
    // It says what it stands in for (#710).
    assert!(package.manifest.description.contains("Stands in for"));
}

// ---- Templater-lite -----------------------------------------------------

#[test]
fn templater_lite_lists_its_templates_with_the_fields_each_asks_for() {
    let list = ActionList::new();
    let notes = Notes::default();
    vault(&list, &notes);
    templates(&notes);
    let _host = load(&list, &FakeUi::new(), "templater-lite");

    let out = run(&list, "SHOWCASE_TEMPLATER_LIST", &Value::Null).unwrap();
    let by_path: BTreeMap<&str, &Value> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["path"].as_str().unwrap(), &t["fields"]))
        .collect();
    assert_eq!(by_path.len(), 2, "only Templates/: {out}");
    // Named by their files: a template's own `title` is a field.
    let titles: Vec<&str> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Daily", "Meeting"]);
    let meeting: Vec<&str> = by_path["Templates/Meeting.md"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    for f in ["title", "date", "who"] {
        assert!(meeting.contains(&f), "{f} in {meeting:?}");
    }
}

#[test]
fn templater_lite_makes_a_note_from_a_template_asking_for_its_fields() {
    let list = ActionList::new();
    let notes = Notes::default();
    vault(&list, &notes);
    templates(&notes);
    let ui = FakeUi::new()
        // which template, its title, then {{who}}
        .answer(json!("Templates/Meeting.md"))
        .answer(json!("Standup"))
        .answer(json!("the band"));
    let _host = load(&list, &ui, "templater-lite");

    let out = run(
        &list,
        "SHOWCASE_TEMPLATER_NEW",
        &json!({ "date": "2026-11-14" }),
    )
    .unwrap();
    assert_eq!(out, "Inbox/Standup.md");
    let (properties, body) = notes.get("Inbox/Standup.md").expect("the note made");
    assert_eq!(body, "# Standup\n\nMet on 2026-11-14 with the band.\n");
    assert_eq!(
        properties,
        json!({ "title": "Standup", "tags": ["meeting"], "date": "2026-11-14" })
    );
    let asked: Vec<&str> = ui.asked().iter().map(|q| q.request.op()).collect();
    assert_eq!(asked, ["prompt", "prompt", "prompt"]);
    assert!(
        ui.notices()
            .iter()
            .any(|n| n.text == "Made Inbox/Standup.md")
    );

    // Given everything, it asks nothing: what an agent or the CLI runs.
    let quiet = FakeUi::new();
    let list = ActionList::new();
    vault(&list, &notes);
    let _host = load(&list, &quiet, "templater-lite");
    let out = run(
        &list,
        "SHOWCASE_TEMPLATER_NEW",
        &json!({
            "template": "Templates/Daily.md",
            "title": "2026-11-15",
            "date": "2026-11-15",
            "folder": "Journal",
        }),
    )
    .unwrap();
    assert_eq!(out, "Journal/2026-11-15.md");
    assert!(quiet.asked().is_empty(), "{:?}", quiet.asked());
    let (properties, body) = notes.get("Journal/2026-11-15.md").unwrap();
    assert_eq!(properties["title"], "2026-11-15");
    assert!(body.starts_with("# 2026-11-15\n"), "{body}");
}

#[test]
fn templater_lite_inserts_the_date_at_the_cursor_as_one_undo_step() {
    let list = ActionList::new();
    let host = load(&list, &FakeUi::new(), "templater-lite");
    let editor = Arc::new(MemoryEditor::new("note/Inbox.md", "note", "Due: \nnext"));
    host.set_editors(editor.clone());
    editor.put_caret("Due: ".len());

    let out = run(
        &list,
        "SHOWCASE_TEMPLATER_DATE",
        &json!({ "date": "2026-11-14" }),
    )
    .unwrap();
    assert_eq!(out, "2026-11-14");
    assert_eq!(editor.text(), "Due: 2026-11-14\nnext");
    let _ = editor.undo();
    assert_eq!(editor.text(), "Due: \nnext", "one undo step");
}

#[test]
fn templater_lite_declares_its_page_its_slash_row_its_key_and_its_reach() {
    let package = Package::read(&example("templater-lite")).unwrap();
    let m = &package.manifest;
    assert!(
        m.description
            .contains("Stands in for Templater and QuickAdd")
    );
    // R3: its tab is a TSX page.
    let view = m.views.iter().find(|v| v.id == "templates").unwrap();
    assert_eq!(view.page.as_deref(), Some("pages/templates.tsx"));
    assert!(
        example("templater-lite")
            .join("pages/templates.tsx")
            .is_file()
    );
    // It reaches the vault only through these actions, and never deletes.
    assert_eq!(
        m.capabilities.actions,
        ["VAULT_LIST", "VAULT_READ", "VAULT_CREATE", "EXT_VIEW_OPEN"]
    );
    // The vault's actions are the documents: it reads them and makes
    // notes, so it asks for both (`capabilities::VAULT_ACTIONS`).
    assert_eq!(m.capabilities.reads, ["documents"]);
    assert_eq!(m.capabilities.writes, ["editor", "documents"]);
    // C11: a slash row for the date, and a key for it.
    let slash = &m.contributes.slash;
    assert_eq!(slash.len(), 1);
    assert_eq!(slash[0].action, "SHOWCASE_TEMPLATER_DATE");
    assert_eq!(slash[0].label, "Today's date");
    let date = m
        .actions
        .iter()
        .find(|a| a.id == "SHOWCASE_TEMPLATER_DATE")
        .unwrap();
    assert_eq!(date.shortcut, "Mod-Alt-D");
}

#[test]
fn templater_lite_makes_a_prompted_title_a_safe_file_name() {
    let list = ActionList::new();
    let notes = Notes::default();
    vault(&list, &notes);
    templates(&notes);
    let ui = FakeUi::new()
        .answer(json!("../../etc/pass:wd"))
        .answer(json!("nobody"));
    let _host = load(&list, &ui, "templater-lite");
    let out = run(
        &list,
        "SHOWCASE_TEMPLATER_NEW",
        &json!({ "template": "Templates/Meeting.md", "date": "2026-11-14" }),
    )
    .unwrap();
    // No folder climbed out of, no `:`: one file in Inbox/.
    assert_eq!(out, "Inbox/-.-etc-pass-wd.md");
}
