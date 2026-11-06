//! The keyboard map, and icons and colours, as Notebook Navigator's
//! (#339): keys read by the map and done in their pane, Vim's and a
//! person's own kept; icons and colours set on folders, tags, properties
//! and notes, inherited, read from and written to frontmatter, and
//! following what moves.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::Clock;
use architect_navigator::state::Paint;
use architect_navigator::{
    FileOp, Icon, KeyAction, KeyEvent, KeyScope, MemorySource, NavigatorSettings, NavigatorSource,
    NavigatorState, NoteFile, Outcome, Pane, PropertyValue, SourceEvent, VaultIndex,
};

const NOW: i64 = 1_793_889_000_000;

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(NoteFile::new("Projects/Alpha/Spec.md", NOW, NOW))
        .with_file(NoteFile::new("Projects/Plan.md", NOW - 1, NOW - 1).tag("work/team"))
        .with_file(NoteFile::new("Projects/Meeting notes.md", NOW - 2, NOW - 2))
        .with_file(
            NoteFile::new("Inbox/Starred.md", NOW - 3, NOW - 3)
                .property("icon", PropertyValue::Text("LiStar".to_owned()))
                .property("color", PropertyValue::Text("gold".to_owned())),
        )
        .with_file(NoteFile::new("Inbox/scan.pdf", NOW - 4, NOW - 4))
}

