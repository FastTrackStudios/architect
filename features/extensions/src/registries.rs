//! Registries with their publishers' keys, and development installs.
//!
//! Where an app's extensions come from past what it links in (feature
//! `host`, #493): the registries it searches and installs from, each with
//! its publishers' keys, and the folders it runs as development copies.
//!
//! [`Registries`] keeps them in one JSON file of the app's
//! (`<app dir>/extension-registries.json` in the Architect app), written
//! on every change: the registries a person adds in Settings → Extensions
//! or with `EXT_REGISTRY_ADD` (a git repository or a folder), whether
//! development installs are on, and the folders loaded with `EXT_DEV`.
//! The app's own registries (the store's index) are given in code with
//! [`Registries::with_builtin`]: always there, never written, never
//! removed.
//!
//! A registry's keys ([`Publisher`]) are trusted for that registry's
//! archives only ([`Source::trust`]): a third party signs its `.archext`
//! with its own key, its registry entry carries the public half, and an
//! archive from that registry signed by that key installs while one
//! signed by any other key is refused. A `.archext` installed from a file
//! ([`Extensions::install_archive`](crate::extensions::Extensions::install_archive))
//! is checked against the org key and every registered publisher's.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect_extension_archive::Trust;
use serde::{Deserialize, Serialize};

use crate::install::{FolderRegistry, GitRegistry, Source};

/// A publisher's key, as a registry entry carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher {
    /// Who signs with it (`Studio`).
    pub name: String,
    /// The base64 line of its minisign `.pub`.
    pub key: String,
}

/// One registry: a git repository or a folder with an `index.toml`, and
/// the keys its archives may be signed by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    /// What the person calls it; unique among the app's registries.
    pub name: String,
    /// Its git repository's URL (cloned under the app's folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// Its folder, used where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// With `git`: the folder of the repository holding its `index.toml`
    /// (the repository's root when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// The publishers whose archives it may list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Publisher>,
}

impl Registry {
    /// The registry in git repository `url`.
    pub fn git(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            git: Some(url.into()),
            folder: None,
            dir: None,
            keys: Vec::new(),
        }
    }

    /// The registry in `folder`.
    pub fn folder(name: impl Into<String>, folder: impl AsRef<Path>) -> Self {
        Self {
            name: name.into(),
            git: None,
            folder: Some(folder.as_ref().display().to_string()),
            dir: None,
            keys: Vec::new(),
        }
    }

    /// A git registry whose index is folder `dir` of the repository
    /// (`GitRegistry::in_dir`).
    #[must_use]
    pub fn in_dir(mut self, dir: impl Into<String>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Also trusts archives it lists signed by `key` (the base64 line of
    /// a minisign `.pub`), called `name`.
    #[must_use]
    pub fn trusting(mut self, name: impl Into<String>, key: impl Into<String>) -> Self {
        self.keys.push(Publisher {
            name: name.into(),
            key: key.into().trim().to_owned(),
        });
        self
    }

    /// Its publishers' keys.
    #[must_use]
    pub fn trust(&self) -> Trust {
        self.keys
            .iter()
            .fold(Trust::none(), |t, p| t.with(p.name.clone(), p.key.clone()))
    }

    /// Why it cannot be added: no name, neither or both of `git` and
    /// `folder`, or a key that is not a minisign public key.
    ///
    /// # Errors
    ///
    /// The reason.
    pub fn check(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a registry needs a name".to_owned());
        }
        match (self.git.as_deref(), self.folder.as_deref()) {
            (Some(url), None) if !url.trim().is_empty() => {}
            (None, Some(dir)) if !dir.trim().is_empty() => {}
            (None, None) => return Err(format!("{}: give `git` (a URL) or `folder`", self.name)),
            _ => return Err(format!("{}: give `git` or `folder`, not both", self.name)),
        }
        for p in &self.keys {
            if !is_public_key(&p.key) {
                return Err(format!(
                    "{}: {}'s key is not a minisign public key (the base64 line of its .pub)",
                    self.name, p.name
                ));
            }
        }
        Ok(())
    }

    /// It as a [`Source`], a git registry checked out under `checkouts`.
    #[must_use]
    pub fn source(&self, checkouts: &Path) -> Arc<dyn Source> {
        let keyed = |name: &str, key: &str| (name.to_owned(), key.to_owned());
        let keys: Vec<(String, String)> =
            self.keys.iter().map(|p| keyed(&p.name, &p.key)).collect();
        match (&self.git, &self.folder) {
            (Some(url), _) => Arc::new(Named {
                name: self.name.clone(),
                inner: keys.into_iter().fold(
                    {
                        let git = GitRegistry::new(url.clone(), checkouts.join(slug(&self.name)));
                        match self.dir.as_deref().filter(|d| !d.trim().is_empty()) {
                            Some(dir) => git.in_dir(dir),
                            None => git,
                        }
                    },
                    |r, (n, k)| r.trusting(n, k),
                ),
            }),
            (None, folder) => Arc::new(Named {
                name: self.name.clone(),
                inner: keys.into_iter().fold(
                    FolderRegistry::new(folder.clone().unwrap_or_default()),
                    |r, (n, k)| r.trusting(n, k),
                ),
            }),
        }
    }
}

