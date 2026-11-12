//! Packaged core extensions as the standard build ships them: a folder per
//! extension in `core/<id>/` beside the binary, checked against its lock
//! before anything in it is used.
//!
//! `cargo xtask core fetch` puts each folder there from the publisher's
//! release, after checking the archive's sha256 (the list's pin) and the
//! publisher's signature over the release's sums. At run time the folder is
//! checked again, against its own `architect.lock.json` (every file the
//! page loads, with its `sha256-…` digest, base64, as `build-web` writes
//! it), so a folder edited after the fetch is refused whole:
//! - the folder, the lock or a file the lock names is not a regular file
//!   (or, for the folder, a directory) of its own: links, folders and
//!   special files are refused, never followed;
//! - the lock is unreadable, names nothing, names a path that is not a plain
//!   file name in the folder, or does not name the stylesheet it declares;
//! - a file it names is missing, or its bytes are not its digest.
//!
//! Only the files the lock names (and the lock) are served.
//!
//! Today the one use is a share-page island ([`Island`],
//! `Distribution::island`). Bundled `.archext` packages (present on first
//! run, updated through the registry) are #631's next step.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use architect_sites::SiteIsland;
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

/// A packaged core extension this build uses, as the list names it
/// (generated into `list.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packaged {
    /// Its id: its folder is `core/<id>/`.
    pub id: &'static str,
    /// The release it was fetched from.
    pub version: &'static str,
    /// It is a share-page island.
    pub island: Option<Island>,
}

/// A share-page island's views and stylesheet (`SiteIsland`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Island {
    pub progress: Option<&'static str>,
    pub transport: Option<&'static str>,
    pub lyrics: Option<&'static str>,
    pub stylesheet: Option<&'static str>,
}

/// The lock every packaged folder carries.
pub const LOCK: &str = "architect.lock.json";

/// The largest file a folder may hold.
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// Why a folder is not used.
#[derive(Debug, thiserror::Error)]
pub enum PackagedError {
    /// No folder, or no lock in it: not shipped (not a refusal).
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
fn regular(dir: &Path, name: &str) -> Result<Vec<u8>, PackagedError> {
    let path = dir.join(name);
    let missing = || PackagedError::Missing(name.to_owned());
    let at = std::fs::symlink_metadata(&path).map_err(|_| missing())?;
    if !at.file_type().is_file() || at.len() > MAX_FILE {
        return Err(PackagedError::NotAFile(name.to_owned()));
    }
    let mut file = std::fs::File::open(&path).map_err(|_| missing())?;
    let opened = file.metadata().map_err(|_| missing())?;
    if !opened.file_type().is_file() || !same_file(&at, &opened) {
        return Err(PackagedError::NotAFile(name.to_owned()));
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
/// [`PackagedError`]: no folder or no lock ([`PackagedError::Absent`]), or
/// a folder that is not exactly what its lock says.
pub fn files(
    dir: &Path,
    stylesheet: Option<&str>,
) -> Result<Vec<(String, Cow<'static, [u8]>)>, PackagedError> {
    let folder = dir.display().to_string();
    match std::fs::symlink_metadata(dir) {
        Err(_) => return Err(PackagedError::Absent(folder)),
        Ok(m) if !m.file_type().is_dir() => return Err(PackagedError::NotAFile(folder)),
        Ok(_) => {}
    }
    let lock_bytes = match regular(dir, LOCK) {
        Err(PackagedError::Missing(_)) => return Err(PackagedError::Absent(folder)),
        other => other?,
    };
    let lock: Lock =
        serde_json::from_slice(&lock_bytes).map_err(|e| PackagedError::Lock(e.to_string()))?;
    if lock.files.is_empty() {
        return Err(PackagedError::Lock("it names no files".to_owned()));
    }
    if let Some(sheet) = stylesheet
        && !lock.files.contains_key(sheet)
    {
        return Err(PackagedError::Unlocked(sheet.to_owned()));
    }
    // Every name first: nothing is read for a lock that names a path.
    if let Some(name) = lock.files.keys().find(|n| !plain(n) || *n == LOCK) {
        return Err(PackagedError::Path(name.clone()));
    }
    let mut out: Vec<(String, Cow<'static, [u8]>)> =
        Vec::with_capacity(lock.files.len().saturating_add(1));
    for (name, want) in &lock.files {
        let bytes = regular(dir, name)?;
        if digest(&bytes) != *want {
            return Err(PackagedError::Altered(name.clone()));
        }
        out.push((name.clone(), Cow::Owned(bytes)));
    }
    out.push((LOCK.to_owned(), Cow::Owned(lock_bytes)));
    Ok(out)
}

/// `packaged`'s island from `core/<id>/`, if it is one, is there and
/// matches its lock. A folder that does not match is refused with one
/// warning, and the node serves no island for it (a share link's
/// now-playing view keeps Architect's own bar and controls).
#[must_use]
pub fn island(core: &Path, packaged: &Packaged) -> Option<SiteIsland> {
    let views = packaged.island?;
    let dir = core.join(packaged.id);
    match files(&dir, views.stylesheet) {
        Ok(files) => Some(with_views(SiteIsland::new(packaged.id, files), views)),
        Err(PackagedError::Absent(_)) => None,
        Err(error) => {
            tracing::warn!(
                core.id = packaged.id,
                core.version = packaged.version,
                core.outcome = "refused",
                core.reason = %error,
                "a packaged core extension does not match its lock; not serving it"
            );
            None
        }
    }
}

fn with_views(island: SiteIsland, views: Island) -> SiteIsland {
    let island = match views.progress {
        Some(v) => island.progress(v),
        None => island,
    };
    let island = match views.transport {
        Some(v) => island.transport(v),
        None => island,
    };
    let island = match views.lyrics {
        Some(v) => island.lyrics(v),
        None => island,
    };
    match views.stylesheet {
        Some(s) => island.stylesheet(s),
        None => island,
    }
}
