//! The app headless, end to end: `architect-desktop serve`'s node
//! (`architect_desktop::node`), two of them on one account in one org,
//! against the real issuer (`apps/auth-server` in process, as
//! `features/accounts/tests/issuer.rs` runs it) and real iroh endpoints on
//! loopback.
//!
//! The always-on node holds the org's vault and its Site; the other node
//! starts with an empty copy of the vault and knows only how to dial the
//! always-on one. Each signs in to the account and registers itself in
//! its address book, so each lets the other in. Then:
//!
//! - the note on the always-on node reaches the other one;
//! - an edit on the other one comes back to the always-on node's file;
//! - an agent's edit over the always-on node's `/mcp` reaches the other;
//! - the always-on node answers `/health`, its HTTP face, and serves the
//!   vault as a Site.
//!
//! Live, with sweeps an hour apart so only the live bridges can carry it:
//! an edit reaches the other node's document within a second, both ways;
//! and a vault seeded on both nodes separately becomes one document per
//! note (the second seed adopts the first's lineage), its text once.
//!
//! A signed-in web client (vox over WebSocket, an issuer session as the
//! bearer) lists the org's notes and edits one live on the org's lane;
//! a caller with no session, or not in the org, is refused.
//!
//! And in an issuer org (`indie`: Grace its owner, Alan a member), with
//! nodes on two accounts: Grace's node hosts the org's enrolment lane,
//! Alan's enrols itself there and syncs the vault; Alan enrols a device
//! from the app (`ORG_ENROL`), never above his role; Grace sees both
//! (`ORG_DEVICES`) and revokes Alan's node (`ORG_REVOKE`), whose next
//! call is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::future_not_send
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use std::sync::Arc;

use architect::action::ActionList;
use architect::iroh_link;
use architect_accounts::{AccountStore, Accounts, Issuer, MemorySecrets};
use architect_auth::db::{AuthSeaOrmStorage, Migrator};
use architect_desktop::node::service::Notes as _;
use architect_desktop::node::service::node::NodeHttpClient;
use architect_desktop::node::service::{Node as _, NodeStatus};
use architect_desktop::node::{self, Config, IrohMode, Running};
use auth_server::{ServerConfig, server};
use crdt::peer::DocPeerClient;
use sea_orm::Database;
use sea_orm_migration::MigratorTrait;
use serde_json::{Value, json};

const ORG: &str = "test-org";
const MCP_TOKEN: &str = "test-mcp-bearer";

async fn issuer() -> String {
    issuer_with(Vec::new()).await
}

