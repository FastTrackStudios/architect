//! A document's file: every finished step on disk, crash recovery,
//! save as fold, branches and versions, blobs, kinds, and the bytes a
//! browser reads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crdt::file::{
    DiskStorage, DocFile, FileError, MAIN, MemoryStorage, Storage, VersionKind, hash_of,
};
use crdt::{CrdtDoc, DocKind, DocKinds, Migration};
use loro::LoroDoc;
use uuid::Uuid;

fn note() -> DocKind {
    DocKind::new("test.note", "Note").extension("note")
}

fn text(doc: &LoroDoc) -> String {
    doc.get_text("body").to_string()
}

fn steps(doc: &LoroDoc) -> Vec<i64> {
    doc.get_list("steps")
        .to_vec()
        .into_iter()
        .map(|v| v.into_i64().unwrap())
        .collect()
}

fn type_at(doc: &LoroDoc, at: usize, s: &str) {
    doc.get_text("body").insert(at, s).unwrap();
}

#[tokio::test]
async fn a_new_file_says_its_kind_and_schema_and_main_is_stamped() {
    let store = MemoryStorage::new();
    let kind = note().schema(3, Vec::new());
    let file = DocFile::create(store.clone(), &kind).await.unwrap();
    assert_eq!(file.meta("kind").await.as_deref(), Some("test.note"));
    assert_eq!(file.meta("schema").await.as_deref(), Some("3"));
    assert!(file.id().await.is_some());
    let main = file.open_branch(MAIN).await.unwrap();
    assert_eq!(crdt::kinds::stamp_of(main.loro()), Some(3));
    assert_eq!(
        crdt::kinds::kind_of(main.loro()).as_deref(),
        Some("test.note")
    );
    // The stamp is the document's first step, and not one to undo.
    let versions = file.versions(MAIN).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].name, "New Note");
    assert_eq!(main.history().next_undo(), None);
}

#[tokio::test]
async fn every_finished_step_is_on_disk_when_it_returns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Song.note");
    let file = DocFile::create(DiskStorage::open(&path).unwrap(), &note())
        .await
        .unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("Type hello", |d| type_at(d, 0, "hello"))
        .await
        .unwrap();
    main.transact("Type world", |d| type_at(d, 5, " world"))
        .await
        .unwrap();
    assert_eq!(main.undo().await.unwrap().as_deref(), Some("Type world"));

    // Read by another handle while the first is still open: no save,
    // no close, it is all there.
    let again = DocFile::open(DiskStorage::open(&path).unwrap(), &note())
        .await
        .unwrap();
    assert_eq!(again.recovered(), None);
    let reopened = again.open_branch(MAIN).await.unwrap();
    assert_eq!(text(reopened.loro()), "hello");
    let names: Vec<String> = again
        .versions(MAIN)
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(
        names,
        ["New Note", "Type hello", "Type world", "Undo Type world"]
    );
}

#[tokio::test]
async fn a_torn_write_is_dropped_and_every_finished_step_comes_back() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    for i in 0..10 {
        main.transact(&format!("Step {i}"), |d| {
            d.get_list("steps").push(i).unwrap();
        })
        .await
        .unwrap();
    }
    main.save("v1").await.unwrap();
    for i in 10..15 {
        main.transact(&format!("Step {i}"), |d| {
            d.get_list("steps").push(i).unwrap();
        })
        .await
        .unwrap();
    }
    // The process dies: nothing closed, nothing dropped, and the write
    // it was in the middle of is half there.
    std::mem::forget(main);
    std::mem::forget(file);
    let mut bytes = store.bytes();
    let whole = bytes.len();
    bytes.extend_from_slice(&[200, 0, 0, 0, 5, 1, 2, 3]);

    let file = DocFile::from_bytes(bytes, &note()).await.unwrap();
    assert_eq!(file.recovered().unwrap().dropped_bytes, 8);
    let main = file.open_branch(MAIN).await.unwrap();
    assert_eq!(steps(main.loro()), (0..15).collect::<Vec<_>>());
    // Recovered from the snapshot of the save and the five steps since.
    assert_eq!(file.to_bytes().await.unwrap().len(), whole);
}

