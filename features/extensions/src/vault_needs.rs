//! What a vault needs (`docs/content/architecture/extensions.md`, "What a
//! project needs"): the extensions it names, and the small ones it carries.
//!
//! A vault names the extensions it uses in `.architect/extensions.toml`,
//! the same [`Requirements`] a project document keeps in its `meta`:
//!
//! ```toml
//! [[extensions]]
//! id = "fts.metronome"
//! name = "Metronome"
//! version = "^0.1"     # the versions it works with; "*" for any
//! required = true      # false: offered, not asked
//! ```
//!
//! Small extensions can travel in the vault, as signed archives in
//! `.architect/extensions/*.archext` ([`Carried`]). A carried archive is a
//! source for one plan only, never one of the app's registries (so no
//! update, search or other install ever takes from it), and only for the
//! ids the vault names. It is trusted only when signed by the org key, or
//! by the key the installed copy of its id is signed by: a vault cannot
//! bring a new publisher. It is read cheaply ([`architect_extension_archive::peek`]:
//! the signed index and the manifest), cached by file, size and time, and
//! nothing of it runs before the person says yes.
//!
//! What is read here is read as files of the vault's own folder: no link
//! is followed at any level, the needs file is at most [`MAX_NEEDS`] bytes
//! and an archive at most [`MAX_ARCHIVE`].
//!
//! `.architect/` is the vault's own folder (its decks live there too) and
//! syncs with the vault's files today; which top-level dot-folders sync is
//! #683's to decide, and this needs it to keep `extensions.toml` and
//! `extensions/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use architect_extension_archive::Trust;

use crate::index::{Index, Listing, Location};
use crate::install::{Fetched, InstallError, Installs, Source};
use crate::project::{Plan, Requirements};

/// Where a vault names its extensions, from its root.
pub const NEEDS: &str = ".architect/extensions.toml";
/// Where a vault carries extensions' archives, from its root.
pub const CARRIED: &str = ".architect/extensions";
/// The most a needs file may be.
pub const MAX_NEEDS: u64 = 64 * 1024;
/// The most a carried archive may be.
pub const MAX_ARCHIVE: u64 = 16 * 1024 * 1024;
/// The most a carried archive's stream is read for its index and manifest.
const MAX_PEEK: u64 = 64 * 1024 * 1024;

/// `rel` under `root`, refusing a link at any level (`.architect` itself
/// included): the path, when every part is a real folder and the last a
/// real file (`file`) or folder. `Ok(None)` when it is not there.
fn real(root: &Path, rel: &str, file: bool) -> Result<Option<PathBuf>, String> {
    let mut at = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        let meta = match std::fs::symlink_metadata(&at) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{rel}: {e}")),
        };
        if meta.file_type().is_symlink() {
            return Err(format!("{rel}: {part} is a link; it is not followed"));
        }
        let last = i.saturating_add(1) == parts.len();
        let ok = if last && file {
            meta.is_file()
        } else {
            meta.is_dir()
        };
        if !ok {
            return Err(format!(
                "{rel}: {part} is not a {}",
                if last && file { "file" } else { "folder" }
            ));
        }
    }
    Ok(Some(at))
}

