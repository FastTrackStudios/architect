//! The vault as the navigator's source: what it lists, what it derives per
//! note, and the events of the app's own changes and of edits outside it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect_navigator::{NavigatorSource, PropertyValue, SourceEvent, TaskCounts, VaultSource};
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
    std::fs::write(root.join("Notes/Deep/Deeper/Ideas.md"), "Short. ![[sun.png]] #idea\n").unwrap();
    std::fs::write(root.join("Board.base"), "views:\n  - type: table\n    name: All\n").unwrap();
    let s = VaultSource::named(OpenVault::open(root).unwrap(), "Demo");
    (tmp, s)
}

fn heard(source: &VaultSource) -> (Arc<Mutex<Vec<SourceEvent>>>, architect_navigator::Subscription) {
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
    for f in ["Attachments", "Empty", "Notes", "Notes/Deep", "Notes/Deep/Deeper"] {
        assert!(folders.contains(&f.to_owned()), "{f} in {folders:?}");
    }
    let mut paths: Vec<String> = s.files().into_iter().map(|f| f.path).collect();
    paths.sort();
    assert_eq!(
        paths,
        ["Attachments/sun.png", "Board.base", "Notes/Deep/Deeper/Ideas.md", "Notes/Plan.md"]
    );
    let plan = s.file("Notes/Plan.md").unwrap();
    assert_eq!(plan.title(true), "The plan");
    assert_eq!(plan.get("priority"), Some(&PropertyValue::Number(2.0)));
    assert_eq!(plan.get("done"), Some(&PropertyValue::Bool(false)));
    assert!(plan.tags.contains(&"work/plan".to_owned()), "{:?}", plan.tags);
    assert_eq!(plan.preview.as_deref(), Some("Plan See my ideas. One Two Three"));
    assert_eq!(plan.tasks, Some(TaskCounts { total: 3, unfinished: 2 }));
    let image = plan.feature_image.unwrap();
    assert!(image.starts_with("file://") && image.ends_with("Attachments/sun.png"), "{image}");
    // An embed is a feature image too, found by its name.
    let ideas = s.file("Notes/Deep/Deeper/Ideas.md").unwrap();
    assert!(ideas.feature_image.unwrap().ends_with("Attachments/sun.png"));
    assert!(ideas.tags.contains(&"idea".to_owned()));
}

#[test]
fn the_apps_own_changes_are_events() {
    let (tmp, s) = source();
    let (events, _sub) = heard(&s);
    s.save("Notes/Plan.md", "Rewritten.\n").unwrap();
    let path = s.create_note("Notes/New").unwrap();
    assert_eq!(path, "Notes/New/Untitled.md");
    let r = s.rename("Notes/Deep/Deeper/Ideas.md", "Notes/Thoughts.md").unwrap();
    assert!(r.relinked.is_empty(), "named as `Ideas`, now `Thoughts`: rewritten? {r:?}");
    let got = events.lock().unwrap().clone();
    assert!(matches!(&got[0], SourceEvent::FileModified(f) if f.path == "Notes/Plan.md"
        && f.preview.as_deref() == Some("Rewritten.")));
    assert!(got.contains(&SourceEvent::FolderCreated("Notes/New".to_owned())), "{got:?}");
    assert!(got.iter().any(|e| matches!(e, SourceEvent::FileCreated(f) if f.path == path)));
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
    let r = s.rename("Notes/Deep/Deeper/Ideas.md", "Notes/Deep/Deeper/Brainstorm.md").unwrap();
    assert_eq!(r.relinked, ["Notes/Plan.md"]);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("Notes/Plan.md")).unwrap(),
        "See [[Brainstorm]].\n"
    );
    let got = events.lock().unwrap().clone();
    assert!(got.iter().any(|e| matches!(e, SourceEvent::FileModified(f) if f.path == "Notes/Plan.md")));
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
        let modified = got.iter().any(|e| matches!(e, SourceEvent::FileModified(f)
            if f.path == "Notes/Plan.md" && f.preview.as_deref() == Some("Pulled from git.")));
        let created = got.iter().any(|e| matches!(e, SourceEvent::FileCreated(f) if f.path == "Outside.md"));
        if modified && created {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "heard {got:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
