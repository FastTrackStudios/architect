//! The tag tree, the property browser, their operations and drops, and
//! the search a ⌘/Ctrl-click builds, each as Notebook Navigator does it
//! (#337), over a vault in memory.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_navigator::list::Clock;
use architect_navigator::state::Drag;
use architect_navigator::tags::{TAGGED, UNTAGGED};
use architect_navigator::{
    FileOp, Join, Key, MemorySource, Mods, NavRowKind, NavTarget, NavigatorSettings,
    NavigatorSource, NavigatorState, NoteFile, Operand, Outcome, Pane, PropertyValue, Query,
    TreeSort, VaultIndex,
};

const NOW: i64 = 1_793_880_000_000;

fn text(t: &str) -> PropertyValue {
    PropertyValue::Text(t.to_owned())
}

fn vault() -> MemorySource {
    MemorySource::new("Lab")
        .with_file(
            NoteFile::new("Work/Plan.md", NOW, NOW)
                .tag("work")
                .tag("Project/Alpha")
                .property("status", text("doing"))
                .property("type", text("plan")),
        )
        .with_file(
            NoteFile::new("Work/Spec.md", NOW, NOW)
                .tag("project/alpha/spec")
                .tag("project/beta")
                .property("status", text("done")),
        )
        .with_file(
            NoteFile::new("Home/Garden.md", NOW, NOW)
                .tag("home")
                .property("status", text("[[Notes/Doing|doing]]"))
                .property(
                    "type",
                    PropertyValue::List(vec!["plan".into(), "list".into()]),
                ),
        )
        .with_file(NoteFile::new("Home/Loose.md", NOW, NOW).property("status", text("")))
        .with_file(NoteFile::new("Home/Photo.png", NOW, NOW))
}

fn settings() -> NavigatorSettings {
    NavigatorSettings {
        property_keys: "Status, type, missing".to_owned(),
        ..NavigatorSettings::default()
    }
}

fn state(source: &MemorySource, settings: NavigatorSettings) -> NavigatorState {
    let mut s = NavigatorState::new(
        VaultIndex::read(source),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 600.0);
    s
}

/// The tag rows: (name, level, count shown as own/below).
fn tag_rows(s: &NavigatorState) -> Vec<(String, u16, usize, usize)> {
    s.nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Tag { name, count, .. } => {
                Some((name.clone(), r.level, count.current, count.descendants))
            }
            _ => None,
        })
        .collect()
}

fn property_rows(s: &NavigatorState) -> Vec<(String, u16, usize)> {
    s.nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Property { name, count, .. } => {
                Some((name.clone(), r.level, count.total()))
            }
            _ => None,
        })
        .collect()
}

fn files(s: &NavigatorState) -> Vec<String> {
    s.list_order().into_iter().map(str::to_owned).collect()
}

/// Writes `out` to `source` and takes in what it said, as the view does.
fn apply(s: &mut NavigatorState, source: &MemorySource, out: Outcome) {
    let Outcome::Write { ops, .. } = out else {
        panic!("not a write: {out:?}");
    };
    source.write(&ops).unwrap();
    s.replace_index(VaultIndex::read(source));
    s.after_write(&ops);
}

fn names(rows: &[(String, u16, usize, usize)]) -> Vec<&str> {
    rows.iter().map(|r| r.0.as_str()).collect()
}

#[test]
fn the_tag_tree_nests_counts_once_and_lists_untagged_last() {
    let src = vault();
    let mut s = state(&src, settings());
    // The "Tags" row (every tagged note, counted with notes from below
    // only), roots A to Z, the first spelling seen, Untagged last.
    assert_eq!(
        tag_rows(&s),
        [
            ("Tags".to_owned(), 0, 0, 0),
            ("home".to_owned(), 1, 1, 0),
            ("Project".to_owned(), 1, 0, 0),
            ("work".to_owned(), 1, 1, 0),
            ("Untagged".to_owned(), 1, 1, 0),
        ]
    );
    // Open: alpha's own note and the one below it; beta beside it.
    s.toggle_nav(&NavTarget::Tag("project".to_owned()));
    s.toggle_nav(&NavTarget::Tag("Project/Alpha".to_owned()));
    let rows = tag_rows(&s);
    assert_eq!(
        names(&rows),
        [
            "Tags", "home", "Project", "Alpha", "spec", "beta", "work", "Untagged"
        ]
    );
    // With notes from below, a parent counts each note below it once.
    let mut below = settings();
    below.include_descendants = true;
    s.set_settings(below);
    let rows = tag_rows(&s);
    assert_eq!(rows[0], ("Tags".to_owned(), 0, 3, 0), "every tagged note");
    assert_eq!(rows[2], ("Project".to_owned(), 1, 0, 2));
    assert_eq!(rows[3], ("Alpha".to_owned(), 2, 1, 1));
}

