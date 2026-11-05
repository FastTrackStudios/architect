//! The list pane in full (#335), against the model: grouping by date,
//! folder and property (with "None" last), folding groups, the pinned
//! section, compact rows, images and thumbnails, a row's tags, task
//! progress, properties and count, names and dates from frontmatter, a tag
//! clicked, the manual order and its keys, folder notes, and each place's
//! own list — each as Notebook Navigator does it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::grouping;
use architect_navigator::list::{Clock, Thumb, parse_date};
use architect_navigator::{
    Appearance, FileOp, Grouping, Key, ListMode, ListRowKind, MemorySource, Mods,
    NavigatorSettings, NavigatorSource, NavigatorState, NoteFile, OpenIn, Outcome, Pane,
    PropertyValue, SortOption, TextCount, VaultIndex,
};

const DAY: i64 = 86_400_000;
/// 2026-11-05 12:00 UTC.
const NOW: i64 = 1_793_880_000_000;

fn text(v: &str) -> PropertyValue {
    PropertyValue::Text(v.to_owned())
}

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(
            NoteFile::new("Projects/Plan.md", NOW - 5 * DAY, NOW)
                .preview("The plan")
                .property("status", text("doing"))
                .tag("work")
                .tag("projects/alpha")
                .tasks(4, 1)
                .words(1234),
        )
        .with_file(
            NoteFile::new("Projects/Alpha/Spec.md", NOW - 4 * DAY, NOW - DAY)
                .property("status", text("done"))
                .tag("projects/alpha"),
        )
        .with_file(
            NoteFile::new("Projects/Alpha/Notes.md", NOW - 3 * DAY, NOW - 40 * DAY)
                .property("status", text("doing")),
        )
        .with_file(NoteFile::new("Projects/Beta/Todo.md", NOW - 3 * DAY, NOW - 2 * DAY))
        .with_file(NoteFile::new("Projects/Cover.png", NOW - 9 * DAY, NOW - 9 * DAY))
        .with_file(NoteFile::new("Projects/Paper.pdf", NOW - 9 * DAY, NOW - 8 * DAY))
        .with_file(NoteFile::new("Welcome.md", NOW - 9 * DAY, NOW - 2 * DAY).tag("work"))
}

fn state_with(source: &MemorySource, settings: NavigatorSettings) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(source),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 600.0);
    s.set_view(Pane::Files, 2000.0);
    s
}

fn headers(s: &NavigatorState) -> Vec<String> {
    s.list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            ListRowKind::Header { label, .. } => Some(label.clone()),
            _ => None,
        })
        .collect()
}

