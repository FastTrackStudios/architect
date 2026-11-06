//! A provider's containers and items through the navigator
//! (`ProviderSource`): projects as folders, their tasks as notes, a drag
//! to another project a move, a new note an item made, an open read back
//! as its item.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};

use architect_navigator::list::Clock;
use architect_navigator::{
    FileOp, NavigatorSettings, NavigatorSource, NavigatorState, Pane, ProviderRequest,
    ProviderSource, VaultIndex,
};
use architect_navigator_api::{Item, Node, Open, Snapshot};

const NOW: i64 = 1_793_880_000_000;

fn projects() -> Snapshot {
    Snapshot::new("Projects")
        .node(Node::new("Projects/Alpha/project.md", "Alpha"))
        .node(
            Node::new("Projects/Beta/project.md", "Beta/Gamma")
                .child(Node::new("m1", "Milestone one")),
        )
        // The same title twice: told apart, never merged.
        .node(Node::new("Projects/Alpha 2/project.md", "Alpha"))
        .item(
            Item::new("Tasks/Write.md", "Projects/Alpha/project.md", "Write it")
                .dates(NOW - 1000, NOW)
                .progress(0, 1)
                .badge("high")
                .opens(Open::File {
                    path: "Tasks/Write.md".to_owned(),
                }),
        )
        .item(Item::new("Tasks/Ship.md", "m1", "Ship it").dates(NOW - 2000, NOW))
        .item(Item::new("loose", "", "Loose end").dates(NOW - 3000, NOW))
}

fn state(source: &ProviderSource) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(source),
        NavigatorSettings::default(),
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 600.0);
    s.set_view(Pane::Files, 600.0);
    s
}

#[test]
fn containers_are_folders_and_items_are_notes() {
    let source = ProviderSource::with(projects());
    assert_eq!(source.name(), "Projects");
    assert_eq!(
        source.folders(),
        ["Alpha", "Beta∕Gamma", "Beta∕Gamma/Milestone one", "Alpha 2"]
    );
    assert_eq!(
        source.path_of("Tasks/Ship.md").as_deref(),
        Some("Beta∕Gamma/Milestone one/Ship it.md")
    );
    assert_eq!(source.path_of("loose").as_deref(), Some("Loose end.md"));
    let mut s = state(&source);
    s.select_folder("Alpha");
    let listed: Vec<_> = s.list_order().into_iter().map(str::to_owned).collect();
    assert_eq!(listed, ["Alpha/Write it.md"]);
    // What the navigator opens is read back as the item.
    let item = source.item_at("Alpha/Write it.md").unwrap();
    assert_eq!(item.id, "Tasks/Write.md");
    assert_eq!(
        source.node_at("Alpha 2").unwrap().id,
        "Projects/Alpha 2/project.md"
    );
    // The task's progress is the note's.
    let file = source
        .files()
        .into_iter()
        .find(|f| f.path == "Alpha/Write it.md")
        .unwrap();
    assert_eq!(file.tasks.map(|t| (t.total, t.unfinished)), Some((1, 1)));
}

#[test]
fn a_drag_is_a_move_and_a_new_note_is_an_item_made() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let a = Arc::clone(&asked);
    let source = ProviderSource::with(projects()).on_request(move |r| a.lock().unwrap().push(r));
    source
        .write(&[
            FileOp::Move {
                from: "Alpha/Write it.md".to_owned(),
                to: "Alpha 2/Write it.md".to_owned(),
            },
            FileOp::CreateFile {
                path: "Beta∕Gamma/New task.md".to_owned(),
                text: String::new(),
            },
        ])
        .unwrap();
    assert_eq!(
        *asked.lock().unwrap(),
        [
            ProviderRequest::Create {
                container: "Projects/Beta/project.md".to_owned(),
                title: "New task".to_owned()
            },
            ProviderRequest::Move {
                items: vec!["Tasks/Write.md".to_owned()],
                to: "Projects/Alpha 2/project.md".to_owned()
            },
        ]
    );
    // Not a provider's: a tag written.
    assert!(source.write(&[FileOp::DeleteTag("x".to_owned())]).is_err());
    // No one to ask: read-only.
    assert!(
        ProviderSource::with(projects())
            .write(&[FileOp::CreateFile {
                path: "x.md".to_owned(),
                text: String::new()
            }])
            .is_err()
    );
}

#[test]
fn a_new_snapshot_resets_and_the_same_one_does_nothing() {
    let source = ProviderSource::new();
    let heard = Arc::new(Mutex::new(0));
    let h = Arc::clone(&heard);
    let _sub = source.subscribe(Box::new(move |_| *h.lock().unwrap() += 1));
    assert!(!source.loaded());
    assert!(source.load(projects()));
    assert!(!source.load(projects()));
    let mut moved = projects();
    moved.items[1].container = "Projects/Alpha/project.md".to_owned();
    assert!(source.load(moved));
    assert_eq!(*heard.lock().unwrap(), 2);
    assert_eq!(
        source.path_of("Tasks/Ship.md").as_deref(),
        Some("Alpha/Ship it.md")
    );
}

/// A snapshot that keeps its order (Today before Later): its folders show
/// so until the person orders them by hand; one that does not, by name.
#[test]
fn an_ordered_snapshot_keeps_its_order() {
    let when = |ordered: bool| {
        let s = Snapshot::new("Calendar")
            .node(Node::new("today", "Today"))
            .node(Node::new("tomorrow", "Tomorrow"))
            .node(Node::new("later", "Later"));
        if ordered { s.ordered() } else { s }
    };
    let index = VaultIndex::read(&ProviderSource::with(when(true)));
    let opts = architect_navigator::tree::TreeOptions::default();
    assert_eq!(
        architect_navigator::tree::ordered_children(&index, "", &opts),
        ["Today", "Tomorrow", "Later"]
    );
    let index = VaultIndex::read(&ProviderSource::with(when(false)));
    assert_eq!(
        architect_navigator::tree::ordered_children(&index, "", &opts),
        ["Later", "Today", "Tomorrow"]
    );
    // By hand wins.
    let index = VaultIndex::read(&ProviderSource::with(when(true)));
    let mut opts = architect_navigator::tree::TreeOptions::default();
    opts.child_order
        .insert(String::new(), vec!["Later".to_owned()]);
    assert_eq!(
        architect_navigator::tree::ordered_children(&index, "", &opts),
        ["Later", "Today", "Tomorrow"]
    );
}
