//! The issue's bar, headless on Blitz: an agent renders a surface of more
//! than ten catalog components with `A2UI_RENDER`, a component outside the
//! catalog shows as a labelled placeholder and is never drawn, a patch
//! redraws it live, the data model round-trips through Loro, and a press
//! runs an action — a mutating one only after the person's yes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, ActionMeta, CONFIRM_TTL, Effect, Principal};
use architect_a2ui::events::{self, Outcome};
use architect_a2ui::store::ROOT;
use architect_a2ui::ui::{A2uiSurface, use_a2ui};
use architect_a2ui::{CATALOG_ID, Surfaces, actions};
use architect_guide::run::{App, Headless};
use architect_guide::{Target, ViewSize};
use crdt::CrdtDoc;
use dioxus::prelude::*;
use serde_json::{Value, json};

static SAVE: ActionMeta = ActionMeta {
    id: "TEST_SAVE",
    trait_name: "",
    method_name: "",
    display_name: "Save the form",
    description: "Keeps what the form says.",
    category: "Test",
    group: "Test",
    shortcut: "",
    toggleable: false,
    undo: false,
};

#[derive(facet::Facet)]
struct Form {
    name: String,
    subscribe: bool,
}

fn agent() -> ActionCaller {
    ActionCaller::new(Principal::Service {
        name: "agent".to_owned(),
    })
}

/// Thirteen catalog components, and one that is not.
fn surface() -> Value {
    json!({ "messages": [
        { "createSurface": { "surfaceId": "form", "catalogId": CATALOG_ID,
          "dataModel": { "user": { "name": "Ada", "subscribe": false, "volume": 40, "plan": ["pro"], "tab": 0 },
                         "items": [ { "title": "First" }, { "title": "Second" } ] },
          "components": [
            { "id": "root", "component": "Column", "children": ["title", "card", "tabs", "list", "row", "evil"] },
            { "id": "title", "component": "Text", "variant": "h2", "text": "Sign up" },
            { "id": "card", "component": "Card", "child": "fields" },
            { "id": "fields", "component": "Column", "children": ["greet", "name", "subscribe", "volume", "plan", "rule"] },
            { "id": "greet", "component": "Text", "text": { "path": "/user/name" } },
            { "id": "name", "component": "TextField", "label": "Name", "value": { "path": "/user/name" } },
            { "id": "subscribe", "component": "CheckBox", "label": "Subscribe", "value": { "path": "/user/subscribe" } },
            { "id": "volume", "component": "Slider", "label": "Volume", "value": { "path": "/user/volume" } },
            { "id": "plan", "component": "ChoicePicker", "label": "Plan",
              "options": [ { "label": "Free", "value": "free" }, { "label": "Pro", "value": "pro" } ],
              "value": { "path": "/user/plan" } },
            { "id": "rule", "component": "Divider" },
            { "id": "tabs", "component": "Tabs", "activeTab": { "path": "/user/tab" },
              "tabs": [ { "title": "One", "child": "one" }, { "title": "Two", "child": "two" } ] },
            { "id": "one", "component": "Text", "text": "Tab one" },
            { "id": "two", "component": "Text", "text": "Tab two" },
            { "id": "list", "component": "List", "children": { "componentId": "item", "path": "/items" } },
            { "id": "item", "component": "Text", "text": { "path": "title" } },
            { "id": "row", "component": "Row", "children": ["save", "pic"] },
            { "id": "save", "component": "Button", "variant": "primary", "child": "save_label",
              "action": { "event": { "name": "TEST_SAVE",
                "context": { "name": { "path": "/user/name" }, "subscribe": { "path": "/user/subscribe" } } } } },
            { "id": "save_label", "component": "Text", "text": "Save" },
            { "id": "pic", "component": "Image", "url": "https://example.com/a.png", "description": "a picture" },
            { "id": "evil", "component": "Script", "src": "https://evil.example/x.js" }
          ] } }
    ] })
}

fn root() -> Element {
    let surfaces = use_context::<Surfaces>();
    let list = use_context::<Arc<ActionList>>();
    use_a2ui(surfaces, (*list).clone());
    rsx! { A2uiSurface { id: "form" } }
}