/// The issuer with OIDC `clients` registered.
async fn issuer_with(clients: Vec<architect_auth::OidcClientConfig>) -> String {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let config = ServerConfig {
        bind_addr: "127.0.0.1:0".into(),
        database_url: "sqlite::memory:".into(),
        session_ttl_seconds: 3600,
        oidc_clients: clients,
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
    tokio::spawn(async move { axum_serve(listener, app).await });
    origin
}

async fn axum_serve(listener: tokio::net::TcpListener, app: architect::http::Router) {
    architect::http::axum::serve(listener, app).await.unwrap();
}

/// A session of Ada's, written where a node reads it.
async fn session(origin: &str, dir: &Path) -> PathBuf {
    session_of(origin, dir, "ada@local.test").await
}

/// A session of `email`'s, written where a node reads it.
async fn session_of(origin: &str, dir: &Path, email: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let signed_in = Issuer::new(origin)
        .sign_in_password(email, auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    let file = dir.join("session");
    std::fs::write(&file, signed_in.session.token).unwrap();
    file
}

fn config(name: &str, dir: &Path, origin: &str, session: PathBuf) -> Config {
    Config {
        name: name.to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        issuer: Some(origin.to_owned()),
        session_file: Some(session),
        iroh: IrohMode::Local,
        sweep_secs: 1,
        ..Config::default()
    }
}

fn note(node: &Running) -> PathBuf {
    node.state.vault_root("notes").unwrap().join("hello.md")
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..600 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {what}");
}

fn reads(path: &Path, want: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains(want))
}

async fn status(node: &Running) -> NodeStatus {
    node.state.status().await.unwrap()
}

async fn mcp(node: &Running, method: &str, params: Value) -> Value {
    reqwest::Client::new()
        .post(format!("http://{}/mcp", node.http))
        .bearer_auth(MCP_TOKEN)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_syncs_between_two_headless_nodes_through_the_always_on_one() {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();

    // ── The always-on node: the org's vault, its Site, MCP ────────────
    let always_dir = tmp.path().join("always-on");
    std::fs::create_dir_all(always_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        always_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nKept on the always-on node.\n",
    )
    .unwrap();
    std::fs::write(always_dir.join("mcp-token"), MCP_TOKEN).unwrap();
    let mut always = config(
        "always-on",
        &always_dir,
        &origin,
        session(&origin, &always_dir).await,
    );
    always.org = Some(ORG.to_owned());
    always.sites = [("/".to_owned(), "notes".to_owned())].into();
    always.mcp_token_file = Some(always_dir.join("mcp-token"));
    let always = node::start(always).await.unwrap();

    // ── A second node: an empty copy of the vault, the first's address ─
    let laptop_dir = tmp.path().join("laptop");
    std::fs::create_dir_all(&laptop_dir).unwrap();
    let mut laptop = config(
        "laptop",
        &laptop_dir,
        &origin,
        session(&origin, &laptop_dir).await,
    );
    laptop.peers = vec![always.dial().unwrap()];
    let laptop = node::start(laptop).await.unwrap();

    // Both on the account, in its org, each letting the other in.
    let (a, l) = (always.endpoint_id(), laptop.endpoint_id());
    until("each node lets the other in", || {
        always.state.admits(&l) && laptop.state.admits(&a)
    })
    .await;
    let (sa, sl) = (status(&always).await, status(&laptop).await);
    assert!(sa.signed_in && sl.signed_in, "{sa:?} {sl:?}");
    assert_eq!(sa.org.as_deref(), Some(ORG));
    assert_eq!(sl.org.as_deref(), Some(ORG), "one org for the account");

    // The always-on node's note reaches the laptop.
    until("the note reaches the laptop", || {
        reads(&note(&laptop), "Kept on the always-on node.")
    })
    .await;

    // An edit on the laptop comes back to the always-on node's file.
    laptop
        .state
        .write(
            "notes".into(),
            "hello.md".into(),
            "# Hello\n\nKept on the always-on node.\n\nEdited on the laptop.\n".into(),
        )
        .await
        .unwrap();
    until("the laptop's edit reaches the always-on node", || {
        reads(&note(&always), "Edited on the laptop.")
    })
    .await;

    // An agent writes on the always-on node over MCP; the laptop gets it.
    assert_eq!(
        architect_desktop::node::service::notes::notes_actions::IDS,
        ["NOTES_LIST", "NOTES_READ", "NOTES_WRITE", "NOTES_CREATE"]
    );
    let wrote = mcp(
        &always,
        "tools/call",
        json!({"name": "run", "arguments": {"id": "NOTES_WRITE", "args": {
            "vault": "notes",
            "path": "hello.md",
            "text": "# Hello\n\nKept on the always-on node.\n\nEdited on the laptop.\n\nAnd by an agent.\n",
        }}}),
    )
    .await;
    assert_eq!(wrote["result"]["isError"], false, "{wrote}");
    until("the agent's edit reaches the laptop", || {
        reads(&note(&laptop), "And by an agent.")
    })
    .await;

    // ── Its HTTP side ────────────────────────────────────────────────
    let http = reqwest::Client::new();
    let health = http
        .get(format!("http://{}/health", always.http))
        .send()
        .await
        .unwrap();
    assert!(health.status().is_success());
    let face = NodeHttpClient::at(format!("http://{}", always.http))
        .status()
        .await
        .unwrap();
    assert_eq!(face.endpoint, a);
    assert_eq!(face.sites, ["/"]);
    assert!(face.notes >= 1 && face.devices >= 1, "{face:?}");
    // MCP is its bearer's only.
    let refused = http
        .post(format!("http://{}/mcp", always.http))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);
    // The vault as a Site, rebuilt with the edits.
    let mut page = String::new();
    for _ in 0..100 {
        page = http
            .get(format!("http://{}/hello/", always.http))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if page.contains("And by an agent.") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(page.contains("And by an agent."), "{page}");
    // A Site's pages carry the shares' Content-Security-Policy (#520).
    let res = http
        .get(format!("http://{}/hello/", always.http))
        .send()
        .await
        .unwrap();
    let csp = res.headers()["content-security-policy"].to_str().unwrap();
    assert!(
        csp.starts_with("default-src 'self'; script-src 'self' 'sha256-"),
        "{csp}"
    );

    laptop.stop().await;
    always.stop().await;
}

/// The app signed in as `email`, in org `indie`, with its Action List.
async fn app(origin: &str, email: &str) -> (Accounts, Arc<ActionList>) {
    let store = Arc::new(AccountStore::in_memory(Arc::new(MemorySecrets::new())));
    let accounts = Accounts::new(store, Issuer::new(origin), "architect-test").without_browser();
    let list = Arc::new(ActionList::new());
    architect_accounts::actions::register(&list, &accounts).unwrap();
    accounts
        .sign_in_password(email, auth_server::dev::DEV_PASSWORD)
        .await
        .unwrap();
    accounts.refresh_orgs().await.unwrap();
    list.run_json("ORG_SWITCH", br#"{"org":"indie"}"#).unwrap();
    (accounts, list)
}

async fn run(list: &ActionList, id: &str, args: Value) -> Result<Value, String> {
    list.run_json_async(id, args.to_string().as_bytes())
        .await
        .map(|out| serde_json::from_slice(&out).unwrap())
        .map_err(|e| format!("{e:?}"))
}

/// `from`'s endpoint asks `to` for its documents over the peer lane.
async fn lists(from: &Running, to: &Running) -> Result<usize, String> {
    let link = iroh_link::connect(&from.endpoint, dial_addr(to))
        .await
        .map_err(|e| e.to_string())?;
    let client = vox::initiator_on(link)
        .establish::<DocPeerClient>()
        .await
        .map_err(|e| format!("{e:?}"))?;
    client
        .list()
        .await
        .map(|l| l.len())
        .map_err(|e| format!("{e:?}"))
}

fn dial_addr(node: &Running) -> iroh::EndpointAddr {
    let sock = node.endpoint.bound_sockets().into_iter().next().unwrap();
    iroh::EndpointAddr::from_parts(node.endpoint.id(), [iroh::TransportAddr::Ip(sock)])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_enrols_in_an_org_and_a_revoke_refuses_the_next_call() {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();

    // ── Grace's node hosts `indie` ────────────────────────────────────
    let host_dir = tmp.path().join("host");
    std::fs::create_dir_all(host_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        host_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nThe org's note.\n",
    )
    .unwrap();
    let mut host = config(
        "indie-host",
        &host_dir,
        &origin,
        session_of(&origin, &host_dir, "grace@local.test").await,
    );
    host.host_org = Some("indie".to_owned());
    let host = node::start(host).await.unwrap();
    let at = format!("http://{}", host.http);

    // ── Alan's node enrols itself there ──────────────────────────────
    let alan_dir = tmp.path().join("alan");
    let mut alan = config(
        "alans-node",
        &alan_dir,
        &origin,
        session_of(&origin, &alan_dir, "alan@local.test").await,
    );
    alan.enrol_at = Some(at.clone());
    alan.enrol_org = Some("indie".to_owned());
    alan.peers = vec![host.dial().unwrap()];
    let alan = node::start(alan).await.unwrap();
    let (h, a) = (host.endpoint_id(), alan.endpoint_id());
    until("Alan's node is enrolled, and lets the host in", || {
        host.state.admits(&a) && alan.state.admits(&h)
    })
    .await;
    until("the org's note reaches Alan's node", || {
        reads(&note(&alan), "The org's note.")
    })
    .await;
    assert!(lists(&alan, &host).await.unwrap() >= 1);

    // ── A member enrols a device from the app, never above his role ──
    let (_alan_app, alan_list) = app(&origin, "alan@local.test").await;
    let laptop = "a1b2c3d4e5f6";
    let above = run(
        &alan_list,
        "ORG_ENROL",
        json!({"host": at, "endpoint": laptop, "name": "Alan's laptop", "role": "admin"}),
    )
    .await;
    assert!(
        above.unwrap_err().contains("at most"),
        "a member enrols as admin"
    );
    let enrolled = run(
        &alan_list,
        "ORG_ENROL",
        json!({"host": at, "endpoint": laptop, "name": "Alan's laptop", "platform": "macos"}),
    )
    .await
    .unwrap();
    assert_eq!(enrolled["device"]["role"], "Member", "{enrolled}");
    let alans = run(&alan_list, "ORG_DEVICES", json!({})).await.unwrap();
    assert_eq!(
        alans["devices"].as_array().unwrap().len(),
        2,
        "his own: {alans}"
    );

    // The owner sees every enrolment.
    let (_grace_app, grace_list) = app(&origin, "grace@local.test").await;
    let devices = run(&grace_list, "ORG_DEVICES", json!({"host": at}))
        .await
        .unwrap();
    let endpoints: Vec<&str> = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["endpoint"].as_str().unwrap())
        .collect();
    assert!(
        endpoints.contains(&a.as_str()) && endpoints.contains(&laptop),
        "{devices}"
    );

    // ── The owner revokes Alan's node: its next call is refused ──────
    let revoked = run(&grace_list, "ORG_REVOKE", json!({"endpoint": a}))
        .await
        .unwrap();
    assert_eq!(revoked["devices"].as_array().unwrap().len(), 1, "{revoked}");
    assert!(!host.state.admits(&a));
    let refused = lists(&alan, &host).await;
    assert!(refused.is_err(), "a revoked device listed {refused:?}");

    alan.stop().await;
    host.stop().await;
}

/// Two nodes on Ada's account, the laptop dialling the always-on one,
/// with sweeps an hour apart: whatever moves between them moves live.
async fn live_pair(tmp: &Path, origin: &str, seed_laptop: Option<&str>) -> (Running, Running) {
    let always_dir = tmp.join("always-on");
    std::fs::create_dir_all(always_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        always_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nSeeded on the always-on node.\n",
    )
    .unwrap();
    let mut always = config(
        "always-on",
        &always_dir,
        origin,
        session(origin, &always_dir).await,
    );
    always.sweep_secs = 3600;
    let always = node::start(always).await.unwrap();

    let laptop_dir = tmp.join("laptop");
    if let Some(text) = seed_laptop {
        std::fs::create_dir_all(laptop_dir.join("data/vaults/notes")).unwrap();
        std::fs::write(laptop_dir.join("data/vaults/notes/hello.md"), text).unwrap();
    }
    let mut laptop = config(
        "laptop",
        &laptop_dir,
        origin,
        session(origin, &laptop_dir).await,
    );
    laptop.sweep_secs = 3600;
    laptop.peers = vec![always.dial().unwrap()];
    let laptop = node::start(laptop).await.unwrap();
    let (a, l) = (always.endpoint_id(), laptop.endpoint_id());
    until("each node lets the other in", || {
        always.state.admits(&l) && laptop.state.admits(&a)
    })
    .await;
    (always, laptop)
}

/// Until `node`'s live document of `hello.md` holds `want`; how long it took.
async fn live_text(node: &Running, want: &str) -> Duration {
    let started = std::time::Instant::now();
    for _ in 0..600 {
        if node
            .note_text("notes", "hello.md")
            .await
            .is_some_and(|t| t.contains(want))
        {
            return started.elapsed();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out: {want:?} never reached {}", node.endpoint_id());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_reaches_the_other_node_live_within_a_second() {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let (always, laptop) = live_pair(tmp.path(), &origin, None).await;
    live_text(&laptop, "Seeded on the always-on node.").await;

    let body = "# Hello\n\nSeeded on the always-on node.\n\nLive from the always-on node.\n";
    always
        .state
        .write("notes".into(), "hello.md".into(), body.into())
        .await
        .unwrap();
    let there = live_text(&laptop, "Live from the always-on node.").await;

    let body = format!("{body}\nLive from the laptop.\n");
    laptop
        .state
        .write("notes".into(), "hello.md".into(), body)
        .await
        .unwrap();
    let back = live_text(&always, "Live from the laptop.").await;
    assert!(
        there < Duration::from_secs(1) && back < Duration::from_secs(1),
        "live sync took {there:?} there and {back:?} back"
    );
    // And the file follows, after the write-behind.
    until("the laptop's file has the edit", || {
        reads(&note(&laptop), "Live from the always-on node.")
    })
    .await;

    laptop.stop().await;
    always.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_seeded_on_two_nodes_becomes_one_document() {
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let (always, laptop) = live_pair(
        tmp.path(),
        &origin,
        Some("# Hello\n\nSeeded on the always-on node.\n"),
    )
    .await;

    // After the second seed adopts the first's lineage, an edit on one
    // reaches the other merged into one copy of the text, not two.
    let body = "# Hello\n\nSeeded on the always-on node.\n\nAfter both seeds.\n";
    let mut arrived = false;
    for _ in 0..20 {
        always
            .state
            .write("notes".into(), "hello.md".into(), body.into())
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        if laptop
            .note_text("notes", "hello.md")
            .await
            .is_some_and(|t| t.contains("After both seeds."))
        {
            arrived = true;
            break;
        }
    }
    assert!(arrived, "the edit never reached the second seed");
    for node in [&always, &laptop] {
        let text = node.note_text("notes", "hello.md").await.unwrap();
        assert_eq!(
            text.matches("Seeded on the always-on node.").count(),
            1,
            "one document, the text once: {text:?}"
        );
        assert_eq!(text.matches("After both seeds.").count(), 1, "{text:?}");
    }
    until("the laptop's file has one copy", || {
        std::fs::read_to_string(note(&laptop)).is_ok_and(|t| {
            t.contains("After both seeds.")
                && t.matches("Seeded on the always-on node.").count() == 1
        })
    })
    .await;

    laptop.stop().await;
    always.stop().await;
}

/// A symlink in a vault is not the vault's (#675): the node lists none
/// (`NOTES_LIST`, the doc peer), reads none (`NOTES_READ`) and syncs
/// none to the other node — and hiding one deletes nothing on either
/// side. A note the laptop already holds as a plain file under a name
/// that is a symlink on the always-on node keeps its file on both; a
/// synced note swapped for a dangling symlink on the always-on node is
/// not taken for a deletion, so the laptop keeps its copy.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_symlinked_note_is_neither_synced_nor_deleted_on_either_node() {
    use std::os::unix::fs::symlink;
    let secret = "outside the vault: never synced";
    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(outside.join("dir")).unwrap();
    std::fs::write(outside.join("secret.md"), secret).unwrap();
    std::fs::write(outside.join("dir/secret.md"), secret).unwrap();

    // The always-on node: a note, a note to hide later, and links.
    let always_dir = tmp.path().join("always-on");
    let always_vault = always_dir.join("data/vaults/notes");
    std::fs::create_dir_all(&always_vault).unwrap();
    std::fs::write(always_vault.join("hello.md"), "# Hello\n\nKept.\n").unwrap();
    std::fs::write(
        always_vault.join("Hidden.md"),
        "# Hidden\n\nSynced first.\n",
    )
    .unwrap();
    symlink("hello.md", always_vault.join("Alias.md")).unwrap();
    symlink(outside.join("secret.md"), always_vault.join("Out.md")).unwrap();
    symlink(outside.join("dir"), always_vault.join("Ext")).unwrap();
    std::fs::write(always_dir.join("mcp-token"), MCP_TOKEN).unwrap();
    let mut always = config(
        "always-on",
        &always_dir,
        &origin,
        session(&origin, &always_dir).await,
    );
    always.mcp_token_file = Some(always_dir.join("mcp-token"));
    let always = node::start(always).await.unwrap();

    // The laptop: its own plain `Alias.md`, as a copy synced before the
    // always-on node stopped listing its symlink.
    let laptop_dir = tmp.path().join("laptop");
    let laptop_vault = laptop_dir.join("data/vaults/notes");
    std::fs::create_dir_all(&laptop_vault).unwrap();
    let alias = "# Alias\n\nThe laptop's own copy.\n";
    std::fs::write(laptop_vault.join("Alias.md"), alias).unwrap();
    let mut laptop = config(
        "laptop",
        &laptop_dir,
        &origin,
        session(&origin, &laptop_dir).await,
    );
    laptop.peers = vec![always.dial().unwrap()];
    let laptop = node::start(laptop).await.unwrap();
    let (a, l) = (always.endpoint_id(), laptop.endpoint_id());
    until("each node lets the other in", || {
        always.state.admits(&l) && laptop.state.admits(&a)
    })
    .await;
    until("the notes reach the laptop", || {
        reads(&laptop_vault.join("hello.md"), "Kept.")
            && reads(&laptop_vault.join("Hidden.md"), "Synced first.")
    })
    .await;

    // NOTES_LIST and NOTES_READ, over MCP as an agent calls them.
    let listed = mcp(
        &always,
        "tools/call",
        json!({"name": "run", "arguments": {"id": "NOTES_LIST", "args": {}}}),
    )
    .await;
    let listed = listed.to_string();
    for link in ["Alias.md", "Out.md", "Ext/"] {
        assert!(!listed.contains(link), "{link} listed: {listed}");
    }
    for path in ["Out.md", "Ext/secret.md", "Alias.md"] {
        let read = mcp(
            &always,
            "tools/call",
            json!({"name": "run", "arguments": {"id": "NOTES_READ", "args": {
                "vault": "notes", "path": path,
            }}}),
        )
        .await;
        assert_eq!(read["result"]["isError"], true, "{path} read: {read}");
        assert!(!read.to_string().contains(secret), "{path} read: {read}");
        let direct = always.state.read("notes".into(), path.into()).await;
        assert!(direct.is_err(), "{path} read: {direct:?}");
    }

    // A synced note hidden on the always-on node: swapped for a dangling
    // symlink, which is not a deletion. Then an edit, so the laptop is
    // known to have caught up past the swap.
    symlink("Gone.md", always_vault.join(".hidden")).unwrap();
    std::fs::rename(always_vault.join(".hidden"), always_vault.join("Hidden.md")).unwrap();
    always
        .state
        .write(
            "notes".into(),
            "hello.md".into(),
            "# Hello\n\nKept.\n\nAfter the swap.\n".into(),
        )
        .await
        .unwrap();
    until("the edit after the swap reaches the laptop", || {
        reads(&laptop_vault.join("hello.md"), "After the swap.")
    })
    .await;
    // A few more sweeps, for anything still to arrive.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Nothing removed or written through on either side.
    assert_eq!(
        std::fs::read_to_string(laptop_vault.join("Hidden.md")).unwrap(),
        "# Hidden\n\nSynced first.\n",
        "the laptop's copy of the hidden note was removed or changed"
    );
    assert_ne!(laptop.note_deleted("notes", "Hidden.md").await, Some(true));
    assert_eq!(
        std::fs::read_to_string(laptop_vault.join("Alias.md")).unwrap(),
        alias
    );
    for link in ["Alias.md", "Out.md", "Ext", "Hidden.md"] {
        let meta = std::fs::symlink_metadata(always_vault.join(link)).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "{link} on the always-on node is no longer a link"
        );
    }
    assert_eq!(
        std::fs::read_link(always_vault.join("Alias.md")).unwrap(),
        Path::new("hello.md")
    );
    assert!(reads(&always_vault.join("hello.md"), "After the swap."));
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.md")).unwrap(),
        secret
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("dir/secret.md")).unwrap(),
        secret
    );
    assert_eq!(std::fs::read_dir(outside.join("dir")).unwrap().count(), 1);
    for path in ["Out.md", "Ext"] {
        assert!(
            !laptop_vault.join(path).exists(),
            "{path} reached the laptop"
        );
    }
    for entry in walkdir_files(&laptop_vault) {
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        assert!(
            !text.contains(secret),
            "{} holds the outside file",
            entry.display()
        );
    }

    laptop.stop().await;
    always.stop().await;
}

/// Every plain file under `dir`.
#[cfg(unix)]
fn walkdir_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => out.extend(walkdir_files(&path)),
            Ok(t) if t.is_file() => out.push(path),
            _ => {}
        }
    }
    out
}

