//! The workspace model: every operation, and the invariants after it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_workspace::Store as _;
use architect_workspace::{
    Above, Area, Axis, Bar, Chrome, Corner, Dest, Direction, Float, FormClass, Group, GroupId,
    LinkId, MemoryStore, Node, Pane, PaneId, Pin, Rect, Screenset, Screensets, Shown, Side, Snap,
    Target, WindowId, Workspace, area_rects, bar_drop, drop_at, drop_in, float_rect, neighbour,
    scroll_to, slides, snap_at,
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

// ---- Where a drag lands, where a float shows ----

#[test]
fn a_drop_near_the_window_edge_docks_there_but_not_at_the_top() {
    let host = Rect::new(0.0, 0.0, 1000.0, 600.0);
    let groups = [(GroupId(1), Rect::new(0.0, 0.0, 1000.0, 600.0))];
    let left = drop_at(4.0, 300.0, host, &groups).expect("a drop");
    assert_eq!(left.dest, Dest::Dock(Side::Left));
    let bottom = drop_at(500.0, 596.0, host, &groups).expect("a drop");
    assert_eq!(bottom.dest, Dest::Dock(Side::Bottom));
    // The top edge is the title bar's: it splits the group instead.
    let top = drop_at(500.0, 4.0, host, &groups).expect("a drop");
    assert_eq!(
        top.dest,
        Dest::Edge {
            group: GroupId(1),
            side: Side::Top
        }
    );
}

#[test]
fn a_drop_on_a_group_splits_at_its_bands_and_tabs_in_its_middle() {
    let host = Rect::new(0.0, 0.0, 1000.0, 600.0);
    let a = Rect::new(100.0, 50.0, 400.0, 500.0);
    let b = Rect::new(500.0, 50.0, 400.0, 500.0);
    let groups = [(GroupId(1), a), (GroupId(2), b)];
    let middle = drop_at(300.0, 300.0, host, &groups).expect("a drop");
    assert_eq!(
        middle.dest,
        Dest::Group {
            group: GroupId(1),
            index: None
        }
    );
    assert_eq!(middle.highlight, a);
    let right = drop_at(890.0, 300.0, host, &groups).expect("a drop");
    assert_eq!(
        right.dest,
        Dest::Edge {
            group: GroupId(2),
            side: Side::Right
        }
    );
    // The band lit is the half the new group would take.
    assert_eq!(right.highlight, Rect::new(700.0, 50.0, 200.0, 500.0));
    assert_eq!(drop_at(1200.0, 300.0, host, &groups), None);
}

#[test]
fn the_first_group_listed_wins_so_floats_go_first() {
    let host = Rect::new(0.0, 0.0, 1000.0, 600.0);
    let float = (GroupId(9), Rect::new(300.0, 200.0, 300.0, 200.0));
    let center = (GroupId(1), Rect::new(0.0, 0.0, 1000.0, 600.0));
    let d = drop_at(450.0, 300.0, host, &[float, center]).expect("a drop");
    assert_eq!(
        d.dest,
        Dest::Group {
            group: GroupId(9),
            index: None
        }
    );
}

#[test]
fn a_float_shows_snapped_pinned_or_where_it_was_put() {
    let area = Rect::new(0.0, 0.0, 1000.0, 600.0);
    let mut float = Float {
        group: Group::new(GroupId(3)),
        rect: Rect::new(100.0, 100.0, 300.0, 200.0),
        snap: None,
        pin: None,
        minimised: false,
    };
    assert_eq!(float_rect(&float, area), float.rect);
    float.pin = Some(Pin {
        corner: Corner::TopRight,
        above: Above::Window,
    });
    assert_eq!(
        float_rect(&float, area),
        Rect::new(688.0, 12.0, 300.0, 200.0)
    );
    float.snap = Some(Snap::Half(Side::Left));
    assert_eq!(float_rect(&float, area), Rect::new(0.0, 0.0, 500.0, 600.0));
    float.snap = Some(Snap::Maximised);
    assert_eq!(float_rect(&float, area), area);
}

#[test]
fn a_float_let_go_at_the_top_fills_the_window_and_at_a_side_halves_it() {
    let host = Rect::new(0.0, 0.0, 1000.0, 600.0);
    assert_eq!(snap_at(500.0, 2.0, host), Some(Snap::Maximised));
    assert_eq!(snap_at(2.0, 300.0, host), Some(Snap::Half(Side::Left)));
    assert_eq!(snap_at(998.0, 300.0, host), Some(Snap::Half(Side::Right)));
    assert_eq!(snap_at(500.0, 300.0, host), None);
}

#[test]
fn a_tab_moved_to_a_window_comes_back_when_the_window_closes() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Tab).unwrap();
    ws.open(pane("c"), Target::Tab).unwrap();
    let main_group = group_of(&ws, "a");
    ws.move_pane(&id("b"), Dest::Window(None)).unwrap();
    ws.move_pane(&id("c"), Dest::Window(None)).unwrap();
    assert_eq!(ws.windows.len(), 3);
    // Every window has its own id, never reused.
    let ids: Vec<_> = ws.windows.iter().map(|w| w.id.0.clone()).collect();
    assert_eq!(ids.len(), 3);
    assert_ne!(ids[1], ids[2]);
    let second = ws.windows[1].id.clone();
    // Its OS window moved: the layout keeps where.
    let at = Rect::new(40.0, 50.0, 700.0, 500.0);
    ws.set_window_bounds(&second, at).unwrap();
    assert_eq!(ws.windows[1].bounds, Some(at));
    assert!(ws.set_window_bounds(&WindowId::main(), at).is_err());

    let back = ws.close_window(&second).unwrap();
    assert_eq!(back, vec![id("b")]);
    assert_eq!(ws.windows.len(), 2);
    assert_eq!(ws.locate(&id("b")).unwrap().window, 0);
    assert_eq!(group_of(&ws, "b"), main_group);
    assert_eq!(ws.active, Some(id("b")));
    // A new window after a close is another id.
    ws.move_pane(&id("a"), Dest::Window(None)).unwrap();
    assert!(!ws.windows.iter().any(|w| w.id == second));
    // The main window does not close.
    assert!(ws.close_window(&WindowId::main()).is_err());
}

