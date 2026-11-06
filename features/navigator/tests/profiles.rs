//! Vault profiles, hiding, shortcuts and recent notes, as Notebook
//! Navigator's (#338), driven through the navigator's state.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::Clock;
use architect_navigator::tree::{RECENT, SHORTCUTS};
use architect_navigator::{
    FileVisibility, HideTarget, MemorySource, NavRowKind, NavTarget, NavigatorSettings,
    NavigatorState, NoteFile, Outcome, Pane, PropertyValue, Shortcut, ShortcutKind, SourceEvent,
    VaultIndex,
};

const NOW: i64 = 1_793_880_000_000;

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(NoteFile::new("Work/Plan.md", NOW, NOW).tag("work"))
        .with_file(NoteFile::new("Work/Private/Salary.md", NOW, NOW).tag("work"))
        .with_file(NoteFile::new("Journal/2026/Nov.md", NOW, NOW).tag("journal"))
        .with_file(NoteFile::new("Journal/Index.md", NOW, NOW).tag("archive/old"))
        .with_file(
            NoteFile::new("Home/Draft.md", NOW, NOW)
                .property("draft", PropertyValue::Bool(true))
                .tag("home"),
        )
        .with_file(NoteFile::new("Home/Untitled 3.md", NOW, NOW))
        .with_file(NoteFile::new("Home/Photo.png", NOW, NOW))
}

fn state(src: &MemorySource) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(src),
        NavigatorSettings {
            include_descendants: true,
            ..NavigatorSettings::default()
        },
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 600.0);
    s
}

fn listed(s: &NavigatorState) -> Vec<String> {
    let mut v: Vec<String> = s.list_order().into_iter().map(str::to_owned).collect();
    v.sort();
    v
}

fn folders(s: &NavigatorState) -> Vec<String> {
    s.nav_rows()
        .iter()
        .filter_map(|r| r.folder().map(str::to_owned))
        .collect()
}

