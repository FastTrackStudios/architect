//! Every vault the person can reach, and the providers, in one navigator
//! (`Mounts`, #443): the left list's top rows, Recent Files/, Vaults/ (a
//! flat list of every org's top-level folders with its badge), Tags/
//! from the vaults alone; writes routed to their vault, never across.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};

use architect_navigator::list::Clock;
use architect_navigator::tree::{ASSETS, VAULTS};
use architect_navigator::{
    FileOp, MemorySource, Mount, Mounts, NavRowKind, NavTarget, NavigatorSettings, NavigatorSource,
    NavigatorState, NoteFile, OrgBadge, Pane, Place, PropertyValue, ProviderSource, SourceEvent,
    VaultIndex,
};
use architect_navigator_api::{Item, Node, Snapshot};

const NOW: i64 = 1_793_880_000_000;

fn note(path: &str, tags: &[&str]) -> NoteFile {
    let mut f = NoteFile::new(path, NOW, NOW);
    f.tags = tags.iter().map(|t| (*t).to_owned()).collect();
    f
}

fn cody() -> MemorySource {
    MemorySource::new("codywright")
        .with_folder("Bible Study")
        .with_folder("Bible Study/Psalms")
        .with_folder("Health")
        .with_file(note("Bible Study/Psalm 23.md", &["scripture"]))
        .with_file({
            let mut f = note("Bible Study/Bible Study.md", &[]);
            f.properties
                .push(("wiki".to_owned(), PropertyValue::Bool(true)));
            f
        })
        .with_file(note("Health/Run.md", &["habit"]))
}

fn fts() -> MemorySource {
    MemorySource::new("fasttrackstudio")
        .with_folder("Music Theory")
        .with_folder("Audio Production")
        .with_file(note("Music Theory/Modes.md", &["scripture", "theory"]))
        .with_file(note("Audio Production/Compression.md", &[]))
}

fn queue() -> ProviderSource {
    ProviderSource::with(
        Snapshot::new("Queue")
            .node(Node::new("inbox", "Inbox"))
            .node(Node::new("today", "Today"))
            // A task's tags are its note's: never a second count in Tags/.
            .item(Item::new("t1", "today", "Write it").tag("scripture")),
    )
}

fn mounts() -> Mounts {
    Mounts::new("Navigator")
        .with(
            Mount::provider("tasks/queue", "Inbox", Place::Top)
                .icon("inbox")
                .flat(),
            Arc::new(queue()),
        )
        .with(
            Mount::vault(OrgBadge::new("codywright", "Cody Wright", "#047857")),
            Arc::new(cody()),
        )
        .with(
            Mount::vault(OrgBadge::new(
                "fasttrackstudio",
                "FastTrackStudio",
                "#1d4ed8",
            )),
            Arc::new(fts()),
        )
}

fn state(source: &dyn NavigatorSource) -> NavigatorState {
    let settings = NavigatorSettings {
        folder_notes: true,
        ..NavigatorSettings::default()
    };
    let mut s = NavigatorState::new(
        VaultIndex::read(source),
        settings,
        Clock {
            now: NOW,
            offset: 0,
        },
    );
    s.set_view(Pane::Navigation, 2000.0);
    s.set_view(Pane::Files, 2000.0);
    s
}

/// The rows' names, sections as `Name/`.
fn names(s: &NavigatorState) -> Vec<String> {
    s.nav_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            NavRowKind::Folder { name, .. } => Some(name.clone()),
            NavRowKind::Section { name, .. } => Some(format!("{name}/")),
            NavRowKind::Tag { name, .. } => Some(format!("#{name}")),
            _ => None,
        })
        .collect()
}

fn listed(s: &NavigatorState) -> Vec<String> {
    s.list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            architect_navigator::ListRowKind::File { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_vaults_top_folders_are_one_flat_list_with_its_orgs_badge() {
    let m = mounts();
    assert_eq!(
        m.folders()[..3],
        ["~tasks:queue", "~tasks:queue/Inbox", "~tasks:queue/Today"]
    );
    let s = state(&m);
    let rows = names(&s);
    // The top row first, with no header; then Vaults/, by name across
    // both orgs; then Tags/.
    assert_eq!(
        rows[..6],
        [
            "Inbox",
            "Vaults/",
            "Audio Production",
            "Bible Study",
            "Health",
            "Music Theory"
        ]
    );
    assert_eq!(
        s.badge("@codywright/Bible Study").unwrap().org,
        "codywright"
    );
    assert_eq!(
        s.badge("@fasttrackstudio/Music Theory").unwrap().name,
        "FastTrackStudio"
    );
    // Only the top folders carry it, and a top row none.
    assert!(s.badge("@codywright/Bible Study/Psalms").is_none());
    assert!(s.badge("~tasks:queue").is_none());
    assert!(s.is_wiki("@codywright/Bible Study"));
    assert!(!s.is_wiki("@codywright/Health"));
    assert_eq!(
        s.mount_icon("~tasks:queue"),
        architect_navigator::Icon::parse("inbox")
    );
    // The Queue's containers are its list's groups, not rows under it.
    assert!(!rows.contains(&"Today".to_owned()));
}

#[test]
fn tags_merge_across_the_vaults_and_leave_providers_out() {
    let m = mounts();
    let mut s = state(&m);
    let tagged = s.tag_tree().get("scripture").expect("a tag of both vaults");
    // Psalm 23 and Modes: the Queue's task is not counted again.
    assert_eq!(tagged.notes.len(), 2);
    s.select_tag("scripture");
    let listed: Vec<String> = listed(&s);
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|p| p.starts_with('@')));
}