/// The org lane at `node` as a browser reaches it: `/org/indie/vox` over
/// WebSocket with `token` (if any) as the bearer.
async fn web<C: vox::FromVoxLane>(node: &Running, token: Option<String>) -> C {
    let at = architect::connect::Endpoint::url(format!("ws://{}/org/indie/vox", node.http))
        .with_bearer(token);
    architect::connect::Pool::new().client(&at).await.unwrap()
}

fn token_in(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("session")).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_in_web_client_edits_a_note_live_on_the_org_lane() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::{DocSyncClient, SyncedDoc};

    let origin = issuer().await;
    let tmp = tempfile::tempdir().unwrap();
    let host_dir = tmp.path().join("host");
    std::fs::create_dir_all(host_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        host_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nThe org's note.\n",
    )
    .unwrap();
    let mut host = config(
        "indie-host",
        &host_dir,
        &origin,
        session_of(&origin, &host_dir, "grace@local.test").await,
    );
    host.host_org = Some("indie".to_owned());
    let host = node::start(host).await.unwrap();
    // Its status names the org it hosts: what a web app opens.
    assert_eq!(status(&host).await.hosts.as_deref(), Some("indie"));

    // Alan, a member, signed in at the issuer: the notes, then one live.
    let alan_dir = tmp.path().join("alan-web");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let alan = token_in(&alan_dir);
    let notes: NotesClient = web(&host, Some(alan.clone())).await;
    let listed = notes.list().await.unwrap();
    assert!(listed.iter().any(|n| n.path == "hello.md"), "{listed:?}");

    let sync: DocSyncClient = web(&host, Some(alan)).await;
    let doc_id = vault_proto::collab_doc_id("notes", "hello.md");
    let replica = crdt::CrdtDoc::ephemeral();
    let mut synced = SyncedDoc::new(doc_id, replica.clone());
    let session = tokio::spawn(async move {
        let _ = synced.run(&sync).await;
    });
    let body = || {
        replica
            .loro()
            .get_text(vault_proto::COLLAB_BODY_CONTAINER)
            .to_string()
    };
    until("the note reaches the browser", || {
        body().contains("The org's note.")
    })
    .await;
    let text = replica.loro().get_text(vault_proto::COLLAB_BODY_CONTAINER);
    text.insert(text.len_unicode(), "\nTyped in the browser.\n")
        .unwrap();
    replica.loro().commit();
    live_text(&host, "Typed in the browser.").await;
    until("the host's file has it", || {
        reads(&note(&host), "Typed in the browser.")
    })
    .await;
    session.abort();

    // No session, or a session not in the org: refused.
    let anonymous: NotesClient = web(&host, None).await;
    assert!(anonymous.list().await.is_err());
    let ada_dir = tmp.path().join("ada-web");
    session_of(&origin, &ada_dir, "ada@local.test").await;
    let ada: NotesClient = web(&host, Some(token_in(&ada_dir))).await;
    let refused = ada.list().await.unwrap_err();
    assert!(
        format!("{refused:?}").contains("not a member"),
        "{refused:?}"
    );

    host.stop().await;
}