fn state(src: &MemorySource, settings: NavigatorSettings) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(src),
        NavigatorSettings {
            include_descendants: true,
            ..settings
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

fn press(s: &mut NavigatorState, key: &str) -> Option<Outcome> {
    let scope = if s.focus == Pane::Navigation {
        KeyScope::Navigation
    } else {
        KeyScope::List
    };
    let a = s.keymap().action(scope, &KeyEvent::plain(key), true)?;
    s.run_key(a)
}

#[test]
fn keys_are_read_by_the_map_and_done_in_their_pane() {
    let src = vault();
    let mut s = state(&src, NavigatorSettings::default());
    s.focus_pane(Pane::Files);
    let first = s.selection().cursor().map(str::to_owned);
    press(&mut s, "ArrowDown");
    assert_ne!(s.selection().cursor().map(str::to_owned), first);
    assert_eq!(press(&mut s, "j"), None, "no Vim keys by default");
    press(&mut s, "ArrowLeft");
    assert_eq!(s.focus, Pane::Navigation, "list:focus-navigation");
    // Vim's, from the settings.
    let mut s = state(
        &src,
        NavigatorSettings {
            vim_keys: true,
            ..NavigatorSettings::default()
        },
    );
    s.focus_pane(Pane::Files);
    let at = s.selection().cursor().map(str::to_owned);
    assert!(matches!(
        press(&mut s, "j"),
        Some(Outcome::Open { settle: true, .. })
    ));
    assert_ne!(s.selection().cursor().map(str::to_owned), at);
    press(&mut s, "h");
    assert_eq!(s.focus, Pane::Navigation);
    // A person's own: kept between runs, and back to the default.
    s.bind_keys(KeyAction::PaneMoveDown, Some(vec!["n".to_owned()]));
    let kept = s.view_state();
    let mut again = state(&src, NavigatorSettings::default());
    again.restore(&kept);
    assert_eq!(
        again
            .keymap()
            .action(KeyScope::List, &KeyEvent::plain("n"), true),
        Some(KeyAction::PaneMoveDown)
    );
    assert_eq!(
        again
            .keymap()
            .action(KeyScope::List, &KeyEvent::plain("ArrowDown"), true),
        None,
        "replaced"
    );
    again.bind_keys(KeyAction::PaneMoveDown, None);
    assert_eq!(
        again
            .keymap()
            .action(KeyScope::List, &KeyEvent::plain("ArrowDown"), true),
        Some(KeyAction::PaneMoveDown)
    );
}

#[test]
fn icons_and_colours_are_set_inherited_and_follow_what_moves() {
    let src = vault();
    let mut s = state(&src, NavigatorSettings::default());
    assert!(
        s.paint(Paint::Icon, "folder:Projects", Some("LiRocket"))
            .unwrap()
            .is_none()
    );
    s.paint(Paint::Colour, "folder:Projects", Some("tomato"))
        .unwrap();
    s.paint(Paint::Background, "tag:work", Some("#eef"))
        .unwrap();
    assert_eq!(
        s.folder_icon("Projects"),
        Some(Icon::Lucide("rocket".to_owned()))
    );
    assert_eq!(
        s.folder_icon("Projects/Alpha"),
        None,
        "icons are a folder's own"
    );
    assert_eq!(
        s.colour("folder:Projects/Alpha"),
        Some("tomato"),
        "inherited"
    );
    assert_eq!(s.background("tag:work/team"), Some("#eef"));
    assert!(
        s.paint(Paint::Icon, "folder:Projects", Some("not an icon!"))
            .is_err()
    );
    assert!(s.paint(Paint::Icon, "nowhere", Some("star")).is_err());
    // Off: not inherited.
    let mut flat = s.settings().clone();
    flat.inherit_folder_colors = false;
    s.set_settings(flat);
    assert_eq!(s.colour("folder:Projects/Alpha"), None);
    // A folder moved: its paint goes with it; kept between runs.
    s.apply(&SourceEvent::FolderRenamed {
        from: "Projects".to_owned(),
        to: "Work".to_owned(),
    });
    assert_eq!(s.colour("folder:Work"), Some("tomato"));
    assert_eq!(s.colour("folder:Projects"), None);
    let kept = s.view_state();
    let mut again = state(&src, NavigatorSettings::default());
    again.restore(&kept);
    assert_eq!(
        again.own_icon("folder:Work"),
        Some(Icon::Lucide("rocket".to_owned()))
    );
}

#[test]
fn notes_take_icons_from_themselves_their_names_and_types_and_frontmatter() {
    let src = vault();
    let s = state(
        &src,
        NavigatorSettings {
            file_name_icons: true,
            file_name_icon_map: "meeting=users".to_owned(),
            file_type_icons: true,
            ..NavigatorSettings::default()
        },
    );
    assert_eq!(
        s.file_icon("Projects/Meeting notes.md"),
        Some(Icon::Lucide("users".to_owned()))
    );
    assert_eq!(
        s.file_icon("Inbox/scan.pdf"),
        Some(Icon::Lucide("file-text".to_owned()))
    );
    assert_eq!(
        s.file_icon("Projects/Plan.md"),
        Some(Icon::Lucide("file-text".to_owned()))
    );
    // The frontmatter's, with names and dates from frontmatter.
    assert_eq!(
        s.file_icon("Inbox/Starred.md"),
        Some(Icon::Lucide("file-text".to_owned()))
    );
    let mut s = state(
        &src,
        NavigatorSettings {
            use_frontmatter: true,
            ..NavigatorSettings::default()
        },
    );
    assert_eq!(
        s.file_icon("Inbox/Starred.md"),
        Some(Icon::Lucide("star".to_owned()))
    );
    assert_eq!(s.colour("file:Inbox/Starred.md"), Some("gold"));
    assert_eq!(s.file_icon("Projects/Plan.md"), None, "no rules on");
    // Written to the frontmatter, in its short form; taken off again.
    let out = s
        .paint(Paint::Icon, "file:Projects/Plan.md", Some("phosphor:house"))
        .unwrap();
    let Some(Outcome::Write { ops, .. }) = out else {
        panic!("a write")
    };
    assert_eq!(
        ops,
        [FileOp::SetProperty {
            path: "Projects/Plan.md".to_owned(),
            key: "icon".to_owned(),
            value: PropertyValue::Text("ph:house".to_owned()),
        }]
    );
    src.write(&ops).unwrap();
    let out = s.paint(Paint::Icon, "file:Inbox/Starred.md", None).unwrap();
    let Some(Outcome::Write { ops, .. }) = out else {
        panic!("a write")
    };
    assert_eq!(
        ops,
        [FileOp::RemoveProperty {
            path: "Inbox/Starred.md".to_owned(),
            key: "icon".to_owned(),
        }]
    );
    src.write(&ops).unwrap();
    assert!(src.file("Inbox/Starred.md").unwrap().get("icon").is_none());
}
