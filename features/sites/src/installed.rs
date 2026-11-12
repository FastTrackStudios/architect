//! The blocks of the extensions installed in a folder (#488).
//!
//! A node holds no running app, but it can hold extensions: a folder of
//! packages, one per extension (`<dir>/<id>/extension.toml`), as an
//! install writes them. [`InstalledBlocks`] draws every fence those
//! packages claim, the way the app draws them:
//!
//! - an `[[embeds]] page` by its page, once per block ([`crate::PageBlocks`],
//!   feature `pages`);
//! - an `[[embeds]] view` of a WASI component (`[wasm] component`) by
//!   that view, mounted once per block ([`crate::ComponentBlocks`],
//!   feature `wasi`), so a renderer written in Rust (Keyflow's engraver)
//!   runs on the node without being linked into it.
//!
//! The folder is read again when what is in it changes (a package added,
//! removed, or its `extension.toml` written), at the next build.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use architect_extensions::contrib::Claim;
use architect_extensions::manifest::Manifest;
use architect_extensions::share_layers::ShareLayersDecl;

use crate::blocks::{Block, BlockRenderer, Blocks, Drawn};

/// The blocks of the extensions installed in a folder, as one renderer.
pub struct InstalledBlocks {
    dir: PathBuf,
    held: Mutex<Held>,
}

#[derive(Default)]
struct Held {
    /// What the folder was when `blocks` was read; `None` before.
    stamp: Option<Vec<(String, Option<SystemTime>)>>,
    blocks: Blocks,
    /// Each extension's renderer, by id, for [`InstalledBlocks::ids`].
    ids: Vec<String>,
}

impl std::fmt::Debug for InstalledBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledBlocks")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl InstalledBlocks {
    /// The extensions installed in `dir` (missing: none yet).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            held: Mutex::new(Held::default()),
        }
    }

    /// The folder it reads.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The extensions that draw a block, by id, as the folder is now.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.current().1
    }

    /// The renderers as the folder is now: read again if it changed.
    fn current(&self) -> (Blocks, Vec<String>) {
        let stamp = stamp(&self.dir);
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.stamp.as_ref() != Some(&stamp) {
            let (blocks, ids) = read(&self.dir, &stamp);
            *held = Held {
                stamp: Some(stamp),
                blocks,
                ids,
            };
        }
        (held.blocks.clone(), held.ids.clone())
    }
}

impl BlockRenderer for InstalledBlocks {
    fn name(&self) -> &'static str {
        "installed"
    }

    fn claims(&self, claim: &Claim) -> bool {
        Blocks::claims(&self.current().0, claim)
    }

    fn drawn_by(&self, claim: &Claim) -> String {
        self.current().0.drawn_by(claim)
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        match self.current().0.draw(block) {
            // Already made safe (`Blocks::draw`); written as it is.
            Some((_, drawn)) => drawn.map(Drawn::Html),
            None => Err(format!("no extension claims {}", block.claim)),
        }
    }
}

/// The `[[share_layers]]` rules (#528) of the extensions installed in a
/// folder: which layer of a share link each file of their note types is
/// in. Read again when what is in the folder changes.
///
/// A slot is read as the installer reads it
/// (`architect_extensions::install`): its own `extension.toml`, else the
/// package its `installed.toml` links to (a development copy installed
/// from a path). A slot whose manifest does not read where it resolves is
/// broken.
pub struct InstalledLayers {
    dir: PathBuf,
    held: Mutex<(Option<Vec<SlotStamp>>, Arc<Installed>)>,
}

/// One installed extension's layer rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Its slot's folder name under the extensions folder.
    pub folder: String,
    /// The id its manifest claims.
    pub id: String,
    /// Its rules (none: it declares none).
    pub rules: Vec<ShareLayersDecl>,
}

/// The installed extensions' layer rules, as the folder was when read.
#[derive(Debug, Default)]
pub struct Installed {
    /// Counts up each time the folder is read again (it changed).
    pub generation: u64,
    /// Each extension read.
    pub packages: Vec<Package>,
    /// Each manifest that could not be read or is not a manifest, and
    /// why. While there is one, which layer a file is in is not known.
    pub broken: Vec<(PathBuf, String)>,
}

/// One file as stamped: when it was written, its size and its inode
/// (0 where there is none), so two writes in one tick of a coarse clock
/// are still told apart.
type FileStamp = Option<(SystemTime, u64, u64)>;