/// The status line of a WebSocket upgrade to the org lane at `node`, from
/// a browser page on `origin` (none: a native client).
async fn upgrade_status(node: &Running, origin: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = tokio::net::TcpStream::connect(node.http).await.unwrap();
    let origin = origin.map_or_else(String::new, |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "GET /org/indie/vox HTTP/1.1\r\nHost: {}\r\n{origin}Connection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: vox.v1\r\n\r\n",
        node.http
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = [0_u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8_lossy(&buf[..n])
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Where a browser's sign-in comes back to, in these tests.
const WEB_CALLBACK: &str = "http://127.0.0.1:9/auth/callback";

/// A web app the issuer knows: `first_party` as the browser app
/// (`architect`, registered `first_party`, with rotation as production's).
fn web_client(id: &str, first_party: bool) -> architect_auth::OidcClientConfig {
    architect_auth::OidcClientConfig {
        client_id: id.to_owned(),
        client_secret: None,
        name: id.to_owned(),
        redirect_uris: vec![WEB_CALLBACK.to_owned()],
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
        public_client: true,
        skip_consent: true,
        first_party,
        rotate_refresh_tokens: first_party,
        disabled: false,
    }
}

/// The browser's redirect sign-in as `client` for the person holding
/// `session` (`apps/web`'s, with PKCE): its token response.
async fn signed_in_by_redirect(
    origin: &str,
    client: &str,
    session: &str,
) -> auth_client::oidc::TokenResponse {
    use auth_client::oidc;
    let pkce = oidc::Pkce::from_entropy([7; 32], [9; 16]);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize =
        oidc::authorize_url(origin, client, WEB_CALLBACK, &pkce, "openid email profile");
    let answer = http
        .get(authorize)
        .bearer_auth(session)
        .send()
        .await
        .unwrap();
    let back = answer
        .headers()
        .get("location")
        .and_then(|l| l.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let code = back
        .split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("code="))
        .unwrap_or_else(|| panic!("no code in the redirect ({})", answer.status()))
        .to_owned();
    let body = oidc::token_request_body(client, WEB_CALLBACK, &code, &pkce);
    let text = http
        .post(format!("{origin}{}", oidc::TOKEN_PATH))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    oidc::token_response_from(&text).unwrap()
}

/// #628: a browser signed in by redirect holds only the issuer's access
/// token (client `architect`, no issuer session). The node relays it to
/// the issuer's `whoami` and org list, which take a first party's access
/// token, so the org lane serves it, live; the node verifies no JWT
/// itself. A third party's access token and an id token stay anonymous.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_signed_in_by_redirect_reaches_the_org_lane_with_its_access_token() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::{DocSyncClient, SyncedDoc};

    let origin = issuer_with(vec![
        web_client("architect", true),
        web_client("elsewhere", false),
    ])
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let host_dir = tmp.path().join("host");
    std::fs::create_dir_all(host_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        host_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nThe org's note.\n",
    )
    .unwrap();
    let mut host = config(
        "indie-host",
        &host_dir,
        &origin,
        session_of(&origin, &host_dir, "grace@local.test").await,
    );
    host.host_org = Some("indie".to_owned());
    let host = node::start(host).await.unwrap();

    // Alan, a member, signs in to the web app by redirect.
    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let alan = token_in(&alan_dir);
    let web_app = signed_in_by_redirect(&origin, "architect", &alan).await;
    let notes: NotesClient = web(&host, Some(web_app.access_token.clone())).await;
    let listed = notes.list().await.unwrap();
    assert!(listed.iter().any(|n| n.path == "hello.md"), "{listed:?}");

    let sync: DocSyncClient = web(&host, Some(web_app.access_token.clone())).await;
    // The web app's text area over the browser's replica, as `apps/web`
    // opens a note: it shows the note as read, before the session has
    // attached.
    let replica = crdt::CrdtDoc::ephemeral();
    let read = notes
        .read("notes".to_owned(), "hello.md".to_owned())
        .await
        .unwrap();
    let mut text = architect_web::live_text::LiveText::new(
        replica.loro().clone(),
        vault_proto::COLLAB_BODY_CONTAINER,
        read.clone(),
    );
    // Typed at once, before the session attached: the document does not
    // hold the note yet, so it is not taken (taken, it would add a second
    // copy of the note, or text at the wrong place).
    assert_eq!(
        text.typed(&format!("Typed too soon. {read}")).unwrap(),
        architect_web::live_text::Typed::NotLive
    );
    let (attached_tx, mut attached) = tokio::sync::mpsc::unbounded_channel();
    let mut synced = SyncedDoc::new(
        vault_proto::collab_doc_id("notes", "hello.md"),
        replica.clone(),
    )
    .on_attached(std::sync::Arc::new(move || {
        let _ = attached_tx.send(());
    }));
    let session = tokio::spawn(async move {
        let _ = synced.run(&sync).await;
    });
    tokio::time::timeout(Duration::from_secs(20), attached.recv())
        .await
        .expect("the session attached");
    let shown = text.attach();
    assert!(shown.contains("The org's note."), "{shown}");
    let typed = format!("{shown}\nTyped by redirect.\n");
    assert!(matches!(
        text.typed(&typed).unwrap(),
        architect_web::live_text::Typed::Merged { .. }
    ));
    until("the host's file has it", || {
        reads(&note(&host), "Typed by redirect.")
    })
    .await;
    session.abort();
    // Once, not twice, and nothing typed before the attach.
    let file = std::fs::read_to_string(note(&host)).unwrap();
    assert_eq!(file.matches("The org's note.").count(), 1, "{file}");
    assert!(!file.contains("Typed too soon."), "{file}");

    // #628 H1: a new note never empties one made since the listing.
    std::fs::write(
        host_dir.join("data/vaults/notes/Untitled.md"),
        "Someone else's note, made after this tab listed the vault.\n",
    )
    .unwrap();
    let refused = notes
        .create("notes".to_owned(), "Untitled.md".to_owned(), String::new())
        .await;
    assert!(
        matches!(
            &refused,
            Err(vox::VoxError::User(e)) if matches!(**e, vault_proto::notes::NotesError::Exists(_))
        ),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(host_dir.join("data/vaults/notes/Untitled.md")).unwrap(),
        "Someone else's note, made after this tab listed the vault.\n"
    );
    notes
        .create(
            "notes".to_owned(),
            "Untitled 2.md".to_owned(),
            String::new(),
        )
        .await
        .expect("a free name is made");
    assert_eq!(
        std::fs::read_to_string(host_dir.join("data/vaults/notes/Untitled 2.md")).unwrap(),
        ""
    );

    // #628 L3: a page on another origin cannot open the lane; the app's
    // own origin and a native client (no `Origin`) reach the bearer check.
    for (origin, want) in [
        (Some("https://evil.example".to_owned()), "403"),
        (Some(format!("http://{}", host.http)), "101"),
        (None, "101"),
    ] {
        let status = upgrade_status(&host, origin.as_deref()).await;
        assert!(status.contains(want), "{origin:?}: {status}");
    }

    // A third party's access token, and the browser's id token: anonymous.
    let other = signed_in_by_redirect(&origin, "elsewhere", &alan).await;
    for (label, token) in [
        ("a third party's access token", other.access_token),
        ("an id token", web_app.id_token.clone().unwrap_or_default()),
    ] {
        let refused: NotesClient = web(&host, Some(token)).await;
        assert!(refused.list().await.is_err(), "{label} was let in");
    }

    host.stop().await;
}

/// #628 M2, N2: a browser's live note session ends once the credential
/// its call was admitted with stops resolving: here its grant is revoked
/// at the issuer, as the web app's Sign out does. The node re-checks that
/// credential every `lane_recheck_secs`, so the session does not outlive
/// the sign-in until the connection happens to drop. The credential is
/// watched however it came: the connection's bearer (the browser's
/// subprotocol), or the call's own `authorization`, with or without
/// another bearer on the connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_note_session_ends_when_its_web_sign_in_is_revoked() {
    live_note_ends_on_revoke(Bearer::Connection).await;
}

/// N2: the review's variant: no bearer on the connection, the token sent
/// with the call (`authorization` metadata), as a native client may.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_note_opened_with_a_per_call_bearer_ends_when_it_is_revoked() {
    live_note_ends_on_revoke(Bearer::PerCall).await;
}

