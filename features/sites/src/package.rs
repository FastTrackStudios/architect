//! A web package: an extension's built web folder (`architect-ext
//! build-web`'s), loaded by a node from disk as an island, from its
//! manifest alone.
//!
//! A distribution ships client files only: the node it runs on is plain
//! Architect, told where the folder is (`ARCHITECT_WEB_PACKAGES`). The
//! folder is checked against its own `architect.lock.json` (every file
//! the page loads, with its `sha256-…` digest, base64, as `build-web`
//! writes it) before anything in it is used, and refused whole when:
//! - the folder, the lock or a file the lock names is not a regular file
//!   (or, for the folder, a directory) of its own: links, folders and
//!   special files are refused, never followed;
//! - the lock is unreadable, names nothing, names a path that is not a
//!   plain file name in the folder, or does not name the stylesheet the
//!   package declares;
//! - a file it names is missing, over [`MAX_FILE`], or its bytes are not
//!   its digest;
//! - its `architect.json` is not locked, or the manifest in it does not
//!   read under the rules every manifest keeps (`Manifest::check`).
//!
//! Only the files the lock names (and the lock) are served.
//!
//! [`load`] then builds the [`SiteIsland`] the manifest describes: its id,
//! its `[site]` table's views and stylesheet (`SiteDecl`), a note claim
//! for each `[[embeds]]` drawn by a view that claims note types, and what
//! its `[capabilities]` say it reads; with the manifest's `[[favourites]]`
//! and `[[share_layers]]` beside it ([`WebPackage`]).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use architect_extensions::manifest::Manifest;
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use crate::island::{READS_NOTE, SiteIsland};

/// The lock every web package carries.
pub const LOCK: &str = "architect.lock.json";

/// The file that carries a web package's manifest (its `manifest` member).
pub const METADATA: &str = "architect.json";

/// The largest file a folder may hold.
pub const MAX_FILE: u64 = 64 * 1024 * 1024;

/// A verified folder's files, each with its bytes.
pub type Files = Vec<(String, Cow<'static, [u8]>)>;

/// Why a folder is not used.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// No folder, or no lock in it: nothing was shipped there.
    #[error("no {LOCK} in {0}")]
    Absent(String),
    #[error("{LOCK} is unreadable: {0}")]
    Lock(String),
    #[error("{LOCK} names a path that is not a plain file name in the folder: {0}")]
    Path(String),
    #[error("{0} is missing")]
    Missing(String),
    #[error("{0} is not a regular file of its own (a link, a folder or a special file)")]
    NotAFile(String),
    #[error("{0} is not what {LOCK} says")]
    Altered(String),
    #[error("{LOCK} does not name the stylesheet {0}")]
    Unlocked(String),
    /// [`METADATA`] is not one of the locked files.
    #[error("{LOCK} does not name {METADATA}, which carries the manifest")]
    NoManifest,
    /// [`METADATA`] is not JSON with a `manifest`, or the manifest breaks
    /// a rule (a `[site]` view that is not one of its `[[views]]`).
    #[error("the manifest in {METADATA} is refused: {0}")]
    Manifest(String),
    /// The manifest's id is not one a path may carry (`SiteIsland::id_ok`).
    #[error("{0:?} is not an id a node serves an island under")]
    Id(String),
}

#[derive(serde::Deserialize)]
struct Lock {
    files: BTreeMap<String, String>,
}

/// `sha256-<base64>`, the lock's form.
fn digest(bytes: &[u8]) -> String {
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(bytes))
    )
}

