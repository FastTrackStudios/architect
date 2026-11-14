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
//! `.architect/extensions/*.archext` ([`Carried`]): a source like a
//! registry's, checked the same way (signed by the org key or a publisher
//! a registry here trusts), never run before the person says yes.
//!
//! `.architect/` is the vault's own folder (its decks live there too) and
//! syncs to every device and share guest with the vault's files (#683
//! keeps it synced while it narrows which top-level dot-folders do).

use std::path::{Path, PathBuf};

use architect_extension_archive::Trust;

use crate::index::{Index, Listing};
use crate::install::{Fetched, InstallError, Installs, Source};
use crate::project::{Plan, Requirements};

/// Where a vault names its extensions, from its root.
pub const NEEDS: &str = ".architect/extensions.toml";
/// Where a vault carries extensions' archives, from its root.
pub const CARRIED: &str = ".architect/extensions";

/// The extensions the vault at `root` names: none when it names none.
///
/// # Errors
///
/// The file is there and cannot be read, or is not requirements.
pub fn read(root: &Path) -> Result<Requirements, String> {
    let path = root.join(NEEDS);
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{NEEDS}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Requirements::default()),
        Err(e) => Err(format!("{NEEDS}: {e}")),
    }
}

/// Writes `requirements` as what the vault at `root` names.
///
/// # Errors
///
/// It cannot be written.
pub fn write(root: &Path, requirements: &Requirements) -> Result<(), String> {
    let path = root.join(NEEDS);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = toml::to_string(requirements).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The archives a vault carries, as a source: each `.archext` in
/// `.architect/extensions/` that opens (signed by a key trusted here) is a
/// listing; one that does not is left out and said ([`Self::refused`]).
pub struct Carried {
    dir: PathBuf,
    installs: Installs,
    trust: Trust,
}

impl Carried {
    /// The archives the vault at `root` carries, opened by `installs`
    /// (its trust) and `publishers` (the registries' keys).
    #[must_use]
    pub fn new(root: &Path, installs: Installs, publishers: Trust) -> Self {
        Self {
            dir: root.join(CARRIED),
            installs,
            trust: publishers,
        }
    }

    fn archives(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "archext") && p.is_file())
            .collect();
        out.sort();
        out
    }

    fn opened(&self) -> Vec<(PathBuf, Result<Listing, InstallError>)> {
        self.archives()
            .into_iter()
            .map(|path| {
                let listing = std::fs::read(&path)
                    .map_err(|e| InstallError::Io(format!("{}: {e}", path.display())))
                    .and_then(|bytes| {
                        self.installs
                            .read_archive_trusting(&bytes, &self.name(), &self.trust)
                            .map(|(_, _, l)| l)
                    });
                (path, listing)
            })
            .collect()
    }

    /// The archives it carries that are refused, each with why (unsigned,
    /// signed by a key not trusted here, changed).
    #[must_use]
    pub fn refused(&self) -> Vec<(String, InstallError)> {
        self.opened()
            .into_iter()
            .filter_map(|(path, l)| {
                l.err().map(|e| {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (name, e)
                })
            })
            .collect()
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
        Ok(Index {
            name: self.name(),
            listings: self
                .opened()
                .into_iter()
                .filter_map(|(_, l)| l.ok())
                .collect(),
        })
    }

    fn fetch(&self, listing: &Listing, _into: &Path) -> Result<Fetched, InstallError> {
        for (path, l) in self.opened() {
            if l.is_ok_and(|l| l.id == listing.id && l.version == listing.version) {
                let bytes = std::fs::read(&path)
                    .map_err(|e| InstallError::Io(format!("{}: {e}", path.display())))?;
                return Ok(Fetched::Archive(bytes));
            }
        }
        Err(InstallError::NotFound(format!(
            "{} {} in this vault",
            listing.id, listing.version
        )))
    }

    fn trust(&self) -> Trust {
        self.trust.clone()
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
}
