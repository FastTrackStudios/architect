//! Theme files on disk (feature `fs`).
//!
//! Themes directories and extensions'
//! theme files are read into a [`Registry`] (a broken file keeps its last
//! good theme), a watcher that reloads the store when one changes, and
//! the install of an app's shipped themes that never overwrites a
//! person's edit.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::model::{Problem, Registry, Source, Theme};
use crate::store::{Reloader, ThemeStore};

/// One place themes are read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dir {
    /// Every `*.json` in a folder, each `<id>.json`.
    Folder(PathBuf),
    /// The theme files an extension declares (`[[themes]]` in its
    /// `extension.toml`), relative to its folder.
    Extension {
        id: String,
        root: PathBuf,
        files: Vec<String>,
    },
}

/// Where an app's themes come from, over architect's built-in ones: a
/// later place wins over an earlier one for the same id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeDirs {
    dirs: Vec<Dir>,
}

impl ThemeDirs {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a folder of theme files.
    #[must_use]
    pub fn folder(mut self, path: impl Into<PathBuf>) -> Self {
        self.dirs.push(Dir::Folder(path.into()));
        self
    }

    /// Adds an extension's theme files.
    #[must_use]
    pub fn extension(
        mut self,
        id: impl Into<String>,
        root: impl Into<PathBuf>,
        files: Vec<String>,
    ) -> Self {
        self.dirs.push(Dir::Extension {
            id: id.into(),
            root: root.into(),
            files,
        });
        self
    }

    #[must_use]
    pub fn dirs(&self) -> &[Dir] {
        &self.dirs
    }

    /// The folders to watch.
    #[must_use]
    pub fn watched(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .dirs
            .iter()
            .flat_map(|d| match d {
                Dir::Folder(path) => vec![path.clone()],
                Dir::Extension { root, files, .. } => files
                    .iter()
                    .filter_map(|f| root.join(f).parent().map(Path::to_path_buf))
                    .collect(),
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The built-in themes and every file here. A file that does not
    /// read is a problem, and the theme it was in `previous` (read from
    /// the same file) is kept: the last good one.
    #[must_use]
    pub fn load(&self, previous: &Registry) -> Registry {
        let mut registry = Registry::builtin();
        for dir in &self.dirs {
            for (id, path, source) in files_of(dir) {
                match read(&id, &path, source.clone()) {
                    Ok(theme) => registry.insert(theme),
                    Err(problems) => {
                        registry.report(problems);
                        if let Some(good) = previous.get(&id).filter(|t| t.source == source) {
                            registry.insert(good.clone());
                        }
                    }
                }
            }
        }
        registry
    }

    /// [`Self::load`] as the store's reloader.
    #[must_use]
    pub fn reloader(&self) -> Reloader {
        let dirs = self.clone();
        Arc::new(move |previous: &Registry| dirs.load(previous))
    }

    /// Loads the themes into `store` and makes `THEME_RELOAD` (and
    /// [`watch`]) read these places again.
    pub fn install_into(&self, store: &ThemeStore) -> bool {
        store.set_reloader(self.reloader());
        store.reload()
    }
}

/// Every theme file in `dir`: its id, path and source, by id.
fn files_of(dir: &Dir) -> Vec<(String, PathBuf, Source)> {
    let mut out: Vec<(String, PathBuf, Source)> = match dir {
        Dir::Folder(folder) => std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .filter_map(|p| {
                let id = p.file_stem()?.to_str()?.to_owned();
                let source = Source::File(p.display().to_string());
                Some((id, p, source))
            })
            .collect(),
        Dir::Extension { id, root, files } => files
            .iter()
            .filter_map(|f| {
                let path = root.join(f);
                let theme = path.file_stem()?.to_str()?.to_owned();
                let source = Source::Extension {
                    id: id.clone(),
                    file: f.clone(),
                };
                Some((theme, path, source))
            })
            .collect(),
    };
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Reads one theme file.
pub fn read(id: &str, path: &Path, source: Source) -> Result<Theme, Vec<Problem>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| vec![Problem::new(id, &source, format!("cannot be read: {e}"))])?;
    Theme::parse(id, &text, source)
}

/// A themes folder watched: the store reloads when a file in it changes
/// (once per burst of changes), until this drops.
pub struct ThemeWatch {
    _watcher: notify::RecommendedWatcher,
}

/// How long the watcher waits for a burst of writes to end (an editor
/// saves as several events) before reloading.
const QUIET: Duration = Duration::from_millis(80);

/// Watches `dirs`' folders (the ones that exist) and reloads `store` on
/// any change to a `.json` file in them. The reload runs on the
/// watcher's own thread; the store tells its listeners from there.
pub fn watch(store: &ThemeStore, dirs: &ThemeDirs) -> notify::Result<ThemeWatch> {
    use notify::{RecursiveMode, Watcher};

    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let touches_theme = event.is_ok_and(|e| {
            e.paths
                .iter()
                .any(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        });
        if touches_theme {
            let _ = tx.send(());
        }
    })?;
    for folder in dirs.watched() {
        if folder.is_dir() {
            watcher.watch(&folder, RecursiveMode::NonRecursive)?;
        }
    }
    let store = store.clone();
    std::thread::Builder::new()
        .name("theme-watch".to_owned())
        .spawn(move || {
            // Ends when the watcher (the sender's owner) drops.
            while rx.recv().is_ok() {
                while rx.recv_timeout(QUIET).is_ok() {}
                store.reload();
            }
        })
        .map_err(|e| notify::Error::generic(&e.to_string()))?;
    Ok(ThemeWatch { _watcher: watcher })
}

/// A theme an app ships: written into the person's themes folder, so it
/// can be read and edited there.
#[derive(Debug, Clone, Copy)]
pub struct Shipped {
    pub id: &'static str,
    /// The current text.
    pub text: &'static str,
    /// Every earlier version the app shipped, exactly: a file that is one
    /// of these is upgraded; any other is the person's and is kept.
    pub previous: &'static [&'static str],
}

/// What [`install`] did with one shipped theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// There was none: written.
    Written,
    /// It was an earlier shipped version: replaced.
    Upgraded,
    /// Already the current version.
    Current,
    /// Edited by the person: left alone.
    Kept,
}

/// Writes `shipped` into `dir`: a missing file is written, an earlier
/// shipped version upgraded, an edited one kept.
pub fn install(dir: &Path, shipped: &[Shipped]) -> std::io::Result<Vec<(&'static str, Installed)>> {
    std::fs::create_dir_all(dir)?;
    let mut out = Vec::new();
    for s in shipped {
        let path = dir.join(format!("{}.json", s.id));
        let outcome = match std::fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_atomic(&path, s.text)?;
                Installed::Written
            }
            Err(e) => return Err(e),
            Ok(have) if have == s.text => Installed::Current,
            Ok(have) if s.previous.contains(&have.as_str()) => {
                write_atomic(&path, s.text)?;
                Installed::Upgraded
            }
            Ok(_) => Installed::Kept,
        };
        out.push((s.id, outcome));
    }
    Ok(out)
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
