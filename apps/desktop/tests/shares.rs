//! Share links on the headless node, end to end (#374): the issuer in
//! process (`apps/auth-server`, as `tests/node.rs` runs it), a node that
//! hosts the org `indie` (Grace its owner, Alan a member; Ada is in
//! another org), its vault holding a shared folder and private notes the
//! shared ones link to.
//!
//! - A folder's share serves that folder, and nothing outside it: a link
//!   to a private note is plain text, its embed and attachment are not
//!   there, and asking for them is `404`.
//! - Each audience lets in its people and no one else: public and link
//!   for anyone; org for the org's members (bearer or cookie), signing in
//!   for nobody, refused for others; invited for the people named.
//! - An expired share is `404`; a revoke is `404` on the very next
//!   request.
//! - A live edit (a signed-in web client's `DocSync` on the org lane)
//!   is on the shared page within seconds, and its `_live` stream fires.
//! - A share that takes comments takes them; a read-only one refuses.
//! - A share with a domain of its own is served at that host's root.
//! - A share with two audiences (#510) serves each only its notes and
//!   files: the other audience's page and file are `404` on its link,
//!   nothing of them is listed there, a note that leaves an audience's
//!   filter is `404` there, and an audience is removed alone.
//! - A link's files page switches between the views (in order, by note,
//!   by a property), each with the audience's filter applied; its
//!   "Download all" zip, streamed by the node, holds exactly the
//!   audience's files, in list order, in folders by the view; one group
//!   downloads alone. `SHARE_AUDIENCE_ADD` / `SHARE_AUDIENCES` /
//!   `SHARE_ARCHIVE` / `SHARE_AUDIENCE_REMOVE` do the same from the app.
//! - From the app (`architect-accounts`' Action List, signed in as Grace):
//!   `SHARE_CREATE` makes a link that opens, `SHARE_LIST` lists it,
//!   `PUBLISH_DOMAIN` gives it a domain, `SHARE_REVOKE` takes it down.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use architect_accounts::Issuer;
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_desktop::node::{self, Config, IrohMode, Running};
use architect_spaces::{
    Audience, NewAudience, NewShare, Share, ShareAccess, ShareChange, ShareLinksClient, ShareScope,
};
use auth_server::{ServerConfig, server};
use reqwest::StatusCode;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;

async fn issuer() -> String {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
        ..ServerConfig::local()
    };
    let auth = server::build_engine(&config, AuthSeaOrmStorage::new(db.clone())).unwrap();
    assert!(
        auth_server::dev::seed(&auth, &config.database_url)
            .await
            .unwrap()
    );
    let app = server::app_router_with_db(&config, auth, db).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await.unwrap() });
    origin
}

/// A session of `email`'s, written to `dir/session`; its token.
async fn session_of(origin: &str, dir: &Path, email: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let signed_in = Issuer::new(origin)
        .sign_in_password(email, auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    std::fs::write(dir.join("session"), &signed_in.session.token).unwrap();
    signed_in.session.token
}

fn touch(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

struct World {
    host: Running,
    issuer: String,
    grace: String,
    alan: String,
    ada: String,
    _tmp: tempfile::TempDir,
}

/// Grace's node hosting `indie`, its vault seeded; Alan's and Ada's sessions.
async fn world() -> World {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("host");
    let vault = dir.join("data/vaults/notes");
    touch(
        &vault,
        "Shared/index.md",
        "---\ntitle: The album\n---\nRead [[Private/plan|the plan]] and [[track]].\n\n![[cover.png]]\n\n![[art.png]]\n",
    );
    touch(&vault, "Shared/track.md", "# Track\n\nMix notes.\n");
    touch(&vault, "Shared/art.png", "shared png");
    touch(&vault, "Private/plan.md", "# Plan\n\nThe secret plan.\n");
    touch(&vault, "Private/cover.png", "private png");
    // A setlist (#510): a chart per part, each attaching its file.
    for (n, song, part) in [
        (1, "Joy to the World", "violin"),
        (2, "Joy to the World", "cello"),
        (3, "Silent Night", "violin"),
        (4, "Noel", "cello"),
    ] {
        let slug = song.split(' ').next().unwrap().to_lowercase();
        touch(
            &vault,
            &format!("Set/0{n}-{slug}-{part}.md"),
            &format!(
                "---\ntitle: {song} ({part})\nsong: {song}\ninstrument: {part}\n---\nThe {part} part of {song}.\n\n![[charts/{slug}-{part}.pdf]]\n"
            ),
        );
        touch(
            &vault,
            &format!("Set/charts/{slug}-{part}.pdf"),
            &format!("%PDF {song} for {part}"),
        );
    }
    let grace = session_of(&origin, &dir, "grace@local.test").await;
    let config = Config {
        name: "indie-host".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        issuer: Some(origin.clone()),
        session_file: Some(dir.join("session")),
        host_org: Some("indie".to_owned()),
        iroh: IrohMode::Local,
        sweep_secs: 3600,
        ..Config::default()
    };
    let host = node::start(config).await.unwrap();
    let alan = session_of(&origin, &tmp.path().join("alan"), "alan@local.test").await;
    let ada = session_of(&origin, &tmp.path().join("ada"), "ada@local.test").await;
    World {
        host,
        issuer: origin,
        grace,
        alan,
        ada,
        _tmp: tmp,
    }
}

impl World {
    async fn lane<C: vox::FromVoxLane>(&self, token: &str) -> C {
        let at =
            architect::connect::Endpoint::url(format!("ws://{}/org/indie/vox", self.host.http))
                .with_bearer(Some(token.to_owned()));
        architect::connect::Pool::new().client(&at).await.unwrap()
    }

    async fn share(&self, scope: ShareScope, audience: Audience, access: ShareAccess) -> Share {
        let links: ShareLinksClient = self.lane(&self.grace).await;
        links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope,
                title: None,
                audience,
                access,
                expires_at: None,
                domain: None,
            })
            .await
            .unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.host.http)
    }
}