/// minisign's public key line: base64 of 42 bytes, `Ed` first.
fn is_public_key(line: &str) -> bool {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(line.trim())
        .is_ok_and(|raw| raw.len() == 42 && raw.starts_with(b"Ed"))
}

/// A folder name for `name`'s checkout.
fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_owned();
    if s.is_empty() {
        "registry".to_owned()
    } else {
        s
    }
}

/// A source under the name the person gave it (the index's own name may
/// say something else, or be unreadable before the first refresh).
struct Named<S> {
    name: String,
    inner: S,
}

impl<S: Source> Source for Named<S> {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn refresh(&self) -> Result<(), crate::install::InstallError> {
        self.inner.refresh()
    }

    fn index(&self) -> Result<crate::index::Index, crate::install::InstallError> {
        self.inner.index()
    }

    fn fetch(
        &self,
        listing: &crate::index::Listing,
        into: &Path,
    ) -> Result<crate::install::Fetched, crate::install::InstallError> {
        self.inner.fetch(listing, into)
    }

    fn trust(&self) -> Trust {
        self.inner.trust()
    }
}

/// What the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    registries: Vec<Registry>,
    /// Development installs on (a debug build has them on regardless).
    #[serde(default)]
    dev_installs: bool,
    /// The folders loaded as development copies, loaded again on start.
    #[serde(default)]
    dev: Vec<String>,
}

/// A registry as [`Registries::list`] gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub registry: Registry,
    /// The app's own: not in the file, not removable.
    pub builtin: bool,
}

struct State {
    file: Option<PathBuf>,
    checkouts: PathBuf,
    builtin: Vec<Registry>,
    kept: Kept,
    /// Development installs on whatever the file says.
    dev_forced: bool,
    /// Each registry's source, built once (its name the key).
    built: Vec<(Registry, Arc<dyn Source>)>,
}