#[test]
fn a_tab_in_a_window_goes_back_to_the_main_window() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.open(pane("b"), Target::Window(None)).unwrap();
    assert_eq!(ws.windows.len(), 2);
    ws.to_main_window(&id("b")).unwrap();
    // Its window, empty, closed.
    assert_eq!(ws.windows.len(), 1);
    assert_eq!(group_of(&ws, "b"), group_of(&ws, "a"));
    assert_eq!(ws.active, Some(id("b")));
    // Already there: it just comes forward.
    ws.activate(&id("a")).unwrap();
    ws.to_main_window(&id("b")).unwrap();
    assert_eq!(ws.active, Some(id("b")));
}

#[test]
fn closing_a_window_whose_main_window_is_empty_makes_it_a_group() {
    let mut ws = Workspace::new();
    ws.open(pane("a"), Target::Tab).unwrap();
    ws.move_pane(&id("a"), Dest::Window(None)).unwrap();
    let window = ws.windows[1].id.clone();
    ws.close_window(&window).unwrap();
    assert_eq!(ws.windows.len(), 1);
    assert_eq!(ws.locate(&id("a")).unwrap().area, Area::Center);
}

#[test]
fn a_tab_let_go_on_a_tab_bar_goes_between_the_tabs_there() {
    let g = GroupId(7);
    let bar = Bar {
        group: g,
        rect: Rect::new(0.0, 0.0, 400.0, 36.0),
        tabs: vec![
            Rect::new(0.0, 6.0, 100.0, 30.0),
            Rect::new(100.0, 6.0, 100.0, 30.0),
            Rect::new(200.0, 6.0, 100.0, 30.0),
        ],
    };
    let index = |x: f32| match bar_drop(&bar, x).dest {
        Dest::Group { index, .. } => index,
        other => panic!("{other:?}"),
    };
    assert_eq!(index(10.0), Some(0));
    assert_eq!(index(60.0), Some(1));
    assert_eq!(index(149.0), Some(1));
    assert_eq!(index(260.0), Some(3));
    assert_eq!(index(390.0), Some(3));
    // The line sits in the gap.
    assert_eq!(bar_drop(&bar, 60.0).highlight.x, 99.0);
    assert_eq!(bar_drop(&bar, 390.0).highlight.x, 299.0);

    // The bar wins over its group's edges; elsewhere the group's rules hold.
    let host = Rect::new(0.0, 0.0, 1000.0, 800.0);
    let groups = [(g, Rect::new(0.0, 0.0, 400.0, 800.0))];
    let on_bar = drop_in(150.0, 20.0, host, std::slice::from_ref(&bar), &groups).unwrap();
    assert!(matches!(on_bar.dest, Dest::Group { index: Some(2), .. }));
    let below = drop_in(200.0, 400.0, host, std::slice::from_ref(&bar), &groups).unwrap();
    assert!(matches!(below.dest, Dest::Group { index: None, .. }));
    // Outside the window: nothing here (the host makes it a window).
    assert!(drop_in(1200.0, 20.0, host, std::slice::from_ref(&bar), &groups).is_none());
}