#[tokio::test]
async fn a_tail_cut_anywhere_reopens_with_the_steps_before_it() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    let mut ends = vec![store.bytes().len()];
    for i in 0..3 {
        main.transact(&format!("Step {i}"), |d| {
            d.get_list("steps").push(i).unwrap();
        })
        .await
        .unwrap();
        ends.push(store.bytes().len());
    }
    let bytes = store.bytes();
    // Cut at every byte of the last step's record: the first two steps
    // always come back, the third only once its record is whole.
    for cut in ends[2]..=ends[3] {
        let file = DocFile::from_bytes(bytes[..cut].to_vec(), &note())
            .await
            .unwrap();
        let main = file.open_branch(MAIN).await.unwrap();
        let want: Vec<i64> = if cut == ends[3] {
            vec![0, 1, 2]
        } else {
            vec![0, 1]
        };
        assert_eq!(steps(main.loro()), want, "cut at {cut}");
    }
}

/// The child half of `kill_9_mid_edit_reopens_with_every_finished_step`:
/// does nothing unless that test started this binary to be killed.
#[tokio::test]
async fn kill_9_child() {
    let Ok(path) = std::env::var("CRDT_FILE_KILL_CHILD") else {
        return;
    };
    let file = DocFile::create(DiskStorage::open(&path).unwrap(), &note())
        .await
        .unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    let mut out = std::io::stdout().lock();
    // Bounded only so the loop has an end; it is killed long before.
    for i in 0_i64..1_000_000 {
        main.transact(&format!("Step {i}"), |d| {
            d.get_list("steps").push(i).unwrap();
            type_at(d, 0, "x");
        })
        .await
        .unwrap();
        if i % 7 == 6 {
            main.save(&format!("v{i}")).await.unwrap();
        }
        // Said only once the step has returned: on disk.
        writeln!(out, "finished {i}").unwrap();
        out.flush().unwrap();
    }
}

#[tokio::test]
async fn kill_9_mid_edit_reopens_with_every_finished_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Killed.note");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "kill_9_child", "--nocapture", "--test-threads=1"])
        .env("CRDT_FILE_KILL_CHILD", &path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut finished: Option<i64> = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        if let Some(n) = line.strip_prefix("finished ") {
            finished = Some(n.parse().unwrap());
            if finished >= Some(40) {
                break;
            }
        }
    }
    // SIGKILL: no destructor, no flush, mid-step.
    child.kill().unwrap();
    child.wait().unwrap();
    let finished = finished.expect("the child finished steps");

    let file = DocFile::open(DiskStorage::open(&path).unwrap(), &note())
        .await
        .unwrap();
    let main = file.open_branch(MAIN).await.unwrap();
    let kept = steps(main.loro());
    // Every step it said was finished, in order, none twice; perhaps the
    // one it finished as it was killed, never half of one.
    assert!(kept.len() as i64 > finished, "{} <= {finished}", kept.len());
    assert_eq!(kept, (0..kept.len() as i64).collect::<Vec<_>>());
    assert_eq!(text(main.loro()).len(), kept.len());
}

#[tokio::test]
async fn save_names_a_version_and_folds_the_steps() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    for i in 0..50 {
        main.transact(&format!("Type {i}"), |d| type_at(d, 0, "ab"))
            .await
            .unwrap();
    }
    let before = store.bytes().len();
    let saved = main.save("Sent to Dan").await.unwrap();
    assert_eq!(saved.kind, VersionKind::Checkpoint);
    assert!(store.bytes().len() < before, "folding made it smaller");
    main.transact("Type more", |d| type_at(d, 0, "z"))
        .await
        .unwrap();

    let file = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    let main = file.open_branch(MAIN).await.unwrap();
    assert_eq!(text(main.loro()).len(), 101);
    // Folded steps keep their names and places.
    let versions = file.versions(MAIN).await.unwrap();
    assert_eq!(versions.len(), 1 + 50 + 1 + 1);
    assert_eq!(versions[51].name, "Sent to Dan");
    assert_eq!(versions[52].name, "Type more");
    // A folded step still opens, read-only.
    let at_ten = file.open_version(&versions[10]).await.unwrap();
    assert_eq!(text(&at_ten).len(), 20);
    assert!(at_ten.is_detached(), "read-only");
}

