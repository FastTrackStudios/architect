//! Every workspace action, driven over MCP (`architect-mcp`'s `run` tool)
//! against a headless workspace: what a model sees and does.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]

use architect::action::{ActionList, Effect};
use architect_mcp::Server;
use architect_workspace::control::{Env, HeadlessWorkspace, State, ViewSpec, ids};
use architect_workspace::{FormClass, Pane, Screenset, Screensets, Side, Target, Workspace};
use serde_json::{Value, json};

/// Two notes in the center, files and search in the left dock.
fn workspace() -> Workspace {
    let mut w = Workspace::new();
    for (id, title) in [("note-a", "A"), ("note-b", "B")] {
        let _ = w.open(Pane::new(id, "note", title), Target::Tab);
    }
    let _ = w.open(
        Pane::new("files", "files", "Files"),
        Target::Dock(Side::Left),
    );
    let _ = w.open(
        Pane::new("search", "search", "Search"),
        Target::Dock(Side::Left),
    );
    let _ = w.activate(&architect_workspace::PaneId::new("note-a"));
    w
}

fn env() -> Env {
    Env::new(vec![
        ViewSpec::new("note", "Note").many(),
        ViewSpec::new("files", "Files"),
        ViewSpec::new("search", "Search"),
        ViewSpec::new("graph", "Graph"),
    ])
    .with_windows()
}

fn screensets() -> Screensets {
    Screensets::default().with_builtins([Screenset::new("Write")
        .slot(1)
        .layout(FormClass::Desk, workspace())])
}

struct Model {
    server: Server,
    list: ActionList,
    host: HeadlessWorkspace,
}

fn model() -> Model {
    let list = ActionList::new();
    let host = HeadlessWorkspace::new(State::new(workspace()).with_screensets(screensets()), env());
    host.install(&list).expect("installed");
    let server = Server::new("workspace", "0", list.clone());
    Model { server, list, host }
}

impl Model {
    fn tool(&self, args: &Value) -> (Value, bool) {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "run", "arguments": args}});
        let reply: Value =
            serde_json::from_str(&self.server.handle(&request.to_string()).expect("a reply"))
                .expect("json");
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().expect("text");
        let value = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
        (value, result["isError"].as_bool().unwrap_or(false))
    }

    /// Runs `id` over MCP; its answer.
    fn run(&self, id: &str, args: Value) -> Value {
        let (out, failed) = self.tool(&json!({"id": id, "args": args}));
        assert!(!failed, "{id} {args} failed: {out}");
        out
    }

    /// Runs `id` over MCP, which must fail; its error.
    fn fails(&self, id: &str, args: Value) -> String {
        let (out, failed) = self.tool(&json!({"id": id, "args": args}));
        assert!(failed, "{id} {args} should fail, gave {out}");
        out["error"].as_str().unwrap_or_default().to_owned()
    }

    fn layout(&self) -> Value {
        self.run(ids::LAYOUT, json!({}))
    }

    fn active(&self) -> String {
        self.layout()["active"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn pane(&self, pane: &str) -> Value {
        self.run(ids::PANE_GET, json!({"pane": pane}))
    }
}

fn main_groups(layout: &Value) -> Vec<Vec<String>> {
    layout["windows"][0]["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|g| {
            g["tabs"]
                .as_array()
                .expect("tabs")
                .iter()
                .map(|t| t.as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn the_list_is_sound_and_says_what_each_action_does_to_the_app() {
    let m = model();
    m.list.validate().expect("valid");
    m.list.probe_hostile_args().expect("no garbage taken");
    for id in ids::ALL {
        assert!(m.list.info(id).is_some(), "{id} is listed");
    }
    let effect = |id: &str| m.list.info(id).map(|i| i.effect);
    for read in [
        ids::LAYOUT,
        ids::PANE_GET,
        ids::NEIGHBOR,
        ids::VIEWS,
        ids::EXPORT,
        ids::WORKSPACES_LIST,
    ] {
        assert_eq!(effect(read), Some(Effect::ReadOnly), "{read}");
    }
    for destructive in [ids::CLOSE_OTHERS, ids::CLOSE_WINDOW, ids::WORKSPACES_DELETE] {
        assert_eq!(
            effect(destructive),
            Some(Effect::Destructive),
            "{destructive}"
        );
    }
    assert_eq!(effect(ids::OPEN), Some(Effect::Mutating));
    // A model reads what to send: a target is required.
    let (described, _) = m.tool(&json!({"id": ids::LAYOUT}));
    assert!(described.is_object());
    let schema = m
        .list
        .info(ids::MOVE)
        .and_then(|i| i.params_schema)
        .expect("schema");
    assert!(
        schema.contains("\"pane\"") && schema.contains("\"to\""),
        "{schema}"
    );
}

#[test]
fn opening_names_its_place_and_leaves_the_focus_where_it_was() {
    let m = model();
    let views = m.run(ids::VIEWS, json!({}));
    assert_eq!(views["views"].as_array().map(Vec::len), Some(4));

    let a = m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-c", "title": "C",
        "at": {"beside": {"pane": "note-a"}}}),
    );
    assert_eq!(a["changed"], true);
    assert_eq!(a["pane"], "note-c");
    assert_eq!(main_groups(&a["layout"]), [["note-a", "note-c", "note-b"]]);
    assert_eq!(m.active(), "note-a", "opening did not take the focus");
    assert_eq!(
        m.pane("note-c")["showing"],
        false,
        "the person's tab still shows"
    );

    // The same id again: nothing new, and safe to retry.
    let again = m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-c",
        "at": {"beside": {"pane": "note-a"}}}),
    );
    assert_eq!(
        (again["changed"].clone(), again["reason"].clone()),
        (json!(false), json!("already_open"))
    );

    let graph = m.run(ids::OPEN, json!({"view": "graph", "at": {"split": {"pane": "note-a", "side": "right"}}, "focus": true}));
    assert_eq!(graph["pane"], "graph");
    assert_eq!(m.active(), "graph", "focus: true takes it");
    assert_eq!(main_groups(&m.layout()).len(), 2);

    // A view's own id, a fresh one for a document.
    let fresh = m.run(
        ids::OPEN,
        json!({"view": "note", "at": {"group": {"group": graph["group"]}}}),
    );
    assert!(
        fresh["pane"]
            .as_str()
            .is_some_and(|p| p.starts_with("note-"))
    );
    let floated = m.run(
        ids::OPEN,
        json!({"view": "note", "id": "scratch", "at": {"float": {}}}),
    );
    assert_eq!(m.pane("scratch")["place"], "float");
    assert_eq!(floated["changed"], true);
    let docked = m.run(
        ids::OPEN,
        json!({"view": "note", "id": "outline", "at": {"dock": {"side": "right"}}}),
    );
    assert_eq!(m.pane("outline")["place"], "right_dock");
    assert_eq!(docked["window"], "main");
    m.run(
        ids::OPEN,
        json!({"view": "note", "id": "elsewhere", "at": {"window": {}}}),
    );
    assert_ne!(m.pane("elsewhere")["window"], "main");
    m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-d", "at": {"replace": {"pane": "note-c"}}}),
    );
    assert!(
        m.fails(ids::PANE_GET, json!({"pane": "note-c"}))
            .contains("note-c"),
        "replaced"
    );

    // What a call gets wrong fails, and names it.
    assert!(
        m.fails(ids::OPEN, json!({"view": "nope", "at": {"window": {}}}))
            .contains("WORKSPACE_VIEWS")
    );
    assert!(
        m.fails(
            ids::OPEN,
            json!({"view": "graph", "id": "other", "at": {"window": {}}})
        )
        .contains("single")
    );
    assert!(
        m.fails(
            ids::OPEN,
            json!({"view": "note", "at": {"beside": {"pane": "ghost"}}})
        )
        .contains("ghost")
    );
    // No place, no open: there is no "wherever the person is".
    let (_, failed) = m.tool(&json!({"id": ids::OPEN, "args": {"view": "note"}}));
    assert!(failed);
}

