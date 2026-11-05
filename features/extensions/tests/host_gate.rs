//! An extension's principal reaching a server's `PermissionsGate` (#178).
//!
//! The lab's metronome runs `APP_STATUS` through the host service. Here
//! the app's `APP_STATUS` is answered by a vox-served server behind a
//! `PermissionsGate` (`architect::host::EngineHost`): the app calls it on
//! the extension's behalf with a token its issuer minted for the caller
//! the action list handed it, so the extension never holds anyone's
//! session token. The gate resolves that token to
//! `Principal::Extension { id, on_behalf_of }`: its engine decides on the
//! extension and the person behind it, and the service sees the same
//! caller (`architect::permissions_gate::caller`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use architect::LayerRouter;
use architect::action::{ActionCaller, ActionList, ActionMeta, Effect, Principal, RawJson};
use architect::connect::{Endpoint, Pool};
use architect::host::EngineHost;
use architect::permissions_gate::PermissionsGate;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_permissions::{
    Action, BoxIdentityFuture, Decision, IdentityResolver, MethodPermit, PermissionEngine,
    Resource, ServicePermits,
};
use architect_settings::Registry;
use serde_json::{Value, json};

const ID: &str = "fts.metronome";
const STATUS: &str = "FTS_METRONOME_STATUS";

mod status {
    /// The app's status, served: what the gate let through says who it
    /// was for.
    #[vox::service]
    pub trait StatusService {
        /// `describe()` of the caller the gate resolved.
        async fn problems(&self) -> Vec<String>;
    }

    /// Remembers who each call it served was for.
    #[derive(Clone, Default)]
    pub struct Served(pub std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    impl StatusService for Served {
        async fn problems(&self) -> Vec<String> {
            let who = architect::permissions_gate::caller()
                .map_or_else(|| "none".to_owned(), |who| who.describe());
            self.0.lock().unwrap().push(who.clone());
            vec![format!("served to {who}")]
        }
    }
}

use status::{
    Served, StatusServiceClient, StatusServiceDispatcher, status_service_service_descriptor,
};

const PERMITS: ServicePermits = ServicePermits {
    service: "status",
    methods: &[MethodPermit::new("problems", "read", "status/**")],
};

/// The tokens the app's issuer minted, and for whom: what the server
/// resolves a bearer to. The extension is never given one.
#[derive(Clone, Default)]
struct Issuer(Arc<Mutex<HashMap<String, Principal>>>);

impl Issuer {
    fn mint(&self, who: Principal) -> String {
        let mut tokens = self.0.lock().unwrap();
        let token = format!("t{}", tokens.len());
        tokens.insert(token.clone(), who);
        token
    }
}

impl IdentityResolver for Issuer {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let who = bearer_token
            .and_then(|t| self.0.lock().unwrap().get(t).cloned())
            .unwrap_or(Principal::Anonymous);
        Box::pin(async move { who })
    }
}

/// The server's engine: an extension may read the status for bob, nobody
/// else may; it remembers every caller it decided on.
#[derive(Clone, Default)]
struct Engine(Arc<Mutex<Vec<(Principal, bool)>>>);

impl PermissionEngine for Engine {
    fn check(&self, who: &Principal, _: &Resource, _: &Action) -> Decision {
        let allowed = who.extension_id() == Some(ID)
            && matches!(who.acting_for(), Some(Principal::User { user_id }) if user_id == "bob");
        self.0.lock().unwrap().push((who.clone(), allowed));
        if allowed {
            Decision::Allow
        } else {
            Decision::Deny {
                reason: format!("{} may not read the status", who.describe()),
            }
        }
    }
}

async fn serve(served: Served, engine: Engine, issuer: Issuer) -> SocketAddr {
    let router = LayerRouter::new().with(
        status_service_service_descriptor(),
        StatusServiceDispatcher::new(served),
    );
    let gate = PermissionsGate::new(Arc::new(engine), Arc::new(issuer))
        .permit(status_service_service_descriptor(), &PERMITS);
    let app = EngineHost::new(router, gate, "127.0.0.1:0").into_app();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { architect::http::axum::serve(listener, app).await });
    at
}

static APP_STATUS: ActionMeta = ActionMeta {
    id: "APP_STATUS",
    trait_name: "",
    method_name: "APP_STATUS",
    display_name: "App status",
    description: "The app's problems, from its server.",
    category: "App",
    group: "App",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// The app's `APP_STATUS`: the server's answer, asked for whoever runs it
/// (the extension, acting for its caller), with a token minted for them.
fn app_status(list: &ActionList, at: SocketAddr, issuer: Issuer, rt: tokio::runtime::Handle) {
    list.register_async_with(
        &APP_STATUS,
        Effect::ReadOnly,
        move |caller: &ActionCaller, (): ()| {
            let who = caller.principal().cloned().unwrap_or(Principal::Anonymous);
            let token = issuer.mint(who);
            let call = rt.spawn(async move {
                let endpoint = Endpoint::url(format!("ws://{at}/vox")).with_bearer(Some(token));
                // A pool of its own: identity is per connection.
                let client: StatusServiceClient = Pool::default()
                    .client(&endpoint)
                    .await
                    .map_err(|e| format!("connect: {e}"))?;
                client.problems().await.map_err(|e| e.to_string())
            });
            async move {
                let problems = call.await.map_err(|e| e.to_string())??;
                Ok(RawJson::new(json!({ "problems": problems }).to_string()))
            }
        },
    )
    .unwrap();
}

fn lab_metronome() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/lab/extensions/metronome")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn user(id: &str) -> Principal {
    Principal::User {
        user_id: id.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_extensions_principal_reaches_the_gate_of_a_server_the_app_calls_for_it() {
    let (served, engine, issuer) = (Served::default(), Engine::default(), Issuer::default());
    let at = serve(served.clone(), engine.clone(), issuer.clone()).await;

    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    app_status(&list, at, issuer, tokio::runtime::Handle::current());
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    ext.install_actions().unwrap();
    let dev = tmp.path().join("dev/metronome");
    copy(&lab_metronome(), &dev);
    let _ = ext.dev(&dev);

    let run = |who: Principal| {
        let list = list.clone();
        async move {
            list.run_json_async_with(&ActionCaller::new(who), STATUS, b"null")
                .await
                .map(|out| serde_json::from_slice::<Value>(&out).unwrap_or(Value::Null))
        }
    };

    // Run by bob: the metronome's APP_STATUS reached the server as the
    // metronome acting for bob, and the gate let it through.
    let out = run(user("bob")).await.unwrap();
    assert_eq!(out["problems"], 1, "the server's one line: {out}");
    let bob_via_metronome = Principal::extension(ID, Some(user("bob")));
    assert!(
        engine
            .0
            .lock()
            .unwrap()
            .contains(&(bob_via_metronome.clone(), true)),
        "{:?}",
        engine.0.lock().unwrap()
    );

    // Run by mallory: the same extension, but the gate decides on the
    // person behind it too, and refuses.
    let _ = run(user("mallory")).await;
    let decided = engine.0.lock().unwrap().clone();
    assert!(
        decided.contains(&(Principal::extension(ID, Some(user("mallory"))), false)),
        "{decided:?}"
    );
    // Nobody reached it as anyone but the extension.
    assert!(
        decided
            .iter()
            .all(|(who, _)| who.extension_id() == Some(ID)),
        "{decided:?}"
    );

    // The service saw the caller the gate resolved, and only bob's call.
    assert_eq!(
        *served.0.lock().unwrap(),
        vec!["extension:fts.metronome for user:bob".to_owned()]
    );
}
