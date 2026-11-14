//! The sync watcher and folders (#682 re-check): the native watcher
//! attaches to a new folder only once its own event is handled, so a
//! note written into a folder before then makes no event of its own.
//! The watcher walks every folder an event finds, so such a note still
//! gets its `Put` and, later, its `Delete`; a folder that goes deletes
//! the notes recorded below it; a folder renamed inside the vault is a
//! `Delete` under the old name and a `Put` under the new, so a peer
//! loses nothing. Run under both watchers (`TASK_WATCH_POLL=1`).
//!
//! Every vault here is a temp dir.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::Duration;

use tokio::sync::broadcast::Receiver;
use vault_live::Backend;
use vault_proto::VaultEvent;

/// A watched vault at `<tmp>/vault` holding `Note.md`, with `<tmp>/outside`
/// beside it.
async fn watched() -> (
    tempfile::TempDir,
    Backend,
    Receiver<VaultEvent>,
    vault_live::sync::WatcherHandle,
) {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("Note.md"), "# Note\n").unwrap();
    fs::create_dir_all(tmp.path().join("outside")).unwrap();
    let b = Backend::single("v", vault).unwrap();
    let rx = b.channel("v").await.subscribe();
    let watch = b.start_watcher("v").await.expect("start watcher");
    tokio::time::sleep(Duration::from_millis(300)).await;
    (tmp, b, rx, watch)
}

/// Every event before the watcher reports `marker`, a note this writes
/// once the window before it has passed.
async fn events_before_marker(
    rx: &mut Receiver<VaultEvent>,
    vault: &Path,
    marker: &str,
) -> Vec<VaultEvent> {
    tokio::time::sleep(Duration::from_millis(1200)).await;
    fs::write(vault.join(marker), "marker").unwrap();
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(8))
        .unwrap();
    loop {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("{marker} never arrived; before it: {seen:?}"))
            .expect("rx recv ok");
        match evt {
            VaultEvent::Put { ref path, .. } if path == marker => return seen,
            other => seen.push(other),
        }
    }
}

/// Wait until every path in `want` has had an event of the kind `pick`
/// accepts; the events seen on the way are returned.
async fn until_all(
    rx: &mut Receiver<VaultEvent>,
    want: &[&str],
    what: &str,
    pick: fn(&VaultEvent) -> Option<&str>,
) -> Vec<VaultEvent> {
    let mut left: Vec<&str> = want.to_vec();
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(8))
        .unwrap();
    while !left.is_empty() {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no {what} for {left:?}; seen: {seen:?}"))
            .expect("rx recv ok");
        if let Some(path) = pick(&evt) {
            left.retain(|p| *p != path);
        }
        seen.push(evt);
    }
    seen
}

fn put_path(e: &VaultEvent) -> Option<&str> {
    match e {
        VaultEvent::Put { path, .. } => Some(path),
        _ => None,
    }
}

fn delete_path(e: &VaultEvent) -> Option<&str> {
    match e {
        VaultEvent::Delete { path } => Some(path),
        _ => None,
    }
}

/// A new folder and a note written into it at once (before the native
/// watcher can have attached to the folder): the note's `Put` arrives,
/// and deleting it sends its `Delete`.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_in_a_new_folder_puts_and_deletes() {
    let (_tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    fs::create_dir_all(vault.join("New/Deep")).unwrap();
    fs::write(vault.join("New/Deep/Fresh.md"), "fresh").unwrap();
    until_all(&mut rx, &["New/Deep/Fresh.md"], "Put", put_path).await;

    fs::remove_file(vault.join("New/Deep/Fresh.md")).unwrap();
    until_all(&mut rx, &["New/Deep/Fresh.md"], "Delete", delete_path).await;
}

/// A folder of notes moved in from outside the vault: each note gets its
/// `Put`, and a note deleted after sends its `Delete`.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_moved_in_puts_its_notes_and_deletes_one_removed() {
    let (tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    let src = tmp.path().join("outside/Album");
    fs::create_dir_all(src.join("Disc 2")).unwrap();
    fs::write(src.join("One.md"), "one").unwrap();
    fs::write(src.join("Disc 2/Two.md"), "two").unwrap();
    fs::write(src.join("cover.png"), "not a note").unwrap();
    fs::rename(&src, vault.join("Album")).unwrap();
    let seen = until_all(
        &mut rx,
        &["Album/One.md", "Album/Disc 2/Two.md"],
        "Put",
        put_path,
    )
    .await;
    assert!(
        !seen.iter().any(|e| delete_path(e).is_some()),
        "moving a folder in deleted: {seen:?}"
    );

    fs::remove_file(vault.join("Album/Disc 2/Two.md")).unwrap();
    until_all(&mut rx, &["Album/Disc 2/Two.md"], "Delete", delete_path).await;
}

