//! The navigator's model without a window: the tree, the list, selection
//! and history, reveal, collapse and expand all, the keys, and a source's
//! changes, each as Notebook Navigator does it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::{Clock, date_group};
use architect_navigator::source::Subscription;
use architect_navigator::tree::natural_cmp;
use architect_navigator::{
    Key, ListRowKind, MemorySource, Mods, NavRowKind, NavigatorSettings, NavigatorSource,
    NavigatorState, NoteFile, OpenIn, Outcome, Pane, SortOption, SourceEvent, VaultIndex,
};

const DAY: i64 = 86_400_000;
/// 2026-11-05 12:00 UTC.
const NOW: i64 = 1_793_880_000_000;

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_folder("Archive")
        .with_file(NoteFile::new("Welcome.md", NOW - 9 * DAY, NOW - 2 * DAY).preview("Hello"))
        .with_file(NoteFile::new("Projects/Plan.md", NOW - 5 * DAY, NOW).preview("The plan"))
        .with_file(NoteFile::new(
            "Projects/Alpha/Spec.md",
            NOW - 4 * DAY,
            NOW - DAY,
        ))
        .with_file(NoteFile::new(
            "Projects/Alpha/Notes 10.md",
            NOW - 3 * DAY,
            NOW - 40 * DAY,
        ))
        .with_file(NoteFile::new(
            "Projects/Alpha/Notes 2.md",
            NOW - 3 * DAY,
            NOW - 400 * DAY,
        ))
        .with_file(NoteFile::new("Journal/2026-11-05.md", NOW, NOW))
}

