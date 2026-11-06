//! The vault as the navigator's source: what it lists, what it derives per
//! note, and the events of the app's own changes and of edits outside it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect_navigator::{
    FileOp, NavigatorSource, PropertyValue, SourceEvent, TaskCounts, VaultSource,
};
use vault::actions::OpenVault;

fn source() -> (tempfile::TempDir, VaultSource) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for d in ["Attachments", "Notes/Deep/Deeper", "Empty"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("Attachments/sun.png"), b"\x89PNG\r\n").unwrap();
    std::fs::write(
        root.join("Notes/Plan.md"),
        "---\ntitle: The plan\nfeature: \"[[Attachments/sun.png]]\"\npriority: 2\ndone: false\n\
         tags: [work/plan]\n---\n# Plan\nSee [[Ideas|my ideas]].\n- [ ] One\n- [x] Two\n- [ ] Three\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Notes/Deep/Deeper/Ideas.md"),
        "Short. ![[sun.png]] #idea\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Board.base"),
        "views:\n  - type: table\n    name: All\n",
    )
    .unwrap();
    let s = VaultSource::named(OpenVault::open(root).unwrap(), "Demo");
    (tmp, s)
}

fn heard(
    source: &VaultSource,
) -> (
    Arc<Mutex<Vec<SourceEvent>>>,
    architect_navigator::Subscription,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = Arc::clone(&events);
    let sub = source.subscribe(Box::new(move |ev| e.lock().unwrap().push(ev.clone())));
    (events, sub)
}

#[test]
fn it_lists_folders_files_and_what_each_note_derives() {
    let (_tmp, s) = source();
    assert_eq!(s.name(), "Demo");
    let folders = s.folders();
    for f in [
        "Attachments",
        "Empty",
        "Notes",
        "Notes/Deep",
        "Notes/Deep/Deeper",
    ] {
        assert!(folders.contains(&f.to_owned()), "{f} in {folders:?}");
    }
    let mut paths: Vec<String> = s.files().into_iter().map(|f| f.path).collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "Attachments/sun.png",
            "Board.base",
            "Notes/Deep/Deeper/Ideas.md",
            "Notes/Plan.md"
        ]
    );
    let plan = s.file("Notes/Plan.md").unwrap();
    assert_eq!(plan.title(&["title"]), "The plan");
    assert_eq!(plan.get("priority"), Some(&PropertyValue::Number(2.0)));
    assert_eq!(plan.get("done"), Some(&PropertyValue::Bool(false)));
    assert!(
        plan.tags.contains(&"work/plan".to_owned()),
        "{:?}",
        plan.tags
    );
    assert_eq!(
        plan.preview.as_deref(),
        Some("Plan See my ideas. One Two Three")
    );
    assert_eq!(
        plan.tasks,
        Some(TaskCounts {
            total: 3,
            unfinished: 2
        })
    );
    assert!(plan.words.is_some_and(|w| w > 3), "{:?}", plan.words);
    // Its feature image is a path in the vault; the view draws it from
    // the source's URL for it.
    assert_eq!(plan.feature_image.as_deref(), Some("Attachments/sun.png"));
    let url = s.resource_url("Attachments/sun.png").unwrap();
    assert!(
        url.starts_with("file://") && url.ends_with("Attachments/sun.png"),
        "{url}"
    );
    // By its name, as an embed gives it, too; nothing for what is not there.
    assert_eq!(
        s.resource_url("[[sun.png|alt]]").as_deref(),
        Some(url.as_str())
    );
    assert!(s.resource_url("Attachments/none.png").is_none());
    assert!(s.resource_url("../outside.png").is_none());
    // An embed is a feature image too, found by its name.
    let ideas = s.file("Notes/Deep/Deeper/Ideas.md").unwrap();
    assert_eq!(ideas.feature_image.as_deref(), Some("Attachments/sun.png"));
    assert!(ideas.tags.contains(&"idea".to_owned()));
}

