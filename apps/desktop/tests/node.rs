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
use architect_desktop::node::service::Notes as _;
use architect_desktop::node::service::node::NodeHttpClient;
use architect_desktop::node::service::{Node as _, NodeStatus};
use architect_desktop::node::{self, Config, IrohMode, Running};
use auth_server::{ServerConfig, server};
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
    let signed_in = Issuer::new(origin)
        .sign_in_password("ada@local.test", auth_server::dev::DEV_PASSWORD)
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