#[tokio::test]
async fn branches_fork_open_side_by_side_merge_back_and_delete() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("Verse", |d| type_at(d, 0, "verse"))
        .await
        .unwrap();
    let mut acoustic = main.fork("acoustic").await.unwrap();
    assert!(matches!(
        main.fork("acoustic").await,
        Err(FileError::BranchExists(_))
    ));

    // Side by side, each on its own.
    acoustic
        .transact("Chorus", |d| type_at(d, 5, " chorus"))
        .await
        .unwrap();
    main.transact("Intro", |d| type_at(d, 0, "intro "))
        .await
        .unwrap();
    assert_eq!(text(main.loro()), "intro verse");
    assert_eq!(text(acoustic.loro()), "verse chorus");

    let names: Vec<String> = file.branches().await.into_iter().map(|b| b.name).collect();
    assert_eq!(names, [MAIN, "acoustic"]);
    assert_eq!(
        file.branches().await[1].parent.as_deref(),
        Some(MAIN),
        "lineage"
    );

    // Reopened, a branch is its parent at the fork plus its own steps.
    let file2 = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    assert_eq!(
        text(file2.open_branch("acoustic").await.unwrap().loro()),
        "verse chorus"
    );

    // Merged back: both edits kept, as a step.
    main.merge(&acoustic).await.unwrap();
    assert_eq!(text(main.loro()), "intro verse chorus");
    assert_eq!(
        file.versions(MAIN).await.unwrap().last().unwrap().name,
        "Merge acoustic"
    );

    // A branch of a branch outlives its parent.
    let mut live = acoustic.fork("live").await.unwrap();
    live.transact("Outro", |d| type_at(d, 12, " outro"))
        .await
        .unwrap();
    file.delete_branch("acoustic").await.unwrap();
    assert!(matches!(
        file.delete_branch(MAIN).await,
        Err(FileError::DeleteMain)
    ));
    let reopened = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    let names: Vec<String> = reopened
        .branches()
        .await
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(names, [MAIN, "live"]);
    assert_eq!(reopened.branches().await[1].parent.as_deref(), Some(MAIN));
    assert_eq!(
        text(reopened.open_branch("live").await.unwrap().loro()),
        "verse chorus outro"
    );
    assert!(matches!(
        reopened.open_branch("acoustic").await,
        Err(FileError::NoBranch(_))
    ));
}

#[tokio::test]
async fn a_version_opens_read_only_and_restores_as_a_new_branch() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("One", |d| type_at(d, 0, "one"))
        .await
        .unwrap();
    main.checkpoint("First draft").await.unwrap();
    main.transact("Two", |d| type_at(d, 3, " two"))
        .await
        .unwrap();
    main.save("Second draft").await.unwrap();

    let versions = file.versions(MAIN).await.unwrap();
    let draft = versions
        .iter()
        .find(|v| v.name == "First draft")
        .unwrap()
        .clone();
    assert_eq!(draft.kind, VersionKind::Checkpoint);
    let old = file.open_version(&draft).await.unwrap();
    assert_eq!(text(&old), "one");
    assert!(old.is_detached(), "read-only");

    let mut restored = file.restore(&draft, "back to one").await.unwrap();
    assert_eq!(text(restored.loro()), "one");
    restored
        .transact("Three", |d| type_at(d, 3, " three"))
        .await
        .unwrap();
    // The branch it came from is as it was.
    assert_eq!(
        text(file.open_branch(MAIN).await.unwrap().loro()),
        "one two"
    );
    let reopened = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    assert_eq!(
        text(reopened.open_branch("back to one").await.unwrap().loro()),
        "one three"
    );

    file.delete_checkpoint(MAIN, "First draft").await.unwrap();
    assert!(
        !file
            .versions(MAIN)
            .await
            .unwrap()
            .iter()
            .any(|v| v.name == "First draft")
    );
}

#[tokio::test]
async fn two_handles_on_one_branch_lose_nothing_when_one_saves() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut a = file.open_branch(MAIN).await.unwrap();
    let mut b = file.open_branch(MAIN).await.unwrap();
    a.transact("A", |d| d.get_map("m").insert("a", 1).unwrap())
        .await
        .unwrap();
    b.transact("B", |d| d.get_map("m").insert("b", 2).unwrap())
        .await
        .unwrap();
    // A folds from the file, so B's step is in the snapshot.
    a.save("both").await.unwrap();
    let reopened = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    let main = reopened.open_branch(MAIN).await.unwrap();
    assert_eq!(main.loro().get_map("m").len(), 2);
    a.refresh().await.unwrap();
    assert_eq!(a.loro().get_map("m").len(), 2);
}

#[tokio::test]
async fn blobs_are_kept_once_by_hash_and_survive_a_fold() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let fx = vec![7_u8; 4096];
    let hash = file.put_blob("fx", &fx).await.unwrap();
    assert_eq!(hash, hash_of(&fx));
    let len = store.bytes().len();
    assert_eq!(file.put_blob("fx", &fx).await.unwrap(), hash);
    assert_eq!(store.bytes().len(), len, "not written twice");
    assert_eq!(file.blob(&hash).await.unwrap(), Some(fx.clone()));
    assert_eq!(file.blob(&hash_of(b"nope")).await.unwrap(), None);

    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("Add FX", |d| {
        d.get_map("fx").insert("chain", hash.as_str()).unwrap()
    })
    .await
    .unwrap();
    main.save("v1").await.unwrap();
    let reopened = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    assert_eq!(reopened.blobs().await.len(), 1);
    assert_eq!(reopened.blobs().await[0].size, 4096);
    assert_eq!(reopened.blob(&hash).await.unwrap(), Some(fx));
}

