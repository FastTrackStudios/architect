//! A node with an island (studio#445), for the browser test
//! `apps/web/test/share-island.test.mjs`: as `architect-desktop serve`
//! starts one (its config from `ARCHITECT_*`), and with the built web
//! folder at `ARCHITECT_ISLAND_FOLDER` as an island, its view
//! `ARCHITECT_ISLAND_VIEW` in both slots of the now-playing view, its
//! stylesheet `ARCHITECT_ISLAND_SHEET` when set, its id
//! `ARCHITECT_ISLAND_ID` (`lab.wasi-counter` when unset), the transport
//! slot's view `ARCHITECT_ISLAND_TRANSPORT` and a lyrics view
//! `ARCHITECT_ISLAND_LYRICS` when set (Studio's island as Studio declares
//! it: `progress`, `transport`, `lyrics`, for the web app's frame host on
//! the local stack, #495), note embeds from `ARCHITECT_ISLAND_EMBEDS`
//! (`view=type,type;view=type`: a view drawn at the head of a note of
//! those types in the web app), WebRTC granted when
//! `ARCHITECT_ISLAND_WEBRTC=1` (and declared); with no
//! `ARCHITECT_ISLAND_FOLDER`, no island. `ARCHITECT_FAVOURITES` names a
//! TOML file of `[[favourites]]` (a distribution's compiled-in extensions'
//! declarations: a share page's phone shell, `apps/web/test/share-layout.test.mjs`).
//! An island or favourites are otherwise only what a distribution links
//! in: this is a test's stand-in, never a node's configuration.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::borrow::Cow;
use std::path::Path;

use architect_desktop::node::{Config, start_with_shell};
use architect_sites::SiteIsland;

fn files(root: &Path, dir: &Path, out: &mut Vec<(String, Cow<'static, [u8]>)>) {
    for entry in std::fs::read_dir(dir).expect("the island's folder") {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, Cow::Owned(std::fs::read(&path).unwrap())));
        }
    }
}

fn island(folder: &str) -> SiteIsland {
    let view = std::env::var("ARCHITECT_ISLAND_VIEW").unwrap_or_else(|_| "counter".to_owned());
    let mut list = Vec::new();
    files(Path::new(folder), Path::new(folder), &mut list);
    let id = std::env::var("ARCHITECT_ISLAND_ID").unwrap_or_else(|_| "lab.wasi-counter".to_owned());
    let transport = std::env::var("ARCHITECT_ISLAND_TRANSPORT").unwrap_or_else(|_| view.clone());
    let mut island = SiteIsland::new(id, list)
        .progress(view)
        .transport(transport);
    if let Ok(lyrics) = std::env::var("ARCHITECT_ISLAND_LYRICS") {
        island = island.lyrics(lyrics);
    }
    if let Ok(embeds) = std::env::var("ARCHITECT_ISLAND_EMBEDS") {
        for one in embeds.split(';') {
            if let Some((view, types)) = one.split_once('=') {
                island = island.embed(view.trim(), types.split(',').map(str::trim));
            }
        }
    }
    if let Ok(sheet) = std::env::var("ARCHITECT_ISLAND_SHEET") {
        island = island.stylesheet(sheet);
    }
    // The operator's grant of WebRTC (`share-webview.test.mjs`): it takes
    // effect only when the island's manifest declares it too.
    if std::env::var("ARCHITECT_ISLAND_WEBRTC").is_ok_and(|v| v == "1") {
        island = island.grant_webrtc();
    }
    island
}

/// `[[favourites]]` from the TOML file at `path`.
fn favourites(path: &str) -> Vec<architect_favourites::TypeFavourites> {
    let text = std::fs::read_to_string(path).expect("the favourites file");
    let table: toml::Table = toml::from_str(&text).expect("TOML");
    table
        .get("favourites")
        .cloned()
        .map(|v| v.try_into().expect("[[favourites]]"))
        .unwrap_or_default()
}

fn main() {
    let islands: Vec<SiteIsland> = std::env::var("ARCHITECT_ISLAND_FOLDER")
        .ok()
        .map(|f| island(&f))
        .into_iter()
        .collect();
    let favourites = std::env::var("ARCHITECT_FAVOURITES")
        .ok()
        .map(|f| favourites(&f))
        .unwrap_or_default();
    let config = Config::load(None, |k| std::env::var(k).ok()).expect("the config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let running = start_with_shell(
            config,
            architect_sites::Blocks::new(),
            Vec::new(),
            islands,
            favourites,
        )
        .await
        .expect("the node started");
        println!("island node on http://{}", running.http);
        // Until the test kills it.
        std::future::pending::<()>().await;
        drop(running);
    });
}
