//! The workspace model: every operation, and the invariants after it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_workspace::{
    Area, Axis, Chrome, Dest, Direction, FormClass, GroupId, LinkId, Node, Pane, PaneId, Rect,
    Screensets, Shown, Side, Snap, Target, Workspace, area_rects, neighbour,
};

fn pane(id: &str) -> Pane {
    Pane::new(id, "test.view", id)
}

fn id(s: &str) -> PaneId {
    PaneId::new(s)
}

/// The main window's center tree.
fn center(ws: &Workspace) -> &Node {
    ws.windows
        .first()
        .and_then(|w| w.center.as_ref())
        .expect("a main center")
}

fn group_of(ws: &Workspace, p: &str) -> GroupId {
    ws.locate(&id(p)).expect("open").group
}

fn tabs(ws: &Workspace, g: GroupId) -> Vec<String> {
    ws.group(g)
        .expect("group")
        .tabs
        .iter()
        .map(|t| t.0.clone())
        .collect()
}

#[test]
fn a_new_workspace_is_a_main_window_with_one_empty_group() {
    let ws = Workspace::new();
    assert_eq!(ws.windows.len(), 1);
    assert!(matches!(center(&ws), Node::Group(g) if g.tabs.is_empty()));
}

#[test]
fn tabs_open_beside_the_active_one_and_become_active() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Tab).unwrap();
    ws.activate(&id("a")).unwrap();
    ws.open(pane("c"), Target::Tab).unwrap();
    let g = group_of(&ws, "a");
    assert_eq!(tabs(&ws, g), ["a", "c", "b"]);
    assert_eq!(ws.active, Some(id("c")));
    assert_eq!(ws.recent.first(), Some(&id("c")));
}

#[test]
fn opening_an_open_id_brings_it_forward() {
    let mut ws = Workspace::new();
    ws.open(
        pane("timer"),
        Target::Float(Rect::new(0.0, 0.0, 200.0, 90.0)),
    )
    .unwrap();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("timer"), Target::Tab).unwrap();
    assert_eq!(ws.panes.len(), 2);
    assert_eq!(ws.active, Some(id("timer")));
}

#[test]
fn here_replaces_the_active_tab_unless_it_is_pinned() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Here).unwrap();
    let g = group_of(&ws, "b");
    assert_eq!(tabs(&ws, g), ["b"]);
    assert!(ws.pane(&id("a")).is_none());
    ws.set_pinned(&id("b"), true).unwrap();
    ws.open(pane("c"), Target::Here).unwrap();
    assert_eq!(tabs(&ws, g), ["b", "c"]);
}

#[test]
fn a_split_puts_a_new_group_beside_the_active_one() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Split(Side::Right)).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!("a split")
    };
    assert_eq!(split.axis, Axis::Row);
    assert_eq!(split.children.len(), 2);
    assert!((split.children[0].share - 0.5).abs() < 1e-4);
    // Splitting again on the same axis joins the row, not a nested split.
    ws.open(pane("c"), Target::Split(Side::Right)).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!("a split")
    };
    assert_eq!(split.children.len(), 3);
    // Across the axis it nests.
    ws.open(pane("d"), Target::Split(Side::Bottom)).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!("a split")
    };
    assert!(
        split
            .children
            .iter()
            .any(|c| matches!(&c.node, Node::Split(s) if s.axis == Axis::Column))
    );
}

#[test]
fn moving_the_last_tab_out_of_a_group_removes_it_and_collapses_the_split() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Split(Side::Right)).unwrap();
    let ga = group_of(&ws, "a");
    ws.move_pane(
        &id("b"),
        Dest::Group {
            group: ga,
            index: None,
        },
    )
    .unwrap();
    assert!(matches!(center(&ws), Node::Group(g) if g.tabs.len() == 2));
}