#[test]
fn a_section_folds_and_unfolds() {
    let m = mounts();
    let mut s = state(&m);
    s.toggle_section(VAULTS);
    assert!(!names(&s).contains(&"Bible Study".to_owned()));
    s.toggle_section(VAULTS);
    assert!(names(&s).contains(&"Bible Study".to_owned()));
    // No Assets/ without a mount there.
    assert!(!names(&s).contains(&"Assets/".to_owned()));
    let _ = ASSETS;
    let kept = s.view_state();
    s.toggle_section(VAULTS);
    assert_eq!(s.view_state().sections_folded, ["vaults"]);
    s.restore(&kept);
    assert!(names(&s).contains(&"Bible Study".to_owned()));
}

#[test]
fn writes_go_to_their_vault_and_never_across() {
    let (c, f) = (cody(), fts());
    let m = Mounts::new("N")
        .with(
            Mount::vault(OrgBadge::new("codywright", "Cody", "#000")),
            Arc::new(c.clone()),
        )
        .with(
            Mount::vault(OrgBadge::new("fasttrackstudio", "FTS", "#000")),
            Arc::new(f.clone()),
        );
    m.write(&[FileOp::CreateFile {
        path: "@codywright/Health/Swim.md".to_owned(),
        text: String::new(),
    }])
    .unwrap();
    assert!(c.files().iter().any(|n| n.path == "Health/Swim.md"));
    assert!(!f.files().iter().any(|n| n.path.ends_with("Swim.md")));
    // Moving into another org's vault is an ownership change: refused,
    // and nothing written.
    let err = m
        .write(&[FileOp::Move {
            from: "@codywright/Health/Run.md".to_owned(),
            to: "@fasttrackstudio/Music Theory/Run.md".to_owned(),
        }])
        .unwrap_err();
    assert!(err.contains("another org"), "{err}");
    assert!(c.files().iter().any(|n| n.path == "Health/Run.md"));
    // A tag renamed is renamed in every vault.
    m.write(&[FileOp::RenameTag {
        from: "scripture".to_owned(),
        to: "bible".to_owned(),
    }])
    .unwrap();
    assert!(
        c.files()
            .iter()
            .any(|n| n.tags.contains(&"bible".to_owned()))
    );
    assert!(
        f.files()
            .iter()
            .any(|n| n.tags.contains(&"bible".to_owned()))
    );
}

#[test]
fn a_vaults_changes_come_through_under_its_mount() {
    let c = cody();
    let m = Mounts::new("N").with(
        Mount::vault(OrgBadge::new("codywright", "Cody", "#000")),
        Arc::new(c.clone()),
    );
    let heard = Arc::new(Mutex::new(Vec::<String>::new()));
    let h = Arc::clone(&heard);
    let _sub = m.subscribe(Box::new(move |e: &SourceEvent| {
        if let SourceEvent::FileCreated(f) = e {
            h.lock().unwrap().push(f.path.clone());
        }
    }));
    c.write(&[FileOp::CreateFile {
        path: "Health/Swim.md".to_owned(),
        text: String::new(),
    }])
    .unwrap();
    assert_eq!(*heard.lock().unwrap(), ["@codywright/Health/Swim.md"]);
    // A mount's description changed: read everything again.
    let resets = Arc::new(Mutex::new(0));
    let r = Arc::clone(&resets);
    let _sub2 = m.subscribe(Box::new(move |e: &SourceEvent| {
        if matches!(e, SourceEvent::Reset) {
            *r.lock().unwrap() += 1;
        }
    }));
    m.describe("@codywright", |mount| mount.count = Some(3));
    assert_eq!(*resets.lock().unwrap(), 1);
    assert_eq!(m.mounts_now()[0].count, Some(3));
}

#[test]
fn a_top_rows_list_takes_its_mounts_look() {
    let m = Mounts::new("N").with(
        Mount::provider("tasks/queue", "Inbox", Place::Top)
            .flat()
            .listed(architect_navigator::Appearance {
                descendants: Some(true),
                grouping: Some(architect_navigator::Grouping::Folder),
                ..architect_navigator::Appearance::default()
            }),
        Arc::new(queue()),
    );
    let s = state(&m);
    let look = s.settings_for(&NavTarget::Folder("~tasks:queue".to_owned()));
    assert!(look.include_descendants);
    assert_eq!(look.grouping, architect_navigator::Grouping::Folder);
}