#[test]
fn reordering_a_tab_within_its_own_bar_lands_where_it_was_let_go() {
    let mut ws = Workspace::new();
    for p in ["a", "b", "c", "d"] {
        ws.open(pane(p), Target::Tab).unwrap();
    }
    let g = group_of(&ws, "a");
    // "a" let go before "d" (index 3, counted before it leaves).
    ws.move_pane(
        &id("a"),
        Dest::Group {
            group: g,
            index: Some(3),
        },
    )
    .unwrap();
    assert_eq!(tabs(&ws, g), ["b", "c", "a", "d"]);
    ws.move_pane(
        &id("d"),
        Dest::Group {
            group: g,
            index: Some(0),
        },
    )
    .unwrap();
    assert_eq!(tabs(&ws, g), ["d", "b", "c", "a"]);
}

#[test]
fn stacked_panes_slide_over_the_ones_before_them() {
    let (w, spine, view) = (400.0, 32.0, 900.0);
    // Unscrolled: side by side until the strip's end, where the rest pile
    // their spines at the right.
    let s = slides(4, w, spine, view, 0.0);
    assert_eq!(s[0].x, 0.0);
    assert_eq!(s[1].x, 400.0);
    assert_eq!(s[2].x, 800.0);
    // The fourth would start past the end: its spine is kept at the right,
    // over the third.
    assert_eq!(s[3].x, view - spine);
    assert!(s[3].over && s[2].covered);
    assert!(!s[2].over && !s[1].covered);

    // Scrolled far: the first panes leave their spines at the left.
    let far = slides(4, w, spine, view, 1000.0);
    assert_eq!(far[0].x, 0.0);
    assert_eq!(far[1].x, spine);
    assert_eq!(far[2].x, 2.0 * spine);
    assert!(far[1].over && far[0].covered);
    assert_eq!(far[3].x, 3.0f32.mul_add(w, -1000.0).max(3.0 * spine));

    // A spine pressed scrolls its pane just past the spines before it.
    let to = scroll_to(2, w, spine);
    let shown = slides(4, w, spine, view, to);
    assert_eq!(shown[2].x, 2.0 * spine);
    assert_eq!(scroll_to(0, w, spine), 0.0);
}

#[test]
fn a_screen_has_a_form_class() {
    assert_eq!(FormClass::of(1440.0, 900.0, false), FormClass::Desk);
    // A touchscreen PC is a desk.
    assert_eq!(FormClass::of(1080.0, 1920.0, false), FormClass::Desk);
    assert_eq!(
        FormClass::of(1180.0, 820.0, true),
        FormClass::TabletLandscape
    );
    assert_eq!(
        FormClass::of(820.0, 1180.0, true),
        FormClass::TabletPortrait
    );
    assert_eq!(FormClass::of(393.0, 852.0, true), FormClass::PhonePortrait);
    assert_eq!(FormClass::of(852.0, 393.0, true), FormClass::PhoneLandscape);
}

