//! Editing an open vault as an app does: saving a note, making one,
//! renaming and moving one (its links follow), and hearing outside edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use vault::actions::OpenVault;

fn vault() -> (tempfile::TempDir, OpenVault) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("Notes")).unwrap();
    std::fs::create_dir_all(root.join("Empty")).unwrap();
    std::fs::write(root.join("Ionian.md"), "# Ionian\nThe major scale.\n").unwrap();
    std::fs::write(
        root.join("Notes/Modes.md"),
        "See [[Ionian]], [[Ionian#Steps|the steps]] and ![[Ionian]].\n```\n[[Ionian]]\n```\n",
    )
    .unwrap();
    std::fs::write(root.join("Notes/Other.md"), "Links to [[Dorian]].\n").unwrap();
    let open = OpenVault::open(root).unwrap();
    (tmp, open)
}

#[test]
fn a_save_writes_the_file_and_keeps_its_frontmatter() {
    let (tmp, v) = vault();
    let text = "---\ntags: [music/theory]\nlevel: 3\n---\nThe major scale, edited.\n";
    v.save("Ionian.md", text).unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("Ionian.md")).unwrap(),
        text
    );
    assert_eq!(v.text("Ionian.md").as_deref(), Some(text));
    // The app's own write is not an outside change.
    assert!(v.refresh().unwrap().is_empty());
}

#[test]
fn a_new_note_takes_the_next_untitled_name() {
    let (tmp, v) = vault();
    let first = v.untitled("Notes");
    assert_eq!(first, "Notes/Untitled.md");
    v.create(&first, "").unwrap();
    assert!(tmp.path().join("Notes/Untitled.md").exists());
    assert_eq!(v.untitled("Notes"), "Notes/Untitled 1.md");
    assert!(v.create("Notes/Untitled.md", "").is_err(), "taken");
    assert!(v.create("../out.md", "").is_err(), "out of the vault");
}

#[test]
fn a_rename_and_a_move_rewrite_the_links_that_named_the_note() {
    let (tmp, v) = vault();
    let r = v.rename("Ionian.md", "Major.md").unwrap();
    assert_eq!(r.relinked, ["Notes/Modes.md"]);
    let modes = std::fs::read_to_string(tmp.path().join("Notes/Modes.md")).unwrap();
    assert_eq!(
        modes, "See [[Major]], [[Major#Steps|the steps]] and ![[Major]].\n```\n[[Ionian]]\n```\n",
        "a fence's text is left alone"
    );
    // Moved into a folder: a link by name still names it.
    let r = v.rename("Major.md", "Notes/Theory/Major.md").unwrap();
    assert!(tmp.path().join("Notes/Theory/Major.md").exists());
    assert!(!tmp.path().join("Major.md").exists());
    assert!(
        r.relinked.is_empty(),
        "named by its name, which did not change"
    );
    assert!(v.text("Notes/Theory/Major.md").is_some());
    assert!(
        v.rename("Notes/Other.md", "Notes/Modes.md").is_err(),
        "taken"
    );
}

#[test]
fn a_refresh_says_what_changed_outside_the_app() {
    let (tmp, v) = vault();
    assert!(v.folders().contains("Empty"), "an empty folder is a folder");
    std::fs::write(tmp.path().join("Notes/Other.md"), "Changed.\n").unwrap();
    std::fs::write(tmp.path().join("New.md"), "New.\n").unwrap();
    std::fs::remove_file(tmp.path().join("Ionian.md")).unwrap();
    std::fs::create_dir_all(tmp.path().join("Made")).unwrap();
    let c = v.refresh().unwrap();
    assert_eq!(c.created, ["New.md"]);
    assert_eq!(c.modified, ["Notes/Other.md"]);
    assert_eq!(c.removed, ["Ionian.md"]);
    assert_eq!(c.folders_created, ["Made"]);
    assert_eq!(v.text("Notes/Other.md").as_deref(), Some("Changed.\n"));
}

