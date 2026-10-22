//! `FileKv` as architect-atom's `Kv`: `KvRows<FileKv>` stores rows and
//! the call queue in files, under the same keys `LocalStorageRows` uses
//! in the browser.

use architect::settings::FileKv;
use architect::{Kv, KvRows, LocalRow, LocalRows, MemoryKv};
use futures_lite::future::block_on;

fn row(key: &str, json: &str) -> LocalRow {
    LocalRow {
        key: key.into(),
        owner: None,
        json: json.into(),
    }
}

#[test]
fn file_kv_is_a_kv() {
    let dir = tempfile::tempdir().unwrap();
    let kv = FileKv::new(dir.path());
    assert_eq!(Kv::get(&kv, "rows.task.org/1"), None);
    Kv::set(&kv, "rows.task.org/1", "[1]").unwrap();
    assert_eq!(Kv::get(&kv, "rows.task.org/1").as_deref(), Some("[1]"));
    assert!(dir.path().join("rows.task.org%2F1").is_file());
    Kv::remove(&kv, "rows.task.org/1");
    assert_eq!(Kv::get(&kv, "rows.task.org/1"), None);
    Kv::remove(&kv, "never-set");
}

#[test]
fn kv_rows_over_files_round_trip_and_match_memory_keys() {
    let dir = tempfile::tempdir().unwrap();
    let files = KvRows::new(FileKv::new(dir.path()), "app/");
    let memory = KvRows::new(MemoryKv::default(), "app/");

    block_on(async {
        let both: [&dyn LocalRows; 2] = [&files, &memory];
        for rows in both {
            rows.replace("task", "org-1", vec![row("a", "{\"n\":1}")])
                .await
                .unwrap();
            rows.upsert("task", "org-1", row("b", "{\"n\":2}"))
                .await
                .unwrap();
            rows.queue_push("call-1".into()).await.unwrap();
        }
        let got = files.load("task", "org-1").await.unwrap().unwrap();
        assert_eq!(got, vec![row("a", "{\"n\":1}"), row("b", "{\"n\":2}")]);
        assert_eq!(files.load("task", "other").await.unwrap(), None);
        assert_eq!(
            files.queue_list().await.unwrap(),
            memory.queue_list().await.unwrap()
        );

        // A fresh handle on the same dir sees everything: it is durable.
        let reopened = KvRows::new(FileKv::new(dir.path()), "app/");
        assert_eq!(reopened.load("task", "org-1").await.unwrap().unwrap(), got);
        assert_eq!(
            reopened.load("task", "org-1").await.unwrap(),
            memory.load("task", "org-1").await.unwrap()
        );
    });

    // The same keys the browser's localStorage would hold, one file each.
    let mut names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["app%2Fqueue", "app%2Frows.task.org-1"]);
}