fn state(source: &MemorySource) -> NavigatorState {
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

fn folders(s: &NavigatorState) -> Vec<(String, u16)> {
    s.nav_rows()
        .iter()
        .filter_map(|r| r.folder().map(|f| (f.to_owned(), r.level)))
        .collect()
}

fn files(s: &NavigatorState) -> Vec<String> {
    s.list_order().into_iter().map(str::to_owned).collect()
}

#[test]
fn the_tree_flattens_depth_first_in_natural_order_with_the_root_on_top() {
    let mut s = state(&vault());
    assert_eq!(
        folders(&s),
        [("", 0), ("Archive", 1), ("Journal", 1), ("Projects", 1)].map(|(p, l)| (p.to_owned(), l))
    );
    s.toggle("Projects");
    assert_eq!(folders(&s)[4], ("Projects/Alpha".to_owned(), 2));
    // The root hidden: the top folders are level 0.
    let mut settings = NavigatorSettings::default();
    settings.show_root = false;
    s.set_settings(settings);
    assert_eq!(folders(&s)[0], ("Archive".to_owned(), 0));
    // Counts: Projects holds one note and three below.
    let row = s
        .nav_rows()
        .iter()
        .find(|r| r.folder() == Some("Projects"))
        .unwrap();
    let NavRowKind::Folder {
        count,
        has_children,
        ..
    } = &row.kind
    else {
        panic!()
    };
    assert_eq!(
        (count.current, count.descendants, *has_children),
        (1, 3, true)
    );
    assert!(natural_cmp("Note 2", "Note 10").is_lt());
    assert!(natural_cmp("alpha", "Beta").is_lt());
}

#[test]
fn manual_root_order_comes_first_and_moves() {
    let mut s = state(&vault());
    s.move_child("", "Projects", -2);
    assert_eq!(folders(&s)[1].0, "Projects");
    assert_eq!(folders(&s)[2].0, "Archive");
    s.reset_order("");
    assert_eq!(folders(&s)[1].0, "Archive");
}

#[test]
fn the_list_sorts_and_groups_by_date() {
    let s = &mut state(&vault());
    s.click_folder("Projects");
    let mut settings = NavigatorSettings::default();
    settings.include_descendants = true;
    s.set_settings(settings.clone());
    assert_eq!(
        files(s),
        [
            "Projects/Plan.md",
            "Projects/Alpha/Spec.md",
            "Projects/Alpha/Notes 10.md",
            "Projects/Alpha/Notes 2.md"
        ]
    );
    let headers: Vec<String> = s
        .list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            ListRowKind::Header { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(headers, ["Today", "Yesterday", "September", "2025"]);
    // From a folder below, the row says which.
    let spec = s
        .list_rows()
        .iter()
        .find(|r| r.file() == Some("Projects/Alpha/Spec.md"))
        .unwrap();
    assert!(
        matches!(&spec.kind, ListRowKind::File { parent: Some(p), .. } if p == "Projects/Alpha")
    );
    settings.sort = SortOption::TitleAsc;
    s.set_settings(settings);
    assert_eq!(
        files(s),
        [
            "Projects/Alpha/Notes 2.md",
            "Projects/Alpha/Notes 10.md",
            "Projects/Plan.md",
            "Projects/Alpha/Spec.md"
        ]
    );
    assert!(
        s.list_rows()
            .iter()
            .all(|r| !matches!(r.kind, ListRowKind::Header { .. }))
    );
    let c = Clock {
        now: NOW,
        offset: 0,
    };
    assert_eq!(date_group(NOW + DAY, c).1, "Future");
    assert_eq!(date_group(NOW - 3 * DAY, c).1, "Previous 7 days");
    assert_eq!(date_group(NOW - 20 * DAY, c).1, "Previous 30 days");
}

#[test]
fn clicks_select_one_toggle_and_range() {
    let s = &mut state(&vault());
    let mut settings = NavigatorSettings::default();
    settings.include_descendants = true;
    settings.sort = SortOption::FilenameAsc;
    s.set_settings(settings);
    s.click_folder("Projects");
    let order = files(s);
    assert_eq!(order.len(), 4);
    let open = s.click_file(&order[0], Mods::NONE);
    assert_eq!(
        open,
        Some(Outcome::Open {
            path: order[0].clone(),
            how: OpenIn::Here,
            settle: false
        })
    );
    // Shift: the range from the cursor.
    s.click_file(&order[2], Mods::SHIFT);
    assert_eq!(s.selection().files().len(), 3);
    assert_eq!(s.selection().cursor(), Some(order[2].as_str()));
    // Cmd: out, and the cursor moves to the first still selected.
    let open = s.click_file(&order[2], Mods::COMMAND);
    assert_eq!(s.selection().files().len(), 2);
    assert_eq!(
        open,
        Some(Outcome::Open {
            path: order[0].clone(),
            how: OpenIn::Here,
            settle: false
        })
    );
    // Out again: the cursor's neighbour goes; the last can't be let go.
    assert_eq!(s.click_file(&order[1], Mods::COMMAND), None);
    assert_eq!(s.click_file(&order[0], Mods::COMMAND), None);
    assert_eq!(s.selection().files().len(), 1);
    s.select_all();
    assert_eq!(s.selection().files().len(), 4);
}

#[test]
fn history_goes_back_and_forward_through_folders() {
    let s = &mut state(&vault());
    s.click_folder("Projects");
    s.click_folder("Journal");
    s.click_folder("Archive");
    s.back();
    assert_eq!(s.selection().folder(), Some("Journal"));
    s.back();
    assert_eq!(s.selection().folder(), Some("Projects"));
    s.forward();
    assert_eq!(s.selection().folder(), Some("Journal"));
    // A new selection drops what was ahead.
    s.click_folder("Projects");
    assert!(!s.selection().can_go_forward());
    // Kept to the last hundred.
    for i in 0..150 {
        s.click_folder(if i % 2 == 0 { "Archive" } else { "Journal" });
    }
    assert_eq!(s.selection().history().0.len(), 100);
}

#[test]
fn the_open_note_is_revealed_and_collapse_all_keeps_it_in_view() {
    let s = &mut state(&vault());
    s.set_active(Some("Projects/Alpha/Spec.md"));
    assert_eq!(s.selection().folder(), Some("Projects/Alpha"));
    assert_eq!(s.selection().cursor(), Some("Projects/Alpha/Spec.md"));
    assert!(s.expanded().contains("Projects"));
    // Smart collapse: the chain stays open; again, everything opens.
    s.toggle("Archive");
    s.collapse_or_expand_all();
    assert!(s.expanded().contains("Projects"));
    assert!(!s.expanded().contains("Archive"));
    s.collapse_or_expand_all();
    // Every folder that holds one opens (Projects; Alpha holds none).
    assert!(s.expanded().contains("Projects") && s.any_expanded());
    // The shortest path: with notes from below, a folder that lists the
    // note already stays selected.
    let mut settings = NavigatorSettings::default();
    settings.include_descendants = true;
    s.set_settings(settings);
    s.collapse_all();
    s.click_folder("Projects");
    s.set_active(Some("Projects/Alpha/Notes 2.md"));
    assert_eq!(s.selection().folder(), Some("Projects"));
    // The reveal command goes to its real folder.
    s.reveal_active();
    assert_eq!(s.selection().folder(), Some("Projects/Alpha"));
}

#[test]
fn keys_move_through_both_panes() {
    let s = &mut state(&vault());
    // The folders alone (the tags' keys: tests/tags_properties.rs).
    let mut settings = NavigatorSettings::default();
    settings.tag_tree = false;
    s.set_settings(settings);
    s.focus = Pane::Navigation;
    s.key(Key::Down, Mods::NONE);
    assert_eq!(s.selection().folder(), Some("Archive"));
    s.key(Key::End, Mods::NONE);
    assert_eq!(s.selection().folder(), Some("Projects"));
    // → opens a closed folder, then moves to the list.
    s.key(Key::Right, Mods::NONE);
    assert!(s.expanded().contains("Projects"));
    s.key(Key::Right, Mods::NONE);
    assert_eq!(s.focus, Pane::Files);
    assert_eq!(s.selection().cursor(), Some("Projects/Plan.md"));
    // ← back; ← closes; ← again goes up.
    s.key(Key::Left, Mods::NONE);
    assert_eq!(s.focus, Pane::Navigation);
    s.key(Key::Left, Mods::NONE);
    assert!(!s.expanded().contains("Projects"));
    s.key(Key::Left, Mods::NONE);
    assert_eq!(s.selection().folder(), Some(""));
    // The list: ↓ opens (once the keys settle), ⇧↓ grows, ⌘A all.
    s.click_folder("Projects");
    let mut settings = NavigatorSettings::default();
    settings.include_descendants = true;
    s.set_settings(settings);
    s.focus_pane(Pane::Files);
    let first = files(s)[0].clone();
    assert_eq!(s.selection().cursor(), Some(first.as_str()));
    let out = s.key(Key::Down, Mods::NONE);
    assert!(matches!(out, Some(Outcome::Open { settle: true, .. })));
    s.key(Key::Down, Mods::SHIFT);
    assert_eq!(s.selection().files().len(), 2);
    s.key(Key::Up, Mods::SHIFT);
    assert_eq!(s.selection().files().len(), 1);
    s.key(Key::A, Mods::COMMAND);
    assert_eq!(s.selection().files().len(), 4);
    assert!(matches!(
        s.key(Key::Enter, Mods::COMMAND),
        Some(Outcome::Open {
            how: OpenIn::Split,
            ..
        })
    ));
    assert_eq!(s.key(Key::Right, Mods::NONE), Some(Outcome::FocusEditor));
}

#[test]
fn a_source_change_moves_the_selection_and_the_rows() {
    let source = vault();
    let s = &mut state(&source);
    let (tx, rx) = std::sync::mpsc::channel();
    let _sub: Subscription = source.subscribe(Box::new(move |e: &SourceEvent| {
        let _ = tx.send(e.clone());
    }));
    s.click_folder("Projects");
    s.click_file("Projects/Plan.md", Mods::NONE);
    source.rename("Projects/Plan.md", "Projects/Roadmap.md");
    source.put(NoteFile::new("Projects/New.md", NOW, NOW + 1));
    source.delete_folder("Journal");
    for e in rx.try_iter() {
        s.apply(&e);
    }
    assert_eq!(s.selection().cursor(), Some("Projects/Roadmap.md"));
    assert_eq!(files(s), ["Projects/New.md", "Projects/Roadmap.md"]);
    assert!(folders(s).iter().all(|(f, _)| f != "Journal"));
}
