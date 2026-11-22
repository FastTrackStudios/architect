//! An extension's server-side part on the org's node (#583, #710 H16).
//!
//! The showcase's Bank link (`examples/showcase/bank-link`) has one node
//! action: a token exchange that holds a client id and secret. Here the
//! node (`NodeHost`) loads its node part, with its operator's secrets from
//! a folder, and serves `NodeActions` over vox behind a permissions gate,
//! as the org lane does. An app's host loads the same package and runs the
//! action through the generated `NodeActionsClient`: the exchange reaches
//! a local mock of the bank with both secrets in its headers, and the app
//! gets the item's id and never a secret. And the rules: what may run on
//! the node, what the node part keeps, who may run it, a missing secret,
//! secrets that are insert-only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc
)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::LayerRouter;
use architect::action::{ActionCaller, ActionList, Principal};
use architect::connect::{Endpoint, Pool};
use architect::host::EngineHost;
use architect::permissions_gate::PermissionsGate;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::manifest::{Manifest, ManifestError};
use architect_extensions::node::{
    NodeActionsClient, NodeActionsDispatcher, NodeHost, NodeLane, PERMITS,
    node_actions_service_descriptor,
};
use architect_extensions::{Origin, Package};
use architect_permissions::{
    Action, BoxIdentityFuture, Decision, IdentityResolver, PermissionEngine, Resource,
};
use architect_settings::Registry;
use serde_json::{Value, json};

#[path = "support/server.rs"]
mod server;
use server::{EXCHANGE_ID, EXCHANGE_SECRET, serve as serve_bank};

const EXT: &str = "showcase.bank-link";
const EXCHANGE: &str = "SHOWCASE_BANK_LINK_EXCHANGE";

fn bank_link() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/examples/showcase/bank-link")
}

// ── The org lane, in small ──────────────────────────────────────────

/// Bearers and who they are: `member`, `viewer` and `admin` of the org.
#[derive(Clone, Default)]
struct Members(Arc<Mutex<HashMap<String, Principal>>>);

impl IdentityResolver for Members {
    fn resolve<'a>(&'a self, bearer: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer
            .and_then(|t| self.0.lock().unwrap().get(t).cloned())
            .unwrap_or(Principal::Anonymous);
        Box::pin(async move { who })
    }
}

/// The org's roles, as the node's engine has them
/// (`apps/desktop/src/node/org.rs` `engine`): a viewer reads, a member
/// writes, an admin administers.
struct Roles;

impl PermissionEngine for Roles {
    fn check(&self, who: &Principal, _: &Resource, action: &Action) -> Decision {
        let Principal::User { user_id } = who else {
            return Decision::Deny {
                reason: "not a member".to_owned(),
            };
        };
        let may: &[&str] = match user_id.as_str() {
            "viewer" => &["read"],
            "member" => &["read", "write"],
            "admin" => &["read", "write", "admin"],
            _ => &[],
        };
        if may.contains(&action.as_str()) {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("{user_id} may not {}", action.as_str()),
            }
        }
    }
}

async fn serve_node(node: NodeHost) -> SocketAddr {
    let members = Members::default();
    for who in ["viewer", "member", "admin"] {
        members.0.lock().unwrap().insert(
            who.to_owned(),
            Principal::User {
                user_id: who.to_owned(),
            },
        );
    }
    let router = LayerRouter::new().with(
        node_actions_service_descriptor(),
        NodeActionsDispatcher::new(node),
    );
    let gate = PermissionsGate::new(Arc::new(Roles), Arc::new(members))
        .permit(node_actions_service_descriptor(), &PERMITS);
    let app = EngineHost::new(router, gate, "127.0.0.1:0").into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });
    at
}

async fn client(at: SocketAddr, bearer: &str) -> NodeActionsClient {
    let endpoint = Endpoint::url(format!("ws://{at}/vox")).with_bearer(Some(bearer.to_owned()));
    // A pool of its own: identity is per connection.
    Pool::default().client(&endpoint).await.unwrap()
}

/// The node's operator's secrets, as sops-nix writes them: a file each.
fn operator_secrets(dir: &Path, secret: &str) {
    let ext = dir.join(EXT);
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("client_id"), format!("{EXCHANGE_ID}\n")).unwrap();
    std::fs::write(ext.join("client_secret"), format!("{secret}\n")).unwrap();
}

/// An app's host with the Bank link loaded, its node actions sent to
/// `lane`.
fn app(lane: Option<Arc<dyn NodeLane>>) -> (ActionList, Host) {
    let list = ActionList::new();
    let mut host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    if let Some(lane) = lane {
        host = host.with_node(lane);
    }
    host.load(&Package::read(bank_link()).unwrap(), Origin::Installed)
        .unwrap();
    (list, host)
}

