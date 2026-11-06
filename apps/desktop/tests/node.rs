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
        ["NOTES_LIST", "NOTES_READ", "NOTES_WRITE"]
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