#[test]
fn moving_splitting_and_focusing_by_name() {
    let m = model();
    let split = m.run(ids::SPLIT, json!({"pane": "note-b", "side": "bottom"}));
    assert_eq!(split["changed"], true);
    let b_group = split["group"].as_u64().expect("its new group");
    assert_eq!(m.layout()["windows"][0]["splits"][0]["axis"], "column");
    assert_eq!(m.active(), "note-a");

    // A lone tab split off itself would change nothing.
    let lone = m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    assert_eq!(lone["reason"], "only_tab");

    let n = m.run(
        ids::NEIGHBOR,
        json!({"pane": "note-a", "direction": "down"}),
    );
    assert_eq!(n["neighbor"], "note-b");
    let none = m.run(ids::NEIGHBOR, json!({"pane": "note-a", "direction": "up"}));
    assert!(none["neighbor"].is_null());

    let f = m.run(ids::FOCUS, json!({"from": "note-a", "direction": "down"}));
    assert_eq!(
        (f["changed"].clone(), m.active()),
        (json!(true), "note-b".to_owned())
    );
    let again = m.run(ids::FOCUS, json!({"pane": "note-b"}));
    assert_eq!(again["reason"], "already_focused");
    let edge = m.run(ids::FOCUS, json!({"from": "note-b", "direction": "down"}));
    assert_eq!(edge["reason"], "no_neighbor");
    assert!(m.fails(ids::FOCUS, json!({})).contains("pane"));

    // Into a group by id, beside a pane, to a group's edge, a dock, a float.
    m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"group": {"group": b_group, "index": 0}}}),
    );
    assert_eq!(m.pane("files")["group"].as_u64(), Some(b_group));
    assert_eq!(m.active(), "note-b", "moving did not take the focus");
    m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"beside": {"pane": "note-a"}}}),
    );
    assert_eq!(main_groups(&m.layout())[0], ["note-a", "files"]);
    let edge = m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"edge": {"group": b_group, "side": "right"}}}),
    );
    assert_ne!(edge["group"].as_u64(), Some(b_group));
    m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"dock": {"side": "bottom"}}}),
    );
    assert_eq!(m.pane("files")["place"], "bottom_dock");
    m.run(ids::MOVE, json!({"pane": "files", "to": {"float": {"rect": {"x": 10.0, "y": 10.0, "w": 300.0, "h": 200.0}}}}));
    assert_eq!(m.pane("files")["float_rect"]["w"], 300.0);
    let focus = m.run(ids::MOVE, json!({"pane": "files", "to": {"split": {"pane": "note-a", "side": "left"}}, "focus": true}));
    assert_eq!(
        (focus["changed"].clone(), m.active()),
        (json!(true), "files".to_owned())
    );

    // Back and forth between windows.
    let out = m.run(ids::MOVE, json!({"pane": "files", "to": {"window": {"bounds": {"x": 0.0, "y": 0.0, "w": 400.0, "h": 300.0}}}}));
    let window = out["window"].as_str().expect("a window").to_owned();
    assert_ne!(window, "main");
    m.run(
        ids::OPEN,
        json!({"view": "graph", "at": {"beside": {"pane": "files"}}}),
    );
    m.run(
        ids::MOVE,
        json!({"pane": "note-a", "to": {"window": {"id": window}}}),
    );
    assert_eq!(m.pane("note-a")["window"], window.as_str());
    m.run(
        ids::MOVE,
        json!({"pane": "note-a", "to": {"window": {"id": "main"}}}),
    );
    assert_eq!(m.pane("note-a")["window"], "main");
    assert!(
        m.fails(
            ids::MOVE,
            json!({"pane": "files", "to": {"group": {"group": 999}}})
        )
        .contains("999")
    );
}