async fn run(list: &ActionList, args: &Value) -> Result<Value, String> {
    list.run_json_async_with(&ActionCaller::app(), EXCHANGE, args.to_string().as_bytes())
        .await
        .map(|out| serde_json::from_slice(&out).unwrap())
        .map_err(|e| e.to_string())
}

// ── Tests ───────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn an_app_runs_the_exchange_on_the_node_which_alone_holds_the_secret() {
    let bank = serve_bank();
    let api = format!("http://127.0.0.1:{}", bank.port);
    let tmp = tempfile::tempdir().unwrap();
    operator_secrets(tmp.path(), EXCHANGE_SECRET);

    let node = NodeHost::with_operator_secrets(tmp.path());
    let (loaded, _) = node.load_folder(bank_link().parent().unwrap());
    assert_eq!(
        loaded,
        vec![EXT.to_owned()],
        "only the Bank link has a node part"
    );
    let at = serve_node(node.clone()).await;

    let lane: Arc<dyn NodeLane> = Arc::new(client(at, "member").await);
    let (list, host) = app(Some(lane));

    // The node lists it, with nothing missing.
    let listed = client(at, "viewer").await.list().await.unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].id, EXCHANGE);
    assert!(listed[0].missing.is_empty(), "{listed:?}");

    // The app runs it as any other action: it ran on the node.
    let out = run(
        &list,
        &json!({ "public_token": "public-sandbox-1", "api": api }),
    )
    .await
    .unwrap();
    assert_eq!(out, json!({ "linked": true, "item_id": "item-42" }));

    // The bank heard both secrets, put in by the node's host.
    let heard = bank.heard.to("/item/public_token/exchange");
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].1["plaid-client-id"], EXCHANGE_ID);
    assert_eq!(heard[0].1["plaid-secret"], EXCHANGE_SECRET);

    // The app holds no secret, and never answered with one.
    let mine = host.secrets();
    assert!(mine.names(EXT).is_empty(), "{mine:?}");
    assert!(!out.to_string().contains(EXCHANGE_SECRET));
    assert!(!out.to_string().contains("access-sandbox-kept"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_may_not_run_it_and_only_an_admin_sees_which_secrets_are_set() {
    let tmp = tempfile::tempdir().unwrap();
    let node = NodeHost::new(Some(tmp.path().join("secrets.json")));
    assert!(node.load(&Package::read(bank_link()).unwrap()).unwrap());
    let at = serve_node(node).await;

    let viewer = client(at, "viewer").await;
    let refused = viewer
        .run(EXT.to_owned(), EXCHANGE.to_owned(), "{}".to_owned())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("viewer may not write"), "{refused}");
    assert!(viewer.secrets(EXT.to_owned()).await.is_err());

    // A member runs it: the node has no secret yet, and says which.
    let member = client(at, "member").await;
    let missing = member
        .run(
            EXT.to_owned(),
            EXCHANGE.to_owned(),
            json!({ "public_token": "public-x" }).to_string(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(missing.contains("needs its secret client_id"), "{missing}");
    assert!(
        member
            .set_secret(EXT.to_owned(), "client_id".to_owned(), "x".to_owned())
            .await
            .is_err(),
        "a member sets no secret"
    );

    // An admin sets them, insert-only: the service says which are set,
    // never a value.
    let admin = client(at, "admin").await;
    for (id, value) in [
        ("client_id", EXCHANGE_ID),
        ("client_secret", EXCHANGE_SECRET),
    ] {
        admin
            .set_secret(EXT.to_owned(), id.to_owned(), value.to_owned())
            .await
            .unwrap();
    }
    let states = admin.secrets(EXT.to_owned()).await.unwrap();
    assert!(states.iter().all(|s| s.set), "{states:?}");
    assert!(!format!("{states:?}").contains(EXCHANGE_SECRET));
    let undeclared = admin
        .set_secret(EXT.to_owned(), "other".to_owned(), "x".to_owned())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        undeclared.contains("holds no node secret other"),
        "{undeclared}"
    );
    // Kept in the node's private file, which no call reads back.
    let file = std::fs::read_to_string(tmp.path().join("secrets.json")).unwrap();
    assert!(file.contains(EXCHANGE_SECRET));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operators_node_takes_no_secret_from_the_lane_and_its_fetch_keeps_to_its_hosts() {
    let bank = serve_bank();
    let tmp = tempfile::tempdir().unwrap();
    operator_secrets(tmp.path(), "a-wrong-secret");
    let node = NodeHost::with_operator_secrets(tmp.path());
    node.load(&Package::read(bank_link()).unwrap()).unwrap();
    let at = serve_node(node).await;
    let admin = client(at, "admin").await;

    let refused = admin
        .set_secret(EXT.to_owned(), "client_secret".to_owned(), "x".to_owned())
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("set by its deployment"), "{refused}");

    // The bank refuses the wrong secret, and the node says so.
    let api = format!("http://127.0.0.1:{}", bank.port);
    let failed = admin
        .run(
            EXT.to_owned(),
            EXCHANGE.to_owned(),
            json!({ "public_token": "public-x", "api": api }).to_string(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        failed.contains("the bank refused the exchange (400)"),
        "{failed}"
    );

    // A host it did not declare is refused before anything is sent.
    let elsewhere = format!("http://localhost:{}", bank.port);
    let failed = admin
        .run(
            EXT.to_owned(),
            EXCHANGE.to_owned(),
            json!({ "public_token": "public-x", "api": elsewhere }).to_string(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(failed.contains("may not reach localhost"), "{failed}");
    assert_eq!(bank.heard.to("/item/public_token/exchange").len(), 1);

    // Only its node actions: nothing else of an extension runs there.
    let not_a_node_action = admin
        .run(EXT.to_owned(), "APP_STATUS".to_owned(), "{}".to_owned())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        not_a_node_action.contains("runs no node action APP_STATUS"),
        "{not_a_node_action}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_node_an_app_says_where_it_runs() {
    let (list, _host) = app(None);
    let why = run(&list, &json!({ "public_token": "public-x" }))
        .await
        .unwrap_err();
    assert!(why.contains("runs on the org's node"), "{why}");
}

// ── The rules ───────────────────────────────────────────────────────

fn manifest(kind: &str, action: &str) -> Result<Manifest, ManifestError> {
    Manifest::parse(&format!(
        r#"
id = "lab.node"
name = "Node"
version = "0.1.0"
kind = "{kind}"

[capabilities]
hosts = ["api.example.com"]
reads = ["selection"]
ui = ["notify"]

[[secrets]]
id = "key"
title = "Key"

[[views]]
id = "panel"
title = "Panel"
kind = "remote"

[[actions]]
id = "LAB_NODE_APP"
name = "In the app"
script = "app.luau"

[[actions]]
id = "LAB_NODE_RUN"
name = "On the node"
{action}
"#
    ))
}

#[test]
fn only_code_that_holds_a_secret_runs_on_the_node() {
    let ok = manifest(
        "luau",
        r#"script = "n.luau"
runs = "node"
secrets = ["key"]"#,
    )
    .unwrap();
    assert!(ok.has_node_part());

    let why = |kind: &str, action: &str| manifest(kind, action).unwrap_err().to_string();
    assert!(
        why("luau", "script = \"n.luau\"\nruns = \"node\"")
            .contains("holds none of its [[secrets]]")
    );
    assert!(
        why(
            "luau",
            "script = \"n.luau\"\nruns = \"node\"\nsecrets = [\"nope\"]"
        )
        .contains("secret \"nope\" is not one of its [[secrets]]")
    );
    assert!(
        why("luau", "script = \"n.luau\"\nsecrets = [\"key\"]")
            .contains("only an action that runs on the node names `secrets`")
    );
    assert!(
        why(
            "luau",
            "script = \"n.luau\"\nruns = \"node\"\nsecrets = [\"key\"]\ntoggle = true"
        )
        .contains("runs in the app")
    );
    let wasm = Manifest::parse(
        r#"
id = "lab.node"
name = "Node"
version = "0.1.0"
kind = "wasm"

[capabilities]
hosts = ["api.example.com"]

[[secrets]]
id = "key"
title = "Key"

[[actions]]
id = "LAB_NODE_RUN"
name = "On the node"
runs = "node"
secrets = ["key"]
"#,
    )
    .unwrap_err()
    .to_string();
    assert!(
        wasm.contains("only a luau extension's action runs on the node"),
        "{wasm}"
    );
}

#[test]
fn the_node_part_is_its_node_actions_their_secrets_and_its_hosts() {
    let m = manifest(
        "luau",
        r#"script = "n.luau"
runs = "node"
secrets = ["key"]"#,
    )
    .unwrap();
    let part = m.node_part().unwrap();
    assert_eq!(
        part.actions
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["LAB_NODE_RUN"]
    );
    assert_eq!(part.capabilities.hosts, ["api.example.com"]);
    assert!(part.capabilities.reads.is_empty() && part.capabilities.ui.is_empty());
    assert!(part.views.is_empty());
    assert_eq!(part.secrets.len(), 1);
    part.check().unwrap();

    let app_only = manifest("luau", r#"script = "a2.luau""#).unwrap();
    assert!(app_only.node_part().is_none());
}