/// The extensions the vault at `root` names: none when it names none.
///
/// # Errors
///
/// The file (or `.architect`) is a link, is larger than [`MAX_NEEDS`],
/// cannot be read, or is not requirements.
pub fn read(root: &Path) -> Result<Requirements, String> {
    let Some(path) = real(root, NEEDS, true)? else {
        return Ok(Requirements::default());
    };
    let size = std::fs::symlink_metadata(&path)
        .map_err(|e| format!("{NEEDS}: {e}"))?
        .len();
    if size > MAX_NEEDS {
        return Err(format!("{NEEDS}: {size} bytes, more than {MAX_NEEDS}"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{NEEDS}: {e}"))?;
    toml::from_str(&text).map_err(|e| format!("{NEEDS}: {e}"))
}

/// Writes `requirements` as what the vault at `root` names.
///
/// # Errors
///
/// It cannot be written, or `.architect` is a link.
pub fn write(root: &Path, requirements: &Requirements) -> Result<(), String> {
    if real(root, ".architect", false)?.is_none() {
        std::fs::create_dir_all(root.join(".architect")).map_err(|e| e.to_string())?;
    }
    if real(root, NEEDS, true).is_err() {
        return Err(format!("{NEEDS} is not a plain file"));
    }
    let text = toml::to_string(requirements).map_err(|e| e.to_string())?;
    let path = root.join(NEEDS);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// One carried archive, as last read: keyed by its path, size and time,
/// and the keys it was checked against.
type CacheKey = (PathBuf, u64, Option<std::time::SystemTime>, String);

static CACHE: LazyLock<Mutex<BTreeMap<CacheKey, Result<Listing, String>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// The archives a vault carries, as a source for one plan.
///
/// Each `.archext` in `.architect/extensions/` whose id the vault names,
/// signed by the org key or by the key its installed copy is signed by.
/// One that is not is left out and said ([`Self::refused`]).
pub struct Carried {
    root: PathBuf,
    installs: Installs,
    /// The ids the vault names: no other is listed.
    ids: BTreeSet<String>,
}

impl Carried {
    /// The archives the vault at `root` carries of `requirements`' ids,
    /// checked against `installs` (its trust, and each installed copy's
    /// signer).
    #[must_use]
    pub fn new(root: &Path, installs: Installs, requirements: &Requirements) -> Self {
        Self {
            root: root.to_path_buf(),
            installs,
            ids: requirements
                .extensions
                .iter()
                .map(|r| r.id.clone())
                .collect(),
        }
    }

    /// What it may be signed by for `id`: the app's keys (the org key),
    /// and the key the installed copy of `id` is signed by.
    fn trust_for(&self, id: &str) -> Trust {
        let mut trust = self.installs.trust().clone();
        if let Some(key) = self.installs.get(id).and_then(|i| i.signer_key) {
            trust = trust.with("an installed copy's signer", key);
        }
        trust
    }

    /// Every key any of its ids may be signed by: what is checked first.
    fn trust_all(&self) -> Trust {
        self.ids
            .iter()
            .fold(self.installs.trust().clone(), |t, id| {
                t.and(&self.trust_for(id))
            })
    }

    /// Each archive file: its name and path, or why it is refused.
    fn files(
        &self,
    ) -> (
        Vec<(String, PathBuf, u64, Option<std::time::SystemTime>)>,
        Vec<(String, String)>,
    ) {
        let mut ok = Vec::new();
        let mut refused = Vec::new();
        let dir = match real(&self.root, CARRIED, false) {
            Ok(Some(dir)) => dir,
            Ok(None) => return (ok, refused),
            Err(why) => return (ok, vec![(CARRIED.to_owned(), why)]),
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return (ok, refused);
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".archext") {
                continue;
            }
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                refused.push((name, "a link or not a file; it is not followed".to_owned()));
                continue;
            }
            if meta.len() > MAX_ARCHIVE {
                refused.push((
                    name,
                    format!("{} bytes, more than {MAX_ARCHIVE}", meta.len()),
                ));
                continue;
            }
            ok.push((name, path, meta.len(), meta.modified().ok()));
        }
        (ok, refused)
    }

    /// One archive read (or as cached): its listing, or why it is refused.
    fn read_one(
        &self,
        path: &Path,
        size: u64,
        time: Option<std::time::SystemTime>,
    ) -> Result<Listing, String> {
        let trust = self.trust_all();
        let keys: Vec<String> = trust.keys().iter().map(|k| k.key.clone()).collect();
        let key = (path.to_path_buf(), size, time, keys.join(","));
        if let Some(hit) = CACHE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return hit.clone();
        }
        let got = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| self.check(&bytes));
        CACHE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, got.clone());
        got
    }

    /// `bytes` read as a carried archive: signed, by a key its id may be
    /// signed by, of an id the vault names.
    fn check(&self, bytes: &[u8]) -> Result<Listing, String> {
        let p = architect_extension_archive::peek(bytes, &self.trust_all(), MAX_PEEK)
            .map_err(|e| e.to_string())?;
        let id = p.index.id.clone();
        if !self.ids.contains(&id) {
            return Err(format!("{id} is not one this vault names"));
        }
        if !self
            .trust_for(&id)
            .keys()
            .iter()
            .any(|k| k.key.trim() == p.signer_key.trim())
        {
            return Err(format!(
                "signed by a key not trusted for {id}: a vault's archive must be signed by the org key or by the key its installed copy is signed by"
            ));
        }
        let text = p
            .manifest
            .ok_or_else(|| "it has no extension.toml".to_owned())?;
        let m = crate::manifest::Manifest::parse(&text).map_err(|e| e.to_string())?;
        if m.id != id || m.version.to_string() != p.index.version {
            return Err(format!(
                "its manifest says {} {}, its index {id} {}",
                m.id, m.version, p.index.version
            ));
        }
        Ok(Listing {
            id,
            name: m.name,
            version: m.version,
            kind: p.index.kind,
            description: m.description,
            category: String::new(),
            location: Location::Archive {
                url: format!("this vault: {}", self.name()),
                sha256: None,
            },
            platforms: p.index.platforms,
            changelog: String::new(),
        })
    }

    /// The archives it carries that are refused, each with why (a link,
    /// too large, unsigned, signed by a key not trusted for its id, an id
    /// the vault does not name).
    #[must_use]
    pub fn refused(&self) -> Vec<(String, String)> {
        let (files, mut refused) = self.files();
        for (name, path, size, time) in files {
            if let Err(why) = self.read_one(&path, size, time) {
                refused.push((name, why));
            }
        }
        refused
    }
}

