//! The showcase's Recipe scaler (#710 R7, #536,
//! `examples/showcase/recipe-scaler`): an extension written only in
//! TypeScript, loaded from its folder as a development copy as `just
//! showcase` loads it. Its actions (`actions/*.ts`) run through the
//! extensions host on `QuickJS`; its page (`pages/scaler.tsx`) draws the
//! recipes they list, scales them as the person presses + and −, and
//! writes the shopping list through them.
//!
//! The vault is a small stand-in for the app's `VAULT_*` actions
//! (`features/vault/vault/src/actions.rs`), holding the showcase vault's
//! recipes (`apps/ui/demo-vault/Recipes`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::hostui::FakeUi;
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::js::action::TsRunner;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;
use serde_json::{Value, json};

const ID: &str = "showcase.recipes";

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

// ---- the vault's actions, in memory -------------------------------------

/// Notes by path: their properties and body.
#[derive(Clone, Default)]
struct Notes(Arc<Mutex<BTreeMap<String, (Value, String)>>>);

impl Notes {
    fn get(&self, path: &str) -> Option<(Value, String)> {
        self.0.lock().unwrap().get(path).cloned()
    }

    fn put(&self, path: &str, properties: Value, body: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(path.to_owned(), (properties, body.to_owned()));
    }

    fn shopping(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("Shopping/"))
            .cloned()
            .collect()
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

#[derive(facet::Facet)]
struct ListArgs {
    #[facet(default)]
    folder: Option<String>,
}

#[derive(facet::Facet)]
struct PathArgs {
    path: String,
}

#[derive(facet::Facet)]
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

fn vault(list: &ActionList, notes: &Notes) {
    let n = notes.clone();
    list.register_with(
        &LIST,
        Effect::ReadOnly,
        move |_: &ActionCaller, a: ListArgs| {
            let folder = a.folder.unwrap_or_default();
            let out: Vec<Value> =
                n.0.lock()
                    .unwrap()
                    .keys()
                    .filter(|p| folder.is_empty() || p.starts_with(&format!("{folder}/")))
                    .map(|p| json!({ "path": p }))
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
            let title = a.path.rsplit('/').next().unwrap().trim_end_matches(".md");
            Ok::<_, String>(raw(&json!({
                "path": a.path, "title": title, "properties": properties, "body": body,
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
                return Err(format!("already exists: {}", a.path));
            }
            let properties: Value = serde_json::from_str(a.properties.as_deref().unwrap_or("{}"))
                .map_err(|e| e.to_string())?;
            n.put(&a.path, properties, a.body.as_deref().unwrap_or_default());
            Ok::<_, String>(raw(&json!({ "path": a.path })))
        },
    )
    .unwrap();
}

/// The showcase vault's recipes, as `VAULT_READ` answers their
/// frontmatter; each line checked against the note itself.
fn recipes(notes: &Notes) {
    let pancakes = [
        "1 1/2 cup flour",
        "1 1/4 cup milk",
        "1 egg",
        "2 tbsp sugar",
        "3 tsp baking powder",
        "3 tbsp melted butter",
        "a pinch of salt",
    ];
    let chili = [
        "500 g minced beef",
        "2 cans kidney beans",
        "1 can chopped tomatoes",
        "1 onion",
        "2 cloves garlic",
        "2 tsp chili powder",
        "1 tsp cumin",
    ];
    for (name, servings, lines) in [("Pancakes", 4, &pancakes), ("Chili", 6, &chili)] {
        let file = repo(&format!("apps/ui/demo-vault/Recipes/{name}.md"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(&format!("servings: {servings}")), "{name}");
        for line in lines {
            assert!(text.contains(&format!("  - {line}\n")), "{name}: {line}");
        }
        notes.put(
            &format!("Recipes/{name}.md"),
            json!({ "servings": servings, "ingredients": lines, "tags": ["recipe"] }),
            "",
        );
    }
    // A note in the folder that is no recipe.
    notes.put("Recipes/Notes.md", json!({}), "Ideas.");
}

struct Lab {
    list: ActionList,
    notes: Notes,
    ui: FakeUi,
    ext: Extensions,
    _tmp: tempfile::TempDir,
}

fn lab_with(ui: FakeUi) -> Lab {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let notes = Notes::default();
    vault(&list, &notes);
    recipes(&notes);
    let host = Host::new(list.clone(), SharedSections::new(Registry::new())).with_ui(ui.clone());
    host.add_runner(TsRunner::new(AppHost::new(list.clone())));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_contribution_actions().unwrap();
    let _ = ext.dev(repo("examples/showcase/recipe-scaler"));
    assert!(ext.host().get(ID).is_some(), "{:?}", ext.problems());
    Lab {
        list,
        notes,
        ui,
        ext,
        _tmp: tmp,
    }
}

fn run(lab: &Lab, id: &str, args: &Value) -> Result<Value, ActionError> {
    futures::executor::block_on(lab.list.run_json_async_with(
        &ActionCaller::app(),
        id,
        args.to_string().as_bytes(),
    ))
    .map(|out| serde_json::from_slice(&out).unwrap())
}

#[test]
fn its_typescript_actions_list_scale_and_write_a_shopping_list() {
    let lab = lab_with(FakeUi::new());
    let listed = run(&lab, "SHOWCASE_RECIPES_LIST", &json!({})).unwrap();
    let titles: Vec<&str> = listed["recipes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Chili", "Pancakes"]);
    let skipped = listed["skipped"][0].as_str().unwrap();
    assert!(
        skipped.contains("Recipes/Notes.md") && skipped.contains("servings"),
        "{skipped}"
    );

    let made = run(
        &lab,
        "SHOWCASE_RECIPES_SCALE",
        &json!({ "path": "Recipes/Pancakes.md", "servings": 6 }),
    )
    .unwrap();
    assert_eq!(made["path"], "Shopping/Pancakes for 6.md");
    assert_eq!(
        made["items"],
        json!([
            "2 1/4 cup flour",
            "1.88 cup milk",
            "1 1/2 egg",
            "3 tbsp sugar",
            "4 1/2 tsp baking powder",
            "4 1/2 tbsp melted butter",
            "a pinch of salt",
        ])
    );
    let (properties, body) = lab.notes.get("Shopping/Pancakes for 6.md").unwrap();
    assert_eq!(properties["servings"], 6);
    assert_eq!(properties["scaled_from"], 4);
    assert_eq!(properties["recipe"], "[[Pancakes]]");
    assert!(body.contains("- [ ] 2 1/4 cup flour"), "{body}");
    assert!(
        body.contains("Scaled from 4 servings of [[Pancakes]]"),
        "{body}"
    );
    // It said so, as its extension (a toast goes off the run's thread:
    // it may come just after the answer).
    let started = Instant::now();
    while lab.ui.notices().is_empty() {
        assert!(started.elapsed() < Duration::from_secs(5), "no toast");
        std::thread::sleep(Duration::from_millis(5));
    }
    let notices = lab.ui.notices();
    let last = notices.last().unwrap();
    assert_eq!(last.text, "Shopping list: Shopping/Pancakes for 6.md");
    assert_eq!(last.from.id, ID);

    // Again: a new note beside it, never over it.
    let again = run(
        &lab,
        "SHOWCASE_RECIPES_SCALE",
        &json!({ "path": "Recipes/Pancakes.md", "servings": 6 }),
    )
    .unwrap();
    assert_eq!(again["path"], "Shopping/Pancakes for 6 (2).md");
}

#[test]
fn run_from_the_palette_it_asks_which_recipe_and_how_many() {
    let lab = lab_with(
        FakeUi::new()
            .answer(json!("Recipes/Chili.md"))
            .answer(json!("3")),
    );
    let made = run(&lab, "SHOWCASE_RECIPES_SCALE", &json!({})).unwrap();
    assert_eq!(made["path"], "Shopping/Chili for 3.md");
    assert_eq!(made["items"][0], "250 g minced beef");
    assert_eq!(made["items"][2], "1/2 can chopped tomatoes");
    // A cancelled question makes nothing.
    let cancelled = lab_with(FakeUi::new().answer(Value::Null));
    assert_eq!(
        run(&cancelled, "SHOWCASE_RECIPES_SCALE", &json!({})).unwrap(),
        Value::Null
    );
    assert!(cancelled.notes.shopping().is_empty());
}

#[test]
fn servings_that_are_no_whole_number_are_refused_and_nothing_is_written() {
    for said in ["0", "abc", "2.5", "-3"] {
        let lab = lab_with(
            FakeUi::new()
                .answer(json!("Recipes/Chili.md"))
                .answer(json!(said)),
        );
        let why = match run(&lab, "SHOWCASE_RECIPES_SCALE", &json!({})) {
            Err(ActionError::Failed { message, .. }) => message,
            other => panic!("{said}: scaled: {other:?}"),
        };
        assert!(why.contains("whole number from 1 to 1000"), "{said}: {why}");
        assert!(lab.notes.shopping().is_empty(), "{said}");
    }
}

#[test]
fn a_note_that_is_no_recipe_is_refused_at_its_line() {
    let lab = lab_with(FakeUi::new());
    let why = match run(
        &lab,
        "SHOWCASE_RECIPES_SCALE",
        &json!({ "path": "Recipes/Notes.md", "servings": 2 }),
    ) {
        Err(ActionError::Failed { message, .. }) => message,
        other => panic!("scaled a note with no recipe: {other:?}"),
    };
    assert!(why.contains("no servings"), "{why}");
    assert!(why.contains("pages/lib/recipe.ts:"), "{why}");
}

// ---- its page -----------------------------------------------------------

struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn frame(&mut self) {
        match self.session.frames.try_recv() {
            Ok(frame) => self.view.apply(frame).unwrap(),
            Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    fn wait_for_text(&mut self, selector: &str, want: &str) {
        let started = Instant::now();
        while self.text(selector).as_deref() != Some(want)
            && !self
                .text(selector)
                .is_some_and(|t| want.len() > 3 && t.contains(want))
        {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "{selector} is {:?}, not {want:?}; log: {:?}",
                self.text(selector),
                self.session.status().log.lines()
            );
            self.frame();
        }
    }

    /// Clicks the `n`th element `selector` matches.
    fn click(&self, selector: &str, n: usize) {
        let doc = self.view.document();
        let nodes = doc.query_selector_all(selector).unwrap();
        let id: u64 = doc
            .get_node(nodes[n])
            .unwrap()
            .attr(blitz_dom::LocalName::from("data-dioxus-id"))
            .unwrap()
            .parse()
            .unwrap();
        self.session.event(ViewEvent {
            element: id,
            name: "click".to_owned(),
            bubbles: true,
            data: EventData::Mouse(Pointer::default()),
        });
    }
}

// A page's action runs go to the app's runtime: a tokio one, as the app has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn its_page_scales_as_you_press_and_writes_the_list_through_its_action() {
    tokio::task::block_in_place(run_the_page);
}

fn run_the_page() {
    let lab = lab_with(FakeUi::new());
    let manifest = lab.ext.host().get(ID).unwrap().manifest;
    let pages = Pages::new(AppHost::new(lab.list.clone())).with_js();
    let file = PageFile::new(
        repo("examples/showcase/recipe-scaler/pages/scaler.tsx"),
        Origin::Override,
    )
    .unwrap();
    let session = pages
        .open(&file, ID, "scaler", Some(gate(&manifest)))
        .unwrap();
    let (to, from) = futures::channel::mpsc::unbounded();
    let mut pane = Pane {
        session,
        view: RemoteDocument::new(to),
        _events: from,
    };
    // The recipes its list action read (Chili, Pancakes); the first,
    // scaled to twice its servings.
    pane.wait_for_text("#servings", "12");
    pane.wait_for_text("#items", "1000 g minced beef");
    // Pancakes: 8, then + twice: 10.
    pane.click(".recipe", 1);
    pane.wait_for_text("#servings", "8");
    pane.wait_for_text("#items", "3 cup flour");
    // The buttons: Chili, Pancakes, −, +, Write.
    pane.click("#scaler button", 3);
    pane.wait_for_text("#servings", "9");
    pane.click("#scaler button", 3);
    pane.wait_for_text("#servings", "10");
    pane.wait_for_text("#items", "3 3/4 cup flour");
    // Write: its action makes the note, and the page says where.
    pane.click("#scaler button", 4);
    pane.wait_for_text("#made", "Wrote Shopping/Pancakes for 10.md");
    let (_, body) = lab.notes.get("Shopping/Pancakes for 10.md").unwrap();
    assert!(body.contains("- [ ] 3 3/4 cup flour"), "{body}");
}
