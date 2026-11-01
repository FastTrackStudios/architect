//! The scope a read is in, the org-scoped paths and document ids, the
//! All-organizations read and the refresh on reconnect.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_orgs::{
    Aggregate, Org, OrgId, Orgs, Refresh, Role, Scope, Scoped, Selection, read_all,
};

fn orgs() -> Orgs {
    Orgs::new(vec![
        Org::new("fts", "FastTrackStudio", Role::Admin),
        Org::new("hp", "Harbor & Pine", Role::Viewer),
        Org::new("me", "Personal", Role::Owner).personal(),
    ])
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn the_window_scope_is_its_org_or_every_org_in_all() {
    let mut o = orgs();
    assert_eq!(o.scope(), Scope::One(OrgId::new("me")));
    assert_eq!(o.scope().writes_to(), Some(&OrgId::new("me")));
    o.switch(Selection::All);
    let all = o.scope();
    assert!(all.is_all());
    assert_eq!(all.orgs().len(), 3);
    assert!(all.contains(&OrgId::new("hp")));
    // Writing in All picks an org first.
    assert_eq!(all.writes_to(), None);
}

#[test]
fn an_orgs_data_lives_under_its_path_and_its_documents_are_its_own() {
    let fts = OrgId::new("fts");
    assert_eq!(fts.path("files/Songs"), "org/fts/files/Songs");
    assert_eq!(fts.path("/settings"), "org/fts/settings");
    assert_eq!(fts.path(""), "org/fts");
    assert_eq!(
        OrgId::of_path("org/fts/files/Songs"),
        Some((OrgId::new("fts"), "files/Songs"))
    );
    assert_eq!(OrgId::of_path("org/fts"), Some((OrgId::new("fts"), "")));
    assert_eq!(OrgId::of_path("vault/notes"), None);
    assert_eq!(OrgId::of_path("org/"), None);
    // The roster is the same document everywhere, and each org's own.
    assert_eq!(fts.doc_id("presence"), OrgId::new("fts").doc_id("presence"));
    assert_ne!(fts.doc_id("presence"), OrgId::new("hp").doc_id("presence"));
    assert_ne!(fts.doc_id("presence"), fts.doc_id("catalogue"));
}

#[test]
fn reading_all_asks_every_org_and_one_failing_hides_none_of_the_others() {
    let mut o = orgs();
    o.switch(Selection::All);
    let found: Aggregate<String, String> = block_on(read_all(&o.scope(), |org| async move {
        if org.0 == "hp" {
            Err("offline".to_owned())
        } else {
            Ok(vec![format!("{}-song", org.0)])
        }
    }));
    assert_eq!(
        found.items,
        vec![
            Scoped {
                org: OrgId::new("fts"),
                item: "fts-song".to_owned()
            },
            Scoped {
                org: OrgId::new("me"),
                item: "me-song".to_owned()
            },
        ]
    );
    assert_eq!(found.failed, vec![(OrgId::new("hp"), "offline".to_owned())]);
    assert!(!found.complete());

    // Switching to one org reads only it.
    o.switch(Selection::One(OrgId::new("fts")));
    let one: Aggregate<String, String> =
        block_on(read_all(&o.scope(), |org| async move { Ok(vec![org.0]) }));
    assert_eq!(one.items.len(), 1);
    assert!(one.complete());
}

#[test]
fn the_list_is_fetched_once_per_connection_and_a_failure_keeps_the_cache() {
    let mut r = Refresh::default();
    assert!(r.due(1));
    let fresh = r.finish(1, Ok(vec![Org::new("fts", "FastTrackStudio", Role::Admin)]));
    assert_eq!(fresh.unwrap().len(), 1);
    assert_eq!(r.fetched(), Some(1));
    // The same connection does not ask again; a reconnect does.
    assert!(!r.due(1));
    assert!(r.due(2));
    // Offline after the reconnect: nothing to replace the cache with.
    assert_eq!(r.finish(2, Err("connection refused".to_owned())), None);
    assert_eq!(r.error(), Some("connection refused"));
    assert!(r.due(2));
    assert!(r.finish(2, Ok(Vec::new())).is_some());
    assert_eq!(r.error(), None);
}