/// N2: a connection signed in as one sign-in that makes the call as
/// another: the call's credential is the one watched.
#[tokio::test(flavor = "multi_thread")]
async fn a_live_note_watches_the_call_s_bearer_not_the_connection_s() {
    live_note_ends_on_revoke(Bearer::PerCallOverAnother).await;
}

/// Which way the revoked token reaches the org lane.
#[derive(Clone, Copy)]
enum Bearer {
    Connection,
    PerCall,
    PerCallOverAnother,
}

/// A host that hosts `indie` with `hello.md`, its lane re-checking every
/// second, asking the issuer at `issuer`; its directory.
async fn indie_host(tmp: &Path, origin: &str, issuer: &str, grace_secs: u64) -> Running {
    let host_dir = tmp.join("host");
    std::fs::create_dir_all(host_dir.join("data/vaults/notes")).unwrap();
    std::fs::write(
        host_dir.join("data/vaults/notes/hello.md"),
        "# Hello\n\nThe org's note.\n",
    )
    .unwrap();
    let mut host = config(
        "indie-host",
        &host_dir,
        issuer,
        session_of(origin, &host_dir, "grace@local.test").await,
    );
    host.host_org = Some("indie".to_owned());
    host.lane_recheck_secs = 1;
    host.lane_issuer_grace_secs = grace_secs;
    node::start(host).await.unwrap()
}

