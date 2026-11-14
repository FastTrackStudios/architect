//! The sync backend follows no symlink (#675): what `NOTES_READ` and
//! `NOTES_LIST` (the node's `Notes` service), the sync manifest, the
//! collab lane and the doc peer read and write. A symlink in a vault —
//! to a file outside it, to a folder outside it, to a private or ignored
//! folder inside it, or a file swapped for one after it was listed — is
//! never listed, read, written or removed, and hiding it deletes
//! nothing: the watcher reports no `Delete` for it.
//!
//! Every vault here is a temp dir.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::Duration;

use vault_live::Backend;
use vault_proto::{IfMatch, VaultEvent, VaultSync, VaultSyncError};

const SECRET: &str = "outside the vault: never read";

/// A vault at `<tmp>/vault` with `Note.md`, and a folder beside it
/// (`<tmp>/outside`) holding `secret.md` and `dir/secret.md`.
fn setup() -> (tempfile::TempDir, Backend) {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("Note.md"), "# Note\n\nThe vault's own.\n").unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(outside.join("dir")).unwrap();
    fs::write(outside.join("secret.md"), SECRET).unwrap();
    fs::write(outside.join("dir/secret.md"), SECRET).unwrap();
    let backend = Backend::single("v", vault).unwrap();
    (tmp, backend)
}

fn listed(b: &Backend) -> Vec<String> {
    let mut paths: Vec<String> = b
        .manifest("v")
        .unwrap()
        .files
        .into_iter()
        .map(|f| f.path)
        .collect();
    paths.sort();
    paths
}

fn assert_outside_untouched(tmp: &Path) {
    let outside = tmp.join("outside");
    assert_eq!(
        fs::read_to_string(outside.join("secret.md")).unwrap(),
        SECRET
    );
    assert_eq!(
        fs::read_to_string(outside.join("dir/secret.md")).unwrap(),
        SECRET
    );
    let mut names: Vec<_> = fs::read_dir(&outside)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["dir", "secret.md"], "nothing written outside");
}

/// The bug on main: `get_file` (behind `NOTES_READ`) returned a file
/// outside the vault through a symlink in it, and the manifest (behind
/// `NOTES_LIST` and sync) listed it.
#[test]
fn get_file_reads_no_file_outside_the_vault_through_a_symlink() {
    let (tmp, b) = setup();
    let vault = b.root("v").unwrap();
    symlink(tmp.path().join("outside/secret.md"), vault.join("Out.md")).unwrap();
    symlink("Note.md", vault.join("Alias.md")).unwrap();

    assert_eq!(listed(&b), ["Note.md"]);
    assert_eq!(b.paths("v").unwrap(), ["Note.md"]);
    for rel in ["Out.md", "Alias.md"] {
        let read = b.get_file("v", rel);
        assert!(
            matches!(read, Err(VaultSyncError::NotFound)),
            "{rel} read through its symlink: {read:?}"
        );
    }
    let index = b.folder_index("v").unwrap();
    let pages: Vec<_> = index.pages.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(pages, ["Note.md"]);
    assert!(b.search_text("v", "never read", 10).unwrap().is_empty());
    assert_eq!(
        b.get_file("v", "Note.md").unwrap().0,
        b"# Note\n\nThe vault's own.\n"
    );
}