#[test]
fn zooming_resizing_and_setting_groups_tabs_and_docks() {
    let m = model();
    let one = m.run(ids::ZOOM, json!({"pane": "note-a"}));
    assert_eq!(one["reason"], "single_group");
    m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    let on = m.run(ids::ZOOM, json!({"pane": "note-a", "mode": "on"}));
    assert_eq!(on["changed"], true);
    assert_eq!(
        m.run(ids::ZOOM, json!({"pane": "note-a", "mode": "on"}))["reason"],
        "already_zoomed"
    );
    assert_eq!(
        m.run(ids::ZOOM, json!({"pane": "note-a", "mode": "toggle"}))["changed"],
        true
    );
    assert_eq!(
        m.run(ids::ZOOM, json!({"pane": "note-a", "mode": "off"}))["reason"],
        "already_unzoomed"
    );
    assert!(
        m.fails(ids::ZOOM, json!({"pane": "files"}))
            .contains("center")
    );

    // Shares, by split and by direction; a dock's size.
    let split = m.host.state().workspace.windows[0]
        .center
        .as_ref()
        .and_then(|c| match c {
            architect_workspace::Node::Split(s) => Some(s.id.0),
            architect_workspace::Node::Group(_) => None,
        })
        .expect("a split");
    m.run(ids::RESIZE, json!({"split": split, "shares": [3.0, 1.0]}));
    assert_eq!(
        m.layout()["windows"][0]["splits"][0]["shares"],
        json!([0.75, 0.25])
    );
    m.run(ids::RESIZE, json!({"split": split, "even": true}));
    assert_eq!(
        m.layout()["windows"][0]["splits"][0]["shares"],
        json!([0.5, 0.5])
    );
    m.run(
        ids::RESIZE,
        json!({"pane": "note-a", "direction": "right", "amount": 0.2}),
    );
    let shares = m.layout()["windows"][0]["splits"][0]["shares"].clone();
    assert!(
        shares[0].as_f64().is_some_and(|s| (s - 0.7).abs() < 1e-4),
        "{shares}"
    );
    let wall = m.run(ids::RESIZE, json!({"pane": "note-a", "direction": "up"}));
    assert_eq!(wall["reason"], "no_neighbor");
    m.run(ids::RESIZE, json!({"dock": "left", "size": 400.0}));
    assert_eq!(m.host.state().workspace.windows[0].docks.left.size, 400.0);
    assert!(m.fails(ids::RESIZE, json!({})).contains("one of"));

    let g = m.pane("files")["group"].clone();
    let set = m.run(
        ids::GROUP_SET,
        json!({"group": g, "stacked": true, "chrome": "bare", "current": "search"}),
    );
    assert_eq!(set["changed"], true);
    assert_eq!(m.pane("search")["showing"], true);
    assert!(
        m.fails(ids::GROUP_SET, json!({"group": g, "current": "note-a"}))
            .contains("not a tab")
    );
    let same = m.run(ids::GROUP_SET, json!({"group": g, "stacked": true}));
    assert_eq!(same["reason"], "unchanged");

    m.run(
        ids::TAB_SET,
        json!({"pane": "note-a", "pinned": true, "link": "song", "title": "Alpha"}),
    );
    let a = m.pane("note-a");
    assert_eq!(
        (a["pinned"].clone(), a["link"].clone(), a["title"].clone()),
        (json!(true), json!("song"), json!("Alpha"))
    );
    m.run(ids::TAB_SET, json!({"pane": "note-a", "link": ""}));
    assert!(m.pane("note-a")["link"].is_null());

    m.run(ids::NAVIGATE, json!({"pane": "note-a", "state": "page-2"}));
    assert_eq!(m.pane("note-a")["can_go_back"], true);
    m.run(ids::NAVIGATE, json!({"pane": "note-a", "go": "back"}));
    assert_eq!(m.pane("note-a")["state"], "");
    assert_eq!(
        m.run(ids::NAVIGATE, json!({"pane": "note-a", "go": "back"}))["reason"],
        "no_history"
    );
    m.run(ids::NAVIGATE, json!({"pane": "note-a", "go": "forward"}));
    assert_eq!(m.pane("note-a")["state"], "page-2");

    m.run(ids::DOCK_SET, json!({"side": "left", "shown": "collapsed"}));
    assert_eq!(
        m.host.state().workspace.windows[0].docks.left.shown,
        architect_workspace::Shown::Collapsed
    );
    assert!(
        m.fails(ids::DOCK_SET, json!({"side": "left"}))
            .contains("shown")
    );
}

