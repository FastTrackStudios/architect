//! The node's islands listing for the web app (#495): `GET
//! /_ext/islands.json` is `architect_sites::island::page_json` of the
//! islands the node was started with, as JSON, never cached, `nosniff`,
//! read-only; every file it names is served at the path it gives, with the
//! digest it gives; a node with no island lists none.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::borrow::Cow;
use std::path::PathBuf;

use architect_desktop::node::shares::ISLANDS_LISTING;
use architect_desktop::node::{self, Config, IrohMode, Running};
use architect_sites::SiteIsland;
use base64::Engine as _;
use sha2::Digest as _;

const GLUE: &[u8] = b"export default async function init() {}\nexport function mount(view) {}\n";
const WASM: &[u8] = b"\0asm\x01\0\0\0";

fn island() -> SiteIsland {
    SiteIsland::new(
        "studio.transport",
        vec![
            ("web-studio-island.js".to_owned(), Cow::Borrowed(GLUE)),
            ("web-studio-island_bg.wasm".to_owned(), Cow::Borrowed(WASM)),
            ("island.css".to_owned(), Cow::Borrowed(b".x{}".as_slice())),
        ],
    )
    .progress("progress")
    .transport("transport")
    .lyrics("lyrics")
    .stylesheet("island.css")
}

async fn start(islands: Vec<SiteIsland>) -> (Running, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("host");
    std::fs::create_dir_all(dir.join("data/vaults/notes")).unwrap();
    let config = Config {
        name: "islands".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        iroh: IrohMode::Local,
        sweep_secs: 3600,
        ..Config::default()
    };
    let running =
        node::start_with_islands(config, architect_sites::Blocks::new(), Vec::new(), islands)
            .await
            .unwrap();
    (running, tmp)
}

fn sha256(bytes: &[u8]) -> String {
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
    )
}

#[tokio::test]
async fn the_islands_listing_is_the_nodes_islands_pinned_and_never_cached() {
    let (node, _tmp) = start(vec![island()]).await;
    let base = format!("http://{}", node.http);
    let client = reqwest::Client::new();
    let r = client
        .get(format!("{base}{ISLANDS_LISTING}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let h = r.headers().clone();
    assert_eq!(h["content-type"], "application/json");
    assert_eq!(h["cache-control"], "no-cache");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(h.contains_key("content-security-policy"));
    assert!(h.get("set-cookie").is_none());
    let json: serde_json::Value = r.json().await.unwrap();
    // Exactly what a share page embeds.
    assert_eq!(json, architect_sites::island::page_json(&[island()]));
    let it = &json["islands"][0];
    assert_eq!(it["id"], "studio.transport");
    assert_eq!(it["transport"], "transport");
    assert_eq!(it["lyrics"], "lyrics");
    let web = &it["web"];
    assert_eq!(web["entry"], "web-studio-island.js");
    let webview = web["webview"].as_str().unwrap();
    assert!(webview.starts_with("/_ext/webview."), "{webview}");
    // Every file it names is there, the bytes its digest says.
    let folder = it["folder"].as_str().unwrap();
    for (name, digest) in web["files"].as_object().unwrap() {
        let got = client
            .get(format!("{base}{folder}{name}"))
            .send()
            .await
            .unwrap();
        assert_eq!(got.status(), 200, "{name}");
        assert_eq!(&sha256(&got.bytes().await.unwrap()), digest, "{name}");
    }
    // The frame's document, with its own sandboxed policy.
    let frame = client.get(format!("{base}{webview}")).send().await.unwrap();
    assert_eq!(frame.status(), 200);
    let csp = frame.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(csp.starts_with("sandbox allow-scripts;"), "{csp}");
    assert!(csp.contains("connect-src 'none'"), "{csp}");
    // Read only.
    let post = client
        .post(format!("{base}{ISLANDS_LISTING}"))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), 405);
}

#[tokio::test]
async fn a_node_without_islands_lists_none() {
    let (node, _tmp) = start(Vec::new()).await;
    let json: serde_json::Value = reqwest::get(format!("http://{}{ISLANDS_LISTING}", node.http))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(json["islands"], serde_json::json!([]));
    assert!(
        json["webview"]
            .as_str()
            .unwrap()
            .starts_with("/_ext/webview.")
    );
}

/// `GET /_ext/favourites.json` (#607): the note types' `[[favourites]]`
/// the node was started with, as declared, each with the extension that
/// declared it and what it may run, never cached, `nosniff`, read-only,
/// nothing else in it; a declaration that breaks the manifest's rules is
/// left out.
#[tokio::test]
async fn the_favourites_listing_is_the_declarations_alone_and_never_cached() {
    use architect_desktop::node::shares::FAVOURITES_LISTING;
    let good: architect_favourites::TypeFavourites = toml::from_str(
        r#"
note_type = "recipe"
[[tabs]]
id = "steps"
label = "Steps"
icon = "list"
target = { view = "lab.cook/steps" }
"#,
    )
    .unwrap();
    let mut bad = good.clone();
    bad.note_type = "other".to_owned();
    bad.tabs[0].target = architect_favourites::Target::Place("https://evil.example/".to_owned());
    let by = architect_favourites::Declarer {
        extension: "lab.cook".to_owned(),
        actions: vec!["PLAYER_".to_owned()],
        effect: architect_favourites::Effect::Mutating,
    };
    let good = architect_favourites::Listed {
        by: by.clone(),
        favourites: good,
    };
    let bad = architect_favourites::Listed {
        by,
        favourites: bad,
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("host");
    std::fs::create_dir_all(dir.join("data/vaults/notes")).unwrap();
    let config = Config {
        name: "favourites".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        iroh: IrohMode::Local,
        sweep_secs: 3600,
        ..Config::default()
    };
    let node = node::start_with_shell(
        config,
        architect_sites::Blocks::new(),
        Vec::new(),
        Vec::new(),
        vec![good.clone(), bad],
    )
    .await
    .unwrap();
    let url = format!("http://{}{FAVOURITES_LISTING}", node.http);
    let client = reqwest::Client::new();
    let r = client.get(&url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let h = r.headers().clone();
    assert_eq!(h["content-type"], "application/json");
    assert_eq!(h["cache-control"], "no-cache");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(h.get("set-cookie").is_none());
    let json: serde_json::Value = r.json().await.unwrap();
    assert_eq!(json, serde_json::json!({ "favourites": [good] }));
    assert_eq!(json["favourites"][0]["by"]["extension"], "lab.cook");
    assert_eq!(json["favourites"][0]["by"]["effect"], "mutating");
    assert_eq!(client.post(&url).send().await.unwrap().status(), 405);
    // A node that declares none lists none.
    let (bare, _bare_tmp) = start(Vec::new()).await;
    let json: serde_json::Value = reqwest::get(format!("http://{}{FAVOURITES_LISTING}", bare.http))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(json, serde_json::json!({ "favourites": [] }));
    drop(tmp);
}