/// A peer's notes as a sync client writes them (`write_file`: the folder
/// made, then the file, straight to disk, no `put_file`), each deleted a
/// little later: every round's `Delete` arrives. The re-check missed 7
/// of 10 on the head before the folder walk.
#[tokio::test(flavor = "multi_thread")]
async fn sync_client_writes_into_new_folders_always_delete() {
    let (_tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    for round in 0..10 {
        let rel = format!("Peer {round}/Sub/Note {round}.md");
        let abs = vault.join(&rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, format!("from a peer, round {round}")).unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        fs::remove_file(&abs).unwrap();
        until_all(&mut rx, &[rel.as_str()], "Delete", delete_path).await;
    }
}

/// A folder deleted whole, and one moved out of the vault: a `Delete` for
/// each note recorded below either, the notes of one moved out included
/// (no event of their own reports them gone).
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_deleted_or_moved_out_deletes_its_notes() {
    let (tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    for dir in ["Gone", "Away"] {
        fs::create_dir_all(vault.join(dir).join("In")).unwrap();
        fs::write(vault.join(dir).join("A.md"), "a").unwrap();
        fs::write(vault.join(dir).join("In/B.md"), "b").unwrap();
    }
    until_all(
        &mut rx,
        &["Gone/A.md", "Gone/In/B.md", "Away/A.md", "Away/In/B.md"],
        "Put",
        put_path,
    )
    .await;

    fs::remove_dir_all(vault.join("Gone")).unwrap();
    fs::rename(vault.join("Away"), tmp.path().join("outside/Away")).unwrap();
    let seen = until_all(
        &mut rx,
        &["Gone/A.md", "Gone/In/B.md", "Away/A.md", "Away/In/B.md"],
        "Delete",
        delete_path,
    )
    .await;
    let mut deleted: Vec<&str> = seen.iter().filter_map(delete_path).collect();
    deleted.sort_unstable();
    let before = deleted.len();
    deleted.dedup();
    assert_eq!(before, deleted.len(), "a Delete sent twice: {seen:?}");
}

/// A folder that can no longer be read (`chmod 000`, a sandbox taking it
/// away) is not deleted: its notes stay on peers. Running as root reads
/// it anyway, so the test proves nothing there and is skipped.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_folder_deletes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    fs::create_dir_all(vault.join("Lock/In")).unwrap();
    fs::write(vault.join("Lock/A.md"), "a").unwrap();
    fs::write(vault.join("Lock/In/B.md"), "b").unwrap();
    until_all(&mut rx, &["Lock/A.md", "Lock/In/B.md"], "Put", put_path).await;

    fs::set_permissions(vault.join("Lock/In"), fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(vault.join("Lock"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(vault.join("Lock")).is_ok() {
        fs::set_permissions(vault.join("Lock"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(vault.join("Lock/In"), fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let seen = events_before_marker(&mut rx, &vault, "After.md").await;
    fs::set_permissions(vault.join("Lock"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(vault.join("Lock/In"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        !seen.iter().any(|e| delete_path(e).is_some()),
        "an unreadable folder sent Deletes: {seen:?}"
    );
}

/// A folder renamed inside the vault: a peer applying the events in the
/// order they come ends with every note, under its new path, and
/// nothing under the old.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_renamed_inside_the_vault_loses_nothing_on_a_peer() {
    let (_tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    fs::create_dir_all(vault.join("Draft/Parts")).unwrap();
    fs::write(vault.join("Draft/Intro.md"), "intro").unwrap();
    fs::write(vault.join("Draft/Parts/One.md"), "one").unwrap();
    let mut peer: BTreeMap<String, String> = BTreeMap::new();
    for e in until_all(
        &mut rx,
        &["Draft/Intro.md", "Draft/Parts/One.md"],
        "Put",
        put_path,
    )
    .await
    {
        apply(&mut peer, e);
    }

    fs::rename(vault.join("Draft"), vault.join("Final")).unwrap();
    for e in events_before_marker(&mut rx, &vault, "Marker.md").await {
        apply(&mut peer, e);
    }
    peer.remove("Note.md");
    assert_eq!(
        peer.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Final/Intro.md", "Final/Parts/One.md"],
        "the peer's notes after the rename"
    );

    // And the renamed notes delete for real.
    fs::remove_file(vault.join("Final/Parts/One.md")).unwrap();
    until_all(&mut rx, &["Final/Parts/One.md"], "Delete", delete_path).await;
}

fn apply(peer: &mut BTreeMap<String, String>, e: VaultEvent) {
    match e {
        VaultEvent::Put { path, sha256, .. } => {
            peer.insert(path, sha256);
        }
        VaultEvent::Delete { path } => {
            peer.remove(&path);
        }
        VaultEvent::Resync => {}
    }
}

/// A folder made behind a symlinked folder (notify follows the link, so
/// its event arrives) is not walked: no `Put` for its notes; and the
/// link removed after sends nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_behind_a_symlink_is_not_walked() {
    let (tmp, b, mut rx, _watch) = watched().await;
    let vault = b.root("v").unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(outside.join("Pkg")).unwrap();
    symlink(outside.join("Pkg"), vault.join("Pkg")).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    fs::create_dir_all(outside.join("Pkg/New")).unwrap();
    fs::write(outside.join("Pkg/New/Behind.md"), "behind").unwrap();
    let seen = events_before_marker(&mut rx, &vault, "Marker1.md").await;
    assert!(seen.is_empty(), "behind the link: {seen:?}");

    fs::remove_file(vault.join("Pkg")).unwrap();
    let seen = events_before_marker(&mut rx, &vault, "Marker2.md").await;
    assert!(seen.is_empty(), "removing the link: {seen:?}");
    assert_eq!(
        fs::read_to_string(outside.join("Pkg/New/Behind.md")).unwrap(),
        "behind"
    );
}