/// `hello.md` opened live over `sync`: the running session, once attached.
async fn attached(
    sync: crdt::sync::DocSyncClient,
) -> tokio::task::JoinHandle<Result<(), crdt::PersistError>> {
    use crdt::sync::SyncedDoc;
    let replica = crdt::CrdtDoc::ephemeral();
    let (attached_tx, mut attached) = tokio::sync::mpsc::unbounded_channel();
    let mut synced = SyncedDoc::new(
        vault_proto::collab_doc_id("notes", "hello.md"),
        replica.clone(),
    )
    .on_attached(std::sync::Arc::new(move || {
        let _ = attached_tx.send(());
    }));
    let session = tokio::spawn(async move { synced.run(&sync).await });
    tokio::time::timeout(Duration::from_secs(20), attached.recv())
        .await
        .expect("the session attached");
    session
}

/// `client` sending `token` with every call (`authorization` metadata).
fn per_call(client: crdt::sync::DocSyncClient, token: &str) -> crdt::sync::DocSyncClient {
    let store = std::sync::Arc::new(auth_client::MemoryTokenStore::new());
    auth_client::TokenStore::save(&*store, &auth_client::StoredSession::new(token)).unwrap();
    client.with_middleware(auth_client::TokenStoreMiddleware::new(store))
}