/// The app's registries, development switch and development folders.
/// Cheap to clone; shared.
#[derive(Clone)]
pub struct Registries {
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for Registries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.state);
        f.debug_struct("Registries")
            .field("file", &s.file)
            .field("registries", &s.kept.registries)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Registries {
    /// The registries kept in `file` (none when it is missing or does
    /// not read), git ones checked out under `checkouts`.
    pub fn open(file: impl Into<PathBuf>, checkouts: impl Into<PathBuf>) -> Self {
        let file = file.into();
        let kept = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self::with_state(Some(file), checkouts.into(), kept)
    }

    /// Registries kept in memory only (tests, a headless host).
    pub fn memory(checkouts: impl Into<PathBuf>) -> Self {
        Self::with_state(None, checkouts.into(), Kept::default())
    }

    fn with_state(file: Option<PathBuf>, checkouts: PathBuf, kept: Kept) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                file,
                checkouts,
                builtin: Vec::new(),
                kept,
                dev_forced: false,
                built: Vec::new(),
            })),
        }
    }

    /// One of the app's own (the store's index): listed first, never
    /// written to the file, never removed. A kept one of the same name
    /// is shadowed.
    #[must_use]
    pub fn with_builtin(self, registry: Registry) -> Self {
        lock(&self.state).builtin.push(registry);
        self
    }

    /// Development installs on whatever the setting says (a debug build).
    #[must_use]
    pub fn with_dev_forced(self, on: bool) -> Self {
        lock(&self.state).dev_forced = on;
        self
    }

    /// Every registry, the app's own first.
    #[must_use]
    pub fn list(&self) -> Vec<Listed> {
        let (own, kept) = {
            let s = lock(&self.state);
            (s.builtin.clone(), s.kept.registries.clone())
        };
        let kept = kept
            .into_iter()
            .filter(|r| !own.iter().any(|b| b.name == r.name))
            .map(|registry| Listed {
                registry,
                builtin: false,
            })
            .collect::<Vec<_>>();
        own.into_iter()
            .map(|registry| Listed {
                registry,
                builtin: true,
            })
            .chain(kept)
            .collect()
    }

    /// Adds `registry` (or replaces the kept one of its name) and writes
    /// the file.
    ///
    /// # Errors
    ///
    /// It does not check ([`Registry::check`]), its name is one of the
    /// app's own, or the file cannot be written.
    pub fn add(&self, registry: Registry) -> Result<(), String> {
        registry.check()?;
        self.change(|s| {
            if s.builtin.iter().any(|b| b.name == registry.name) {
                return Err(format!(
                    "{} is one of the app's own registries",
                    registry.name
                ));
            }
            s.kept.registries.retain(|r| r.name != registry.name);
            s.kept.registries.push(registry);
            Ok(())
        })
    }

    /// Removes the kept registry `name`; false when there was none.
    ///
    /// # Errors
    ///
    /// It is one of the app's own, or the file cannot be written.
    pub fn remove(&self, name: &str) -> Result<bool, String> {
        self.change(|s| {
            if s.builtin.iter().any(|b| b.name == name) {
                return Err(format!("{name} is one of the app's own registries"));
            }
            let before = s.kept.registries.len();
            s.kept.registries.retain(|r| r.name != name);
            Ok(s.kept.registries.len() != before)
        })
    }

    /// Each registry as a source, in [`Self::list`]'s order.
    #[must_use]
    pub fn sources(&self) -> Vec<Arc<dyn Source>> {
        let wanted: Vec<Registry> = self.list().into_iter().map(|l| l.registry).collect();
        let mut s = lock(&self.state);
        let checkouts = s.checkouts.clone();
        let mut built = Vec::with_capacity(wanted.len());
        for r in wanted {
            let source = s
                .built
                .iter()
                .find(|(b, _)| *b == r)
                .map_or_else(|| r.source(&checkouts), |(_, src)| Arc::clone(src));
            built.push((r, source));
        }
        let sources = built.iter().map(|(_, s)| Arc::clone(s)).collect();
        s.built = built;
        sources
    }

    /// Every registry's publishers' keys.
    #[must_use]
    pub fn publishers(&self) -> Trust {
        self.list()
            .iter()
            .fold(Trust::none(), |t, l| t.and(&l.registry.trust()))
    }

    /// Whether development installs are on: the setting, or forced on.
    #[must_use]
    pub fn dev_installs(&self) -> bool {
        let s = lock(&self.state);
        s.dev_forced || s.kept.dev_installs
    }

    /// Whether the setting (not a debug build) has them on.
    #[must_use]
    pub fn dev_setting(&self) -> bool {
        lock(&self.state).kept.dev_installs
    }

    /// Turns the development installs setting on or off.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_dev_installs(&self, on: bool) -> Result<(), String> {
        self.change(|s| {
            s.kept.dev_installs = on;
            Ok(())
        })
    }

    /// The folders kept as development copies.
    #[must_use]
    pub fn dev_folders(&self) -> Vec<PathBuf> {
        let dev = lock(&self.state).kept.dev.clone();
        dev.into_iter().map(PathBuf::from).collect()
    }

    /// Keeps `folder` as a development copy (or forgets it).
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn keep_dev(&self, folder: &Path, keep: bool) -> Result<(), String> {
        let folder = folder.display().to_string();
        self.change(|s| {
            s.kept.dev.retain(|d| *d != folder);
            if keep {
                s.kept.dev.push(folder);
            }
            Ok(())
        })
    }

    /// Runs `f` on the state, then writes the file (outside the lock)
    /// when it succeeded.
    fn change<R>(&self, f: impl FnOnce(&mut State) -> Result<R, String>) -> Result<R, String> {
        let mut s = lock(&self.state);
        let out = f(&mut s)?;
        let (file, kept) = (s.file.clone(), s.kept.clone());
        drop(s);
        save(file.as_deref(), &kept)?;
        Ok(out)
    }
}

fn save(file: Option<&Path>, kept: &Kept) -> Result<(), String> {
    let Some(file) = file else { return Ok(()) };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(kept).map_err(|e| e.to_string())?;
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| format!("{}: {e}", file.display()))
}
