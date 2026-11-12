//! A node with an island (studio#445), for the browser test
//! `apps/web/test/share-island.test.mjs`: as `architect-desktop serve`
//! starts one (its config from `ARCHITECT_*`), and with the built web
//! folder at `ARCHITECT_ISLAND_FOLDER` as an island, its view
//! `ARCHITECT_ISLAND_VIEW` in both slots of the now-playing view, its
//! stylesheet `ARCHITECT_ISLAND_SHEET` when set. An island is otherwise
//! only what a distribution links in (`Distribution::island`): this is a
//! test's stand-in for one, never a node's configuration.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::borrow::Cow;
use std::path::Path;

use architect_desktop::node::{Config, start_with_islands};
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

fn main() {
    let folder = std::env::var("ARCHITECT_ISLAND_FOLDER").expect("ARCHITECT_ISLAND_FOLDER");
    let view = std::env::var("ARCHITECT_ISLAND_VIEW").unwrap_or_else(|_| "counter".to_owned());
    let mut list = Vec::new();
    files(Path::new(&folder), Path::new(&folder), &mut list);
    let mut island = SiteIsland::new("lab.wasi-counter", list)
        .progress(view.clone())
        .transport(view);
    if let Ok(sheet) = std::env::var("ARCHITECT_ISLAND_SHEET") {
        island = island.stylesheet(sheet);
    }
    let config = Config::load(None, |k| std::env::var(k).ok()).expect("the config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let running = start_with_islands(
            config,
            architect_sites::Blocks::new(),
            Vec::new(),
            vec![island],
        )
        .await
        .expect("the node started");
        println!("island node on http://{}", running.http);
        // Until the test kills it.
        std::future::pending::<()>().await;
        drop(running);
    });
}