#[test]
fn the_apps_own_changes_are_events() {
    let (tmp, s) = source();
    let (events, _sub) = heard(&s);
    s.save("Notes/Plan.md", "Rewritten.\n").unwrap();
    let path = s.create_note("Notes/New").unwrap();
    assert_eq!(path, "Notes/New/Untitled.md");
    let r = s
        .rename("Notes/Deep/Deeper/Ideas.md", "Notes/Thoughts.md")
        .unwrap();
    assert!(
        r.relinked.is_empty(),
        "named as `Ideas`, now `Thoughts`: rewritten? {r:?}"
    );
    let got = events.lock().unwrap().clone();
    assert!(
        matches!(&got[0], SourceEvent::FileModified(f) if f.path == "Notes/Plan.md"
        && f.preview.as_deref() == Some("Rewritten."))
    );
    assert!(
        got.contains(&SourceEvent::FolderCreated("Notes/New".to_owned())),
        "{got:?}"
    );
    assert!(
        got.iter()
            .any(|e| matches!(e, SourceEvent::FileCreated(f) if f.path == path))
    );
    assert!(got.iter().any(|e| matches!(e,
        SourceEvent::FileRenamed { from, file } if from == "Notes/Deep/Deeper/Ideas.md" && file.path == "Notes/Thoughts.md")));
    assert!(tmp.path().join("Notes/Thoughts.md").exists());
}

#[test]
fn a_rename_rewrites_backlinks_and_says_the_linking_note_changed() {
    let (tmp, _first) = source();
    std::fs::write(tmp.path().join("Notes/Plan.md"), "See [[Ideas]].\n").unwrap();
    let s = VaultSource::new(OpenVault::open(tmp.path()).unwrap());
    let (events, _sub) = heard(&s);
    let r = s
        .rename(
            "Notes/Deep/Deeper/Ideas.md",
            "Notes/Deep/Deeper/Brainstorm.md",
        )
        .unwrap();
    assert_eq!(r.relinked, ["Notes/Plan.md"]);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("Notes/Plan.md")).unwrap(),
        "See [[Brainstorm]].\n"
    );
    let got = events.lock().unwrap().clone();
    assert!(
        got.iter()
            .any(|e| matches!(e, SourceEvent::FileModified(f) if f.path == "Notes/Plan.md"))
    );
}