async fn live_note_ends_on_revoke(bearer: Bearer) {
    use crdt::sync::DocSyncClient;

    let origin = issuer_with(vec![web_client("architect", true)]).await;
    let tmp = tempfile::tempdir().unwrap();
    let host = indie_host(tmp.path(), &origin, &origin, 300).await;

    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let web_app = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir)).await;
    let token = web_app.access_token.clone();
    let sync: DocSyncClient = match bearer {
        Bearer::Connection => web(&host, Some(token.clone())).await,
        Bearer::PerCall => per_call(web(&host, None).await, &token),
        Bearer::PerCallOverAnother => {
            let other = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir)).await;
            per_call(web(&host, Some(other.access_token)).await, &token)
        }
    };
    let session = attached(sync).await;

    // Still open a few re-checks later: the sign-in is good.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!session.is_finished(), "the session ended while signed in");

    // Sign out: the web app revokes its grant.
    let revoked = reqwest::Client::new()
        .post(format!("{origin}/oauth2/revoke"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(auth_client::oidc::revoke_request_body("architect", &token))
        .send()
        .await
        .unwrap();
    assert!(revoked.status().is_success(), "{}", revoked.status());
    let ended = tokio::time::timeout(Duration::from_secs(15), session).await;
    assert!(
        ended.is_ok(),
        "the live session outlived the revoked sign-in"
    );

    host.stop().await;
}

/// A TCP forwarder to `to` (`http://host:port`) that can be cut: then
/// every connection through it drops and no new one is accepted, as when
/// the issuer goes away. Its origin, and the switch.
async fn cuttable(to: &str) -> (String, tokio::sync::watch::Sender<bool>) {
    forwarder(to, false).await
}

/// A TCP forwarder to `to` that can stall: then it relays nothing more,
/// yet holds every connection open and accepts new ones, as an issuer
/// that hangs (or a path that blackholes) does. Its origin, and the switch.
async fn stallable(to: &str) -> (String, tokio::sync::watch::Sender<bool>) {
    forwarder(to, true).await
}

async fn forwarder(to: &str, stall: bool) -> (String, tokio::sync::watch::Sender<bool>) {
    let target = to.trim_start_matches("http://").to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (cut, cut_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let mut stop = cut_rx.clone();
        loop {
            tokio::select! {
                _ = stop.changed(), if !stall => return,
                accepted = listener.accept() => {
                    let Ok((mut inbound, _)) = accepted else { return };
                    let (target, mut stop) = (target.clone(), cut_rx.clone());
                    tokio::spawn(async move {
                        if *stop.borrow() {
                            // Stalled: held open (in scope), never answered.
                            std::future::pending::<()>().await;
                        }
                        let Ok(mut outbound) = tokio::net::TcpStream::connect(&target).await else {
                            return;
                        };
                        tokio::select! {
                            _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound) => {}
                            _ = stop.changed() => {
                                if stall {
                                    // Both ends stay open in scope.
                                    std::future::pending::<()>().await;
                                }
                            }
                        }
                    });
                }
            }
        }
    });
    (origin, cut)
}

/// N4: the issuer goes away. An open live note is kept for
/// `lane_issuer_grace_secs` after the issuer last answered for it, then
/// ends; a new call meanwhile is refused (it fails closed).
#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_issuer_ends_a_live_note_after_the_grace_and_refuses_new_calls() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::DocSyncClient;

    let origin = issuer_with(vec![web_client("architect", true)]).await;
    let (via, cut) = cuttable(&origin).await;
    let tmp = tempfile::tempdir().unwrap();
    let host = indie_host(tmp.path(), &origin, &via, 6).await;
    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let token = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir))
        .await
        .access_token;
    let notes: NotesClient = web(&host, Some(token.clone())).await;
    assert!(
        notes.list().await.is_ok(),
        "listed while the issuer answers"
    );
    let sync: DocSyncClient = web(&host, Some(token.clone())).await;
    let session = attached(sync).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!session.is_finished(), "the session ended while signed in");

    cut.send(true).unwrap();
    let cut_at = std::time::Instant::now();
    // Within the grace: kept, but a new call is not admitted on an old answer.
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !session.is_finished(),
        "the session ended at once when the issuer went away"
    );
    let fresh: NotesClient = web(&host, Some(token)).await;
    assert!(
        fresh.list().await.is_err(),
        "a new call was admitted while the issuer was unreachable"
    );
    // Past it: ended.
    let ended = tokio::time::timeout(Duration::from_secs(20), session).await;
    assert!(ended.is_ok(), "the live session outlived the grace");
    assert!(
        cut_at.elapsed() >= Duration::from_secs(5),
        "ended before the grace ran out ({:?})",
        cut_at.elapsed()
    );

    host.stop().await;
}

/// R1: the issuer stalls (holds every call, answers none), then the
/// person's sign-in is revoked there. The open live note still ends once
/// the grace is over, and a new call is refused, not left hanging: a call
/// to the issuer that gets no answer in time is no answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_issuer_ends_a_live_note_after_the_grace_and_refuses_new_calls() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::DocSyncClient;

    let origin = issuer_with(vec![web_client("architect", true)]).await;
    let (via, stall) = stallable(&origin).await;
    let tmp = tempfile::tempdir().unwrap();
    let host = indie_host(tmp.path(), &origin, &via, 6).await;
    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let token = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir))
        .await
        .access_token;
    let sync: DocSyncClient = web(&host, Some(token.clone())).await;
    let session = attached(sync).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!session.is_finished(), "the session ended while signed in");

    stall.send(true).unwrap();
    let stalled_at = std::time::Instant::now();
    // Signed out at the issuer itself, which the node cannot hear now.
    let revoked = reqwest::Client::new()
        .post(format!("{origin}/oauth2/revoke"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(auth_client::oidc::revoke_request_body("architect", &token))
        .send()
        .await
        .unwrap();
    assert!(revoked.status().is_success(), "{}", revoked.status());

    // Past the bearer's cache: a new call is refused, promptly.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let fresh: NotesClient = web(&host, Some(token)).await;
    let listed = tokio::time::timeout(Duration::from_secs(10), fresh.list()).await;
    assert!(
        matches!(listed, Ok(Err(_))),
        "a new call was not refused while the issuer stalled: {listed:?}"
    );
    // The live note ends once the grace is over.
    let ended = tokio::time::timeout(Duration::from_secs(20), session).await;
    assert!(ended.is_ok(), "the live session outlived the grace");
    assert!(
        stalled_at.elapsed() >= Duration::from_secs(5),
        "ended before the grace ran out ({:?})",
        stalled_at.elapsed()
    );

    host.stop().await;
}

/// Everything logged, for [`the_node_never_logs_a_browser_bearer_even_at_trace`].
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// #628 N5, R2: a node logging at TRACE (`RUST_LOG=trace`, even with a
/// directive naming the WebSocket library's module, through the filter
/// every host logs with, `architect::host::log_filter`) writes no
/// credential: not a browser's access token sent on the connection or
/// with a call, not as the gated org lane resolves it at the issuer and
/// not as an open live note re-checks it there, and not the node's own
/// session. Without the filter's cap, `tungstenite` dumps the node's
/// `whoami` frames to the issuer, token and all.
///
/// One process per test (nextest), so the subscriber is global: the
/// node's tasks on every thread and the `log` records of the WebSocket
/// library are captured.
#[tokio::test(flavor = "multi_thread")]
async fn the_node_never_logs_a_browser_bearer_even_at_trace() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::DocSyncClient;
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let captured = Captured::default();
    let writer = captured.clone();
    // A more specific directive for the WebSocket library too (R2): the
    // cap holds against it.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .with_filter(architect::host::log_filter(
                    tracing_subscriber::EnvFilter::new(
                        "trace,tungstenite::protocol=trace,tokio_tungstenite=trace",
                    ),
                )),
        )
        .try_init()
        .expect("the only subscriber in this process");
    tracing::trace!("capture-check");

    let origin = issuer_with(vec![web_client("architect", true)]).await;
    let tmp = tempfile::tempdir().unwrap();
    let host = indie_host(tmp.path(), &origin, &origin, 300).await;
    let node_session = token_in(&tmp.path().join("host"));
    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let on_connection = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir))
        .await
        .access_token;
    let with_calls = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir))
        .await
        .access_token;

    let notes: NotesClient = web(&host, Some(on_connection.clone())).await;
    assert!(notes.list().await.is_ok());
    let session = attached(web(&host, Some(on_connection.clone())).await).await;
    let per_call_session = attached(per_call(
        web::<DocSyncClient>(&host, None).await,
        &with_calls,
    ))
    .await;
    // A few re-checks of both at the issuer.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!session.is_finished() && !per_call_session.is_finished());
    session.abort();
    per_call_session.abort();
    host.stop().await;

    let logged = String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned();
    assert!(logged.contains("capture-check"), "the capture works");
    assert!(logged.contains("TRACE"), "captured at TRACE");
    for (what, secret) in [
        ("the connection's access token", on_connection.as_str()),
        ("a call's access token", with_calls.as_str()),
        ("the node's own session", node_session.trim()),
    ] {
        // Its signature alone would do: no part of it is logged.
        let tail = secret.rsplit('.').next().unwrap_or(secret);
        assert!(
            !logged.contains(secret) && !logged.contains(tail),
            "the node logged {what}"
        );
    }
}