/// A symlinked folder pointing out of the vault: not walked into, not
/// read through, and not written through.
#[test]
fn a_symlinked_folder_out_of_the_vault_is_neither_listed_read_nor_written() {
    let (tmp, b) = setup();
    let vault = b.root("v").unwrap();
    symlink(tmp.path().join("outside/dir"), vault.join("Ext")).unwrap();

    assert_eq!(listed(&b), ["Note.md"]);
    assert!(matches!(
        b.get_file("v", "Ext/secret.md"),
        Err(VaultSyncError::NotFound)
    ));
    let put = b.put_file("v", "Ext/new.md", b"planted".to_vec(), IfMatch::Force);
    assert!(matches!(put, Err(VaultSyncError::Refused(_))), "{put:?}");
    let over = b.put_file("v", "Ext/secret.md", b"over".to_vec(), IfMatch::Force);
    assert!(matches!(over, Err(VaultSyncError::Refused(_))), "{over:?}");
    let del = b.delete_file("v", "Ext/secret.md", IfMatch::Force);
    assert!(matches!(del, Err(VaultSyncError::Refused(_))), "{del:?}");
    let moved = b.move_file("v", "Note.md", "Ext/Note.md", IfMatch::Force);
    assert!(
        matches!(moved, Err(VaultSyncError::Refused(_))),
        "{moved:?}"
    );
    assert!(
        vault.join("Note.md").is_file(),
        "the refused move kept the note"
    );
    assert_outside_untouched(tmp.path());
}

/// A symlink inside the vault to its private folder, or to an ignored
/// one (`.git`): the link hands out nothing the vault keeps to itself.
#[test]
fn a_symlink_to_a_private_or_ignored_folder_reads_nothing() {
    let (_tmp, b) = setup();
    let vault = b.root("v").unwrap();
    let private = vault.join("Private");
    fs::create_dir_all(&private).unwrap();
    fs::write(private.join(vault_proto::consts::PRIVATE_MARKER), b"").unwrap();
    fs::write(private.join("acct.md"), "node only").unwrap();
    fs::create_dir_all(vault.join(".git")).unwrap();
    fs::write(vault.join(".git/config"), "[core]").unwrap();
    fs::create_dir_all(vault.join("Shared")).unwrap();
    symlink("../Private/acct.md", vault.join("Shared/acct.md")).unwrap();
    symlink("../.git/config", vault.join("Shared/config.md")).unwrap();
    symlink("Private", vault.join("Priv")).unwrap();
    symlink(".git", vault.join("Git")).unwrap();

    // None of the links is listed (`.git` itself is the manifest's own
    // business, not this test's).
    let paths = listed(&b);
    for link in ["Shared/", "Priv/", "Git/", "Private/"] {
        assert!(
            !paths.iter().any(|p| p.starts_with(link)),
            "{link}: {paths:?}"
        );
    }
    for rel in [
        "Shared/acct.md",
        "Shared/config.md",
        "Priv/acct.md",
        "Git/config",
    ] {
        let read = b.get_file("v", rel);
        assert!(
            matches!(read, Err(VaultSyncError::NotFound)),
            "{rel}: {read:?}"
        );
    }
    // Refused, whichever rule sees it first (the private folder's or the
    // symlink's).
    let put = b.put_file("v", "Priv/acct.md", b"over".to_vec(), IfMatch::Force);
    assert!(put.is_err(), "{put:?}");
    let put = b.put_file("v", "Shared/acct.md", b"over".to_vec(), IfMatch::Force);
    assert!(matches!(put, Err(VaultSyncError::Refused(_))), "{put:?}");
    assert_eq!(
        fs::read_to_string(private.join("acct.md")).unwrap(),
        "node only"
    );
}

