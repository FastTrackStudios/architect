//! A render shows its surface: the app opens its "Agent UI" view through
//! the workspace's actions (#234), beside the pane the person is in, or
//! brings it forward when it is open already. With no workspace on the
//! list it says so (`shown: false`) and renders all the same.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::{ActionCaller, ActionList, CONFIRM_TTL, Principal};
use architect_a2ui::store::ROOT;
use architect_a2ui::{CATALOG_ID, Surfaces, actions};
use architect_workspace::control::{Env, HeadlessWorkspace, State, ViewSpec, ids};
use architect_workspace::{Pane, PaneId, Target, Workspace};
use crdt::CrdtDoc;
use serde_json::{Value, json};

fn agent() -> ActionCaller {
    ActionCaller::new(Principal::Service {
        name: "agent".to_owned(),
    })
}

fn run(list: &ActionList, caller: &ActionCaller, id: &str, args: &Value) -> Value {
    let out = futures::executor::block_on(list.run_json_async_with(
        caller,
        id,
        args.to_string().as_bytes(),
    ))
    .unwrap();
    serde_json::from_slice(&out).unwrap()
}

fn render(surface: &str) -> Value {
    json!({ "messages": [ { "createSurface": { "surfaceId": surface, "catalogId": CATALOG_ID,
        "components": [ { "id": "root", "component": "Text", "text": "Hello" } ] } } ] })
}

/// Two notes, the first the person's.
fn workspace() -> Workspace {
    let mut w = Workspace::new();
    for (id, title) in [("note-a", "A"), ("note-b", "B")] {
        let _ = w.open(Pane::new(id, "note", title), Target::Tab);
    }
    let _ = w.activate(&PaneId::new("note-a"));
    w
}

fn active(host: &HeadlessWorkspace) -> Option<String> {
    host.state().workspace.active.map(|p| p.0)
}

#[test]
fn a_render_opens_the_agent_ui_view_or_brings_it_forward() {
    let list = ActionList::new();
    // An agent's writes ask a person; showing the view is the app's own.
    list.require_confirmation(CONFIRM_TTL);
    let surfaces = Surfaces::new(CrdtDoc::ephemeral(), ROOT);
    actions::register(&list, &surfaces).unwrap();
    let host = HeadlessWorkspace::new(
        State::new(workspace()),
        Env::new(vec![
            ViewSpec::new("note", "Note").many(),
            ViewSpec::new(actions::VIEW, "Agent UI"),
        ]),
    );
    host.install(&list).unwrap();

    let out = run(&list, &agent(), actions::RENDER, &render("one"));
    assert_eq!(out["shown"], json!(true));
    assert_eq!(active(&host).as_deref(), Some(actions::VIEW));
    let panes = |host: &HeadlessWorkspace| {
        host.state()
            .workspace
            .panes
            .iter()
            .filter(|p| p.id.0 == actions::VIEW)
            .count()
    };
    assert_eq!(panes(&host), 1);

    // The person goes back to their note; the next render brings the view
    // forward, and opens no second one.
    run(
        &list,
        &ActionCaller::app(),
        ids::FOCUS,
        &json!({ "pane": "note-a" }),
    );
    assert_eq!(active(&host).as_deref(), Some("note-a"));
    let out = run(&list, &agent(), actions::RENDER, &render("two"));
    assert_eq!(out["shown"], json!(true));
    assert_eq!(active(&host).as_deref(), Some(actions::VIEW));
    assert_eq!(panes(&host), 1);

    // A render that creates nothing (a data model update) moves nothing.
    run(
        &list,
        &ActionCaller::app(),
        ids::FOCUS,
        &json!({ "pane": "note-a" }),
    );
    let update = json!({ "messages": [ { "updateDataModel": { "surfaceId": "two", "path": "/x", "value": 1 } } ] });
    let out = run(&list, &agent(), actions::RENDER, &update);
    assert_eq!(out["shown"], json!(false));
    assert_eq!(active(&host).as_deref(), Some("note-a"));
}

#[test]
fn with_no_workspace_a_render_says_it_is_not_shown() {
    let list = ActionList::new();
    let surfaces = Surfaces::new(CrdtDoc::ephemeral(), ROOT);
    actions::register(&list, &surfaces).unwrap();
    let out = run(&list, &agent(), actions::RENDER, &render("one"));
    assert_eq!(out["surfaces"], json!(["one"]));
    assert_eq!(out["shown"], json!(false));
}
