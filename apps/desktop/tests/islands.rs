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

/// Every file under `dir`, as its path below it with `/` between.
fn files_below(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut walk = vec![dir.to_path_buf()];
    while let Some(at) = walk.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk.push(path);
            } else {
                out.push(
                    path.strip_prefix(dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    out.sort();
    out
}

/// The static `/_ext/` tree (`architect_sites::island::export`, written
/// by `<app> web-ext`) is the node's own answers: for islands and
/// favourites a node was started with, every exported path has the body,
/// the content type and exactly the headers the running node answers
/// there, the documents' own policies and both web view documents among
/// them; and the written tree is those files and their `headers.json`.
#[tokio::test]
async fn the_static_ext_tree_is_what_the_node_answers_path_for_path() {
    use architect_sites::island::{self, WebRtc};
    // One island granted WebRTC (and declaring it), with a nested file
    // and one that is no web file; the plain one beside it.
    let calling = SiteIsland::new(
        "demo.call",
        vec![
            (
                "architect.json".to_owned(),
                Cow::Borrowed(
                    br#"{"module":"web-call.js","manifest":{"id":"demo.call","capabilities":{"webrtc":["peer-connections"]}}}"#
                        .as_slice(),
                ),
            ),
            ("web-call.js".to_owned(), Cow::Borrowed(GLUE)),
            ("web-call_bg.wasm".to_owned(), Cow::Borrowed(WASM)),
            (
                "dist/big.js".to_owned(),
                Cow::Owned(b"// a module large enough to gzip\n".repeat(200)),
            ),
            ("notes.txt".to_owned(), Cow::Borrowed(b"plain".as_slice())),
        ],
    )
    .progress("call")
    .embed_note("call", ["setlist"], ["date"])
    .reads(["documents", "files"])
    .grant_webrtc();
    let islands = vec![calling, island()];
    let favourites: architect_favourites::TypeFavourites = toml::from_str(
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
    let favourites = vec![architect_favourites::Listed {
        by: architect_favourites::Declarer {
            extension: "lab.cook".to_owned(),
            actions: vec!["PLAYER_".to_owned()],
            effect: architect_favourites::Effect::Mutating,
        },
        favourites,
    }];

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("host");
    std::fs::create_dir_all(dir.join("data/vaults/notes")).unwrap();
    let config = Config {
        name: "static-ext".to_owned(),
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
        islands.clone(),
        favourites.clone(),
    )
    .await
    .unwrap();
    let base = format!("http://{}", node.http);
    let client = reqwest::Client::new();

    let all = island::export(&islands, &favourites);
    let paths: Vec<&str> = all.iter().map(|e| e.path.as_str()).collect();
    for must in [
        ISLANDS_LISTING,
        architect_desktop::node::shares::FAVOURITES_LISTING,
        island::sandbox_path(),
        island::webview_path_for(WebRtc::Blocked),
        island::webview_path_for(WebRtc::Granted),
        &format!("{}index.js", island::runtime_path()),
        &format!("{}dist/instance-source.js", island::runtime_path()),
        &format!("{}dist/big.js", islands[0].folder()),
        &format!("{}notes.txt", islands[0].folder()),
        &format!("{}island.css", islands[1].folder()),
    ] {
        assert!(paths.contains(&must), "{must} is not exported: {paths:?}");
    }
    // Every file the listing names is in the tree.
    let listing = island::page_json(&islands);
    for it in listing["islands"].as_array().unwrap() {
        let folder = it["folder"].as_str().unwrap();
        for name in it["web"]["files"].as_object().unwrap().keys() {
            assert!(
                paths.contains(&format!("{folder}{name}").as_str()),
                "{name}"
            );
        }
        assert!(paths.contains(&it["web"]["webview"].as_str().unwrap()));
    }

    for e in &all {
        let r = client
            .get(format!("{base}{}", e.path))
            .header("accept-encoding", "identity")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{}", e.path);
        assert_eq!(r.headers()["content-type"], e.content_type, "{}", e.path);
        // Exactly the node's headers, but the two a server works out for
        // each response itself.
        let mut answered: Vec<(String, String)> = r
            .headers()
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "content-length" | "date"))
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap().to_owned()))
            .collect();
        answered.sort();
        let mut exported: Vec<(String, String)> = e
            .headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        exported.sort();
        assert_eq!(answered, exported, "{}", e.path);
        assert_eq!(
            r.bytes().await.unwrap().as_ref(),
            e.body.as_slice(),
            "{}",
            e.path
        );
    }
    // A browser that takes gzip gets the same file packed, the same
    // headers and the encoding: what a static server does by itself.
    let big = all
        .iter()
        .find(|e| e.path.ends_with("dist/big.js"))
        .unwrap();
    let packed = reqwest::Client::builder()
        .no_gzip()
        .build()
        .unwrap()
        .get(format!("{base}{}", big.path))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(packed.headers()["content-encoding"], "gzip");
    for (name, value) in &big.headers {
        assert_eq!(packed.headers()[*name], value.as_str(), "{name}");
    }

    // Written: those files, each at its path, and their headers.
    let out = tmp.path().join("bundle");
    std::fs::create_dir_all(out.join("_ext/stale")).unwrap();
    std::fs::write(out.join("_ext/stale/old.js"), "old").unwrap();
    let wrote = architect_desktop::write_web_ext(&out, &islands, &favourites).unwrap();
    assert_eq!(wrote, all.len());
    for e in &all {
        let at = out.join(e.path.trim_start_matches('/'));
        assert_eq!(std::fs::read(&at).unwrap(), e.body, "{}", e.path);
    }
    let mut want: Vec<String> = all
        .iter()
        .map(|e| e.path.trim_start_matches("/_ext/").to_owned())
        .chain(["headers.json".to_owned()])
        .collect();
    want.sort();
    assert_eq!(
        files_below(&out.join("_ext")),
        want,
        "nothing else, nothing stale"
    );
    let headers: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("_ext/headers.json")).unwrap()).unwrap();
    assert_eq!(headers, island::headers_json(&all));
    let sandbox = &headers[island::sandbox_path()];
    assert!(
        sandbox["content-security-policy"]
            .as_str()
            .unwrap()
            .starts_with("sandbox allow-scripts;"),
        "{sandbox}"
    );
    assert_eq!(headers[ISLANDS_LISTING]["cache-control"], "no-cache");
    // The headers file is the tree's alone: a node has none.
    for nowhere in [island::HEADERS_PATH, "/_ext/stale/old.js"] {
        let r = client.get(format!("{base}{nowhere}")).send().await.unwrap();
        assert_eq!(r.status(), 404, "{nowhere}");
    }

    // A node with no island answers only what needs none: no island's
    // file, no document that allows WebRTC.
    let (empty, _empty_tmp) = start(Vec::new()).await;
    for e in &all {
        let status = client
            .get(format!("http://{}{}", empty.http, e.path))
            .send()
            .await
            .unwrap()
            .status();
        let needs_island = e.path.starts_with("/_ext/island/")
            || e.path == island::webview_path_for(WebRtc::Granted);
        assert_eq!(status == 404, needs_island, "{}", e.path);
    }
}