/// GET `url`, with `auth` as the bearer (`Bearer …`) or a cookie
/// (`cookie:<token>`); status and body.
async fn get(url: &str, auth: Option<&str>) -> (StatusCode, reqwest::header::HeaderMap, String) {
    let mut req = reqwest::Client::new().get(url);
    match auth {
        Some(a) if a.starts_with("cookie:") => {
            let token = a.trim_start_matches("cookie:");
            req = req.header("cookie", format!("architect_session={token}"));
        }
        Some(token) => req = req.bearer_auth(token),
        None => {}
    }
    let res = req.send().await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.text().await.unwrap())
}

/// Wait until `url` answers 200 with `want` in it; its body.
async fn page_with(url: &str, auth: Option<&str>, want: &str) -> String {
    let mut last = String::new();
    for _ in 0..200 {
        let (status, _, body) = get(url, auth).await;
        if status == StatusCode::OK && body.contains(want) {
            return body;
        }
        last = format!("{status}: {body}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{url} never had `{want}`; last {last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_share_serves_the_folder_live_and_a_revoke_takes_it_down() {
    use crdt::sync::{DocSyncClient, SyncedDoc};
    let w = world().await;
    let share = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let link = format!("/s/{}/", share.code);

    // The folder, rooted at the link: its home note and its other note.
    let home = page_with(&w.url(&link), None, "The album").await;
    // Links are under `/s/` only (#431): the code at the root is not a
    // share, `/s/<code>` without its slash is sent to the link, and an
    // unknown code under `/s/` is 404 from the share layer.
    let (status, _, body) = get(&w.url(&format!("/{}/", share.code)), None).await;
    assert!(!body.contains("The album"), "{status}: {body}");
    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(w.url(&format!("/s/{}", share.code)))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(bare.headers()["location"], link.as_str());
    let (status, _, _) = get(&w.url("/s/abcdefghjkmnpqrs/"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let track = page_with(&w.url(&format!("{link}track/")), None, "Mix notes.").await;
    assert!(home.contains(&format!("href=\"{link}track/\"")), "{home}");
    assert!(track.contains("Track"));

    // Out of scope: the private note's link is plain text, its embed and
    // its attachment are gone, nothing names it, and asking is 404.
    assert!(
        home.contains("<span class=\"site-outside\">the plan</span>"),
        "{home}"
    );
    for leak in ["secret", "Private", "cover.png", "plan/"] {
        assert!(!home.contains(leak), "`{leak}` on the shared page: {home}");
    }
    for path in [
        "Private/plan/",
        "Private/cover.png",
        "plan/",
        "_architect/shares/",
        "%2e%2e/%2e%2e/vaults/notes/Private/plan.md",
    ] {
        let (status, _, body) = get(&w.url(&format!("{link}{path}")), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert!(!body.contains("secret plan"), "{path}");
    }
    let (status, _, art) = get(&w.url(&format!("{link}Shared/art.png")), None).await;
    assert_eq!((status, art.as_str()), (StatusCode::OK, "shared png"));
    // The link's answers keep the code to themselves.
    let (_, headers, _) = get(&w.url(&link), None).await;
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert!(
        headers["x-robots-tag"]
            .to_str()
            .unwrap()
            .contains("noindex")
    );

    // The share is a note of the vault, hidden from every Site.
    let note = w
        .host
        .state
        .vault_root("notes")
        .unwrap()
        .join(format!("_architect/shares/{}.md", share.code));
    assert!(
        std::fs::read_to_string(&note)
            .unwrap()
            .contains("publish: false")
    );

    // A live edit, from a signed-in web client on the org lane: on the
    // shared page in seconds, and the page's event stream fires.
    let mut events = reqwest::Client::new()
        .get(w.url(&format!("{link}_live")))
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let sync: DocSyncClient = w.lane(&w.alan).await;
    let doc_id = vault_proto::collab_doc_id("notes", "Shared/track.md");
    let replica = crdt::CrdtDoc::ephemeral();
    let mut synced = SyncedDoc::new(doc_id, replica.clone());
    let session = tokio::spawn(async move {
        let _ = synced.run(&sync).await;
    });
    for _ in 0..200 {
        let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
        if text.to_string().contains("Mix notes.") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
    text.insert(text.len_unicode(), "\nTyped live in the browser.\n")
        .unwrap();
    replica.loro().commit();
    let started = std::time::Instant::now();
    page_with(
        &w.url(&format!("{link}track/")),
        None,
        "Typed live in the browser.",
    )
    .await;
    let took = started.elapsed();
    assert!(took < Duration::from_secs(8), "took {took:?}");
    let fired = tokio::time::timeout(Duration::from_secs(5), async {
        let mut seen = String::new();
        while let Ok(Some(chunk)) = events.chunk().await {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("data: rebuilt") {
                return seen;
            }
        }
        seen
    })
    .await
    .unwrap();
    assert!(fired.contains("data: rebuilt"), "{fired}");
    session.abort();

    // Revoked: 404 on the next request, and listed as revoked.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let revoked = links.revoke(share.code.clone()).await.unwrap();
    assert!(revoked.revoked_at.is_some());
    let (status, _, body) = get(&w.url(&link), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.contains("The album"));
    let listed = links.shares().await.unwrap();
    assert!(
        listed
            .iter()
            .any(|s| s.code == share.code && s.revoked_at.is_some())
    );
    // A revoke is final.
    assert!(
        links
            .update(share.code.clone(), ShareChange::Expires(None))
            .await
            .is_err()
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_audience_lets_in_its_people_and_no_one_else() {
    let w = world().await;
    let scope = || ShareScope::Notes(vec!["Shared/track.md".to_owned()]);

    // The org's members, signed in: bearer or cookie. Nobody: sign in.
    // Someone in another org: refused.
    let org = w.share(scope(), Audience::Org, ShareAccess::Read).await;
    let link = w.url(&format!("/s/{}/", org.code));
    let alan_cookie = format!("cookie:{}", w.alan);
    page_with(&link, Some(&w.alan), "Mix notes.").await;
    page_with(&link, Some(&alan_cookie), "Mix notes.").await;
    page_with(&link, Some(&w.grace), "Mix notes.").await;
    let (status, headers, body) = get(&link, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("/s/_signin?to="), "{body}");
    assert!(!body.contains("Mix notes."));
    assert_eq!(headers["cache-control"], "no-store");
    let (status, _, body) = get(&link, Some(&w.ada)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!body.contains("Mix notes."));
    let (status, _, _) = get(&link, Some("not-a-session")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A signed-in share's pages are cached by nobody.
    let (_, headers, _) = get(&link, Some(&w.alan)).await;
    assert_eq!(headers["cache-control"], "private, no-store");

    // Invited people: Ada, by email, though she is in no org of this
    // node's; not Alan, a member.
    let invited = w
        .share(
            scope(),
            Audience::Invited(vec!["ADA@local.test".to_owned()]),
            ShareAccess::Read,
        )
        .await;
    let link = w.url(&format!("/s/{}/", invited.code));
    page_with(&link, Some(&w.ada), "Mix notes.").await;
    assert_eq!(get(&link, Some(&w.alan)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(get(&link, None).await.0, StatusCode::UNAUTHORIZED);

    // Public: anyone, and it may be listed.
    let public = w.share(scope(), Audience::Public, ShareAccess::Read).await;
    let link = w.url(&format!("/s/{}/", public.code));
    page_with(&link, None, "Mix notes.").await;
    assert!(!get(&link, None).await.1.contains_key("x-robots-tag"));

    // Only the org's people make shares; a member lists them.
    let ada: ShareLinksClient = w.lane(&w.ada).await;
    assert!(ada.shares().await.is_err());
    let alan: ShareLinksClient = w.lane(&w.alan).await;
    assert_eq!(alan.shares().await.unwrap().len(), 3);

    // Signing in starts the issuer's device sign-in: an approval link,
    // and the node never asks for a password.
    let (status, headers, body) =
        get(&w.url(&format!("/s/_signin?to=/s/{}/", org.code)), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Approve sign-in"), "{body}");
    assert!(!body.to_lowercase().contains("password"), "{body}");
    let set = headers["set-cookie"].to_str().unwrap();
    assert!(
        set.starts_with("architect_signin=") && set.contains("HttpOnly"),
        "{set}"
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_share_expires_takes_comments_when_asked_and_has_its_own_domain() {
    let w = world().await;
    let links: ShareLinksClient = w.lane(&w.grace).await;

    // Comments: on a share that takes them, under the page.
    let talk = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Comment,
        )
        .await;
    let link = format!("/s/{}/", talk.code);
    page_with(&w.url(&link), None, "The album").await;
    let res = reqwest::Client::new()
        .post(w.url(&format!("{link}_comment")))
        .form(&[
            ("page", "/track/"),
            ("text", "Love the <b>bridge</b>."),
            ("name", "Sam"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "followed to the page");
    let track = page_with(
        &w.url(&format!("{link}track/")),
        None,
        "Love the &lt;b&gt;bridge&lt;/b&gt;.",
    )
    .await;
    assert!(track.contains("Sam (not signed in)"), "{track}");
    let comments = links.comments(talk.code.clone()).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].page, "/track/");

    // Read only: no comments.
    let read = w
        .share(
            ShareScope::Folder("Shared".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    page_with(&w.url(&format!("/s/{}/", read.code)), None, "The album").await;
    let res = reqwest::Client::new()
        .post(w.url(&format!("/s/{}/_comment", read.code)))
        .form(&[("page", "/"), ("text", "hi")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // A domain of its own: the share at that host's root.
    links
        .update(
            read.code.clone(),
            ShareChange::Domain(Some("album.example.test".to_owned())),
        )
        .await
        .unwrap();
    let mut body = String::new();
    for _ in 0..200 {
        let res = reqwest::Client::new()
            .get(w.url("/track/"))
            .header("host", "album.example.test")
            .send()
            .await
            .unwrap();
        if res.status() == StatusCode::OK {
            body = res.text().await.unwrap();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(body.contains("Mix notes."), "{body}");
    assert!(body.contains("href=\"/\""), "rooted at the domain: {body}");

    // Expiry: open until then, 404 from then on.
    let soon = links
        .create(NewShare {
            vault: "notes".to_owned(),
            scope: ShareScope::Folder("Shared".to_owned()),
            title: Some("For a moment".to_owned()),
            audience: Audience::Link,
            access: ShareAccess::Read,
            expires_at: Some(node::shares::now() + 2),
            domain: None,
        })
        .await
        .unwrap();
    let link = w.url(&format!("/s/{}/", soon.code));
    page_with(&link, None, "The album").await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let (status, _, body) = get(&link, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // An expiry already past is refused.
    assert!(
        links
            .create(NewShare {
                vault: "notes".to_owned(),
                scope: ShareScope::Vault,
                title: None,
                audience: Audience::Link,
                access: ShareAccess::Read,
                expires_at: Some(1),
                domain: None,
            })
            .await
            .is_err()
    );

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_share_actions_make_list_and_revoke_a_link() {
    use std::sync::Arc;

    use architect::action::ActionList;
    use architect_accounts::{AccountStore, Accounts, MemorySecrets};
    use serde_json::{Value, json};

    let w = world().await;
    // The app, signed in as Grace, in `indie`.
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(&w.issuer), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password("grace@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    let run = |id: &'static str, args: Value| {
        let list = Arc::clone(&list);
        async move {
            list.run_json_async(id, args.to_string().as_bytes())
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
                .map_err(|e| format!("{e:?}"))
        }
    };
    let host = format!("http://{}", w.host.http);

    let made = run(
        "SHARE_CREATE",
        json!({"folder": "Shared", "audience": "link", "expires": "7d", "host": host}),
    )
    .await
    .unwrap();
    let url = made["share"]["url"].as_str().unwrap().to_owned();
    let code = made["share"]["code"].as_str().unwrap().to_owned();
    assert_eq!(url, format!("{host}/s/{code}/"));
    assert_eq!(made["share"]["scope"], "the folder Shared");
    assert_eq!(made["share"]["state"], "live");
    page_with(&url, None, "The album").await;

    let listed = run("SHARE_LIST", json!({})).await.unwrap();
    assert_eq!(listed["shares"].as_array().unwrap().len(), 1, "{listed}");

    let changed = run("SHARE_UPDATE", json!({"code": code, "audience": "org"}))
        .await
        .unwrap();
    assert_eq!(changed["share"]["audience"], "org");
    assert_eq!(get(&url, None).await.0, StatusCode::UNAUTHORIZED);
    page_with(&url, Some(&w.alan), "The album").await;

    let domained = run(
        "PUBLISH_DOMAIN",
        json!({"code": code, "domain": "album.example.test"}),
    )
    .await
    .unwrap();
    assert_eq!(domained["share"]["domain"], "album.example.test");

    // The sheet opens on a scope (from a file's context menu it would be
    // that file).
    let sheet = run("SHARE_SHEET", json!({"tag": "mix"})).await.unwrap();
    assert_eq!(sheet["sheet"], "share");
    assert_eq!(sheet["scope"], "the notes tagged #mix");

    run("SHARE_REVOKE", json!({"code": code})).await.unwrap();
    let (status, _, _) = get(&url, Some(&w.alan)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let listed = run("SHARE_LIST", json!({})).await.unwrap();
    assert_eq!(listed["shares"][0]["state"], "revoked");

    w.host.stop().await;
}

/// The setlist shared with two audiences; the share, the violins' link
/// and the cellos' (each `/s/<code>/`).
async fn setlist(w: &World) -> (Share, String, String) {
    let share = w
        .share(
            ShareScope::Folder("Set".to_owned()),
            Audience::Link,
            ShareAccess::Read,
        )
        .await;
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let add = |name: &str, filter: &str| {
        links.add_audience(
            share.code.clone(),
            NewAudience {
                name: name.to_owned(),
                filter: filter.to_owned(),
            },
        )
    };
    add("violin", "instrument == \"violin\"").await.unwrap();
    let share = add("cello", "instrument == \"cello\"").await.unwrap();
    let path = |name: &str| share.audience_named(name).unwrap().path();
    let (violin, cello) = (path("violin"), path("cello"));
    (share, violin, cello)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_audiences_are_each_served_only_their_own_notes_and_files() {
    let w = world().await;
    let (share, violin, cello) = setlist(&w).await;
    let whole = share.path();
    assert_ne!(violin, cello);
    assert_ne!(violin, whole);
    let status = |url: String| async move { get(&url, None).await.0 };

    // The share's own link is the whole set: every chart and every file.
    page_with(&w.url(&format!("{whole}04-noel-cello/")), None, "Noel").await;
    for page in ["01-joy-violin", "02-joy-cello", "03-silent-violin"] {
        assert_eq!(
            status(w.url(&format!("{whole}{page}/"))).await,
            StatusCode::OK
        );
    }
    for file in ["joy-violin", "joy-cello", "silent-violin", "noel-cello"] {
        let url = w.url(&format!("{whole}Set/charts/{file}.pdf"));
        assert_eq!(status(url).await, StatusCode::OK, "{file}");
    }

    // Each audience's link serves its charts and their files…
    let mine = [
        (
            &violin,
            ["01-joy-violin", "03-silent-violin"],
            ["joy-violin", "silent-violin"],
        ),
        (
            &cello,
            ["02-joy-cello", "04-noel-cello"],
            ["joy-cello", "noel-cello"],
        ),
    ];
    for (link, pages, _) in &mine {
        page_with(&w.url(&format!("{link}{}/", pages[1])), None, "part of").await;
    }
    for (i, (link, pages, files)) in mine.iter().enumerate() {
        let (_, theirs_pages, theirs_files) = &mine[1 - i];
        let mut seen = String::new();
        for page in pages {
            let (code, _, body) = get(&w.url(&format!("{link}{page}/")), None).await;
            assert_eq!(code, StatusCode::OK, "{link}{page}");
            seen.push_str(&body);
        }
        for file in files {
            let url = w.url(&format!("{link}Set/charts/{file}.pdf"));
            let (code, headers, body) = get(&url, None).await;
            assert_eq!(code, StatusCode::OK, "{url}");
            assert_eq!(headers["content-type"], "application/pdf");
            assert!(body.starts_with("%PDF"), "{body}");
        }
        // …and the other audience's are 404 on it: the page route (with
        // and without its slash, and by its file), and the byte route.
        for page in theirs_pages {
            for path in [
                format!("{page}/"),
                page.to_string(),
                format!("{page}/index.html"),
            ] {
                let url = w.url(&format!("{link}{path}"));
                assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
            }
        }
        for file in theirs_files {
            for path in [
                format!("Set/charts/{file}.pdf"),
                format!("Set/charts/{file}%2Epdf"),
                format!("Set/charts/../charts/{file}.pdf"),
            ] {
                let url = w.url(&format!("{link}{path}"));
                assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
            }
        }
        // Nothing of the other audience is listed: not in a page, its
        // nav, the tags or the search index.
        let (code, _, tags) = get(&w.url(&format!("{link}tags/")), None).await;
        assert_eq!(code, StatusCode::OK);
        seen.push_str(&tags);
        let asset = seen
            .split('"')
            .find(|s| s.contains("_site/search."))
            .expect("a search index")
            .to_owned();
        let (code, _, index) = get(&w.url(&asset), None).await;
        assert_eq!(code, StatusCode::OK, "{asset}");
        seen.push_str(&index);
        let other = if i == 0 { "cello" } else { "violin" };
        assert!(
            !seen.to_lowercase().contains(other),
            "{other} in {link}: {seen}"
        );
    }

    // A chart that leaves an audience's filter is 404 on its link, and
    // the audience it joins has it.
    let root = w.host.state.vault_root("notes").unwrap();
    touch(
        &root,
        "Set/03-silent-violin.md",
        "---\ntitle: Silent Night (cello)\nsong: Silent Night\ninstrument: cello\n---\nNow the cello part of Silent Night.\n\n![[charts/silent-violin.pdf]]\n",
    );
    page_with(
        &w.url(&format!("{cello}03-silent-violin/")),
        None,
        "Now the cello",
    )
    .await;
    assert_eq!(
        status(w.url(&format!("{cello}Set/charts/silent-violin.pdf"))).await,
        StatusCode::OK
    );
    for path in ["03-silent-violin/", "Set/charts/silent-violin.pdf"] {
        let url = w.url(&format!("{violin}{path}"));
        assert_eq!(status(url.clone()).await, StatusCode::NOT_FOUND, "{url}");
    }
    assert_eq!(
        status(w.url(&format!("{violin}01-joy-violin/"))).await,
        StatusCode::OK
    );

    // An audience is removed alone: its link is 404 on the next request,
    // the other's and the share's own stay.
    let links: ShareLinksClient = w.lane(&w.grace).await;
    let left = links
        .remove_audience(share.code.clone(), "Violin".to_owned())
        .await
        .unwrap();
    assert_eq!(left.audiences.len(), 1);
    assert_eq!(status(w.url(&violin)).await, StatusCode::NOT_FOUND);
    assert_eq!(
        status(w.url(&format!("{violin}01-joy-violin/"))).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(w.url(&format!("{cello}04-noel-cello/"))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(w.url(&format!("{whole}01-joy-violin/"))).await,
        StatusCode::OK
    );
    // A name is free again, a name taken is refused, and so is a filter
    // that is not a Bases filter.
    let bad = |name: &str, filter: &str| {
        links.add_audience(
            share.code.clone(),
            NewAudience {
                name: name.to_owned(),
                filter: filter.to_owned(),
            },
        )
    };
    assert!(bad("cello", "instrument == \"cello\"").await.is_err());
    assert!(bad("brass", "instrument ==").await.is_err());
    let again = bad("violin", "instrument == \"violin\"").await.unwrap();
    assert_ne!(again.audience_named("violin").unwrap().path(), violin);

    // The whole share revoked takes every audience's link with it.
    links.revoke(share.code.clone()).await.unwrap();
    assert_eq!(
        status(w.url(&format!("{cello}04-noel-cello/"))).await,
        StatusCode::NOT_FOUND
    );

    w.host.stop().await;
}

/// GET `url`; status, headers and the body's bytes.
async fn get_bytes(url: &str) -> (StatusCode, reqwest::header::HeaderMap, Vec<u8>) {
    let res = reqwest::Client::new().get(url).send().await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.bytes().await.unwrap().to_vec())
}

/// The zip at `url`: each file's path and text, in the archive's order.
async fn zip_at(url: &str) -> Vec<(String, String)> {
    let (status, headers, bytes) = get_bytes(url).await;
    assert_eq!(status, StatusCode::OK, "{url}");
    assert_eq!(headers["content-type"], "application/zip");
    // Streamed: the node never knew its length.
    assert!(headers.get("content-length").is_none(), "{headers:?}");
    architect_sites::archive::read_stored(&bytes)
        .unwrap_or_else(|| panic!("{url}: not a zip ({} bytes)", bytes.len()))
        .into_iter()
        .map(|(name, bytes)| (name, String::from_utf8(bytes).unwrap()))
        .collect()
}

fn pairs(files: &[(&str, &str)]) -> Vec<(String, String)> {
    files
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_links_files_page_switches_views_and_its_zip_holds_exactly_its_files_in_order() {
    let w = world().await;
    let (share, violin, cello) = setlist(&w).await;
    let whole = share.path();
    page_with(
        &w.url(&format!("{violin}03-silent-violin/")),
        None,
        "part of",
    )
    .await;
    page_with(&w.url(&format!("{cello}04-noel-cello/")), None, "part of").await;

    // A page of the link points at its files page and its archive.
    let (_, _, chart) = get(&w.url(&format!("{violin}01-joy-violin/")), None).await;
    assert!(
        chart.contains(&format!("href=\"{violin}_files/\"")),
        "{chart}"
    );
    assert!(
        chart.contains(&format!("href=\"{violin}_archive.zip?by=note\"")),
        "{chart}"
    );

    // The files page, in list order: the violins' charts and no others,
    // each linked to its page and its file; the other views a click away.
    let files = format!("{violin}_files/");
    let (status, _, order) = get(&w.url(&files), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(order.contains("data-view=\"order\""), "{order}");
    for view in ["order", "note", "property%3Ainstrument", "property%3Asong"] {
        assert!(
            order.contains(&format!("href=\"{files}?by={view}\"")),
            "{view}: {order}"
        );
    }
    assert!(order.contains(&format!("href=\"{files}?by=order\" aria-current=\"page\"")));
    let at = |html: &str, what: &str| {
        html.find(what)
            .unwrap_or_else(|| panic!("nothing says {what} in {html}"))
    };
    assert!(at(&order, "Joy to the World (violin)") < at(&order, "Silent Night (violin)"));
    assert!(
        order.contains(&format!("href=\"{violin}01-joy-violin/\"")),
        "{order}"
    );
    assert!(
        order.contains(&format!("href=\"{violin}Set/charts/joy-violin.pdf\"")),
        "{order}"
    );
    assert!(order.contains("Download all (2 files)"), "{order}");
    assert!(!order.to_lowercase().contains("cello"), "{order}");

    // By note: a group per chart, each with its own download.
    let (_, _, by_note) = get(&w.url(&format!("{files}?by=note")), None).await;
    assert!(by_note.contains("data-view=\"note\""));
    assert!(
        by_note.contains("<h2>Silent Night (violin)</h2>"),
        "{by_note}"
    );
    assert!(
        by_note.contains("_archive.zip?by=note&amp;group=Silent%20Night%20%28violin%29"),
        "{by_note}"
    );
    // By a property: a group per value, of this audience's notes only.
    let (_, _, by_song) = get(&w.url(&format!("{files}?by=property:song")), None).await;
    assert!(by_song.contains("data-view=\"property:song\""));
    assert!(
        by_song.contains("<h2>Joy to the World</h2>") && by_song.contains("<h2>Silent Night</h2>")
    );
    assert!(!by_song.contains("<h2>Noel</h2>") && !by_song.to_lowercase().contains("cello"));
    // The share's own link groups everyone.
    let (_, _, everyone) = get(
        &w.url(&format!("{whole}_files/?by=property:instrument")),
        None,
    )
    .await;
    assert!(everyone.contains("<h2>violin</h2>") && everyone.contains("<h2>cello</h2>"));
    assert!(everyone.contains("Download all (4 files)"), "{everyone}");
    let (status, _, _) = get(&w.url(&format!("{files}?by=shuffle")), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The violins' zip: exactly their files, a numbered folder per chart
    // in list order, each file's own bytes.
    let joy = "%PDF Joy to the World for violin";
    let silent = "%PDF Silent Night for violin";
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip?by=note"))).await,
        pairs(&[
            ("01 Joy to the World (violin)/joy-violin.pdf", joy),
            ("02 Silent Night (violin)/silent-violin.pdf", silent),
        ])
    );
    // In order: numbered files. By a property: a folder per value.
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip"))).await,
        pairs(&[("01 joy-violin.pdf", joy), ("02 silent-violin.pdf", silent)])
    );
    assert_eq!(
        zip_at(&w.url(&format!("{violin}_archive.zip?by=property:song"))).await,
        pairs(&[
            (
                "Joy to the World/01 Joy to the World (violin)/joy-violin.pdf",
                joy
            ),
            (
                "Silent Night/01 Silent Night (violin)/silent-violin.pdf",
                silent
            ),
        ])
    );
    // The cellos' holds theirs, and none of the violins'.
    assert_eq!(
        zip_at(&w.url(&format!("{cello}_archive.zip?by=note"))).await,
        pairs(&[
            (
                "01 Joy to the World (cello)/joy-cello.pdf",
                "%PDF Joy to the World for cello"
            ),
            ("02 Noel (cello)/noel-cello.pdf", "%PDF Noel for cello"),
        ])
    );
    // One group alone: one song of the whole set, both parts, in order.
    let (status, headers, bytes) = get_bytes(&w.url(&format!(
        "{whole}_archive.zip?by=property:song&group=Joy%20to%20the%20World"
    )))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("Joy to the World.zip"),
        "{headers:?}"
    );
    let names: Vec<String> = architect_sites::archive::read_stored(&bytes)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        [
            "Joy to the World/01 Joy to the World (violin)/joy-violin.pdf",
            "Joy to the World/02 Joy to the World (cello)/joy-cello.pdf"
        ]
    );
    // A group the audience does not have is not there to download, and
    // a group of another audience's is no way round its filter.
    for group in ["Nowhere", "Noel"] {
        let url = w.url(&format!(
            "{violin}_archive.zip?by=property:song&group={group}"
        ));
        assert_eq!(get_bytes(&url).await.0, StatusCode::NOT_FOUND, "{url}");
    }

    w.host.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_apps_audience_actions_add_list_archive_and_remove() {
    use std::sync::Arc;

    use architect::action::ActionList;
    use architect_accounts::{AccountStore, Accounts, MemorySecrets};
    use serde_json::{Value, json};

    let w = world().await;
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(&w.issuer), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password("grace@local.test", auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    let run = |id: &'static str, args: Value| {
        let list = Arc::clone(&list);
        async move {
            list.run_json_async(id, args.to_string().as_bytes())
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap())
                .map_err(|e| format!("{e:?}"))
        }
    };
    let host = format!("http://{}", w.host.http);
    let made = run(
        "SHARE_CREATE",
        json!({"folder": "Set", "title": "Carols", "host": host}),
    )
    .await
    .unwrap();
    let code = made["share"]["code"].as_str().unwrap().to_owned();
    assert_eq!(made["share"]["audiences"], json!([]));

    let added = run(
        "SHARE_AUDIENCE_ADD",
        json!({"code": code, "name": "violin", "filter": "instrument == \"violin\""}),
    )
    .await
    .unwrap();
    let url = added["audience"]["url"].as_str().unwrap().to_owned();
    let audience_code = added["audience"]["code"].as_str().unwrap();
    assert_eq!(url, format!("{host}/s/{audience_code}/"));
    assert_ne!(audience_code, code);
    assert_eq!(added["share"]["audiences"][0]["name"], "violin");
    run(
        "SHARE_AUDIENCE_ADD",
        json!({"code": code, "name": "everyone"}),
    )
    .await
    .unwrap();
    assert!(
        run(
            "SHARE_AUDIENCE_ADD",
            json!({"code": code, "name": "bad", "filter": "=="})
        )
        .await
        .is_err()
    );
    let listed = run("SHARE_AUDIENCES", json!({"code": code})).await.unwrap();
    let names: Vec<&str> = listed["audiences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["violin", "everyone"]);
    assert_eq!(listed["audiences"][0]["filter"], "instrument == \"violin\"");
    page_with(&format!("{url}03-silent-violin/"), None, "part of").await;
    assert_eq!(
        get(&format!("{url}04-noel-cello/"), None).await.0,
        StatusCode::NOT_FOUND
    );

    // The same zip the link gives, saved by the app.
    let tmp = tempfile::tempdir().unwrap();
    let to = tmp.path().join("violins.zip");
    let saved = run(
        "SHARE_ARCHIVE",
        json!({"code": code, "audience": "violin", "to": to.to_string_lossy()}),
    )
    .await
    .unwrap();
    assert_eq!(saved["files"], 2, "{saved}");
    let bytes = std::fs::read(&to).unwrap();
    assert_eq!(saved["bytes"], bytes.len());
    assert_eq!(
        bytes,
        get_bytes(&format!("{url}_archive.zip?by=note")).await.2
    );
    let names: Vec<String> = architect_sites::archive::read_stored(&bytes)
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        [
            "01 Joy to the World (violin)/joy-violin.pdf",
            "02 Silent Night (violin)/silent-violin.pdf"
        ]
    );
    // The whole share's, one group of it.
    let one = tmp.path().join("noel.zip");
    let saved = run(
        "SHARE_ARCHIVE",
        json!({"code": code, "by": "property:song", "group": "Noel", "to": one.to_string_lossy()}),
    )
    .await
    .unwrap();
    assert_eq!(saved["files"], 1, "{saved}");
    // No such audience or group: an error, and no file left behind.
    let none = tmp.path().join("none.zip");
    for args in [
        json!({"code": code, "audience": "brass", "to": none.to_string_lossy()}),
        json!({"code": code, "by": "property:song", "group": "Nowhere", "to": none.to_string_lossy()}),
    ] {
        assert!(run("SHARE_ARCHIVE", args).await.is_err());
        assert!(!none.exists());
    }

    let left = run(
        "SHARE_AUDIENCE_REMOVE",
        json!({"code": code, "name": "violin"}),
    )
    .await
    .unwrap();
    assert_eq!(left["audiences"].as_array().unwrap().len(), 1, "{left}");
    assert_eq!(get(&url, None).await.0, StatusCode::NOT_FOUND);
    assert!(
        run(
            "SHARE_AUDIENCE_REMOVE",
            json!({"code": code, "name": "violin"})
        )
        .await
        .is_err()
    );

    w.host.stop().await;
}