impl Source for Carried {
    fn name(&self) -> String {
        "this vault".to_owned()
    }

    fn refresh(&self) -> Result<(), InstallError> {
        Ok(())
    }

    fn index(&self) -> Result<Index, InstallError> {
        let (files, _) = self.files();
        Ok(Index {
            name: self.name(),
            listings: files
                .into_iter()
                .filter_map(|(_, path, size, time)| self.read_one(&path, size, time).ok())
                .collect(),
        })
    }

    fn fetch(&self, listing: &Listing, _into: &Path) -> Result<Fetched, InstallError> {
        let (files, _) = self.files();
        for (_, path, size, time) in files {
            let Ok(l) = self.read_one(&path, size, time) else {
                continue;
            };
            if l.id == listing.id && l.version == listing.version {
                let bytes = std::fs::read(&path)
                    .map_err(|e| InstallError::Io(format!("{}: {e}", path.display())))?;
                // Checked again as it is handed over: the file may have
                // changed since it was listed.
                self.check(&bytes).map_err(InstallError::Mismatch)?;
                return Ok(Fetched::Archive(bytes));
            }
        }
        Err(InstallError::NotFound(format!(
            "{} {} in this vault",
            listing.id, listing.version
        )))
    }

    /// Only the key its id's installed copy is signed by, past the app's:
    /// [`Self::fetch`] has checked which id that key may sign.
    fn trust(&self) -> Trust {
        self.ids.iter().fold(Trust::none(), |t, id| {
            match self.installs.get(id).and_then(|i| i.signer_key) {
                Some(k) => t.with(format!("{id}'s installed signer"), k),
                None => t,
            }
        })
    }
}

/// What opening a vault takes: its plan, and the archives it carries that
/// were refused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultNeeds {
    pub plan: Plan,
    /// Each carried archive that was refused: its file name and why.
    pub refused: Vec<(String, String)>,
    /// Its `.architect/extensions.toml` did not read: why.
    pub error: Option<String>,
}

impl VaultNeeds {
    /// Whether there is anything to show: something to install or offer,
    /// something that cannot be had, or a refused archive.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.plan.install.is_empty()
            && self.plan.offer.is_empty()
            && self.plan.missing.is_empty()
            && self.refused.is_empty()
            && self.error.is_none()
    }

    /// Whether planning again could change it: something no registry has
    /// yet, or no version of, may come with a registry's refresh. A
    /// platform, a policy or an install's pin does not change by waiting.
    #[must_use]
    pub fn may_change(&self) -> bool {
        self.plan.missing.iter().any(|(_, m)| {
            matches!(
                m,
                crate::project::Missing::NotFound | crate::project::Missing::NoVersion { .. }
            )
        })
    }
}

/// The carried source of the vault at `root` for `requirements`, as the
/// extra source a plan and its install search.
pub(crate) fn carried(
    root: &Path,
    installs: &Installs,
    requirements: &Requirements,
) -> Arc<dyn Source> {
    Arc::new(Carried::new(root, installs.clone(), requirements))
}

#[cfg(test)]
mod tests {
    use super::{read, real};

    #[test]
    fn a_link_at_any_level_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("extensions.toml"), "").unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, vault.join(".architect")).unwrap();
            let e = read(&vault).unwrap_err();
            assert!(e.contains("is a link"), "{e}");
            std::fs::remove_file(vault.join(".architect")).unwrap();
            std::fs::create_dir_all(vault.join(".architect")).unwrap();
            std::os::unix::fs::symlink(
                outside.join("extensions.toml"),
                vault.join(".architect/extensions.toml"),
            )
            .unwrap();
            let e = read(&vault).unwrap_err();
            assert!(e.contains("is a link"), "{e}");
        }
        assert_eq!(
            real(&tmp.path().join("none"), ".architect", false),
            Ok(None)
        );
    }

    #[test]
    fn a_large_needs_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".architect")).unwrap();
        std::fs::write(tmp.path().join(super::NEEDS), vec![b'#'; 70 * 1024]).unwrap();
        assert!(read(tmp.path()).unwrap_err().contains("more than"));
    }
}