/// A slot as stamped: its name, and its `extension.toml`, its
/// `installed.toml` and the manifest it links to.
type SlotStamp = (String, [FileStamp; 3]);

fn file_stamp(path: &Path) -> FileStamp {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = std::os::unix::fs::MetadataExt::ino(&meta);
    #[cfg(not(unix))]
    let inode = 0;
    Some((meta.modified().ok()?, meta.len(), inode))
}

impl std::fmt::Debug for InstalledLayers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledLayers")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// Where a slot's manifest is: its own, or its linked package's. None:
/// nothing is installed there.
fn manifest_of(slot: &Path) -> Option<Result<PathBuf, String>> {
    let own = slot.join("extension.toml");
    if own.is_file() {
        return Some(Ok(own));
    }
    let record = slot.join("installed.toml");
    if !record.is_file() {
        return None;
    }
    Some(
        std::fs::read_to_string(&record)
            .map_err(|e| e.to_string())
            .and_then(|text| text.parse::<toml::Table>().map_err(|e| e.to_string()))
            .and_then(|table| {
                table
                    .get("link")
                    .and_then(toml::Value::as_str)
                    .map(|link| slot.join(link).join("extension.toml"))
                    .ok_or_else(|| {
                        format!(
                            "{} names no package (`link`) and the slot has no extension.toml",
                            record.display()
                        )
                    })
            }),
    )
}

/// What `dir` holds, for [`InstalledLayers`]: each slot and when its
/// manifests were written, in order.
fn slot_stamps(dir: &Path) -> Vec<SlotStamp> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<SlotStamp> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let slot = e.path();
            let linked = match manifest_of(&slot) {
                Some(Ok(path)) => file_stamp(&path),
                _ => None,
            };
            (
                e.file_name().to_string_lossy().into_owned(),
                [
                    file_stamp(&slot.join("extension.toml")),
                    file_stamp(&slot.join("installed.toml")),
                    linked,
                ],
            )
        })
        .collect();
    out.sort();
    out
}

impl InstalledLayers {
    /// The extensions installed in `dir` (missing: none yet).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            held: Mutex::default(),
        }
    }

    /// What the folder holds now: read again if it changed. A manifest
    /// that does not read is warned of once, naming the file.
    #[must_use]
    pub fn read(&self) -> Arc<Installed> {
        let stamp = slot_stamps(&self.dir);
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.0.as_ref() != Some(&stamp) {
            let mut read = Installed {
                generation: held.1.generation.wrapping_add(1),
                ..Installed::default()
            };
            for (folder, _) in &stamp {
                if folder.starts_with('.') {
                    continue;
                }
                let read_one = match manifest_of(&self.dir.join(folder)) {
                    None => continue,
                    Some(Err(why)) => Err((self.dir.join(folder).join("installed.toml"), why)),
                    Some(Ok(path)) => std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|text| Manifest::parse(&text).map_err(|e| e.to_string()))
                        .map_err(|why| (path, why)),
                };
                match read_one {
                    Ok(manifest) => read.packages.push(Package {
                        folder: folder.clone(),
                        id: manifest.id,
                        rules: manifest.share_layers,
                    }),
                    Err((path, error)) => {
                        tracing::warn!(
                            path = %path.display(),
                            %error,
                            "an installed extension's manifest does not read: until it is fixed, share links below Full hold back every attached file"
                        );
                        read.broken.push((path, error));
                    }
                }
            }
            *held = (Some(stamp), Arc::new(read));
        }
        Arc::clone(&held.1)
    }
}

/// What `dir` holds: each package's name and when its manifest was
/// written, in order.
fn stamp(dir: &Path) -> Vec<(String, Option<SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let written = std::fs::metadata(e.path().join("extension.toml"))
                .and_then(|m| m.modified())
                .ok();
            (e.file_name().to_string_lossy().into_owned(), written)
        })
        .collect();
    out.sort();
    out
}

/// The renderers of the packages in `stamp`, under `dir`.
fn read(dir: &Path, stamp: &[(String, Option<SystemTime>)]) -> (Blocks, Vec<String>) {
    let mut blocks = Blocks::new();
    let mut ids = Vec::new();
    for (name, written) in stamp {
        if written.is_none() || name.starts_with('.') {
            continue;
        }
        let package = dir.join(name);
        for renderer in package_blocks(&package) {
            ids.push(renderer.name().to_owned());
            blocks.add(renderer);
        }
    }
    (blocks, ids)
}