/// `ARCHITECT_IROH=local` (what `devenv up`'s node runs with, #541): read
/// from the environment, the node's iroh endpoint has no relay and is
/// bound to loopback alone, so it can neither find nor be found by a peer
/// anywhere else; its status says so to anyone who asks.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_node_has_no_relay_and_listens_on_loopback_only() {
    let tmp = tempfile::tempdir().unwrap();
    let env = [
        ("ARCHITECT_IROH", "local"),
        ("ARCHITECT_BIND", "127.0.0.1:0"),
    ];
    let mut config = Config::load(None, |k| {
        env.iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).to_owned())
    })
    .unwrap();
    config.data_dir = tmp.path().join("data");
    assert_eq!(config.iroh, IrohMode::Local);
    assert!(config.peers.is_empty() && config.enrol_at.is_none());
    let node = node::start(config).await.unwrap();
    assert!(node.endpoint.addr().relay_urls().next().is_none());
    let bound = node.endpoint.bound_sockets();
    assert!(!bound.is_empty());
    assert!(bound.iter().all(|a| a.ip().is_loopback()), "{bound:?}");
    let shown = status(&node).await;
    assert!(shown.relays.is_empty(), "{:?}", shown.relays);
    assert!(
        shown.iroh_addrs.iter().all(|a| a
            .parse::<std::net::SocketAddr>()
            .unwrap()
            .ip()
            .is_loopback()),
        "{:?}",
        shown.iroh_addrs
    );
}

/// Whether `path` in `notes` opens live over `sync` within `within`.
async fn attaches_within(sync: crdt::sync::DocSyncClient, path: &str, within: Duration) -> bool {
    use crdt::sync::SyncedDoc;
    let (attached_tx, mut attached) = tokio::sync::mpsc::unbounded_channel();
    let mut synced = SyncedDoc::new(
        vault_proto::collab_doc_id("notes", path),
        crdt::CrdtDoc::ephemeral(),
    )
    .on_attached(std::sync::Arc::new(move || {
        let _ = attached_tx.send(());
    }));
    let session = tokio::spawn(async move { synced.run(&sync).await });
    let opened = tokio::time::timeout(within, attached.recv()).await.is_ok();
    session.abort();
    opened
}

/// Until `node` has applied the delete of `path` in `notes`: its document
/// open (the delete opens a closed one) with its tombstone set. Each look
/// is bounded, so a wedged registry fails this instead of hanging it.
async fn deleted_on(node: &Running, path: &str) {
    for _ in 0..800 {
        let seen = tokio::time::timeout(Duration::from_secs(5), node.note_deleted("notes", path))
            .await
            .expect("the node's document registry is wedged");
        if seen == Some(true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the node never applied the delete of {path}");
}

/// Whether any directory under `root` is named `name` and holds a file.
fn persisted_under(root: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|e| {
        let path = e.path();
        path.is_dir()
            && ((e.file_name() == name
                && std::fs::read_dir(&path).is_ok_and(|mut d| d.next().is_some()))
                || persisted_under(&path, name))
    })
}

/// A note made over the org lane (as the web app's New note does), its
/// file then deleted on disk while its document is closed: every other
/// note still opens live. The node used to open that document under its
/// own reconcile lock while the registry held its creation lock, and no
/// note on the node attached again until a restart.
///
/// Then a note that was open live (its document saved) on a node that has
/// since restarted: deleting its file must not bring it back. The delete
/// opens the saved document, whose open used to schedule writing the old
/// text back; that write could beat the delete to the note's lock. The
/// interleaving itself is pinned down in `vault-collab`'s tests; this is
/// the same path end to end.
#[tokio::test(flavor = "multi_thread")]
async fn a_note_deleted_on_disk_leaves_every_other_note_live() {
    use architect_desktop::node::service::notes::NotesClient;
    use crdt::sync::DocSyncClient;

    let origin = issuer_with(vec![web_client("architect", true)]).await;
    let tmp = tempfile::tempdir().unwrap();
    let host = indie_host(tmp.path(), &origin, &origin, 300).await;
    let alan_dir = tmp.path().join("alan");
    session_of(&origin, &alan_dir, "alan@local.test").await;
    let web_app = signed_in_by_redirect(&origin, "architect", &token_in(&alan_dir)).await;
    let token = web_app.access_token.clone();

    let notes: NotesClient = web(&host, Some(token.clone())).await;
    notes
        .create("notes".into(), "Untitled.md".into(), String::new())
        .await
        .unwrap();
    // Its file deleted on disk; its document is not open.
    let file = host.state.vault_root("notes").unwrap().join("Untitled.md");
    std::fs::remove_file(&file).unwrap();
    deleted_on(&host, "Untitled.md").await;

    let sync: DocSyncClient = web(&host, Some(token.clone())).await;
    assert!(
        attaches_within(sync, "hello.md", Duration::from_secs(20)).await,
        "another note no longer opens live"
    );
    // `hello.md` was open live: its document is saved.
    let hello = vault_proto::collab_doc_id("notes", "hello.md").to_string();
    until("hello.md's document saved", || {
        persisted_under(&tmp.path().join("host"), &hello)
    })
    .await;
    host.stop().await;

    // Restarted: `hello.md`'s document is closed, and saved with its text.
    let host = indie_host(tmp.path(), &origin, &origin, 300).await;
    let file = host.state.vault_root("notes").unwrap().join("hello.md");
    std::fs::remove_file(&file).unwrap();
    deleted_on(&host, "hello.md").await;
    assert!(!file.exists(), "the deleted note's file came back");
    host.stop().await;
}