/// A workspace with these tabs in its center.
fn with_tabs(names: &[&str]) -> Workspace {
    let mut ws = Workspace::new();
    for n in names {
        ws.open(pane(n), Target::Tab).unwrap();
    }
    ws
}

#[test]
fn a_slot_recalls_its_set_for_the_class_of_screen() {
    let sets = Screensets::default().with_builtins([
        Screenset::new("Edit")
            .slot(1)
            .layout(FormClass::Desk, with_tabs(&["song"])),
        Screenset::new("Mix")
            .slot(2)
            .layout(FormClass::Desk, with_tabs(&["mixer", "meters"]))
            .layout(FormClass::TabletPortrait, with_tabs(&["mixer"])),
    ]);
    assert_eq!(sets.get("mix").unwrap().name, "Mix");
    let mut sets = sets;
    let desk = sets.recall_slot(2, FormClass::Desk).unwrap();
    assert_eq!(desk.panes.len(), 2);
    assert_eq!(sets.active.as_deref(), Some("mix"));
    assert_eq!(sets.working(FormClass::Desk), Some(&desk));
    let upright = sets.recall_slot(2, FormClass::TabletPortrait).unwrap();
    assert_eq!(upright.panes.len(), 1);
    // A class it has no layout for takes the nearest.
    let side = sets.recall_slot(2, FormClass::TabletLandscape).unwrap();
    assert_eq!(side, desk);
    assert!(sets.recall_slot(5, FormClass::Desk).is_none());
}

#[test]
fn the_apps_sets_join_the_ones_kept_without_losing_changes() {
    let mut kept = Screensets::default().with_builtins([Screenset::new("Mix")
        .slot(2)
        .layout(FormClass::Desk, with_tabs(&["mixer"]))]);
    kept.recall("mix", FormClass::Desk);
    kept.set_working(FormClass::Desk, with_tabs(&["mixer", "fx"]));
    assert!(kept.save(FormClass::Desk));
    kept.save_as("Mine", Some(3));

    // The app ships Mix again (renamed, a new slot, a tablet layout).
    let joined = kept.clone().with_builtins([Screenset::new("Mix")
        .slot(4)
        .layout(FormClass::Desk, with_tabs(&["mixer"]))
        .layout(FormClass::TabletPortrait, with_tabs(&["mixer"]))]);
    let mix = joined.get("mix").unwrap();
    assert_eq!(mix.slot, Some(4));
    // The person's desk layout stays; the new class is added.
    assert_eq!(mix.layout_for(FormClass::Desk).unwrap().panes.len(), 2);
    assert!(
        mix.layouts
            .iter()
            .any(|l| l.class == FormClass::TabletPortrait)
    );
    assert!(joined.get("mine").is_some());
}

#[test]
fn a_store_keeps_the_screensets_and_reads_them_back() {
    let store = MemoryStore::new();
    let builtins = || {
        vec![
            Screenset::new("Mix")
                .slot(2)
                .layout(FormClass::Desk, with_tabs(&["mixer"])),
        ]
    };
    // Nothing kept: the app's sets.
    let mut sets = architect_workspace::store::load(&store, builtins());
    assert_eq!(sets.sets.len(), 1);
    sets.set_working(FormClass::Desk, with_tabs(&["a", "b"]));
    assert!(architect_workspace::store::save(&store, &sets));
    let back = architect_workspace::store::load(&store, builtins());
    assert_eq!(back.working(FormClass::Desk).unwrap().panes.len(), 2);
    assert_eq!(back, sets);
    // Something unreadable kept: the app's sets, nothing lost to a panic.
    store.save("{ not json");
    assert_eq!(
        architect_workspace::store::load(&store, builtins())
            .sets
            .len(),
        1
    );
}
