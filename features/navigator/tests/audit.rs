//! What the audit against Notebook Navigator's settings brought in: the
//! collapse button's scope, one branch open, opening on select, folders Z
//! to A, the Properties row off, space between top rows, the navigation
//! rainbow, the first note opened, the homepage and folding the list's
//! groups.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::Clock;
use architect_navigator::settings::{CollapseScope, FolderSort, RainbowMode, RainbowScope};
use architect_navigator::{
    MemorySource, NavRowKind, NavTarget, NavigatorSettings, NavigatorState, NoteFile, Outcome,
    Pane, PropertyValue, VaultIndex,
};

const NOW: i64 = 1_793_880_000_000;

fn vault() -> MemorySource {
    let note = |p: &str| NoteFile::new(p, NOW, NOW);
    MemorySource::new("Lab")
        .with_file(
            note("Alpha/One/a.md")
                .tag("work/plan")
                .property("status", PropertyValue::Text("doing".into())),
        )
        .with_file(note("Alpha/Two/b.md").tag("work/spec"))
        .with_file(note("Beta/Three/c.md").tag("home/garden"))
        .with_file(note("Gamma/d.md").property("status", PropertyValue::Text("done".into())))
        .with_file(note("Home.md"))
}

fn state(s: NavigatorSettings) -> NavigatorState {
    let mut st = NavigatorState::new(
        VaultIndex::read(&vault()),
        s,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    st.set_view(Pane::Navigation, 2000.0);
    st.set_view(Pane::Files, 600.0);
    st
}

fn settings() -> NavigatorSettings {
    NavigatorSettings {
        property_keys: "status".into(),
        ..NavigatorSettings::default()
    }
}

fn folders(s: &NavigatorState) -> Vec<String> {
    s.nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Folder { path, .. } if !path.is_empty() => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn open_tags(s: &NavigatorState) -> usize {
    s.nav_rows()
        .iter()
        .filter(|r| matches!(r.kind, NavRowKind::Tag { expanded: true, .. }))
        .count()
}

#[test]
fn collapse_all_folds_what_its_scope_says() {
    let mut s = state(NavigatorSettings {
        collapse_scope: CollapseScope::Tags,
        ..settings()
    });
    s.toggle_nav(&NavTarget::Tag("work".into()));
    s.set_expanded("Alpha", true);
    assert_eq!(open_tags(&s), 2, "Tags and work");
    s.collapse_or_expand_all();
    assert_eq!(open_tags(&s), 1, "the Tags row alone");
    assert!(
        folders(&s).contains(&"Alpha/One".to_owned()),
        "the folders are left"
    );
    s.collapse_or_expand_all();
    assert_eq!(open_tags(&s), 3, "every tag with tags below opened");
}

#[test]
fn one_branch_open_and_opening_on_select() {
    let mut s = state(NavigatorSettings {
        accordion: true,
        expand_on_select: true,
        ..settings()
    });
    s.set_expanded("Alpha", true);
    s.set_expanded("Alpha/One", true);
    s.click_folder("Beta");
    let shown = folders(&s);
    assert!(
        shown.contains(&"Beta/Three".to_owned()),
        "Beta opened on select: {shown:?}"
    );
    assert!(
        !shown.contains(&"Alpha/One".to_owned()),
        "Alpha folded: {shown:?}"
    );
}

#[test]
fn folders_z_to_a_and_the_properties_row_off() {
    let s = state(NavigatorSettings {
        folder_sort: FolderSort::ZToA,
        properties_row: false,
        ..settings()
    });
    let top: Vec<String> = folders(&s)
        .into_iter()
        .filter(|f| !f.contains('/'))
        .collect();
    assert_eq!(top, ["Gamma", "Beta", "Alpha"]);
    let props: Vec<(u16, String)> = s
        .nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Property { name, .. } => Some((r.level, name.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(props.first(), Some(&(0, "status".to_owned())), "{props:?}");
}

#[test]
fn space_between_top_rows() {
    let plain = state(settings());
    let spaced = state(NavigatorSettings {
        root_spacing: 8,
        ..settings()
    });
    let h = |s: &NavigatorState| {
        (0..s.nav_rows().len())
            .map(|i| s.nav_offsets().height(i))
            .sum::<f64>()
    };
    assert!(h(&spaced) > h(&plain) + 8.0, "{} {}", h(&spaced), h(&plain));
}

#[test]
fn the_rainbow_colours_siblings_and_their_children() {
    let off = state(settings());
    assert!(off.rainbow("folder:Alpha").is_none());
    let mut s = state(NavigatorSettings {
        rainbow: RainbowMode::Text,
        ..settings()
    });
    s.set_expanded("Alpha", true);
    let key = |p: &str| {
        s.nav_rows()
            .iter()
            .find(|r| r.folder() == Some(p))
            .map(|r| r.key.clone())
            .unwrap()
    };
    let (a, _) = s.rainbow(&key("Alpha")).unwrap();
    let (g, _) = s.rainbow(&key("Gamma")).unwrap();
    assert_eq!(
        (a, g),
        ("#ef4444", "#8b5cf6"),
        "first and last of the top level"
    );
    assert_eq!(
        s.rainbow(&key("Alpha/One")).unwrap().0,
        a,
        "a child takes its ancestor's"
    );
    let mut all = state(NavigatorSettings {
        rainbow: RainbowMode::Background,
        rainbow_scope: RainbowScope::All,
        ..settings()
    });
    all.set_expanded("Alpha", true);
    let k = all
        .nav_rows()
        .iter()
        .find(|r| r.folder() == Some("Alpha/Two"))
        .map(|r| r.key.clone())
        .unwrap();
    assert_eq!(
        all.rainbow(&k),
        Some(("#8b5cf6", true)),
        "its own group's last, as a background"
    );
}

#[test]
fn the_first_note_opens_and_the_homepage() {
    let mut s = state(NavigatorSettings {
        open_first_note: true,
        homepage: "Home".into(),
        ..settings()
    });
    s.click_folder("Gamma");
    assert!(matches!(s.first_note(), Some(Outcome::Open { path, .. }) if path == "Gamma/d.md"));
    assert!(matches!(s.homepage().unwrap(), Some(Outcome::Open { path, .. }) if path == "Home.md"));
    let none = state(settings());
    assert_eq!(none.first_note(), None);
    assert_eq!(none.homepage().unwrap(), None);
    let missing = state(NavigatorSettings {
        homepage: "Nope.md".into(),
        ..settings()
    });
    assert!(missing.homepage().is_err());
    let daily = state(NavigatorSettings {
        homepage: "daily".into(),
        ..settings()
    });
    assert!(daily.homepage().unwrap().is_some(), "made when missing");
}

#[test]
fn the_list_s_groups_fold_and_unfold_together() {
    let mut s = state(settings());
    s.select_folder("");
    let headers = |s: &NavigatorState| {
        s.list_rows()
            .iter()
            .filter(|r| matches!(r.kind, architect_navigator::ListRowKind::Header { .. }))
            .count()
    };
    if headers(&s) == 0 {
        return;
    }
    assert!(s.fold_all_groups());
    assert!(!s.fold_all_groups());
}

/// Pinned shortcuts (`pinShortcuts`) leave the tree for a block over it;
/// the recent notes join them when the settings say.
#[test]
fn shortcuts_and_recent_notes_are_pinned_over_the_tree() {
    let mut s = state(NavigatorSettings {
        pin_shortcuts: true,
        pin_recent: true,
        show_recent_notes: true,
        ..settings()
    });
    s.add_shortcut(architect_navigator::Shortcut::note("Gamma/d.md"), None);
    s.set_active(Some("Home.md"));
    let in_tree = |s: &NavigatorState| {
        s.nav_rows().iter().any(|r| {
            matches!(
                r.kind,
                NavRowKind::Shortcut { .. } | NavRowKind::Recent { .. }
            )
        })
    };
    assert!(!in_tree(&s));
    let pinned: Vec<&str> = s.pinned_rows().iter().map(|r| r.key.as_str()).collect();
    assert!(
        pinned.iter().any(|k| k.starts_with("shortcut:"))
            && pinned.iter().any(|k| k.starts_with("recent")),
        "{pinned:?}"
    );
    let mut loose = state(NavigatorSettings {
        show_recent_notes: true,
        ..settings()
    });
    loose.add_shortcut(architect_navigator::Shortcut::note("Gamma/d.md"), None);
    assert!(loose.pinned_rows().is_empty() && in_tree(&loose));
}

/// "Create folder note" (`createFolderNote`): named for its folder, of the
/// settings' type, from the folder note template.
#[test]
fn a_folder_note_is_made_of_its_type_from_its_template() {
    use architect_navigator::settings::FolderNoteType;
    let s = state(NavigatorSettings {
        folder_notes: true,
        folder_note_template: "Templates/Folder.md".into(),
        ..settings()
    });
    let req = s.folder_note_request("Alpha").unwrap();
    assert_eq!(
        (
            req.folder.as_str(),
            req.name.as_deref(),
            req.template.as_deref()
        ),
        ("Alpha", Some("Alpha"), Some("Templates/Folder.md"))
    );
    assert!(s.folder_note_request("").is_err());
    let canvas = state(NavigatorSettings {
        folder_notes: true,
        folder_note_type: FolderNoteType::Canvas,
        ..settings()
    });
    assert_eq!(
        canvas.folder_note_request("Beta").unwrap().kind,
        architect_navigator::state::NewKind::Canvas
    );
    let off = state(settings());
    assert!(off.folder_note_request("Alpha").is_err());
}