#[test]
fn docks_pop_out_and_back_floats_and_windows() {
    let m = model();
    let out = m.run(ids::DOCK_POP_OUT, json!({"side": "left"}));
    assert_eq!(out["window"], "dock-left");
    assert_eq!(m.pane("files")["window"], "dock-left");
    assert_eq!(
        m.run(ids::DOCK_POP_OUT, json!({"side": "left"}))["reason"],
        "already_out"
    );
    assert_eq!(
        m.run(ids::DOCK_POP_OUT, json!({"side": "right"}))["reason"],
        "dock_empty"
    );
    m.run(ids::DOCK_BACK, json!({"side": "left"}));
    assert_eq!(m.pane("files")["window"], "main");
    assert_eq!(
        m.run(ids::DOCK_BACK, json!({"side": "left"}))["reason"],
        "dock_not_out"
    );

    let f = m.run(
        ids::FLOAT,
        json!({"pane": "note-b", "rect": {"x": 20.0, "y": 20.0, "w": 320.0, "h": 240.0},
        "snap": "right_half", "pin": "top_right", "above_all": true}),
    );
    assert_eq!(f["changed"], true);
    let w = m.host.state().workspace;
    let float = &w.windows[0].floats[0];
    assert_eq!(
        float.snap,
        Some(architect_workspace::Snap::Half(Side::Right))
    );
    assert_eq!(
        float.pin.map(|p| p.above),
        Some(architect_workspace::Above::All)
    );
    m.run(ids::MINIMISE, json!({"pane": "note-b"}));
    assert_eq!(m.pane("note-b")["minimised"], true);
    assert_eq!(
        m.run(ids::MINIMISE, json!({"pane": "note-b"}))["reason"],
        "unchanged"
    );
    m.run(ids::RESTORE, json!({"pane": "note-b"}));
    assert_eq!(m.pane("note-b")["minimised"], false);
    assert_eq!(
        m.run(ids::RESTORE, json!({"pane": "note-b"}))["reason"],
        "not_minimised"
    );
    assert!(
        m.fails(ids::MINIMISE, json!({"pane": "note-a"}))
            .contains("not floating")
    );

    // A window: its bounds, closed (destructive), its tabs back, its id
    // and its group's still finding them.
    let moved = m.run(ids::MOVE, json!({"pane": "note-a", "to": {"window": {}}}));
    let window = moved["window"].as_str().expect("window").to_owned();
    let group = moved["group"].clone();
    m.run(ids::WINDOW_SET, json!({"window": window, "bounds": {"x": 5.0, "y": 5.0, "w": 600.0, "h": 400.0}, "maximized": true}));
    let w = m.host.state().workspace;
    assert!(
        w.windows
            .iter()
            .any(|win| win.id.0 == window && win.maximized && win.bounds.is_some())
    );
    assert!(
        m.fails(
            ids::WINDOW_SET,
            json!({"window": "main", "bounds": {"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0}})
        )
        .contains("main")
    );
    m.run(ids::CLOSE_WINDOW, json!({"window": window}));
    assert_eq!(m.pane("note-a")["window"], "main");
    let alias = m.run(ids::GROUP_SET, json!({"group": group, "stacked": true}));
    assert_eq!(
        alias["changed"], true,
        "the gone group's id finds where its panes went"
    );
    let into = m.run(
        ids::MOVE,
        json!({"pane": "search", "to": {"window": {"id": window}}}),
    );
    assert_eq!(
        into["window"], "main",
        "the gone window's id finds the main window"
    );
    assert!(
        m.fails(ids::CLOSE_WINDOW, json!({"window": "main"}))
            .contains("main")
    );
}

#[test]
fn closing_reopening_and_ids_never_handed_out_twice() {
    let m = model();
    let mut seen: Vec<u64> = Vec::new();
    let note = |seen: &mut Vec<u64>, layout: &Value| {
        for g in layout["windows"][0]["groups"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(id) = g["id"].as_u64() {
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
        }
    };
    note(&mut seen, &m.layout());
    // A pane that is not there: nothing at all is closed.
    m.fails(ids::CLOSE, json!({"panes": ["note-a", "ghost"]}));
    assert_eq!(m.pane("note-a")["pane"], "note-a");
    let closed = m.run(ids::CLOSE, json!({"panes": ["note-b"]}));
    assert_eq!(closed["changed"], true);
    assert_eq!(m.active(), "note-a");
    let back = m.run(ids::REOPEN, json!({}));
    assert_eq!(back["pane"], "note-b");
    assert_eq!(m.active(), "note-a", "reopening did not take the focus");
    m.run(ids::CLOSE, json!({"panes": ["note-a", "note-b"]}));
    assert_eq!(m.run(ids::REOPEN, json!({"focus": true}))["pane"], "note-b");
    m.run(ids::REOPEN, json!({}));
    m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-c", "at": {"beside": {"pane": "note-a"}}}),
    );
    m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-d", "at": {"beside": {"pane": "note-a"}}}),
    );
    m.run(ids::TAB_SET, json!({"pane": "note-d", "pinned": true}));
    let others = m.run(ids::CLOSE_OTHERS, json!({"pane": "note-a"}));
    assert_eq!(
        main_groups(&others["layout"]),
        [["note-a", "note-d"]],
        "a pinned tab stays"
    );
    while m.run(ids::REOPEN, json!({}))["changed"] == true {}
    assert_eq!(m.run(ids::REOPEN, json!({}))["reason"], "nothing_closed");

    // Groups made and gone: a new one never takes a gone one's id.
    for side in ["right", "bottom", "left"] {
        let s = m.run(ids::SPLIT, json!({"pane": "note-c", "side": side}));
        note(&mut seen, &s["layout"]);
        let max = seen.iter().max().copied().unwrap_or(0);
        let joined = m.run(
            ids::MOVE,
            json!({"pane": "note-c", "to": {"beside": {"pane": "note-a"}}}),
        );
        note(&mut seen, &joined["layout"]);
        let again = m.run(ids::SPLIT, json!({"pane": "note-c", "side": side}));
        let id = again["group"].as_u64().expect("group");
        assert!(id > max, "group {id} was handed out before");
        note(&mut seen, &again["layout"]);
        m.run(
            ids::MOVE,
            json!({"pane": "note-c", "to": {"beside": {"pane": "note-a"}}}),
        );
    }
}

#[test]
fn a_layout_exports_and_applies_whole_or_not_at_all() {
    let m = model();
    m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    let exported = m.run(ids::EXPORT, json!({}));
    let same = m.run(ids::APPLY, json!({"layout": exported}));
    assert_eq!(
        same["changed"], false,
        "a layout applied over itself changes nothing"
    );

    // Swapped: the notes trade groups, a graph joins, the dock is as it was.
    let mut swapped = exported.clone();
    let center = &mut swapped["windows"][0]["center"]["split"]["children"];
    let a = center[0]["node"].clone();
    center[0]["node"] = center[1]["node"].clone();
    center[1]["node"] = a;
    center[1]["node"]["group"]["tabs"]
        .as_array_mut()
        .expect("tabs")
        .push(json!({"pane": "graph", "view": "graph"}));
    let applied = m.run(ids::APPLY, json!({"layout": swapped}));
    assert_eq!(applied["changed"], true);
    assert_eq!(
        main_groups(&applied["layout"]),
        [vec!["note-b"], vec!["note-a", "graph"]]
    );
    assert_eq!(m.active(), "note-a");

    // What does not fit changes nothing.
    let before = m.layout();
    let mut twice = exported.clone();
    twice["windows"][0]["center"]["split"]["children"][1]["node"]["group"]["tabs"]
        .as_array_mut()
        .expect("tabs")
        .push(json!({"pane": "note-b"}));
    assert!(
        m.fails(ids::APPLY, json!({"layout": twice}))
            .contains("twice")
    );
    let mut unknown = exported;
    unknown["windows"][0]["center"]["split"]["children"][1]["node"]["group"]["tabs"]
        .as_array_mut()
        .expect("tabs")
        .push(json!({"pane": "x", "view": "nope"}));
    assert!(
        m.fails(ids::APPLY, json!({"layout": unknown}))
            .contains("nope")
    );
    assert_eq!(m.layout(), before);

    // Panes it leaves out: kept as tabs, or closed to reopen.
    let only_a =
        json!({"windows": [{"id": "main", "center": {"group": {"tabs": [{"pane": "note-a"}]}}}]});
    let kept = m.run(ids::APPLY, json!({"layout": only_a}));
    let tabs = &main_groups(&kept["layout"])[0];
    assert!(tabs.contains(&"graph".to_owned()) && tabs.contains(&"files".to_owned()));
    let closed = m.run(
        ids::APPLY,
        json!({"layout": only_a, "keep_unplaced": false}),
    );
    assert_eq!(main_groups(&closed["layout"]), [["note-a"]]);
    assert_eq!(m.run(ids::REOPEN, json!({}))["changed"], true);
}

#[test]
fn screensets_by_action() {
    let m = model();
    let list = m.run(ids::WORKSPACES_LIST, json!({}));
    assert_eq!(list["sets"][0]["id"], "write");
    assert_eq!(list["class"], "desk");
    assert_eq!(
        m.run(ids::WORKSPACES_SAVE, json!({}))["reason"],
        "no_screenset"
    );
    assert_eq!(
        m.run(ids::WORKSPACES_RESET, json!({}))["reason"],
        "no_screenset"
    );

    let recalled = m.run(ids::WORKSPACES_LOAD, json!({"slot": 1}));
    assert_eq!(recalled["screenset"], "write");
    assert_eq!(m.run(ids::WORKSPACES_LIST, json!({}))["dirty"], false);
    m.run(ids::CLOSE, json!({"panes": ["note-b"]}));
    assert_eq!(m.run(ids::WORKSPACES_LIST, json!({}))["dirty"], true);
    m.run(ids::WORKSPACES_RESET, json!({}));
    assert_eq!(m.pane("note-b")["pane"], "note-b", "reset brings it back");
    m.run(ids::CLOSE, json!({"panes": ["note-b"]}));
    assert_eq!(m.run(ids::WORKSPACES_SAVE, json!({}))["changed"], true);
    assert_eq!(m.run(ids::WORKSPACES_LIST, json!({}))["dirty"], false);

    let made = m.run(ids::WORKSPACES_SAVE, json!({"name": "Focus"}));
    assert_eq!(made["screenset"], "focus");
    assert!(
        m.fails(ids::WORKSPACES_SAVE, json!({"name": "  "}))
            .contains("name")
    );
    m.run(ids::WORKSPACES_LOAD, json!({"id": "write"}));
    assert!(
        m.fails(ids::WORKSPACES_LOAD, json!({"slot": 9}))
            .contains("slot 9")
    );
    assert_eq!(
        m.run(ids::WORKSPACES_DELETE, json!({"id": "write"}))["reason"],
        "builtin"
    );
    assert_eq!(
        m.run(ids::WORKSPACES_DELETE, json!({"id": "focus"}))["changed"],
        true
    );
    assert_eq!(
        m.run(ids::WORKSPACES_LIST, json!({}))["sets"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn each_change_is_heard_and_a_no_op_is_not() {
    let m = model();
    let since = m.list.latest();
    m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    let (events, next) = m.list.events_since(since, 64, std::time::Duration::ZERO);
    assert!(events.iter().any(|e| matches!(&e.kind,
        architect::action::EventKind::Changed { topic, .. } if topic == "workspace")));
    m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    let (events, _) = m.list.events_since(next, 64, std::time::Duration::ZERO);
    assert!(
        !events
            .iter()
            .any(|e| matches!(&e.kind, architect::action::EventKind::Changed { .. }))
    );
}

/// A model built with the main window's area measured, as a host does.
fn measured_model() -> Model {
    let list = ActionList::new();
    let mut env = env();
    env.measured.areas.insert(
        "main".to_owned(),
        architect_workspace::Rect::new(0.0, 0.0, 1200.0, 800.0),
    );
    let host = HeadlessWorkspace::new(State::new(workspace()).with_screensets(screensets()), env);
    host.install(&list).expect("installed");
    let server = Server::new("workspace", "0", list.clone());
    Model { server, list, host }
}

#[test]
fn the_layout_is_a_tree_with_ids_paths_and_rectangles() {
    let m = measured_model();
    m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    let layout = m.layout();
    let main = &layout["windows"][0];
    assert_eq!(main["area"]["w"], 1200.0);
    let center = &main["center"]["split"];
    assert_eq!(center["path"], "center");
    assert_eq!(center["axis"], "row");
    let left = &center["children"][0]["group"];
    let right = &center["children"][1]["group"];
    assert_eq!(left["path"], "center/0");
    assert_eq!(left["tabs"][0]["pane"], "note-a");
    assert_eq!(left["tabs"][0]["view"], "note");
    assert_eq!(right["tabs"][0]["pane"], "note-b");
    assert_eq!(left["measured"], false, "laid out from the shares");
    // The left dock (files, search) is open: the center starts after it.
    let dock = main["docks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["side"] == "left")
        .unwrap()
        .clone();
    let width = dock["size"].as_f64().unwrap();
    assert_eq!(dock["rect"]["w"].as_f64(), Some(width));
    assert_eq!(left["rect"]["x"].as_f64(), Some(width));
    let half = (1200.0 - width) / 2.0;
    assert!((right["rect"]["x"].as_f64().unwrap() - (width + half)).abs() < 0.01);
    assert_eq!(dock["root"]["group"]["tabs"][1]["pane"], "search");
    assert_eq!(layout["current_document"], "note-a");

    // A split by its path.
    m.run(ids::RESIZE, json!({"path": "center", "shares": [1.0, 3.0]}));
    assert_eq!(
        m.layout()["windows"][0]["center"]["split"]["shares"],
        json!([0.25, 0.75])
    );
    assert!(
        m.fails(ids::RESIZE, json!({"path": "center/0", "even": true}))
            .contains("center/0")
    );

    // A zoomed group fills the center.
    m.run(ids::ZOOM, json!({"pane": "note-b", "mode": "on"}));
    let zoomed = m.layout()["windows"][0]["center"]["split"]["children"][1]["group"].clone();
    assert_eq!(zoomed["zoomed"], true);
    assert_eq!(zoomed["rect"]["x"].as_f64(), Some(width));

    // Floats carry their rect, snap and pin.
    m.run(
        ids::FLOAT,
        json!({"pane": "note-a", "rect": {"x": 1.0, "y": 2.0, "w": 300.0, "h": 200.0},
        "snap": "left_half", "pin": "bottom_left"}),
    );
    let float = m.layout()["windows"][0]["floats"][0].clone();
    assert_eq!(
        (float["snap"].clone(), float["pin"].clone()),
        (json!("left_half"), json!("bottom_left"))
    );
    assert_eq!(float["group"]["tabs"][0]["pane"], "note-a");
}

#[test]
fn every_change_moves_the_revision_on_and_a_stale_write_is_refused() {
    let m = model();
    let r0 = m.layout()["revision"].as_u64().unwrap();
    let split = m.run(
        ids::SPLIT,
        json!({"pane": "note-b", "side": "right", "if_revision": r0}),
    );
    assert_eq!(split["changed"], true);
    let r1 = split["revision"].as_u64().unwrap();
    assert_eq!(r1, r0 + 1);
    assert_eq!(split["layout"]["revision"].as_u64(), Some(r1));

    // Planned on the old layout: refused, with the layout as it is now.
    let stale = m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"dock": {"side": "right"}}, "if_revision": r0}),
    );
    assert_eq!(
        (stale["changed"].clone(), stale["reason"].clone()),
        (json!(false), json!("stale"))
    );
    assert_eq!(stale["revision"].as_u64(), Some(r1));
    assert_eq!(m.pane("files")["place"], "left_dock", "nothing moved");
    let fresh = m.run(
        ids::MOVE,
        json!({"pane": "files", "to": {"dock": {"side": "right"}}, "if_revision": r1}),
    );
    assert_eq!(fresh["changed"], true);

    // A no-op and a view's own state leave the revision where it is.
    let r2 = fresh["revision"].as_u64().unwrap();
    let same = m.run(ids::SPLIT, json!({"pane": "note-b", "side": "right"}));
    assert_eq!(same["revision"].as_u64(), Some(r2));
    m.run(ids::NAVIGATE, json!({"pane": "note-a", "state": "page-2"}));
    assert_eq!(
        m.layout()["revision"].as_u64(),
        Some(r2 + 1),
        "history shows in the tree"
    );
    let saved = m.run(ids::WORKSPACES_SAVE, json!({"if_revision": 0}));
    assert_eq!(saved["reason"], "stale");
}

#[test]
fn a_change_event_says_its_revision_actor_kinds_and_panes() {
    let m = model();
    let since = m.list.latest();
    m.run(
        ids::OPEN,
        json!({"view": "note", "id": "note-c", "at": {"beside": {"pane": "note-a"}}}),
    );
    m.run(
        ids::MOVE,
        json!({"pane": "note-c", "to": {"dock": {"side": "right"}}, "focus": true}),
    );
    let (events, _) = m.list.events_since(since, 64, std::time::Duration::ZERO);
    let changes: Vec<Value> = events
        .iter()
        .filter_map(|e| match &e.kind {
            architect::action::EventKind::Changed { topic, detail } if topic == "workspace" => {
                serde_json::from_str(detail).ok()
            }
            _ => None,
        })
        .collect();
    assert_eq!(changes.len(), 2);
    let (a, b) = (&changes[0], &changes[1]);
    assert_eq!(
        b["revision"].as_u64(),
        a["revision"].as_u64().map(|r| r + 1),
        "no gap"
    );
    assert_eq!(a["actor"], "anonymous", "the MCP server's principal");
    assert!(a["kinds"].as_array().unwrap().contains(&json!("opened")));
    assert_eq!(a["panes"], json!(["note-c"]));
    let kinds = b["kinds"].as_array().unwrap();
    assert!(
        kinds.contains(&json!("moved")) && kinds.contains(&json!("focus")),
        "{kinds:?}"
    );
}

#[test]
fn workspaces_save_over_by_name_rename_slot_and_options() {
    let m = model();
    let since = m.list.latest();
    let made = m.run(ids::WORKSPACES_SAVE, json!({"name": "Reading"}));
    assert_eq!(made["screenset"], "reading");
    // The same name again saves over it (Obsidian's), not a second set.
    m.run(ids::CLOSE, json!({"panes": ["note-b"]}));
    let over = m.run(ids::WORKSPACES_SAVE, json!({"name": "reading"}));
    assert_eq!(over["screenset"], "reading");
    let list = m.run(ids::WORKSPACES_LIST, json!({}));
    assert_eq!(list["sets"].as_array().map(Vec::len), Some(2));
    assert_eq!(list["active"], "reading");
    assert_eq!(list["dirty"], false);

    m.run(
        ids::WORKSPACES_RENAME,
        json!({"id": "reading", "name": "Study"}),
    );
    assert_eq!(
        m.run(ids::WORKSPACES_LOAD, json!({"name": "study"}))["screenset"],
        "reading"
    );
    assert!(
        m.fails(
            ids::WORKSPACES_RENAME,
            json!({"id": "reading", "name": "Write"})
        )
        .contains("another")
    );
    assert!(
        m.fails(ids::WORKSPACES_LOAD, json!({"name": "nope"}))
            .contains("nope")
    );

    // A slot taken from the set that had it.
    m.run(
        ids::WORKSPACES_SET_SLOT,
        json!({"id": "reading", "slot": 1}),
    );
    let list = m.run(ids::WORKSPACES_LIST, json!({}));
    let slot = |id: &str| {
        list["sets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .unwrap()["slot"]
            .clone()
    };
    assert_eq!((slot("reading"), slot("write")), (json!(1), Value::Null));
    assert_eq!(
        m.run(ids::WORKSPACES_LOAD, json!({"slot": 1}))["screenset"],
        "reading"
    );
    assert!(
        m.fails(
            ids::WORKSPACES_SET_SLOT,
            json!({"id": "reading", "slot": 12})
        )
        .contains("1 to 9")
    );

    // Save before loading: the live change goes into the active set first.
    m.run(
        ids::WORKSPACES_OPTIONS,
        json!({"save_before_load": true, "slot_keys": false}),
    );
    let list = m.run(ids::WORKSPACES_LIST, json!({}));
    assert_eq!(
        (list["save_before_load"].clone(), list["slot_keys"].clone()),
        (json!(true), json!(false))
    );
    m.run(
        ids::OPEN,
        json!({"view": "graph", "at": {"beside": {"pane": "note-a"}}}),
    );
    m.run(ids::WORKSPACES_LOAD, json!({"id": "write"}));
    m.run(ids::WORKSPACES_LOAD, json!({"id": "reading"}));
    assert_eq!(
        m.pane("graph")["pane"],
        "graph",
        "kept by the save before loading"
    );

    let deleted = m.run(ids::WORKSPACES_DELETE, json!({"id": "reading"}));
    assert_eq!(deleted["changed"], true);
    assert_eq!(
        m.list.info(ids::WORKSPACES_DELETE).map(|i| i.effect),
        Some(Effect::Destructive)
    );

    // Each change of the saved layouts is heard on `workspaces`.
    let (events, _) = m.list.events_since(since, 256, std::time::Duration::ZERO);
    let heard = events
        .iter()
        .filter(|e| matches!(&e.kind, architect::action::EventKind::Changed { topic, .. } if topic == "workspaces"))
        .count();
    assert!(heard >= 7, "{heard} workspaces events");
}

#[test]
fn a_saved_layout_brings_back_windows_docks_floats_and_pins() {
    let m = model();
    m.run(ids::MOVE, json!({"pane": "note-b", "to": {"window": {"bounds": {"x": 40.0, "y": 50.0, "w": 600.0, "h": 400.0}}}}));
    m.run(
        ids::DOCK_SET,
        json!({"side": "left", "shown": "collapsed", "size": 333.0}),
    );
    m.run(ids::OPEN, json!({"view": "graph", "at": {"float": {"rect": {"x": 5.0, "y": 6.0, "w": 300.0, "h": 200.0}}}}));
    m.run(ids::FLOAT, json!({"pane": "graph", "pin": "top_right"}));
    m.run(ids::TAB_SET, json!({"pane": "note-a", "pinned": true}));
    m.run(ids::WORKSPACES_SAVE, json!({"name": "Everything"}));
    let saved = m.host.state().workspace;

    // Everything undone, then loaded back.
    m.run(ids::WORKSPACES_LOAD, json!({"id": "write"}));
    assert_eq!(m.host.state().workspace.windows.len(), 1);
    m.run(ids::WORKSPACES_LOAD, json!({"id": "everything"}));
    let back = m.host.state().workspace;
    assert!(back.same_arrangement(&saved));
    let popout = back
        .windows
        .iter()
        .find(|w| w.id.0 != "main")
        .expect("the pop-out window");
    assert_eq!(popout.bounds.map(|b| b.w), Some(600.0));
    let main = &back.windows[0];
    assert_eq!(main.docks.left.size, 333.0);
    assert_eq!(main.docks.left.shown, architect_workspace::Shown::Collapsed);
    assert_eq!(main.floats[0].rect.w, 300.0);
    assert!(main.floats[0].pin.is_some());
    assert!(
        back.pane(&architect_workspace::PaneId::new("note-a"))
            .is_some_and(|p| p.pinned)
    );
}

#[test]
fn saved_sets_and_the_working_copy_are_kept_apart() {
    use architect_workspace::{MemoryStore, store};
    let mut sets = screensets();
    let _ = sets.recall("write", FormClass::Desk);
    let mut live = workspace();
    let _ = live.close(&architect_workspace::PaneId::new("note-b"));
    sets.set_working(FormClass::Desk, live.clone());
    sets.options.save_before_load = true;
    let (shared, device) = (MemoryStore::new(), MemoryStore::new());
    assert!(store::save_split(&shared, &device, &sets));
    assert!(
        !shared.get().unwrap_or_default().contains("working"),
        "the device's copy stays on the device"
    );
    let back = store::load_split(&shared, &device, Vec::new());
    assert_eq!(back.working(FormClass::Desk), Some(&live));
    assert_eq!(back.active.as_deref(), Some("write"));
    assert!(back.options.save_before_load);
    // Another device has the sets, and its own working copy (none yet).
    let elsewhere = store::load_split(&shared, &MemoryStore::new(), Vec::new());
    assert_eq!(elsewhere.sets.len(), 1);
    assert!(elsewhere.working.is_empty());
}