#[tokio::test]
async fn the_kind_is_checked_and_a_branch_is_upgraded_as_a_step() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("Title", |d| d.get_map("doc").insert("title", "x").unwrap())
        .await
        .unwrap();
    let bytes = store.bytes();

    let other = DocKind::new("test.chart", "Chart");
    assert!(matches!(
        DocFile::from_bytes(bytes.clone(), &other).await,
        Err(FileError::WrongKind { .. })
    ));

    let mut kinds = DocKinds::new();
    kinds.register(other);
    kinds.register(note());
    let any = DocFile::open_any(MemoryStorage::from_bytes(bytes.clone()), &kinds)
        .await
        .unwrap();
    assert_eq!(any.kind().id, "test.note");

    // A newer build: schema 2, with a migration from 1.
    let v2 = note().schema(
        2,
        vec![Migration {
            from: 1,
            name: "rename title",
            run: |doc| {
                let m = doc.get_map("doc");
                let title = m.get("title").ok_or("no title")?;
                let title = title.into_value().map_err(|_| "not a value")?;
                m.insert("name", title).map_err(|e| e.to_string())?;
                m.delete("title").map_err(|e| e.to_string())?;
                Ok(())
            },
        }],
    );
    let upgraded = MemoryStorage::from_bytes(bytes);
    let file = DocFile::open(upgraded.clone(), &v2).await.unwrap();
    let main = file.open_branch(MAIN).await.unwrap();
    assert!(main.loro().get_map("doc").get("name").is_some());
    assert_eq!(file.meta("schema").await.as_deref(), Some("2"));
    assert_eq!(
        file.versions(MAIN).await.unwrap().last().unwrap().name,
        "Upgrade to schema 2"
    );
    // The migration is not something to undo.
    assert_eq!(main.history().next_undo(), None);

    // And the build that wrote schema 1 refuses what schema 2 wrote.
    assert!(matches!(
        DocFile::open(MemoryStorage::from_bytes(upgraded.bytes()), &note()).await,
        Err(FileError::Schema(_))
    ));
}

#[tokio::test]
async fn bytes_that_are_not_a_document_are_refused() {
    assert!(matches!(
        DocFile::from_bytes(b"SQLite format 3\0........".to_vec(), &note()).await,
        Err(FileError::NotADocument(_))
    ));
    assert!(matches!(
        DocFile::from_bytes(Vec::new(), &note()).await,
        Err(FileError::NotADocument(_))
    ));
}

#[tokio::test]
async fn a_crdt_doc_keeps_its_replica_in_a_branch_of_the_file() {
    let store = MemoryStorage::new();
    let file = DocFile::create(store.clone(), &note()).await.unwrap();
    let mut main = file.open_branch(MAIN).await.unwrap();
    main.transact("Hello", |d| type_at(d, 0, "hello"))
        .await
        .unwrap();

    let id = Uuid::new_v4();
    let replica = CrdtDoc::open_shared(id, std::sync::Arc::new(file.persistence(MAIN)))
        .await
        .unwrap();
    assert_eq!(text(replica.loro()), "hello");
    type_at(replica.loro(), 5, " from sync");
    replica.loro().commit();
    // The replica's writer is fire-and-forget; wait for the step.
    for _ in 0..200 {
        if text(file.open_branch(MAIN).await.unwrap().loro()) == "hello from sync" {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        text(file.open_branch(MAIN).await.unwrap().loro()),
        "hello from sync"
    );
    replica.compact(id).await.unwrap();
    let reopened = DocFile::from_bytes(store.bytes(), &note()).await.unwrap();
    assert_eq!(
        text(reopened.open_branch(MAIN).await.unwrap().loro()),
        "hello from sync"
    );
}

#[tokio::test]
async fn memory_storage_is_a_storage() {
    // The trait is what a browser backend (IndexedDB) implements next.
    async fn round_trip(s: &dyn Storage) {
        s.replace(b"abc").await.unwrap();
        s.append(b"def").await.unwrap();
        s.truncate(4).await.unwrap();
        assert_eq!(s.read().await.unwrap(), b"abcd");
    }
    round_trip(&MemoryStorage::new()).await;
    let dir = tempfile::tempdir().unwrap();
    round_trip(&DiskStorage::open(dir.path().join("f")).unwrap()).await;
}