#[test]
fn an_edit_outside_the_app_is_heard_live() {
    let (tmp, s) = source();
    s.watch().unwrap();
    let (events, _sub) = heard(&s);
    std::fs::write(tmp.path().join("Notes/Plan.md"), "Pulled from git.\n").unwrap();
    std::fs::write(tmp.path().join("Outside.md"), "Made elsewhere.\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let got = events.lock().unwrap().clone();
        let modified = got.iter().any(|e| {
            matches!(e, SourceEvent::FileModified(f)
            if f.path == "Notes/Plan.md" && f.preview.as_deref() == Some("Pulled from git."))
        });
        let created = got
            .iter()
            .any(|e| matches!(e, SourceEvent::FileCreated(f) if f.path == "Outside.md"));
        if modified && created {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "heard {got:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_navigators_writes_land_on_disk_and_are_events() {
    let (tmp, s) = source();
    let (events, _sub) = heard(&s);
    s.write(&[
        FileOp::CreateFolder("Areas/Music".to_owned()),
        FileOp::Move {
            from: "Notes/Plan.md".to_owned(),
            to: "Areas/Music/Plan.md".to_owned(),
        },
        FileOp::Move {
            from: "Attachments/sun.png".to_owned(),
            to: "Areas/sun.png".to_owned(),
        },
        FileOp::SetProperty {
            path: "Areas/Music/Plan.md".to_owned(),
            key: "order".to_owned(),
            value: PropertyValue::Number(3.0),
        },
    ])
    .unwrap();
    let root = tmp.path();
    assert!(root.join("Areas/Music").is_dir());
    assert!(root.join("Areas/Music/Plan.md").is_file());
    assert!(root.join("Areas/sun.png").is_file() && !root.join("Attachments/sun.png").exists());
    let plan = std::fs::read_to_string(root.join("Areas/Music/Plan.md")).unwrap();
    assert!(plan.contains("order: 3"), "{plan}");
    assert!(plan.contains("title: The plan"), "the rest kept: {plan}");
    let got = events.lock().unwrap().clone();
    assert!(
        got.contains(&SourceEvent::FolderCreated("Areas/Music".to_owned())),
        "{got:?}"
    );
    assert!(got.iter().any(|e| matches!(e,
        SourceEvent::FileRenamed { from, file } if from == "Notes/Plan.md" && file.path == "Areas/Music/Plan.md")));
    assert!(got.iter().any(|e| matches!(e,
        SourceEvent::FileRenamed { from, file } if from == "Attachments/sun.png" && file.path == "Areas/sun.png")));
    assert!(
        got.iter().any(|e| matches!(e, SourceEvent::FileModified(f)
        if f.path == "Areas/Music/Plan.md" && f.get("order") == Some(&PropertyValue::Number(3.0))))
    );
    // A folder that is there, a move onto a file: refused.
    assert!(
        s.write(&[FileOp::CreateFolder("Areas".to_owned())])
            .is_err()
    );
    assert!(
        s.write(&[FileOp::Move {
            from: "Board.base".to_owned(),
            to: "Areas/sun.png".to_owned(),
        }])
        .is_err()
    );
}

#[test]
fn a_periodic_note_the_calendar_makes_lands_on_disk_with_its_folders() {
    use architect_navigator::list::Clock;
    use architect_navigator::{
        NavigatorSettings, NavigatorState, OpenIn, Outcome, PeriodKind, VaultIndex,
    };
    let (tmp, s) = source();
    let (events, _sub) = heard(&s);
    let state = NavigatorState::new(
        VaultIndex::read(&s),
        NavigatorSettings {
            periodic_folder: "Journal".to_owned(),
            daily_pattern: "[Daily]/YYYY-MM-DD".to_owned(),
            ..NavigatorSettings::default()
        },
        Clock::default(),
    );
    let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let Outcome::Write { ops, .. } = state
        .open_periodic(PeriodKind::Day, day, OpenIn::Here)
        .unwrap()
    else {
        panic!("a new note")
    };
    s.write(&ops).unwrap();
    let text = std::fs::read_to_string(tmp.path().join("Journal/Daily/2026-11-06.md")).unwrap();
    assert_eq!(text, "# 2026-11-06\n");
    assert!(s.folders().contains(&"Journal/Daily".to_owned()));
    let events = events.lock().unwrap().clone();
    assert!(events.iter().any(
        |e| matches!(e, SourceEvent::FileCreated(f) if f.path == "Journal/Daily/2026-11-06.md")
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SourceEvent::FolderCreated(f) if f == "Journal/Daily"))
    );
    // A second time, it is there already.
    assert!(s.write(&ops).is_err());
}

#[test]
fn tag_writes_change_the_markdown_and_say_which_notes_changed() {
    let (tmp, s) = source();
    let (events, _sub) = heard(&s);
    let read = |p: &str| std::fs::read_to_string(tmp.path().join(p)).unwrap();
    // A tag renamed, with the tags below it: frontmatter and inline.
    s.write(&[FileOp::AddTag {
        paths: vec!["Notes/Deep/Deeper/Ideas.md".to_owned()],
        tag: "work/ideas".to_owned(),
    }])
    .unwrap();
    assert!(read("Notes/Deep/Deeper/Ideas.md").contains("- work/ideas"));
    events.lock().unwrap().clear();
    s.write(&[FileOp::RenameTag {
        from: "work".to_owned(),
        to: "job".to_owned(),
    }])
    .unwrap();
    let plan = read("Notes/Plan.md");
    assert!(
        plan.contains("job/plan") && !plan.contains("work/plan"),
        "{plan}"
    );
    assert!(read("Notes/Deep/Deeper/Ideas.md").contains("job/ideas"));
    let modified: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            SourceEvent::FileModified(f) => Some(f.path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(modified, ["Notes/Deep/Deeper/Ideas.md", "Notes/Plan.md"]);
    assert_eq!(s.file("Notes/Plan.md").unwrap().tags, ["job/plan"]);
    // Removed, cleared, deleted.
    s.write(&[FileOp::RemoveTag {
        paths: vec!["Notes/Deep/Deeper/Ideas.md".to_owned()],
        tag: "IDEA".to_owned(),
    }])
    .unwrap();
    assert_eq!(
        read("Notes/Deep/Deeper/Ideas.md").matches("#idea").count(),
        0
    );
    s.write(&[FileOp::DeleteTag("job".to_owned())]).unwrap();
    assert!(s.file("Notes/Plan.md").unwrap().tags.is_empty());
    assert!(
        !read("Notes/Plan.md").contains("tags:"),
        "{}",
        read("Notes/Plan.md")
    );
    // A property's key alone is written empty (`null`).
    s.write(&[FileOp::SetProperty {
        path: "Notes/Plan.md".to_owned(),
        key: "status".to_owned(),
        value: PropertyValue::Text(String::new()),
    }])
    .unwrap();
    assert!(
        read("Notes/Plan.md").contains("status: null"),
        "{}",
        read("Notes/Plan.md")
    );
}