#[test]
fn reordering_within_a_group_counts_the_index_after_the_removal() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    let g = group_of(&ws, "a");
    ws.move_pane(
        &id("a"),
        Dest::Group {
            group: g,
            index: Some(3),
        },
    )
    .unwrap();
    assert_eq!(tabs(&ws, g), ["b", "c", "a"]);
}

#[test]
fn dropping_on_a_groups_edge_splits_it() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Tab).unwrap();
    let g = group_of(&ws, "a");
    ws.move_pane(
        &id("b"),
        Dest::Edge {
            group: g,
            side: Side::Top,
        },
    )
    .unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!("a split")
    };
    assert_eq!(split.axis, Axis::Column);
    // Top leads: b's group is first.
    assert!(matches!(&split.children[0].node, Node::Group(g) if g.tabs == [id("b")]));
}

#[test]
fn a_closed_pane_reopens_where_it_was() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    let g = group_of(&ws, "a");
    ws.close(&id("b")).unwrap();
    assert_eq!(tabs(&ws, g), ["a", "c"]);
    assert_eq!(ws.reopen().unwrap(), Some(id("b")));
    assert_eq!(tabs(&ws, g), ["a", "b", "c"]);
    assert_eq!(ws.active, Some(id("b")));
}

#[test]
fn closing_the_active_pane_activates_the_most_recent_other() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    ws.activate(&id("a")).unwrap();
    ws.activate(&id("c")).unwrap();
    ws.close(&id("c")).unwrap();
    assert_eq!(ws.active, Some(id("a")));
}

#[test]
fn close_others_keeps_pinned_tabs() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    ws.set_pinned(&id("c"), true).unwrap();
    ws.close_others(&id("a")).unwrap();
    let g = group_of(&ws, "a");
    assert_eq!(tabs(&ws, g), ["a", "c"]);
}

#[test]
fn closed_panes_are_kept_up_to_a_limit() {
    let mut ws = Workspace::new();
    for n in 0..60 {
        let p = format!("p{n}");
        ws.open(pane(&p), Target::Tab).unwrap();
        ws.close(&id(&p)).unwrap();
    }
    assert_eq!(ws.closed.len(), Workspace::CLOSED_KEPT);
}

#[test]
fn a_dock_opens_with_icon_tabs() {
    let mut ws = Workspace::new();
    ws.set_dock_shown(Side::Left, Shown::Hidden).unwrap();
    ws.open(pane("files"), Target::Dock(Side::Left)).unwrap();
    ws.open(pane("search"), Target::Dock(Side::Left)).unwrap();
    let dock = &ws.windows[0].docks.left;
    assert_eq!(dock.shown, Shown::Open);
    let Some(Node::Group(g)) = &dock.root else {
        panic!("a dock group")
    };
    assert_eq!(g.chrome, Chrome::Icons);
    assert_eq!(g.tabs, [id("files"), id("search")]);
    assert!(matches!(
        ws.locate(&id("files")).map(|l| l.area),
        Some(Area::Dock(Side::Left))
    ));
    assert!(ws.open(pane("x"), Target::Dock(Side::Top)).is_err());
}

#[test]
fn the_current_document_skips_docks() {
    let mut ws = Workspace::new();
    ws.open(pane("song"), Target::Tab).unwrap();
    ws.open(pane("files"), Target::Dock(Side::Left)).unwrap();
    assert_eq!(ws.active, Some(id("files")));
    assert_eq!(ws.current_document(), Some(&id("song")));
}