#[test]
fn roots_sort_by_frequency_and_keep_a_hand_made_order() {
    let src = vault()
        .with_file(NoteFile::new("Home/More.md", NOW, NOW).tag("home"))
        .with_file(NoteFile::new("Home/Most.md", NOW, NOW).tag("home"));
    let mut cfg = settings();
    cfg.tag_sort = TreeSort::FrequencyDesc;
    cfg.include_descendants = true;
    let mut s = state(&src, cfg);
    assert_eq!(
        names(&tag_rows(&s)),
        ["Tags", "home", "Project", "work", "Untagged"],
        "3 notes, 2, 1"
    );
    // Moved by hand: the order holds, the rest after it.
    s.move_tag_root("work", -2);
    assert_eq!(
        names(&tag_rows(&s)),
        ["Tags", "work", "home", "Project", "Untagged"]
    );
    s.move_tag_root(UNTAGGED, -10);
    assert_eq!(names(&tag_rows(&s))[1], "Untagged");
    let kept = s.view_state();
    let mut again = state(&src, settings());
    again.restore(&kept);
    assert_eq!(names(&tag_rows(&again))[1..3], ["Untagged", "work"]);
}

#[test]
fn the_tags_can_be_limited_to_the_folder_selected() {
    let src = vault();
    let mut cfg = settings();
    cfg.scope_tags = true;
    cfg.all_tags_row = false;
    let mut s = state(&src, cfg);
    s.click_folder("Home");
    assert_eq!(names(&tag_rows(&s)), ["home", "Untagged"]);
    s.click_folder("Work");
    assert_eq!(names(&tag_rows(&s)), ["Project", "work"]);
    // A tag selected does not scope them: every tag shows.
    s.click_tag("work", Mods::NONE);
    assert_eq!(
        names(&tag_rows(&s)),
        ["home", "Project", "work", "Untagged"]
    );
}

#[test]
fn a_tag_row_lists_its_notes_and_those_below_and_the_virtual_rows_theirs() {
    let src = vault();
    let mut s = state(&src, settings());
    s.click_tag("project/alpha", Mods::NONE);
    assert_eq!(files(&s).len(), 2);
    assert_eq!(s.focus, Pane::Navigation);
    s.click_tag(TAGGED, Mods::NONE);
    assert_eq!(files(&s).len(), 3);
    s.click_tag(UNTAGGED, Mods::NONE);
    assert_eq!(files(&s), ["Home/Loose.md"], "notes alone, not the image");
    // The keys go down into the tags, and → opens one.
    s.click_tag("project", Mods::NONE);
    s.focus = Pane::Navigation;
    s.key(Key::Right, Mods::NONE);
    assert!(names(&tag_rows(&s)).contains(&"Alpha"));
    s.key(Key::Down, Mods::NONE);
    assert_eq!(s.target(), NavTarget::Tag("Project/Alpha".to_owned()));
    s.key(Key::Left, Mods::NONE);
    assert_eq!(s.target(), NavTarget::Tag("Project".to_owned()));
    s.key(Key::Left, Mods::NONE);
    assert!(!names(&tag_rows(&s)).contains(&"Alpha"), "closed");
}