/// Pumps `lab` until `target` is on screen, or panics after a while.
fn shown(lab: &mut Headless, target: &Target) {
    let until = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
    while Instant::now() < until {
        lab.pump();
        if lab
            .desktop()
            .find(target, &architect_guide::run::standard())
            .is_some()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{target:?} never showed");
}

fn absent(lab: &mut Headless, target: &Target) {
    for _ in 0..20 {
        lab.pump();
    }
    assert!(
        lab.desktop()
            .find(target, &architect_guide::run::standard())
            .is_none(),
        "{target:?} is drawn"
    );
}

#[test]
fn an_agents_surface_renders_patches_and_asks_before_a_write() {
    let list = Arc::new(ActionList::new());
    list.require_confirmation(CONFIRM_TTL);
    let saved = Arc::new(Mutex::new(Vec::new()));
    let keep = saved.clone();
    list.register(&SAVE, Effect::Mutating, move |f: Form| {
        keep.lock().unwrap().push((f.name, f.subscribe));
        Ok(())
    })
    .unwrap();
    let surfaces = Surfaces::new(CrdtDoc::ephemeral(), ROOT);
    actions::register(&list, &surfaces).unwrap();
    list.validate().unwrap();

    // The agent renders it, as it would over MCP.
    // Its actions are async (MCP, the CLI and ⌘K use the async doors).
    let run = |id: &str, args: &Value| {
        futures::executor::block_on(list.run_json_async_with(
            &agent(),
            id,
            args.to_string().as_bytes(),
        ))
    };
    let out = run(actions::RENDER, &surface()).unwrap();
    let out: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(out["surfaces"], json!(["form"]));
    assert_eq!(out["outsideCatalog"][0]["component"], "Script");
    assert!(out["components"].as_u64().unwrap() > 10);

    let app = App::new(root, list.clone()).context(surfaces.clone());
    let mut lab = Headless::open(&app, ViewSize::default());
    shown(
        &mut lab,
        &Target::Css("[data-a2ui-surface=\"form\"]".to_owned()),
    );
    shown(&mut lab, &Target::Text("Sign up".to_owned()));
    shown(&mut lab, &Target::Text("Ada".to_owned()));
    shown(&mut lab, &Target::Text("Second".to_owned()));
    shown(&mut lab, &Target::Text("Tab one".to_owned()));
    for kind in [
        "TextField",
        "CheckBox",
        "Slider",
        "ChoicePicker",
        "Tabs",
        "Row",
        "List",
    ] {
        shown(&mut lab, &Target::Css(format!("[data-a2ui=\"{kind}\"]")));
    }
    // Outside the catalog: a placeholder that says so, nothing of it drawn.
    shown(
        &mut lab,
        &Target::Css("[data-a2ui-placeholder=\"evil\"]".to_owned()),
    );
    absent(&mut lab, &Target::Css("[data-a2ui=\"Script\"]".to_owned()));

    // A live patch: the data model, then a component.
    let patch = json!({ "messages": [
        { "updateDataModel": { "surfaceId": "form", "path": "/user/name", "value": "Grace" } },
        { "updateComponents": { "surfaceId": "form", "components": [
            { "id": "one", "component": "Text", "text": "Tab one, again" } ] } },
    ] });
    run(actions::PATCH, &patch).unwrap();
    shown(&mut lab, &Target::Text("Grace".to_owned()));
    shown(&mut lab, &Target::Text("Tab one, again".to_owned()));

    // A press: the save is mutating, the surface the agent's, so it waits
    // for the person.
    let press = |ticket: Option<&str>| {
        futures::executor::block_on(events::press(
            &list,
            &surfaces,
            ("form", "save", ""),
            ticket,
        ))
    };
    let Outcome::Waiting { id, ticket } = press(None) else {
        panic!("a write ran without the person's yes");
    };
    assert_eq!(id, "TEST_SAVE");
    assert!(saved.lock().unwrap().is_empty());
    let pending = list.pending_confirmations();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].args.contains("Grace"), "{}", pending[0].args);
    assert!(list.approve(&ticket));
    assert_eq!(press(Some(&ticket)), Outcome::Ran(Value::Null));
    assert_eq!(*saved.lock().unwrap(), [("Grace".to_owned(), false)]);

    // The same over the Action List (MCP, the CLI): A2UI_PRESS.
    let refused = run(
        actions::PRESS,
        &json!({ "surfaceId": "form", "componentId": "save" }),
    )
    .unwrap_err();
    assert!(refused.to_string().contains("confirm"), "{refused}");

    // The data model survives a reload: another document from these bytes.
    let bytes = surfaces
        .doc()
        .loro()
        .export(crdt::loro::ExportMode::Snapshot)
        .unwrap();
    let other = crdt::loro::LoroDoc::new();
    other.import(&bytes).unwrap();
    let reloaded = Surfaces::new(CrdtDoc::from_loro(other), ROOT);
    assert_eq!(reloaded.get("form"), surfaces.get("form"));
    assert_eq!(reloaded.get("form").unwrap().data["user"]["name"], "Grace");
}