#[test]
fn floats_snap_pin_minimise_and_come_back_to_the_front() {
    let mut ws = Workspace::new();
    ws.open(
        pane("a"),
        Target::Float(Rect::new(10.0, 10.0, 300.0, 200.0)),
    )
    .unwrap();
    ws.open(
        pane("b"),
        Target::Float(Rect::new(40.0, 40.0, 300.0, 200.0)),
    )
    .unwrap();
    ws.snap(&id("a"), Some(Snap::Half(Side::Left))).unwrap();
    assert_eq!(ws.windows[0].floats[0].snap, Some(Snap::Half(Side::Left)));
    ws.set_float_rect(&id("a"), Rect::new(0.0, 0.0, 100.0, 100.0))
        .unwrap();
    assert_eq!(ws.windows[0].floats[0].snap, None);
    ws.minimise(&id("a")).unwrap();
    assert!(ws.windows[0].floats[0].minimised);
    ws.restore(&id("a")).unwrap();
    let last = ws.windows[0].floats.last().unwrap();
    assert_eq!(last.group.tabs, [id("a")]);
    assert!(!last.minimised);
    assert!(ws.snap(&id("nope"), None).is_err());
    // A tab opened while a float is active joins the float…
    ws.open(pane("c"), Target::Tab).unwrap();
    assert!(matches!(
        ws.locate(&id("c")).map(|l| l.area),
        Some(Area::Float(_))
    ));
    // …and a pane that is not floating cannot be minimised.
    ws.open(pane("d"), Target::Dock(Side::Left)).unwrap();
    assert!(ws.minimise(&id("d")).is_err());
}

#[test]
fn a_window_of_its_own_closes_with_its_last_pane() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(
        pane("b"),
        Target::Window(Some(Rect::new(0.0, 0.0, 800.0, 600.0))),
    )
    .unwrap();
    assert_eq!(ws.windows.len(), 2);
    assert_eq!(ws.locate(&id("b")).map(|l| l.window), Some(1));
    ws.close(&id("b")).unwrap();
    assert_eq!(ws.windows.len(), 1);
}

#[test]
fn history_goes_back_and_forward() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.navigate(&id("a"), "one".into()).unwrap();
    ws.navigate(&id("a"), "two".into()).unwrap();
    ws.go(&id("a"), false).unwrap();
    assert_eq!(ws.pane(&id("a")).unwrap().state, "one");
    ws.go(&id("a"), true).unwrap();
    assert_eq!(ws.pane(&id("a")).unwrap().state, "two");
    assert!(ws.go(&id("a"), true).is_err());
}

#[test]
fn tabs_cycle_and_go_to_by_number() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    ws.goto_tab(0).unwrap();
    assert_eq!(ws.active, Some(id("a")));
    ws.goto_tab(8).unwrap();
    assert_eq!(ws.active, Some(id("c")));
    ws.cycle_tab(false).unwrap();
    assert_eq!(ws.active, Some(id("a")));
    ws.cycle_tab(true).unwrap();
    assert_eq!(ws.active, Some(id("c")));
}

#[test]
fn linked_panes_find_each_other_and_stacking_is_per_group() {
    let mut ws = Workspace::new();
    for p in ["song", "mixer", "notes"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    let link = LinkId("song-1".into());
    ws.set_link(&id("song"), Some(link.clone())).unwrap();
    ws.set_link(&id("mixer"), Some(link)).unwrap();
    assert_eq!(ws.linked(&id("song")), [&id("mixer")]);
    let g = group_of(&ws, "song");
    ws.set_stacked(g, true).unwrap();
    assert!(ws.group(g).unwrap().stacked);
}

#[test]
fn shares_are_normalised_and_can_be_evened() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Split(Side::Right)).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!()
    };
    let sid = split.id;
    ws.set_shares(sid, &[3.0, 1.0]).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!()
    };
    assert!((split.children[0].share - 0.75).abs() < 1e-4);
    ws.set_shares(sid, &[-1.0, f32::NAN]).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!()
    };
    let total: f32 = split.children.iter().map(|c| c.share).sum();
    assert!((total - 1.0).abs() < 1e-4 && split.children.iter().all(|c| c.share > 0.0));
    ws.even(sid).unwrap();
    let Node::Split(split) = center(&ws) else {
        panic!()
    };
    assert!((split.children[1].share - 0.5).abs() < 1e-4);
}