#[test]
fn tags_are_added_removed_and_cleared_on_the_notes_selected() {
    let src = vault();
    let mut s = state(&src, settings());
    let plan = vec!["Work/Plan.md".to_owned()];
    let spec = vec!["Work/Spec.md".to_owned()];
    // Its tag below goes; one above it leaves the note alone.
    let out = s.add_tag(&spec, "project/alpha").unwrap();
    apply(&mut s, &src, out);
    assert_eq!(
        s.file("Work/Spec.md").unwrap().tags,
        ["project/beta", "project/alpha"]
    );
    let out = s.add_tag(&plan, "project/alpha/deep").unwrap();
    apply(&mut s, &src, out);
    assert_eq!(
        s.file("Work/Plan.md").unwrap().tags,
        ["work", "Project/Alpha"]
    );
    // Removed: the tag itself, any case.
    let out = s.remove_tag(&plan, "PROJECT/alpha").unwrap();
    apply(&mut s, &src, out);
    assert_eq!(s.file("Work/Plan.md").unwrap().tags, ["work"]);
    let out = s.clear_tags(&plan).unwrap();
    apply(&mut s, &src, out);
    assert!(s.file("Work/Plan.md").unwrap().tags.is_empty());
    // Only notes; a name is checked.
    assert!(s.add_tag(&["Home/Photo.png".to_owned()], "x").is_err());
    assert!(s.add_tag(&plan, "two words").is_err());
    assert!(s.add_tag(&[], "x").is_err());
}

#[test]
fn a_rename_moves_the_tag_below_and_its_colour_order_and_selection() {
    let src = vault();
    let mut s = state(&src, settings());
    s.set_colour("tag:project", Some("#e11d48"));
    s.toggle_nav(&NavTarget::Tag("project".to_owned()));
    s.move_tag_root("project", 5);
    s.click_tag("project/alpha", Mods::NONE);
    assert!(
        s.rename_tag("project", "project/inner").is_err(),
        "inside itself"
    );
    assert!(s.rename_tag("project", "project").is_err(), "the same name");
    assert!(s.rename_tag(TAGGED, "x").is_err());
    let out = s.rename_tag("project", "Code").unwrap();
    apply(&mut s, &src, out);
    assert_eq!(
        s.file("Work/Spec.md").unwrap().tags,
        ["Code/alpha/spec", "Code/beta"]
    );
    assert_eq!(s.target(), NavTarget::Tag("Code/Alpha".to_owned()));
    assert_eq!(files(&s).len(), 2);
    // Its colour, below too (inherited), and its place.
    assert_eq!(s.colour("tag:code/alpha"), Some("#e11d48"));
    assert_eq!(s.colour("tag:project"), None);
    // Still open, and where it was put: after Untagged.
    let rows = tag_rows(&s);
    assert_eq!(
        names(&rows)[3..],
        ["Untagged", "Code", "Alpha", "beta"],
        "{rows:?}"
    );
    // Deleted: the tag and those below, its colour gone, the selection back
    // at the root.
    let out = s.delete_tag("code").unwrap();
    apply(&mut s, &src, out);
    assert!(s.file("Work/Spec.md").unwrap().tags.is_empty());
    assert_eq!(s.colour("tag:code/alpha"), None);
    assert_eq!(s.target(), NavTarget::Folder(String::new()));
}

#[test]
fn a_new_note_in_a_tag_is_tagged_beside_the_note_selected() {
    let src = vault();
    let mut s = state(&src, settings());
    s.click_folder("Work");
    s.click_file("Work/Plan.md", Mods::NONE);
    let out = s.new_note_in_tag("project/alpha").unwrap();
    let Outcome::Write { ops, open } = &out else {
        panic!()
    };
    assert_eq!(
        ops,
        &[FileOp::CreateFile {
            path: "Work/Untitled.md".to_owned(),
            text: "---\ntags:\n- Project/Alpha\n---\n".to_owned(),
        }]
    );
    assert_eq!(open.as_ref().unwrap().0, "Work/Untitled.md");
    apply(&mut s, &src, out);
    assert_eq!(s.file("Work/Untitled.md").unwrap().tags, ["Project/Alpha"]);
    assert_eq!(s.untitled("Work"), "Work/Untitled 1.md");
    assert!(s.new_note_in_tag(UNTAGGED).is_err());
}