/// `<app> web-ext <out-dir>` writes its distribution's own tree (its
/// islands and favourites), and says how it is used when no folder is
/// named.
#[test]
fn the_web_ext_command_writes_its_distributions_tree() {
    use std::process::ExitCode;
    let dist = architect_desktop::Distribution::new("Web Ext Test").island(island());
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("out");
    let ran = architect_desktop::web_ext_command(&dist, out.to_str());
    assert_eq!(format!("{ran:?}"), format!("{:?}", ExitCode::SUCCESS));
    let listing: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("_ext/islands.json")).unwrap()).unwrap();
    assert_eq!(listing, architect_sites::island::page_json(&dist.islands()));
    let folder = listing["islands"][0]["folder"].as_str().unwrap();
    assert_eq!(
        std::fs::read(out.join(format!(
            "{}web-studio-island.js",
            folder.trim_start_matches('/')
        )))
        .unwrap(),
        GLUE
    );
    let headers: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("_ext/headers.json")).unwrap()).unwrap();
    assert_eq!(
        headers,
        architect_sites::island::headers_json(&architect_sites::island::export(
            &dist.islands(),
            &dist.favourites()
        ))
    );
    for missing in [None, Some(""), Some("--help")] {
        let ran = architect_desktop::web_ext_command(&dist, missing);
        assert_eq!(format!("{ran:?}"), format!("{:?}", ExitCode::from(64)));
    }
}