/// One plain file name directly in the folder: never `.`/`..`, a
/// separator or a NUL.
fn plain(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// The bytes of `name` in `dir` if it is a regular file of its own; the
/// file read is the one checked (the open file's identity is compared with
/// the name's), so swapping the name between the two does not serve
/// another file.
fn regular(dir: &Path, name: &str) -> Result<Vec<u8>, PackageError> {
    let path = dir.join(name);
    let missing = || PackageError::Missing(name.to_owned());
    let at = std::fs::symlink_metadata(&path).map_err(|_| missing())?;
    if !at.file_type().is_file() || at.len() > MAX_FILE {
        return Err(PackageError::NotAFile(name.to_owned()));
    }
    let mut file = std::fs::File::open(&path).map_err(|_| missing())?;
    let opened = file.metadata().map_err(|_| missing())?;
    if !opened.file_type().is_file() || !same_file(&at, &opened) {
        return Err(PackageError::NotAFile(name.to_owned()));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE)
        .read_to_end(&mut bytes)
        .map_err(|_| missing())?;
    Ok(bytes)
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len()
}

/// Every file `dir`'s lock names, each checked against it, and the lock
/// itself last. `stylesheet`, when given, must be one of them.
///
/// # Errors
///
/// [`PackageError`]: no folder or no lock ([`PackageError::Absent`]), or
/// a folder that is not exactly what its lock says.
pub fn files(dir: &Path, stylesheet: Option<&str>) -> Result<Files, PackageError> {
    let folder = dir.display().to_string();
    match std::fs::symlink_metadata(dir) {
        Err(_) => return Err(PackageError::Absent(folder)),
        Ok(m) if !m.file_type().is_dir() => return Err(PackageError::NotAFile(folder)),
        Ok(_) => {}
    }
    let lock_bytes = match regular(dir, LOCK) {
        Err(PackageError::Missing(_)) => return Err(PackageError::Absent(folder)),
        other => other?,
    };
    let lock: Lock =
        serde_json::from_slice(&lock_bytes).map_err(|e| PackageError::Lock(e.to_string()))?;
    if lock.files.is_empty() {
        return Err(PackageError::Lock("it names no files".to_owned()));
    }
    if let Some(sheet) = stylesheet
        && !lock.files.contains_key(sheet)
    {
        return Err(PackageError::Unlocked(sheet.to_owned()));
    }
    // Every name first: nothing is read for a lock that names a path.
    if let Some(name) = lock.files.keys().find(|n| !plain(n) || *n == LOCK) {
        return Err(PackageError::Path(name.clone()));
    }
    let mut out: Files = Vec::with_capacity(lock.files.len().saturating_add(1));
    for (name, want) in &lock.files {
        let bytes = regular(dir, name)?;
        if digest(&bytes) != *want {
            return Err(PackageError::Altered(name.clone()));
        }
        out.push((name.clone(), Cow::Owned(bytes)));
    }
    out.push((LOCK.to_owned(), Cow::Owned(lock_bytes)));
    Ok(out)
}

/// What a web package gives a node: its island, and what its manifest
/// declares for the node's shell and share links.
#[derive(Debug, Clone)]
pub struct WebPackage {
    /// Its island, served under `/_ext/` and listed for the web app.
    pub island: SiteIsland,
    /// Its `[[favourites]]`, each with the manifest as its declarer.
    pub favourites: Vec<architect_favourites::Listed>,
    /// Its `[[share_layers]]` rules.
    pub share_layers: Vec<crate::ShareLayersDecl>,
}

/// The web package in `dir`: the folder checked against its lock
/// ([`files`]), then its island built from the manifest in its locked
/// [`METADATA`] and nothing else.
///
/// # Errors
///
/// [`PackageError`]: why the folder is refused. Nothing of a refused
/// folder is used.
pub fn load(dir: &Path) -> Result<WebPackage, PackageError> {
    let files = files(dir, None)?;
    let metadata = files
        .iter()
        .find(|(name, _)| name == METADATA)
        .map(|(_, bytes)| bytes.as_ref())
        .ok_or(PackageError::NoManifest)?;
    let manifest = manifest_of(metadata)?;
    let site = manifest.site.clone().unwrap_or_default();
    if let Some(sheet) = &site.stylesheet
        && !files.iter().any(|(name, _)| name == sheet)
    {
        return Err(PackageError::Unlocked(sheet.clone()));
    }
    let by = manifest.declarer();
    let island = island_of(&manifest, &site, files);
    if !island.id_ok() {
        return Err(PackageError::Id(manifest.id));
    }
    Ok(WebPackage {
        island,
        favourites: manifest
            .favourites
            .into_iter()
            .map(|favourites| architect_favourites::Listed {
                by: by.clone(),
                favourites,
            })
            .collect(),
        share_layers: manifest.share_layers,
    })
}

/// The manifest [`METADATA`]'s bytes carry, read under every manifest's
/// rules.
fn manifest_of(metadata: &[u8]) -> Result<Manifest, PackageError> {
    let json: serde_json::Value =
        serde_json::from_slice(metadata).map_err(|e| PackageError::Manifest(e.to_string()))?;
    let manifest = json
        .get("manifest")
        .ok_or_else(|| PackageError::Manifest("it has no manifest".to_owned()))?;
    Manifest::from_json(manifest).map_err(|e| PackageError::Manifest(e.to_string()))
}

/// The island `manifest` describes, over `files`.
fn island_of(
    manifest: &Manifest,
    site: &architect_extensions::manifest::SiteDecl,
    files: Files,
) -> SiteIsland {
    let mut island =
        SiteIsland::new(manifest.id.clone(), files).reads(manifest.capabilities.reads.clone());
    if let Some(view) = &site.progress {
        island = island.progress(view.clone());
    }
    if let Some(view) = &site.transport {
        island = island.transport(view.clone());
    }
    if let Some(view) = &site.lyrics {
        island = island.lyrics(view.clone());
    }
    if let Some(view) = &site.home {
        island = island.home(view.clone());
    }
    for view in &site.pages {
        island = island.page_view(view.clone());
    }
    if let Some(sheet) = &site.stylesheet {
        island = island.stylesheet(sheet.clone());
    }
    for embed in &manifest.contributes.embeds {
        let (Some(view), false) = (&embed.view, embed.claims.note_type.is_empty()) else {
            continue;
        };
        let types = embed.claims.note_type.clone();
        island = if embed.reads.iter().any(|r| r == READS_NOTE) {
            island.embed_note(view.clone(), types, embed.properties.clone())
        } else {
            island.embed(view.clone(), types)
        };
    }
    island
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

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
id = "lyrics"
title = "Lyrics"

[[views]]
id = "home"
title = "Home"

[[views]]
id = "mixer"
title = "Mixer"

[[views]]
id = "list"
title = "Setlist"

[[views]]
id = "song"
title = "Song"

[site]
progress = "progress"
transport = "transport"
lyrics = "lyrics"
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

[[embeds]]
id = "song"
title = "Song"
view = "song"
claims = { note_type = ["song"] }

[[embeds]]
id = "chart"
title = "Chart"
view = "song"
claims = { fence = ["chart"] }

[[share_layers]]
note_type = "song"
full = ["**/*.wav"]

[[favourites]]
note_type = "setlist"

[[favourites.tabs]]
id = "mixer"
label = "Mixer"
icon = "sliders"
target = { view = "demo.band/mixer" }
"#;

    fn metadata(manifest: &str) -> Vec<u8> {
        let manifest: toml::Value = toml::from_str(manifest).unwrap();
        serde_json::to_vec(&serde_json::json!({
            "module": "band.js", "world": "0.1.0", "manifest": manifest,
        }))
        .unwrap()
    }

    /// A built folder for `manifest` in `dir`, with its lock.
    fn build(dir: &Path, manifest: &str) {
        let files: Vec<(&str, Vec<u8>)> = vec![
            (METADATA, metadata(manifest)),
            ("island.css", b".band{}".to_vec()),
            ("web-band.js", b"export function mount(){}".to_vec()),
            ("web-band_bg.wasm", b"\0asm\x01\0\0\0".to_vec()),
        ];
        let mut locked = serde_json::Map::new();
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
            locked.insert((*name).to_owned(), digest(bytes).into());
        }
        lock(dir, &locked);
        // Not locked: never served.
        std::fs::write(dir.join("BUILT_FROM"), "a commit").unwrap();
    }

    fn lock(dir: &Path, files: &serde_json::Map<String, serde_json::Value>) {
        let text = serde_json::to_vec(&serde_json::json!({"lockfileVersion": 1, "files": files}))
            .unwrap();
        std::fs::write(dir.join(LOCK), text).unwrap();
    }

    fn locked(dir: &Path) -> serde_json::Map<String, serde_json::Value> {
        let lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(LOCK)).unwrap()).unwrap();
        lock["files"].as_object().unwrap().clone()
    }

    #[test]
    fn a_good_folder_gives_the_island_its_manifest_declares() {
        let tmp = tempfile::tempdir().unwrap();
        build(tmp.path(), MANIFEST);
        let package = load(tmp.path()).unwrap();
        let island = &package.island;
        assert_eq!(island.id(), "demo.band");
        assert!(island.file("web-band.js").is_some() && island.file(LOCK).is_some());
        assert!(island.file("BUILT_FROM").is_none(), "only what is locked");
        let json = crate::island::page_json(std::slice::from_ref(island));
        let it = &json["islands"][0];
        assert_eq!(it["progress"], "progress");
        assert_eq!(it["transport"], "transport");
        assert_eq!(it["lyrics"], "lyrics");
        assert_eq!(it["home"], "home");
        assert_eq!(it["views"], serde_json::json!(["mixer"]));
        assert_eq!(it["stylesheet"], "island.css");
        assert_eq!(it["stylesheetIntegrity"], digest(b".band{}"));
        assert_eq!(it["web"]["entry"], "web-band.js");
        assert_eq!(it["reads"], serde_json::json!(["documents", "files"]));
        // A claim that reads the note, one that does not; a fence's claim
        // is no note claim.
        assert_eq!(
            it["embeds"],
            serde_json::json!([
                {"view": "list", "note_type": ["setlist"], "reads": ["documents"], "properties": ["date"]},
                {"view": "song", "note_type": ["song"]},
            ])
        );
        let reads = crate::island::note_reads(std::slice::from_ref(island), "Setlist");
        assert!(reads.text && reads.files);
        assert!(!crate::island::note_reads(std::slice::from_ref(island), "song").text);
        assert_eq!(package.favourites.len(), 1);
        assert_eq!(package.favourites[0].by.extension, "demo.band");
        assert_eq!(package.favourites[0].favourites.note_type, "setlist");
        assert_eq!(package.share_layers.len(), 1);
        assert_eq!(package.share_layers[0].note_type, "song");
    }

    #[test]
    fn a_manifest_with_no_site_table_gives_an_island_with_no_places() {
        let tmp = tempfile::tempdir().unwrap();
        let (head, rest) = MANIFEST.split_once("[site]").unwrap();
        let (_, tail) = rest.split_once("[[embeds]]").unwrap();
        let bare = format!("{head}[[embeds]]{tail}");
        build(tmp.path(), &bare);
        let package = load(tmp.path()).unwrap();
        let json = crate::island::page_json(&[package.island]);
        let it = &json["islands"][0];
        assert!(it["progress"].is_null() && it["home"].is_null() && it["stylesheet"].is_null());
        assert_eq!(it["views"], serde_json::json!([]));
        assert_eq!(it["embeds"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn one_changed_byte_refuses_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        build(tmp.path(), MANIFEST);
        std::fs::write(tmp.path().join("web-band.js"), b"export function mount(){ }").unwrap();
        assert!(matches!(
            load(tmp.path()),
            Err(PackageError::Altered(name)) if name == "web-band.js"
        ));
        // The manifest itself is held to the lock too.
        build(tmp.path(), MANIFEST);
        let mut edited = metadata(MANIFEST);
        edited.push(b'\n');
        std::fs::write(tmp.path().join(METADATA), edited).unwrap();
        assert!(matches!(
            load(tmp.path()),
            Err(PackageError::Altered(name)) if name == METADATA
        ));
    }

    #[test]
    fn a_lock_naming_a_path_outside_the_folder_refuses_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("band");
        std::fs::create_dir(&dir).unwrap();
        build(&dir, MANIFEST);
        std::fs::write(tmp.path().join("secret"), b"outside").unwrap();
        let good = locked(&dir);
        for name in ["../secret", "sub/file", "/etc/hostname", "..", LOCK] {
            let mut files = good.clone();
            files.insert(name.to_owned(), digest(b"outside").into());
            lock(&dir, &files);
            assert!(
                matches!(load(&dir), Err(PackageError::Path(n)) if n == name),
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_a_missing_file_and_an_absent_folder_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("band");
        std::fs::create_dir(&dir).unwrap();
        build(&dir, MANIFEST);
        std::fs::write(tmp.path().join("outside.css"), b".band{}").unwrap();
        std::fs::remove_file(dir.join("island.css")).unwrap();
        assert!(matches!(load(&dir), Err(PackageError::Missing(n)) if n == "island.css"));
        std::os::unix::fs::symlink(tmp.path().join("outside.css"), dir.join("island.css"))
            .unwrap();
        assert!(matches!(load(&dir), Err(PackageError::NotAFile(n)) if n == "island.css"));
        assert!(matches!(
            load(&tmp.path().join("nowhere")),
            Err(PackageError::Absent(_))
        ));
        assert!(matches!(
            load(&tmp.path().join("outside.css")),
            Err(PackageError::NotAFile(_))
        ));
    }

    #[test]
    fn a_site_role_naming_a_missing_view_refuses_it() {
        let tmp = tempfile::tempdir().unwrap();
        build(tmp.path(), &MANIFEST.replace("home = \"home\"", "home = \"nope\""));
        let error = load(tmp.path()).unwrap_err();
        assert!(
            matches!(&error, PackageError::Manifest(why) if why.contains("site.home") && why.contains("nope")),
            "{error}"
        );
    }

    #[test]
    fn the_manifest_and_the_stylesheet_it_declares_must_be_locked() {
        let tmp = tempfile::tempdir().unwrap();
        build(tmp.path(), MANIFEST);
        let mut files = locked(tmp.path());
        files.remove("island.css");
        lock(tmp.path(), &files);
        assert!(matches!(
            load(tmp.path()),
            Err(PackageError::Unlocked(sheet)) if sheet == "island.css"
        ));
        files.remove(METADATA);
        lock(tmp.path(), &files);
        assert!(matches!(load(tmp.path()), Err(PackageError::NoManifest)));
        // An id no path carries.
        build(tmp.path(), &MANIFEST.replace("demo.band", "demo_band"));
        assert!(matches!(load(tmp.path()), Err(PackageError::Id(id)) if id == "demo_band"));
    }
}