#[test]
fn drops_tag_notes_clear_them_and_move_tags() {
    let src = vault();
    let s = state(&src, settings());
    let notes = Drag::Notes(vec!["Home/Loose.md".to_owned()]);
    let tag = |t: &str| NavTarget::Tag(t.to_owned());
    let op = |out: Outcome| match out {
        Outcome::Write { ops, .. } => ops,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        op(s.drop_on(&notes, &tag("home")).unwrap()),
        [FileOp::AddTag {
            paths: vec!["Home/Loose.md".to_owned()],
            tag: "home".to_owned()
        }]
    );
    assert_eq!(
        op(s.drop_on(&notes, &tag(UNTAGGED)).unwrap()),
        [FileOp::ClearTags {
            paths: vec!["Home/Loose.md".to_owned()]
        }]
    );
    assert!(s.drop_on(&notes, &tag(TAGGED)).is_err());
    assert!(
        s.drop_on(
            &Drag::Notes(vec!["Home/Photo.png".to_owned()]),
            &tag("home")
        )
        .is_err()
    );
    // A tag on a tag goes under it; on the "Tags" row, to the top.
    let alpha = Drag::Tag("Project/Alpha".to_owned());
    assert_eq!(
        op(s.drop_on(&alpha, &tag("home")).unwrap()),
        [FileOp::RenameTag {
            from: "Project/Alpha".to_owned(),
            to: "home/Alpha".to_owned()
        }]
    );
    assert_eq!(
        op(s.drop_on(&alpha, &tag(TAGGED)).unwrap()),
        [FileOp::RenameTag {
            from: "Project/Alpha".to_owned(),
            to: "Alpha".to_owned()
        }]
    );
    assert!(s.drop_on(&alpha, &tag("project/alpha/spec")).is_err());
    assert!(s.drop_on(&alpha, &tag("project")).is_err(), "there already");
    assert!(
        s.drop_on(&Drag::Tag("work".to_owned()), &tag(TAGGED))
            .is_err()
    );
    assert!(s.drop_on(&alpha, &tag(UNTAGGED)).is_err());
}

#[test]
fn the_property_browser_shows_the_keys_named_their_values_and_counts() {
    let src = vault();
    let mut s = state(&src, settings());
    // Closed keys at first; a key no note has shows, empty.
    assert_eq!(
        property_rows(&s),
        [
            ("Properties".to_owned(), 0, 0),
            ("missing".to_owned(), 1, 0),
            ("Status".to_owned(), 1, 1),
            ("type".to_owned(), 1, 0),
        ]
    );
    s.toggle_nav(&NavTarget::Property {
        key: "status".to_owned(),
        value: None,
    });
    s.toggle_nav(&NavTarget::Property {
        key: "type".to_owned(),
        value: None,
    });
    // A link's value as its alias, one value with the plain text; a list's
    // items each.
    let rows = property_rows(&s);
    assert_eq!(
        rows[3..6],
        [
            ("doing".to_owned(), 2, 2),
            ("done".to_owned(), 2, 1),
            ("type".to_owned(), 1, 0),
        ]
    );
    assert_eq!(
        rows[6..],
        [("list".to_owned(), 2, 1), ("plan".to_owned(), 2, 2)]
    );
    // A value lists its notes; a key with notes from below off, those with
    // it empty; on, all with it.
    s.click_property("status", Some("doing"), Mods::NONE);
    assert_eq!(files(&s).len(), 2);
    s.click_property("status", None, Mods::NONE);
    assert_eq!(files(&s), ["Home/Loose.md"]);
    let mut below = settings();
    below.include_descendants = true;
    s.set_settings(below);
    assert_eq!(files(&s).len(), 4);
    s.click_property(architect_navigator::properties::ROOT, None, Mods::NONE);
    assert_eq!(files(&s).len(), 4);
    // Most notes first.
    let mut freq = settings();
    freq.property_sort = TreeSort::FrequencyDesc;
    freq.include_descendants = true;
    s.set_settings(freq);
    let keys: Vec<String> = property_rows(&s)
        .into_iter()
        .filter(|r| r.1 == 1)
        .map(|r| r.0)
        .collect();
    assert_eq!(keys, ["Status", "type", "missing"]);
}

