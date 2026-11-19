//! A plain node serves a web package from a folder its configuration
//! names (`ARCHITECT_WEB_PACKAGES`, `Config::web_packages`): with no
//! island in its distribution, the package's island is listed and its
//! files served under `/_ext/` exactly as a compiled-in island's are, its
//! claims, favourites and share layers read from the manifest in the
//! folder; a folder that does not match its lock, or whose id is already
//! served, is left out and the node still starts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use architect_desktop::node::shares::{FAVOURITES_LISTING, ISLANDS_LISTING};
use architect_desktop::node::{self, Config, IrohMode, Parts, Running};
use architect_distribution::Distribution;
use base64::Engine as _;
use sha2::Digest as _;

const GLUE: &[u8] = b"export default async function init() {}\nexport function mount(view) {}\n";
const WASM: &[u8] = b"\0asm\x01\0\0\0";
const SHEET: &[u8] = b".band{}";

const MANIFEST: &str = r#"
id = "demo.band"
name = "Band"
version = "1.2.0"
kind = "wasm"

[capabilities]
reads = ["documents", "files"]

[[views]]
id = "progress"
title = "Progress"

[[views]]
id = "transport"
title = "Transport"

[[views]]
id = "home"
title = "Home"

[[views]]
id = "mixer"
title = "Mixer"

[[views]]
id = "list"
title = "Setlist"

[site]
progress = "progress"
transport = "transport"
home = "home"
pages = ["mixer"]
stylesheet = "island.css"

[[embeds]]
id = "setlist"
title = "Setlist"
view = "list"
claims = { note_type = ["setlist"] }
reads = ["documents"]
properties = ["date"]

[[favourites]]
note_type = "setlist"

[[favourites.tabs]]
id = "mixer"
label = "Mixer"
icon = "sliders"
target = { view = "demo.band/mixer" }
"#;

fn sha256(bytes: &[u8]) -> String {
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
    )
}

/// A built web folder for `manifest` in `dir`, as `architect-ext
/// build-web` leaves one: its files, `architect.json` and the lock.
fn build(dir: &Path, manifest: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let manifest: toml::Value = toml::from_str(manifest).unwrap();
    let metadata = serde_json::to_vec(&serde_json::json!({
        "module": "band.js", "world": "0.1.0", "manifest": manifest,
    }))
    .unwrap();
    let files: [(&str, &[u8]); 4] = [
        ("architect.json", &metadata),
        ("island.css", SHEET),
        ("web-band.js", GLUE),
        ("web-band_bg.wasm", WASM),
    ];
    let mut locked = serde_json::Map::new();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
        locked.insert(name.to_owned(), sha256(bytes).into());
    }
    std::fs::write(
        dir.join("architect.lock.json"),
        serde_json::to_vec(&serde_json::json!({"lockfileVersion": 1, "files": locked})).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("BUILT_FROM"), "a commit").unwrap();
}

/// A node of a distribution with no island, told of `packages`.
async fn start(tmp: &Path, packages: Vec<PathBuf>) -> Running {
    let dir = tmp.join("host");
    std::fs::create_dir_all(dir.join("data/vaults/notes")).unwrap();
    let config = Config {
        name: "web-packages".to_owned(),
        bind: "127.0.0.1:0".to_owned(),
        data_dir: dir.join("data"),
        vaults: [("notes".to_owned(), PathBuf::from("vaults/notes"))].into(),
        iroh: IrohMode::Local,
        sweep_secs: 3600,
        web_packages: packages,
        ..Config::default()
    };
    let dist = Distribution::new("Plain");
    assert!(dist.islands().is_empty(), "no island compiled in");
    let parts = Parts::of(&dist, &config);
    node::start_parts(config, parts).await.unwrap()
}