/// The renderers of the package in `dir`: its embed pages, its
/// component's embed views.
fn package_blocks(dir: &Path) -> Vec<Arc<dyn BlockRenderer>> {
    let mut out: Vec<Arc<dyn BlockRenderer>> = Vec::new();
    #[cfg(feature = "pages")]
    match crate::PageBlocks::load(dir) {
        Ok(pages) if !pages.fences().is_empty() => out.push(Arc::new(pages)),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "an installed extension's embed pages did not load"),
    }
    #[cfg(feature = "wasi")]
    match crate::ComponentBlocks::load(dir) {
        Ok(Some(component)) => out.push(Arc::new(component)),
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "an installed extension's component did not load"),
    }
    #[cfg(not(any(feature = "pages", feature = "wasi")))]
    let _ = dir;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_missing_folder_claims_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = InstalledBlocks::new(tmp.path().join("nope"));
        assert!(!missing.claims(&Claim::Fence("kf".into())));
        assert!(missing.ids().is_empty());
        std::fs::create_dir_all(tmp.path().join("half")).unwrap();
        let empty = InstalledBlocks::new(tmp.path());
        assert!(!empty.claims(&Claim::Fence("kf".into())));
    }

    /// An extension installed after the first build is drawn at the next
    /// one; removed, it is not.
    #[cfg(feature = "pages")]
    #[test]
    fn a_page_extension_installed_later_draws_its_fence() {
        let tmp = tempfile::tempdir().unwrap();
        let installed = InstalledBlocks::new(tmp.path());
        let kf = Claim::Fence("shout".into());
        assert!(!installed.claims(&kf));

        let pkg = tmp.path().join("demo.shout");
        std::fs::create_dir_all(pkg.join("pages")).unwrap();
        std::fs::write(
            pkg.join("extension.toml"),
            "id = \"demo.shout\"\nname = \"Shout\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
             [[embeds]]\nid = \"shout\"\npage = \"pages/shout.tsx\"\nclaims = { fence = [\"shout\"] }\n",
        )
        .unwrap();
        std::fs::write(
            pkg.join("pages/shout.tsx"),
            "import { h, useEmbed } from \"architect\";\n\
             export default function Shout() {\n\
               const e = useEmbed();\n\
               return h(\"svg\", { viewBox: \"0 0 10 10\", \"data-follow-playhead\": \"\" },\n\
                 h(\"rect\", { class: \"bar\", \"data-at\": \"1.5\", onClick: () => {} }),\n\
                 h(\"text\", {}, (e?.source ?? \"\").toUpperCase()));\n\
             }\n",
        )
        .unwrap();
        assert!(installed.claims(&kf));
        assert_eq!(installed.ids(), vec!["demo.shout".to_owned()]);
        let block = Block {
            claim: &kf,
            info: "shout",
            source: "hey",
            page: "a.md",
        };
        let Drawn::Html(html) = installed.render(&block).unwrap() else {
            panic!("html");
        };
        assert!(
            html.contains("<svg viewBox=\"0 0 10 10\" data-follow-playhead=\"\">"),
            "{html}"
        );
        assert!(html.contains("data-at=\"1.5\""), "{html}");
        assert!(html.contains("HEY"), "{html}");

        std::fs::remove_dir_all(&pkg).unwrap();
        assert!(!installed.claims(&kf));
    }

    /// Two writes of a manifest in one tick of a coarse clock: the second
    /// (another size) is read, not the first kept (#540's third review).
    #[test]
    fn a_manifest_rewritten_within_one_tick_is_read_again() {
        let tmp = tempfile::tempdir().unwrap();
        let slot = tmp.path().join("demo.songs");
        std::fs::create_dir_all(&slot).unwrap();
        let manifest = slot.join("extension.toml");
        let head = "id = \"demo.songs\"\nname = \"X\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n";
        std::fs::write(
            &manifest,
            format!("{head}[[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\"]\n"),
        )
        .unwrap();
        let when = std::fs::metadata(&manifest).unwrap().modified().unwrap();
        let layers = InstalledLayers::new(tmp.path());
        assert_eq!(layers.read().packages[0].rules[0].full, ["Media/**"]);
        std::fs::write(&manifest, format!("{head}[[share_layers]]\nnote_type = \"song\"\nfull = [\"Media/**\", \"Stems/**\"]\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&manifest)
            .unwrap()
            .set_modified(when)
            .unwrap();
        assert_eq!(
            layers.read().packages[0].rules[0].full,
            ["Media/**", "Stems/**"]
        );
    }
}