#[test]
fn a_property_is_set_on_notes_dropped_or_selected() {
    let src = vault();
    let mut s = state(&src, settings());
    let loose = vec!["Home/Loose.md".to_owned(), "Work/Plan.md".to_owned()];
    // A value written as first seen (the link kept); a note that has it
    // already left.
    let out = s
        .drop_on(
            &Drag::Notes(loose.clone()),
            &NavTarget::Property {
                key: "status".to_owned(),
                value: Some("doing".to_owned()),
            },
        )
        .unwrap();
    let Outcome::Write { ops, .. } = &out else {
        panic!()
    };
    assert_eq!(
        ops,
        &[FileOp::SetProperty {
            path: "Home/Loose.md".to_owned(),
            key: "status".to_owned(),
            value: text("[[Notes/Doing|doing]]"),
        }]
    );
    apply(&mut s, &src, out);
    // The key alone: empty; a list becomes the one value.
    let out = s
        .set_property(&["Home/Garden.md".to_owned()], "type", Some("list"))
        .unwrap();
    apply(&mut s, &src, out);
    assert_eq!(
        s.file("Home/Garden.md").unwrap().get("type"),
        Some(&text("list"))
    );
    assert!(s.set_property(&loose, "status", Some("doing")).is_err());
    let out = s
        .set_property(&["Work/Plan.md".to_owned()], "Status", None)
        .unwrap();
    apply(&mut s, &src, out);
    assert_eq!(
        s.file("Work/Plan.md").unwrap().get("status"),
        Some(&text(""))
    );
}

#[test]
fn command_click_builds_the_search_and_the_list_follows_it() {
    let src = vault();
    let mut s = state(&src, settings());
    s.click_tag(TAGGED, Mods::NONE);
    assert_eq!(files(&s).len(), 3);
    s.click_tag("project", Mods::COMMAND);
    assert_eq!(s.query().text(), "#project");
    assert_eq!(
        s.target(),
        NavTarget::Tag(TAGGED.to_owned()),
        "not selected"
    );
    assert_eq!(files(&s).len(), 2);
    s.click_property("status", Some("done"), Mods::COMMAND);
    assert_eq!(s.query().text(), "#project AND .status=done");
    assert_eq!(files(&s), ["Work/Spec.md"]);
    let or = Mods {
        shift: true,
        command: true,
    };
    s.click_tag("home", or);
    assert_eq!(s.query().text(), "#project AND .status=done OR #home");
    assert_eq!(files(&s).len(), 2);
    // Again: out, with its AND.
    s.click_property("status", Some("done"), Mods::COMMAND);
    assert_eq!(s.query().text(), "#project OR #home");
    assert_eq!(files(&s).len(), 3);
    // The "Tags" row is not one to search by.
    s.click_tag(UNTAGGED, Mods::COMMAND);
    assert_eq!(s.target(), NavTarget::Tag(UNTAGGED.to_owned()));
    assert!(
        files(&s).is_empty(),
        "no untagged note is #project or #home"
    );
    s.set_query(&Query::default());
    assert_eq!(files(&s), ["Home/Loose.md"]);
}

#[test]
fn the_query_reads_back_quotes_and_binds_and_tighter() {
    let mut q = Query::default();
    q.toggle(Operand::property("Due date", Some("a=b \"c\"")), Join::And);
    q.toggle(Operand::tag("#A/B"), Join::Or);
    let t = q.text();
    assert_eq!(t, r#"."due date"="a\=b \"c\"" OR #a/b"#);
    assert_eq!(Query::parse(&t), Some(q));
    assert_eq!(Query::parse("#a plain words"), None, "the filter search's");
    // a OR b AND c: a, or both b and c.
    let q = Query::parse("#a OR #b AND #c").unwrap();
    let note = |tags: &[&str]| {
        let mut f = NoteFile::new("x.md", 0, 0);
        f.tags = tags.iter().map(|t| (*t).to_owned()).collect();
        f
    };
    assert!(q.matches(&note(&["a"])));
    assert!(!q.matches(&note(&["b"])));
    assert!(q.matches(&note(&["b", "c/d"])), "a tag below matches");
    // `.key` is any key it starts; `.key=value` a value holding it.
    let mut f = NoteFile::new("x.md", 0, 0);
    f.properties.push(("Status".into(), text("In progress")));
    assert!(Query::parse(".stat").unwrap().matches(&f));
    assert!(Query::parse(".status=progress").unwrap().matches(&f));
    assert!(!Query::parse(".status=done").unwrap().matches(&f));
}