/// A file listed, then swapped for a symlink before it is read: the
/// read is refused, never followed; a write or delete of it is refused
/// and leaves both the link and its target as they were.
#[test]
fn a_file_swapped_for_a_symlink_after_listing_is_not_followed() {
    let (tmp, b) = setup();
    let vault = b.root("v").unwrap();
    fs::write(vault.join("Swap.md"), "plain at first").unwrap();
    assert_eq!(listed(&b), ["Note.md", "Swap.md"]);
    assert_eq!(b.get_file("v", "Swap.md").unwrap().0, b"plain at first");

    fs::remove_file(vault.join("Swap.md")).unwrap();
    symlink(tmp.path().join("outside/secret.md"), vault.join("Swap.md")).unwrap();

    assert!(matches!(
        b.get_file("v", "Swap.md"),
        Err(VaultSyncError::NotFound)
    ));
    let put = b.put_file("v", "Swap.md", b"through".to_vec(), IfMatch::Force);
    assert!(matches!(put, Err(VaultSyncError::Refused(_))), "{put:?}");
    let del = b.delete_file("v", "Swap.md", IfMatch::Force);
    assert!(matches!(del, Err(VaultSyncError::Refused(_))), "{del:?}");
    let collab = b.open_collab("v", "Swap.md");
    assert!(
        matches!(collab, Err(VaultSyncError::Refused(_))),
        "{collab:?}"
    );
    let folder = b.set_folder("v", "Swap.md", Some("Home".into()), IfMatch::Force);
    assert!(folder.is_err(), "{folder:?}");
    assert!(
        fs::symlink_metadata(vault.join("Swap.md"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced or removed"
    );
    assert_outside_untouched(tmp.path());
    assert_eq!(listed(&b), ["Note.md"]);
}

/// The watcher reports no `Delete` (and no `Put`) for a symlink: a note
/// swapped for a link, or a dangling link, is hidden, never deleted, so
/// its document is not tombstoned and no peer removes its copy. Nor when
/// the link is removed later (`stow -D`): the path was a link, so its
/// going is no deletion either — whether the link was there before the
/// watcher started or came while it ran. A real edit after them still
/// arrives, and arrives first; a real file deleted still reports one.
#[tokio::test(flavor = "multi_thread")]
async fn the_watcher_reports_no_delete_for_a_hidden_symlink() {
    let (tmp, b) = setup();
    let vault = b.root("v").unwrap();
    fs::write(vault.join("Swap.md"), "plain at first").unwrap();
    fs::write(vault.join("Plain.md"), "deleted for real").unwrap();
    // Links there before the watcher starts.
    symlink("Note.md", vault.join("Before.md")).unwrap();
    fs::create_dir_all(vault.join("sub")).unwrap();
    symlink(tmp.path().join("outside/dir"), vault.join("sub/Ext")).unwrap();
    let tx = b.channel("v").await;
    let mut rx = tx.subscribe();
    let _watch = b.start_watcher("v").await.expect("start watcher");
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A note swapped for a link (atomically, as a dotfiles manager does).
    symlink(tmp.path().join("outside/secret.md"), vault.join(".swap")).unwrap();
    fs::rename(vault.join(".swap"), vault.join("Swap.md")).unwrap();
    // A new link, a dangling one and a symlinked folder.
    symlink("Note.md", vault.join("Alias.md")).unwrap();
    symlink("Gone.md", vault.join("Dangling.md")).unwrap();
    symlink(tmp.path().join("outside/dir"), vault.join("Ext")).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // Every link removed again, as `stow -D` unlinks: the paths are gone.
    for link in [
        "Swap.md",
        "Alias.md",
        "Dangling.md",
        "Ext",
        "Before.md",
        "sub/Ext",
    ] {
        fs::remove_file(vault.join(link)).unwrap();
    }
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // Then a real edit, the marker the links' events would precede.
    fs::write(vault.join("Real.md"), "real").unwrap();

    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("the real edit never arrived")
            .expect("rx recv ok");
        match evt {
            VaultEvent::Put { ref path, .. } if path == "Real.md" => break,
            other => seen.push(other),
        }
    }
    assert!(seen.is_empty(), "the watcher reported a symlink: {seen:?}");
    assert!(!vault.join("Swap.md").exists());
    assert_outside_untouched(tmp.path());

    // A real file deleted is still a deletion, and so is one that
    // replaced a removed link.
    fs::write(vault.join("Swap.md"), "plain again").unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    fs::remove_file(vault.join("Plain.md")).unwrap();
    fs::remove_file(vault.join("Swap.md")).unwrap();
    // (A removal can be reported in more than one debounce window.)
    let mut deleted = std::collections::BTreeSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while deleted.len() < 2 {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("a real file's delete never arrived: {deleted:?}"))
            .expect("rx recv ok");
        if let VaultEvent::Delete { path } = evt {
            deleted.insert(path);
        }
    }
    assert_eq!(Vec::from_iter(deleted), ["Plain.md", "Swap.md"]);
}