async fn json(node: &Running, path: &str) -> serde_json::Value {
    reqwest::get(format!("http://{}{path}", node.http))
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_plain_node_serves_a_web_package_its_configuration_names() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("band");
    build(&folder, MANIFEST);
    let node = start(tmp.path(), vec![folder.clone()]).await;
    let listing = json(&node, ISLANDS_LISTING).await;
    let islands = listing["islands"].as_array().unwrap();
    assert_eq!(islands.len(), 1, "{listing}");
    let it = &islands[0];
    assert_eq!(it["id"], "demo.band");
    assert_eq!(it["progress"], "progress");
    assert_eq!(it["transport"], "transport");
    assert_eq!(it["home"], "home");
    assert_eq!(it["views"], serde_json::json!(["mixer"]));
    assert_eq!(it["stylesheet"], "island.css");
    assert_eq!(it["stylesheetIntegrity"], sha256(SHEET));
    assert_eq!(it["reads"], serde_json::json!(["documents", "files"]));
    // Its claim on a note type, listed as a compiled-in island's is.
    let compiled_in = architect_sites::SiteIsland::new(
        "demo.band",
        vec![("web-band.js".to_owned(), std::borrow::Cow::Borrowed(GLUE))],
    )
    .embed_note("list", ["setlist"], ["date"]);
    assert_eq!(
        it["embeds"],
        architect_sites::island::page_json(&[compiled_in])["islands"][0]["embeds"]
    );
    assert_eq!(
        it["embeds"],
        serde_json::json!([{
            "view": "list", "note_type": ["setlist"],
            "reads": ["documents"], "properties": ["date"],
        }])
    );
    // Its files under `/_ext/`, the bytes the listing's digests say.
    let folder_path = it["folder"].as_str().unwrap();
    assert!(folder_path.starts_with("/_ext/island/demo.band."), "{folder_path}");
    let web = &it["web"];
    assert_eq!(web["entry"], "web-band.js");
    let client = reqwest::Client::new();
    let get = |name: &str| {
        client
            .get(format!("http://{}{folder_path}{name}", node.http))
            .send()
    };
    for (name, digest) in web["files"].as_object().unwrap() {
        let got = get(name).await.unwrap();
        assert_eq!(got.status(), 200, "{name}");
        assert_eq!(&sha256(&got.bytes().await.unwrap()), digest, "{name}");
    }
    let sheet = get("island.css").await.unwrap();
    assert_eq!(sheet.status(), 200);
    assert_eq!(sheet.bytes().await.unwrap().as_ref(), SHEET);
    let lock = get("architect.lock.json").await.unwrap();
    assert_eq!(
        Some(sha256(&lock.bytes().await.unwrap())).as_deref(),
        it["integrity"].as_str()
    );
    // A file of the folder its lock does not name is not served.
    assert_eq!(get("BUILT_FROM").await.unwrap().status(), 404);
    // Its favourites, declared by the package's manifest.
    let favourites = json(&node, FAVOURITES_LISTING).await;
    let listed = favourites["favourites"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{favourites}");
    assert_eq!(listed[0]["by"]["extension"], "demo.band");
    assert_eq!(listed[0]["favourites"]["note_type"], "setlist");
}

#[tokio::test]
async fn a_refused_or_repeated_package_is_left_out_and_the_node_still_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let (good, edited, twin) = (
        tmp.path().join("band"),
        tmp.path().join("edited"),
        tmp.path().join("twin"),
    );
    build(&good, MANIFEST);
    build(&edited, &MANIFEST.replace("demo.band", "demo.edited"));
    std::fs::write(edited.join("web-band.js"), b"export function mount() {}").unwrap();
    // The same id from another folder: the first one listed is kept.
    build(&twin, &MANIFEST.replace("pages = [\"mixer\"]", "pages = []"));
    let node = start(
        tmp.path(),
        vec![edited, tmp.path().join("absent"), good, twin],
    )
    .await;
    let listing = json(&node, ISLANDS_LISTING).await;
    let islands = listing["islands"].as_array().unwrap();
    assert_eq!(islands.len(), 1, "{listing}");
    assert_eq!(islands[0]["id"], "demo.band");
    assert_eq!(islands[0]["views"], serde_json::json!(["mixer"]));
    let favourites = json(&node, FAVOURITES_LISTING).await;
    assert_eq!(favourites["favourites"].as_array().unwrap().len(), 1);
}
