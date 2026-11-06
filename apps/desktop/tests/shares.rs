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
    Audience, NewShare, Share, ShareAccess, ShareChange, ShareLinksClient, ShareScope,
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
    let link = format!("/{}/", share.code);

    // The folder, rooted at the link: its home note and its other note.
    let home = page_with(&w.url(&link), None, "The album").await;
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
    let link = w.url(&format!("/{}/", org.code));
    let alan_cookie = format!("cookie:{}", w.alan);
    page_with(&link, Some(&w.alan), "Mix notes.").await;
    page_with(&link, Some(&alan_cookie), "Mix notes.").await;
    page_with(&link, Some(&w.grace), "Mix notes.").await;
    let (status, headers, body) = get(&link, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("/_share/signin?to="), "{body}");
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
    let link = w.url(&format!("/{}/", invited.code));
    page_with(&link, Some(&w.ada), "Mix notes.").await;
    assert_eq!(get(&link, Some(&w.alan)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(get(&link, None).await.0, StatusCode::UNAUTHORIZED);

    // Public: anyone, and it may be listed.
    let public = w.share(scope(), Audience::Public, ShareAccess::Read).await;
    let link = w.url(&format!("/{}/", public.code));
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
        get(&w.url(&format!("/_share/signin?to=/{}/", org.code)), None).await;
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
    let link = format!("/{}/", talk.code);
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
    page_with(&w.url(&format!("/{}/", read.code)), None, "The album").await;
    let res = reqwest::Client::new()
        .post(w.url(&format!("/{}/_comment", read.code)))
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
    let link = w.url(&format!("/{}/", soon.code));
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
    assert_eq!(url, format!("{host}/{code}/"));
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