#[test]
fn the_watcher_hears_an_outside_edit_and_not_the_apps_own() {
    let (tmp, v) = vault();
    let (tx, rx) = std::sync::mpsc::channel();
    let watch = v.watch(move |c| {
        let _ = tx.send(c);
    });
    let watch = watch.unwrap();
    v.save("Ionian.md", "Saved by the app.\n").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(tmp.path().join("Notes/Other.md"), "From another editor.\n").unwrap();
    // What the watcher hears, until `path` is among what changed (a busy
    // machine may split or join the debounced events).
    let until = |path: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Ok(c) = rx.recv_timeout(Duration::from_millis(200)) {
                let found = c.modified.iter().any(|p| p == path);
                all.push(c);
                if found {
                    return all;
                }
            }
        }
        panic!("never heard {path}: {all:?}");
    };
    let heard = until("Notes/Other.md");
    assert!(
        heard
            .iter()
            .all(|c| !c.modified.contains(&"Ionian.md".to_owned())),
        "the app's own save is no change: {heard:?}"
    );
    assert_eq!(
        v.text("Notes/Other.md").as_deref(),
        Some("From another editor.\n")
    );
    // An outside edit just after the app saved the same note is heard
    // too (someone typing while a `git pull` lands).
    v.save("Ionian.md", "Saved again.\n").unwrap();
    std::fs::write(tmp.path().join("Ionian.md"), "Pulled.\n").unwrap();
    until("Ionian.md");
    assert_eq!(v.text("Ionian.md").as_deref(), Some("Pulled.\n"));
    drop(watch);
}

#[test]
fn a_copy_keeps_the_text_and_the_original() {
    let (tmp, v) = vault();
    let c = v.copy_file("Ionian.md", "Notes/Ionian 1.md").unwrap();
    assert_eq!(c.created, ["Notes/Ionian 1.md"]);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("Notes/Ionian 1.md")).unwrap(),
        "# Ionian\nThe major scale.\n"
    );
    assert!(tmp.path().join("Ionian.md").exists());
    assert!(v.copy_file("Ionian.md", "Notes/Modes.md").is_err(), "taken");
    assert!(
        v.copy_file("Ionian.md", "../out.md").is_err(),
        "out of the vault"
    );
}

#[test]
fn a_folder_moves_with_its_notes_and_files_and_links_by_path_follow() {
    let (tmp, v) = vault();
    let root = tmp.path();
    std::fs::write(root.join("Notes/pic.png"), b"png").unwrap();
    std::fs::write(root.join("Links.md"), "[[Notes/Other]] and [[Other]]\n").unwrap();
    v.refresh().unwrap();
    let moved = v.move_folder("Notes", "Archive/Notes").unwrap();
    let mut tos: Vec<&str> = moved.iter().map(|r| r.to.as_str()).collect();
    tos.sort_unstable();
    assert_eq!(tos, ["Archive/Notes/Modes.md", "Archive/Notes/Other.md"]);
    assert!(!root.join("Notes").exists());
    assert!(root.join("Archive/Notes/pic.png").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("Links.md")).unwrap(),
        "[[Archive/Notes/Other]] and [[Other]]\n"
    );
    assert!(v.folders().contains("Archive/Notes"));
    assert!(
        v.move_folder("Archive", "Archive/In").is_err(),
        "into itself"
    );
    assert!(v.move_folder("Archive", "Empty").is_err(), "taken");
}

#[test]
fn a_folder_copies_and_things_go_to_the_trash_numbered() {
    let (tmp, v) = vault();
    let root = tmp.path();
    v.copy_folder("Notes", "Notes 1").unwrap();
    assert!(root.join("Notes 1/Modes.md").exists());
    assert!(root.join("Notes/Modes.md").exists());
    let (c, at) = v.trash("Notes 1/Modes.md").unwrap();
    assert_eq!(c.removed, ["Notes 1/Modes.md"]);
    assert_eq!(at, ".trash/Modes.md");
    let (_, again) = v.trash("Notes/Modes.md").unwrap();
    assert_eq!(again, ".trash/Modes 1.md");
    let (c, at) = v.trash("Notes 1").unwrap();
    assert_eq!(at, ".trash/Notes 1");
    assert_eq!(c.folders_removed, ["Notes 1"]);
    assert!(root.join(".trash/Notes 1/Other.md").exists());
    assert!(v.with(|v| v.pages.iter().all(|p| !p.rel_path.starts_with(".trash"))));
    assert!(v.trash("Nothing.md").is_err());
}