fn tags(s: &NavigatorState) -> Vec<String> {
    s.nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Tag { key, .. } => Some(key.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_profile_hides_folders_tags_and_notes_by_pattern_frontmatter_and_tag() {
    let src = vault();
    let mut s = state(&src);
    s.expand_all();
    assert_eq!(listed(&s).len(), 7);
    s.edit_profile(|p| {
        p.hidden_folders = vec!["private".into()];
        p.hidden_tags = vec!["archive".into()];
        p.hidden_file_names = vec!["untitled*".into()];
        p.hidden_file_properties = vec!["draft=true".into()];
        p.hidden_file_tags = vec!["journal".into()];
        p.file_visibility = FileVisibility::Documents;
    });
    assert_eq!(listed(&s), ["Journal/Index.md", "Work/Plan.md"]);
    assert!(!folders(&s).contains(&"Work/Private".to_owned()));
    assert!(
        !tags(&s).iter().any(|t| t.starts_with("archive")),
        "{:?}",
        tags(&s)
    );
    // Shown again, all of it.
    s.toggle_hidden();
    assert!(s.show_hidden());
    assert!(folders(&s).contains(&"Work/Private".to_owned()));
    assert!(tags(&s).iter().any(|t| t.starts_with("archive")));
    assert_eq!(listed(&s).len(), 6, "the file types still hold: no image");
    s.toggle_hidden();
    // A change in the source to a hidden note stays hidden.
    s.apply(&SourceEvent::FileCreated(NoteFile::new(
        "Work/Private/Bonus.md",
        NOW,
        NOW,
    )));
    assert!(!listed(&s).contains(&"Work/Private/Bonus.md".to_owned()));
    assert!(s.all().file("Work/Private/Bonus.md").is_some());
    // The context menu's hide and unhide.
    s.hide(&HideTarget::Folder("Work".into()), false);
    assert_eq!(listed(&s), ["Journal/Index.md"]);
    s.hide(&HideTarget::Folder("Work".into()), true);
    assert_eq!(listed(&s).len(), 2);
}

#[test]
fn a_folder_excluded_from_descendants_is_browsed_but_left_out_above() {
    let src = vault();
    let mut s = state(&src);
    s.edit_profile(|p| p.descendant_excluded = vec!["/Journal/2026".into()]);
    s.select_folder("Journal");
    assert_eq!(listed(&s), ["Journal/Index.md"]);
    s.select_folder("");
    assert!(!listed(&s).contains(&"Journal/2026/Nov.md".to_owned()));
    // Still there, and browsed.
    s.select_folder("Journal/2026");
    assert_eq!(listed(&s), ["Journal/2026/Nov.md"]);
}

#[test]
fn profiles_switch_what_is_hidden_and_their_shortcuts() {
    let src = vault();
    let mut s = state(&src);
    s.add_shortcut(Shortcut::note("Work/Plan.md"), None);
    s.add_profile("Focus", true).unwrap();
    assert_eq!(s.profile().name, "Focus");
    assert_eq!(s.profile().shortcuts.len(), 1, "copied from the one in use");
    s.edit_profile(|p| p.hidden_folders = vec!["Home".into(), "Journal".into()]);
    assert_eq!(listed(&s), ["Work/Plan.md", "Work/Private/Salary.md"]);
    // Back to the default: everything again; by number, name or id.
    assert!(s.select_profile("1").unwrap());
    assert_eq!(listed(&s).len(), 7);
    assert!(s.select_profile("focus").unwrap());
    assert_eq!(listed(&s).len(), 2);
    assert!(s.select_profile("nope").is_err());
    // Kept across runs.
    let kept = s.view_state();
    let mut again = state(&src);
    again.restore(&kept);
    assert_eq!(again.profile().name, "Focus");
    assert_eq!(listed(&again).len(), 2);
    again.remove_profile("Focus").unwrap();
    assert_eq!(again.profile().name, "Default");
    assert_eq!(listed(&again).len(), 7);
}

#[test]
fn shortcuts_open_notes_folders_tags_and_searches_reorder_and_show_over_the_folders() {
    let src = vault();
    let mut s = state(&src);
    assert!(s.add_shortcut(Shortcut::note("Work/Plan.md"), None));
    assert!(!s.add_shortcut(Shortcut::note("Work/Plan.md"), None));
    s.add_shortcut(Shortcut::to(&NavTarget::Folder("Journal".into())), None);
    s.add_shortcut(Shortcut::to(&NavTarget::Tag("home".into())), None);
    s.add_shortcut(Shortcut::search("Work notes", "#work"), None);
    // Over the folders: the section, then each, numbered.
    let rows = s.nav_rows();
    let at = rows
        .iter()
        .position(|r| matches!(&r.kind, NavRowKind::Section { id, .. } if id == SHORTCUTS))
        .unwrap();
    assert!(at <= 1, "on top");
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Shortcut { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["Plan", "Journal", "#home", "Work notes"]);
    // 1: a note opens.
    assert_eq!(
        s.open_shortcut(1).unwrap(),
        Some(Outcome::Open {
            path: "Work/Plan.md".into(),
            how: architect_navigator::OpenIn::Here,
            settle: false,
        })
    );
    // 2: a folder is selected; 3: a tag; 4: the search runs.
    s.open_shortcut(2).unwrap();
    assert_eq!(s.target(), NavTarget::Folder("Journal".into()));
    s.open_shortcut(3).unwrap();
    assert_eq!(s.target(), NavTarget::Tag("home".into()));
    s.select_folder("");
    s.open_shortcut(4).unwrap();
    assert_eq!(s.search().text(), "#work");
    assert_eq!(listed(&s).len(), 2);
    assert!(s.open_shortcut(9).is_err());
    // Reordered, renamed, folded.
    assert!(s.move_shortcut(3, 0));
    assert_eq!(s.profile().shortcuts[0].kind, ShortcutKind::Search);
    s.rename_shortcut(1, "The plan");
    assert_eq!(s.profile().shortcuts[1].label(), "The plan");
    s.toggle_section(SHORTCUTS);
    assert!(
        !s.nav_rows()
            .iter()
            .any(|r| matches!(r.kind, NavRowKind::Shortcut { .. }))
    );
    // A note moved takes its shortcut with it; one deleted shows missing.
    s.apply(&SourceEvent::FileRenamed {
        from: "Work/Plan.md".into(),
        file: NoteFile::new("Work/Roadmap.md", NOW, NOW),
    });
    assert_eq!(s.profile().shortcuts[1].target, "Work/Roadmap.md");
}

#[test]
fn recent_notes_are_per_profile_newest_first_and_capped() {
    let src = vault();
    let mut s = state(&src);
    s.set_settings(NavigatorSettings {
        recent_notes_count: 2,
        ..s.settings().clone()
    });
    for p in ["Work/Plan.md", "Home/Draft.md", "Journal/Index.md"] {
        s.set_active(Some(p));
    }
    assert_eq!(s.profile().recent, ["Journal/Index.md", "Home/Draft.md"]);
    let recent: Vec<String> = s
        .nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Recent { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(recent, ["Journal/Index.md", "Home/Draft.md"]);
    assert!(
        s.nav_rows()
            .iter()
            .any(|r| matches!(&r.kind, NavRowKind::Section { id, .. } if id == RECENT))
    );
    // Another profile, its own.
    s.add_profile("Other", true).unwrap();
    assert!(s.profile().recent.is_empty());
    s.set_active(Some("Work/Plan.md"));
    assert_eq!(s.profile().recent, ["Work/Plan.md"]);
    s.select_profile("Default").unwrap();
    assert_eq!(s.profile().recent.len(), 2);
    // A deleted note leaves them.
    s.apply(&SourceEvent::FileDeleted("Home/Draft.md".into()));
    assert_eq!(s.profile().recent, ["Journal/Index.md"]);
}

#[test]
fn a_banner_row_sits_over_the_rest() {
    let src = vault();
    let mut s = state(&src);
    s.edit_profile(|p| p.banner = Some("Home/Photo.png".into()));
    assert!(matches!(
        &s.nav_rows()[1].kind,
        NavRowKind::Banner { src } if src == "Home/Photo.png"
    ));
}