/// The list as `#Heading` and note names, in order.
fn outline(s: &NavigatorState) -> Vec<String> {
    s.list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            ListRowKind::Header { label, .. } => Some(format!("#{label}")),
            ListRowKind::File { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn below() -> NavigatorSettings {
    NavigatorSettings {
        include_descendants: true,
        ..NavigatorSettings::default()
    }
}

#[test]
fn by_folder_the_selected_folders_own_notes_first_then_each_folder_below_in_tree_order() {
    let src = vault();
    let mut s = state_with(
        &src,
        NavigatorSettings {
            grouping: Grouping::Folder,
            ..below()
        },
    );
    s.click_folder("Projects");
    assert_eq!(
        outline(&s),
        [
            "Projects/Plan.md",
            "Projects/Paper.pdf",
            "Projects/Cover.png",
            "#Alpha",
            "Projects/Alpha/Spec.md",
            "Projects/Alpha/Notes.md",
            "#Beta",
            "Projects/Beta/Todo.md",
        ]
    );
    // A folder put first by hand leads its groups too.
    s.move_child("Projects", "Beta", -1);
    assert_eq!(headers(&s), ["Beta", "Alpha"]);
}

#[test]
fn by_property_one_group_per_value_and_none_last() {
    let src = vault();
    let mut s = state_with(
        &src,
        NavigatorSettings {
            grouping: Grouping::Property,
            group_property: "status".to_owned(),
            ..below()
        },
    );
    s.click_folder("Projects");
    assert_eq!(headers(&s), ["doing", "done", "None"]);
    let o = outline(&s);
    let at = |p: &str| o.iter().position(|x| x == p).unwrap();
    assert!(at("#doing") < at("Projects/Plan.md") && at("Projects/Plan.md") < at("#done"));
    assert!(at("#None") < at("Projects/Beta/Todo.md"));
}

#[test]
fn the_grouping_function_is_bases_shaped() {
    let rows = vec![
        ("a", Some(PropertyValue::Number(10.0))),
        ("b", Some(text("Item 10"))),
        ("c", Some(PropertyValue::List(vec!["x".into(), "[[Notes/Y]]".into()]))),
        ("d", None),
        ("e", Some(text("Item 2"))),
        ("f", Some(PropertyValue::Number(2.0))),
        ("g", Some(PropertyValue::List(vec!["x".into(), "[[Notes/Y]]".into()]))),
        ("h", Some(text("  "))),
    ];
    let groups = grouping::by_value(rows, |r| r.1.as_ref(), false);
    let labels: Vec<_> = groups.iter().map(|g| g.label.as_str()).collect();
    // Numbers first, in order; text in natural order; a list one group,
    // its links shown as names; no value (or blank) last, under None.
    assert_eq!(labels, ["2", "10", "Item 2", "Item 10", "x, Y", "None"]);
    assert_eq!(groups[4].items.iter().map(|r| r.0).collect::<Vec<_>>(), ["c", "g"]);
    assert_eq!(groups[5].id, grouping::NONE_ID);
    assert_eq!(groups[5].items.iter().map(|r| r.0).collect::<Vec<_>>(), ["d", "h"]);
    let down = grouping::by_value(vec![1.0, 3.0, 2.0], |_| None, true);
    assert_eq!(down.len(), 1, "every item without a value: one None group");
}

#[test]
fn a_folded_group_keeps_its_heading_and_count_and_hides_its_notes() {
    let src = vault();
    let mut s = state_with(&src, below());
    s.click_folder("Projects");
    assert_eq!(headers(&s), ["Today", "Yesterday", "Previous 7 days", "Previous 30 days", "September"]);
    let (key, count) = s
        .list_rows()
        .iter()
        .find_map(|r| match &r.kind {
            ListRowKind::Header { label, group, count, .. } if label == "Previous 7 days" => {
                Some((group.clone(), *count))
            }
            _ => None,
        })
        .unwrap();
    assert!(key.starts_with("scope=folder:Projects;group=date;id=date:mtime:"), "{key}");
    assert_eq!(count, 1);
    s.toggle_group(&key);
    let o = outline(&s);
    assert!(o.contains(&"#Previous 7 days".to_owned()));
    assert!(!o.contains(&"Projects/Beta/Todo.md".to_owned()));
    // Kept with the view, per folder.
    let kept = s.view_state();
    assert_eq!(kept.collapsed, [key.clone()]);
    s.toggle_group(&key);
    assert!(outline(&s).contains(&"Projects/Beta/Todo.md".to_owned()));
}

#[test]
fn pinned_notes_lead_the_list_once_with_a_line_of_preview_and_no_date() {
    let src = vault();
    let mut s = state_with(&src, NavigatorSettings::default());
    s.click_folder("Projects");
    s.click_file("Projects/Paper.pdf", Mods::NONE);
    s.key(Key::Pin, Mods::NONE);
    let o = outline(&s);
    assert_eq!(&o[..3], ["#Pinned", "Projects/Paper.pdf", "#Today"]);
    assert_eq!(o.iter().filter(|p| *p == "Projects/Paper.pdf").count(), 1);
    let plan = s
        .list_rows()
        .iter()
        .find(|r| r.file() == Some("Projects/Plan.md"))
        .unwrap()
        .clone();
    s.click_file("Projects/Plan.md", Mods::NONE);
    s.toggle_pin(&["Projects/Plan.md".to_owned()]);
    let pinned = s
        .list_rows()
        .iter()
        .find(|r| r.file() == Some("Projects/Plan.md"))
        .unwrap();
    let (ListRowKind::File { lines: before, .. }, ListRowKind::File { lines: after, pinned: true, .. }) =
        (&plan.kind, &pinned.kind)
    else {
        panic!("{pinned:?}")
    };
    assert_eq!((before.preview, before.date), (2, true));
    assert_eq!((after.preview, after.date), (1, false));
    // Pinned in folders, not in tags.
    s.select_tag("work");
    assert!(!headers(&s).contains(&"Pinned".to_owned()));
    // And it follows a rename.
    src.rename("Projects/Plan.md", "Projects/Plan 2.md");
    for e in [architect_navigator::SourceEvent::FileRenamed {
        from: "Projects/Plan.md".into(),
        file: src.file("Projects/Plan 2.md").unwrap(),
    }] {
        s.apply(&e);
    }
    assert!(s.pinned("Projects/Plan 2.md").folder);
}

#[test]
fn only_pin_notes_in_their_folder() {
    let src = vault();
    let mut s = state_with(
        &src,
        NavigatorSettings {
            filter_pinned_by_folder: true,
            ..below()
        },
    );
    s.click_folder("Projects/Alpha");
    s.click_file("Projects/Alpha/Spec.md", Mods::NONE);
    s.key(Key::Pin, Mods::NONE);
    assert_eq!(headers(&s)[0], "Pinned");
    s.click_folder("Projects");
    assert!(!headers(&s).contains(&"Pinned".to_owned()));
}

#[test]
fn compact_rows_are_a_name_tall() {
    let src = vault();
    let mut s = state_with(&src, NavigatorSettings::default());
    s.click_folder("Projects");
    let tall = s.list_offsets().total();
    s.set_settings(NavigatorSettings {
        mode: ListMode::Compact,
        ..NavigatorSettings::default()
    });
    let rows: Vec<_> = s
        .list_rows()
        .iter()
        .enumerate()
        .filter(|(_, r)| r.file().is_some())
        .map(|(i, _)| s.list_offsets().height(i))
        .collect();
    assert!(rows.iter().all(|h| (*h - 28.0).abs() < 0.01), "{rows:?}");
    assert!(s.list_offsets().total() < tall);
}

#[test]
fn images_a_property_the_file_a_cover_or_a_badge() {
    let src = vault()
        .with_file(NoteFile::new("Art/Logo.svg", NOW, NOW))
        .with_file(NoteFile::new("Art/Sketch.excalidraw.md", NOW, NOW))
        .with_file(NoteFile::new("Art/Sketch.excalidraw.png", NOW, NOW))
        .with_file(NoteFile::new("Art/Board.canvas", NOW, NOW))
        .with_file(NoteFile::new("Art/Plain.excalidraw", NOW, NOW))
        .with_file(
            NoteFile::new("Art/Trip.md", NOW, NOW)
                .property("cover", text("[[Art/Logo.svg]]"))
                .feature_image("https://example.com/x.png"),
        )
        .with_file(NoteFile::new("Art/Old.md", NOW, NOW).feature_image("http://insecure/x.png"));
    let mut s = state_with(
        &src,
        NavigatorSettings {
            feature_image_properties: "cover, image".to_owned(),
            ..NavigatorSettings::default()
        },
    );
    s.click_folder("Art");
    fn thumb_of(s: &NavigatorState, p: &str) -> Option<Option<Thumb>> {
        s.list_rows().iter().find_map(|r| match &r.kind {
            ListRowKind::File { path, thumb, .. } if path == p => Some(thumb.clone()),
            _ => None,
        })
    }
    assert_eq!(thumb_of(&s, "Art/Logo.svg"), Some(Some(Thumb::File("Art/Logo.svg".into()))));
    assert_eq!(thumb_of(&s, "Art/Trip.md"), Some(Some(Thumb::File("Art/Logo.svg".into()))));
    assert_eq!(
        thumb_of(&s, "Art/Sketch.excalidraw.md"),
        Some(Some(Thumb::File("Art/Sketch.excalidraw.png".into())))
    );
    assert_eq!(thumb_of(&s, "Art/Plain.excalidraw"), Some(Some(Thumb::Badge("EXCALIDRAW".into()))));
    assert_eq!(thumb_of(&s, "Art/Board.canvas"), Some(Some(Thumb::Badge("CANVAS".into()))));
    assert_eq!(thumb_of(&s, "Art/Old.md"), Some(None), "http is not shown");
    // The drawing's preview is not listed beside it.
    assert_eq!(thumb_of(&s, "Art/Sketch.excalidraw.png"), None);
    // A row with an image is at least 42 + padding tall.
    s.click_folder("Projects");
    let i = s
        .list_rows()
        .iter()
        .position(|r| r.file() == Some("Projects/Paper.pdf"))
        .unwrap();
    assert!(s.list_offsets().height(i) >= 42.0 + 16.0);
    assert_eq!(thumb_of(&s, "Projects/Paper.pdf"), Some(Some(Thumb::Badge("PDF".into()))));
    // None in compact rows, nor with images off.
    s.set_settings(NavigatorSettings {
        show_feature_image: false,
        ..NavigatorSettings::default()
    });
    assert_eq!(thumb_of(&s, "Projects/Paper.pdf"), Some(None));
}

#[test]
fn a_rows_tags_properties_tasks_and_count() {
    let src = vault();
    let mut s = state_with(
        &src,
        NavigatorSettings {
            list_properties: "status".to_owned(),
            text_count: TextCount::Words,
            ..NavigatorSettings::default()
        },
    );
    s.click_folder("Projects");
    let ListRowKind::File { lines, .. } = &s
        .list_rows()
        .iter()
        .find(|r| r.file() == Some("Projects/Plan.md"))
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert!(lines.tags && lines.properties && lines.tasks && lines.count_in_title);
    let t = s.target();
    let cx = s.list_context(&t);
    let f = s.file("Projects/Plan.md").unwrap();
    assert_eq!(
        architect_navigator::list::shown_tags(f, &cx),
        [("work", "work"), ("projects/alpha", "alpha")]
    );
    assert_eq!(
        architect_navigator::list::shown_properties(f, s.effective()),
        [("status".to_owned(), "doing".to_owned())]
    );
    // Done tasks hide when asked.
    s.set_settings(NavigatorSettings {
        hide_task_progress_complete: true,
        ..NavigatorSettings::default()
    });
    src.put(src.file("Projects/Plan.md").unwrap().tasks(4, 0));
    s.apply(&architect_navigator::SourceEvent::FileModified(src.file("Projects/Plan.md").unwrap()));
    let ListRowKind::File { lines, .. } = &s
        .list_rows()
        .iter()
        .find(|r| r.file() == Some("Projects/Plan.md"))
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert!(!lines.tasks);
}

#[test]
fn a_tag_clicked_lists_its_notes_and_those_below_it() {
    let src = vault();
    let mut s = state_with(&src, NavigatorSettings::default());
    s.select_tag("#projects");
    let mut o = s.list_order();
    o.sort_unstable();
    assert_eq!(o, ["Projects/Alpha/Spec.md", "Projects/Plan.md"]);
    // The tag selected is not drawn on its own notes again.
    let t = s.target();
    let cx = s.list_context(&t);
    let shown = architect_navigator::list::shown_tags(s.file("Projects/Plan.md").unwrap(), &cx);
    assert_eq!(shown.len(), 2, "projects/alpha is below projects, so it shows");
    s.select_tag("projects/alpha");
    let t = s.target();
    let cx = s.list_context(&t);
    let shown = architect_navigator::list::shown_tags(s.file("Projects/Plan.md").unwrap(), &cx);
    assert_eq!(shown, [("work", "work")]);
    // Back goes to the tag before, then the folder.
    s.back();
    assert_eq!(s.selection().target().and_then(|t| t.tag()), Some("projects"));
    s.back();
    assert_eq!(s.selection().folder(), Some(""));
}

#[test]
fn names_and_dates_from_frontmatter() {
    let src = MemorySource::new("V")
        .with_file(
            NoteFile::new("a.md", NOW, NOW)
                .property("name", text("Zed"))
                .property("made", text("2020-01-02T03:04:05Z")),
        )
        .with_file(
            NoteFile::new("b.md", NOW, NOW - DAY)
                .property("made", PropertyValue::Number(1_600_000_000.0)),
        );
    let mut s = state_with(
        &src,
        NavigatorSettings {
            use_frontmatter: true,
            frontmatter_name_field: "title, name".to_owned(),
            frontmatter_created_field: "made".to_owned(),
            sort: SortOption::CreatedAsc,
            ..NavigatorSettings::default()
        },
    );
    s.click_folder("");
    // a: 2020-01-02; b: 1.6e9 seconds, 2020-09-13.
    assert_eq!(s.list_order(), ["a.md", "b.md"]);
    assert_eq!(headers(&s), ["2020"]);
    assert_eq!(s.file("a.md").unwrap().title(&s.effective().name_fields()), "Zed");
    assert_eq!(parse_date("2020-01-02T03:04:05Z", 0), Some(1_577_934_245_000));
    assert_eq!(parse_date("2020-01-02", 3_600_000), Some(1_577_923_200_000 - 3_600_000));
    assert_eq!(parse_date("2020-01-02 03:04+01:00", 0), Some(1_577_930_640_000));
    assert_eq!(parse_date("yesterday", 0), None);
}

#[test]
fn the_manual_order_moves_with_command_arrows_and_writes_ranks() {
    let src = MemorySource::new("V")
        .with_file(NoteFile::new("A.md", NOW, NOW).property("sort_index", PropertyValue::Number(2000.0)))
        .with_file(NoteFile::new("B.md", NOW, NOW).property("sort_index", PropertyValue::Number(1000.0)))
        .with_file(NoteFile::new("C.md", NOW, NOW))
        .with_file(NoteFile::new("D.pdf", NOW, NOW));
    let mut s = state_with(
        &src,
        NavigatorSettings {
            sort: SortOption::Manual,
            ..NavigatorSettings::default()
        },
    );
    s.click_folder("");
    assert_eq!(outline(&s), ["B.md", "A.md", "#Unsorted", "C.md", "D.pdf"]);
    s.click_file("C.md", Mods::NONE);
    s.focus = Pane::Files;
    let Some(Outcome::Write { ops, .. }) = s.key(Key::Up, Mods::COMMAND) else {
        panic!("a manual sort writes");
    };
    assert_eq!(
        ops,
        [
            FileOp::SetProperty { path: "C.md".into(), key: "sort_index".into(), value: PropertyValue::Number(2000.0) },
            FileOp::SetProperty { path: "A.md".into(), key: "sort_index".into(), value: PropertyValue::Number(3000.0) },
        ]
    );
    src.write(&ops).unwrap();
    s.replace_index(VaultIndex::read(&src));
    assert_eq!(outline(&s), ["B.md", "C.md", "A.md", "D.pdf"]);
    // Other sorts don't move notes.
    s.set_settings(NavigatorSettings::default());
    assert_eq!(s.key(Key::Up, Mods::COMMAND).map(|_| ()), Some(()), "an ordinary move");
}

fn notes_on() -> NavigatorSettings {
    NavigatorSettings {
        folder_notes: true,
        ..NavigatorSettings::default()
    }
}

#[test]
fn folder_notes_hide_open_with_their_folder_and_convert_set_and_detach() {
    let src = vault().with_file(NoteFile::new("Projects/Alpha/Alpha.md", NOW, NOW));
    let mut s = state_with(&src, notes_on());
    assert_eq!(s.folder_note("Projects/Alpha"), Some("Projects/Alpha/Alpha.md"));
    let out = s.click_folder("Projects/Alpha");
    assert_eq!(
        out,
        Some(Outcome::Open { path: "Projects/Alpha/Alpha.md".into(), how: OpenIn::Here, settle: false })
    );
    assert!(!s.list_order().contains(&"Projects/Alpha/Alpha.md"), "hidden in its list");
    assert_eq!(
        s.open_folder_note("Projects/Alpha", true),
        Some(Outcome::Open { path: "Projects/Alpha/Alpha.md".into(), how: OpenIn::Tab, settle: false })
    );

    // Convert: a folder named for the note, the note moved in, opened.
    let Ok(Outcome::Write { ops, open }) = s.convert_to_folder_note("Projects/Plan.md") else {
        panic!()
    };
    assert_eq!(
        ops,
        [
            FileOp::CreateFolder("Projects/Plan".into()),
            FileOp::Move { from: "Projects/Plan.md".into(), to: "Projects/Plan/Plan.md".into() },
        ]
    );
    assert_eq!(open, Some(("Projects/Plan/Plan.md".into(), OpenIn::Here)));
    src.write(&ops).unwrap();
    s.replace_index(VaultIndex::read(&src));
    assert_eq!(s.folder_note("Projects/Plan"), Some("Projects/Plan/Plan.md"));
    assert!(s.convert_to_folder_note("Projects/Plan/Plan.md").is_err());

    // Set: renamed in place; refused where the folder has one.
    assert!(s.set_as_folder_note("Projects/Alpha/Spec.md").is_err());
    let Ok(Outcome::Write { ops, .. }) = s.set_as_folder_note("Projects/Beta/Todo.md") else {
        panic!()
    };
    assert_eq!(ops, [FileOp::Move { from: "Projects/Beta/Todo.md".into(), to: "Projects/Beta/Beta.md".into() }]);
    assert!(s.set_as_folder_note("Welcome.md").is_err(), "the root has none");

    // Detach: renamed, a note like any other.
    let Ok(Outcome::Write { ops, .. }) = s.detach_folder_note("Projects/Alpha", None) else {
        panic!()
    };
    assert_eq!(ops, [FileOp::Move { from: "Projects/Alpha/Alpha.md".into(), to: "Projects/Alpha/Alpha (note).md".into() }]);
    src.write(&ops).unwrap();
    s.replace_index(VaultIndex::read(&src));
    assert_eq!(s.folder_note("Projects/Alpha"), None);

    // Off, nothing is a folder note.
    s.set_settings(NavigatorSettings::default());
    assert_eq!(s.folder_note("Projects/Plan"), None);
    assert!(s.convert_to_folder_note("Welcome.md").is_err());
}

#[test]
fn pin_all_folder_notes_and_a_fixed_name() {
    let src = vault()
        .with_file(NoteFile::new("Projects/Alpha/index.md", NOW, NOW))
        .with_file(NoteFile::new("Projects/Beta/index.md", NOW, NOW));
    let mut s = state_with(
        &src,
        NavigatorSettings {
            folder_note_name: "index".to_owned(),
            ..NavigatorSettings { include_descendants: true, ..notes_on() }
        },
    );
    s.click_folder("Projects");
    assert_eq!(s.pin_folder_notes(), 2);
    assert_eq!(&outline(&s)[..3], ["#Pinned", "Projects/Alpha/index.md", "Projects/Beta/index.md"]);
    assert_eq!(s.pin_folder_notes(), 0, "pinned already");
}

#[test]
fn a_folder_or_tag_has_its_own_list() {
    let src = vault();
    let mut s = state_with(&src, NavigatorSettings::default());
    s.click_folder("Projects");
    s.set_appearance(
        &s.target(),
        Appearance {
            sort: Some(SortOption::TitleAsc),
            grouping: Some(Grouping::Folder),
            mode: Some(ListMode::Compact),
            title_rows: Some(2),
            preview_rows: Some(0),
            descendants: Some(true),
            ..Appearance::default()
        },
    );
    assert_eq!(s.effective().sort, SortOption::TitleAsc);
    assert!(s.effective().include_descendants);
    assert_eq!(headers(&s), ["Alpha", "Beta"]);
    // Another folder keeps the settings.
    s.click_folder("");
    assert_eq!(s.effective().sort, SortOption::ModifiedDesc);
    assert_eq!(s.effective().mode, ListMode::Standard);
    // A tag's own.
    s.select_tag("work");
    s.edit_appearance(|a| a.grouping = Some(Grouping::None));
    assert!(headers(&s).is_empty());
    // Kept with the view; a field equal to the settings' is let go.
    let kept = s.view_state();
    assert_eq!(kept.appearances.len(), 2);
    s.edit_appearance(|a| a.grouping = Some(Grouping::Date));
    assert_eq!(s.view_state().appearances.len(), 1);
    let mut t = state_with(&src, NavigatorSettings::default());
    t.restore(&kept);
    t.click_folder("Projects");
    assert_eq!(t.effective().mode, ListMode::Compact);
}