#[test]
fn layout_places_groups_by_share_and_finds_neighbours() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Split(Side::Right)).unwrap();
    ws.open(pane("c"), Target::Split(Side::Bottom)).unwrap();
    let placed = area_rects(center(&ws), Rect::new(0.0, 0.0, 1000.0, 800.0));
    assert_eq!(placed.len(), 3);
    let (a, b, c) = (group_of(&ws, "a"), group_of(&ws, "b"), group_of(&ws, "c"));
    let rect = |g| placed.iter().find(|p| p.group == g).unwrap().rect;
    assert_eq!(rect(a), Rect::new(0.0, 0.0, 500.0, 800.0));
    assert_eq!(rect(c), Rect::new(500.0, 400.0, 500.0, 400.0));
    assert_eq!(neighbour(&placed, a, Direction::Right), Some(b));
    assert_eq!(neighbour(&placed, b, Direction::Down), Some(c));
    assert_eq!(neighbour(&placed, c, Direction::Left), Some(a));
    assert_eq!(neighbour(&placed, a, Direction::Left), None);
}

#[test]
fn a_layout_round_trips_and_a_broken_one_is_repaired_on_reading() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Split(Side::Right)).unwrap();
    ws.open(pane("t"), Target::Float(Rect::new(1.0, 2.0, 3.0, 4.0)))
        .unwrap();
    let json = ws.to_json().unwrap();
    assert_eq!(Workspace::from_json(&json).unwrap(), ws);

    // A tab naming no pane, a pane in two groups, and a pane in none.
    let mut broken = ws.clone();
    let gb = group_of(&broken, "b");
    let ga = group_of(&broken, "a");
    broken.group_mut(gb).unwrap().tabs.push(id("ghost"));
    broken.group_mut(gb).unwrap().tabs.push(id("a"));
    broken.panes.push(pane("orphan"));
    let read = Workspace::from_json(&broken.to_json().unwrap()).unwrap();
    assert!(read.locate(&id("ghost")).is_none());
    assert_eq!(read.locate(&id("a")).map(|l| l.group), Some(ga));
    assert!(read.locate(&id("orphan")).is_some());
}

#[test]
fn screensets_recall_save_reset_and_keep_their_builtins() {
    let mut mix = Workspace::new();
    mix.open(pane("mixer"), Target::Tab).unwrap();
    let mut sets = Screensets::default();
    sets.set_working(FormClass::Desk, mix.clone());
    let id_mix = sets.save_as("Mix", Some(4));
    assert_eq!(
        sets.in_slot(4).map(|s| s.id.as_str()),
        Some(id_mix.as_str())
    );
    assert!(!sets.is_dirty(FormClass::Desk));

    // Arranged since: dirty; reset brings the saved one back.
    let mut changed = mix.clone();
    changed
        .open(pane("meter"), Target::Split(Side::Right))
        .unwrap();
    sets.set_working(FormClass::Desk, changed.clone());
    assert!(sets.is_dirty(FormClass::Desk));
    assert_eq!(sets.reset(FormClass::Desk), Some(mix.clone()));
    assert!(!sets.is_dirty(FormClass::Desk));

    // Saved changes stick; a tablet falls back to the desk's layout.
    sets.set_working(FormClass::Desk, changed.clone());
    assert!(sets.save(FormClass::Desk));
    assert_eq!(
        sets.recall(&id_mix, FormClass::TabletLandscape),
        Some(changed)
    );

    // A second set taking slot 4 takes it from the first.
    let other = sets.save_as("Mix", Some(4));
    assert_ne!(other, id_mix);
    assert_eq!(sets.in_slot(4).map(|s| s.id.clone()), Some(other.clone()));

    // A built-in set is not deleted.
    sets.sets
        .iter_mut()
        .find(|s| s.id == id_mix)
        .unwrap()
        .builtin = true;
    assert!(!sets.delete(&id_mix));
    assert!(sets.delete(&other));

    let json = sets.to_json().unwrap();
    assert_eq!(Screensets::from_json(&json).unwrap(), sets);
}
