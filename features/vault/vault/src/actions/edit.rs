//! Editing an open vault from an app: a note's text read and saved, notes
//! made and renamed (the links that named them following), its folders,
//! and what changed on disk ([`Changes`]), read again on request or as a
//! watcher hears it ([`OpenVault::watch`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use vault_live::relink::{PageName, relink};
use vault_obsidian::{Vault, mutate};

use super::OpenVault;

/// What changed in the vault since it was last read: vault paths (notes,
/// bases and attachments alike), and folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub created: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub folders_created: Vec<String>,
    pub folders_removed: Vec<String>,
}

impl Changes {
    /// Nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.modified.is_empty()
            && self.removed.is_empty()
            && self.folders_created.is_empty()
            && self.folders_removed.is_empty()
    }
}

/// A note renamed or moved, and the notes whose links were rewritten to
/// follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    pub from: String,
    pub to: String,
    pub relinked: Vec<String>,
}

/// Every file the snapshot holds, by path: when it last changed and its
/// size (an attachment has no text to compare).
fn stamps(v: &Vault) -> BTreeMap<String, (SystemTime, u64)> {
    let mut out = BTreeMap::new();
    for p in &v.pages {
        out.insert(p.rel_path.clone(), (p.mtime, p.raw.len() as u64));
    }
    for b in &v.bases {
        out.insert(b.rel_path.clone(), (b.mtime, b.raw.len() as u64));
    }
    for a in &v.attachments {
        let meta = std::fs::metadata(v.root.join(&a.rel_path)).ok();
        let at = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.insert(a.rel_path.clone(), (at, meta.map_or(0, |m| m.len())));
    }
    out
}

/// Every folder under `root`, as vault paths, hidden ones (`.obsidian`,
/// `.git`, `.trash`) and what is in them left out.
#[must_use]
pub fn folders_in(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut todo = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            out.insert(path.clone());
            todo.push((e.path(), path));
        }
    }
    out
}

/// `to` is not a note's path a person can mean: empty, absolute, out of
/// the vault, or not markdown.
fn check_note_path(to: &str) -> Result<(), String> {
    let bad = to.is_empty()
        || to.starts_with('/')
        || to.split('/').any(|s| s.is_empty() || s == "." || s == "..")
        || !to.ends_with(".md");
    if bad {
        return Err(format!("{to:?} is not a note's path (a vault path ending in .md)"));
    }
    Ok(())
}

impl OpenVault {
    /// A note's text as the vault holds it (what was last read or saved).
    #[must_use]
    pub fn text(&self, path: &str) -> Option<String> {
        self.read().page(path).map(|p| p.raw.clone())
    }

    /// Every folder, the root left out, as last read.
    #[must_use]
    pub fn folders(&self) -> BTreeSet<String> {
        self.folders
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Notes a folder made here (by [`Self::create`] or [`Self::rename`]).
    fn learn_folders(&self, path: &str) {
        let mut known = self
            .folders
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut at = path;
        while let Some((up, _)) = at.rsplit_once('/') {
            known.insert(up.to_owned());
            at = up;
        }
    }

    /// Writes `text` as the note at `path` (marked as the app's own write,
    /// so a watcher does not hear it as an outside edit).
    ///
    /// # Errors
    ///
    /// No such note, or the file cannot be written.
    pub fn save(&self, path: &str, text: &str) -> Result<(), String> {
        if self.text(path).as_deref() == Some(text) {
            return Ok(());
        }
        let guard = Arc::clone(&self.guard);
        self.write()
            .save_page(path, text, &guard)
            .map_err(|e| e.to_string())
    }

    /// Makes a note at `path` holding `text`.
    ///
    /// # Errors
    ///
    /// The path is taken or not a note's, or the file cannot be written.
    pub fn create(&self, path: &str, text: &str) -> Result<(), String> {
        check_note_path(path)?;
        let guard = Arc::clone(&self.guard);
        mutate::create_page(&mut self.write(), path, &[], "", &guard).map_err(|e| e.to_string())?;
        self.learn_folders(path);
        if !text.is_empty() {
            self.save(path, text)?;
        }
        Ok(())
    }

    /// A path for a new note in `folder` (`""` for the root) that no note
    /// has: `Untitled.md`, then `Untitled 1.md`, … (Obsidian's).
    #[must_use]
    pub fn untitled(&self, folder: &str) -> String {
        let root = self.root();
        let at = |name: &str| {
            if folder.is_empty() {
                name.to_owned()
            } else {
                format!("{folder}/{name}")
            }
        };
        let mut n = 0u32;
        loop {
            let name = if n == 0 {
                "Untitled.md".to_owned()
            } else {
                format!("Untitled {n}.md")
            };
            let path = at(&name);
            if !root.join(&path).exists() {
                return path;
            }
            n = n.saturating_add(1);
        }
    }

    /// Renames or moves the note at `from` to `to`, and rewrites every
    /// wikilink that named it (in every other note, and its own) to name
    /// it there (`vault_live::relink`: by path when it was named by path).
    ///
    /// # Errors
    ///
    /// No note at `from`, `to` is taken or not a note's path, or a file
    /// cannot be written.
    pub fn rename(&self, from: &str, to: &str) -> Result<Renamed, String> {
        check_note_path(to)?;
        if from == to {
            return Ok(Renamed {
                from: from.to_owned(),
                to: to.to_owned(),
                relinked: Vec::new(),
            });
        }
        let guard = Arc::clone(&self.guard);
        let mut v = self.write();
        mutate::move_page(&mut v, from, to, &guard).map_err(|e| e.to_string())?;
        self.learn_folders(to);
        let (old, new) = (PageName::of(from), PageName::of(to));
        let rewrites: Vec<(String, String)> = v
            .pages
            .iter()
            .filter_map(|p| relink(&p.raw, &old, &new).map(|t| (p.rel_path.clone(), t)))
            .collect();
        let mut relinked = Vec::new();
        for (path, text) in rewrites {
            v.save_page(&path, &text, &guard)
                .map_err(|e| e.to_string())?;
            relinked.push(path);
        }
        Ok(Renamed {
            from: from.to_owned(),
            to: to.to_owned(),
            relinked,
        })
    }

    /// Reads the folder again and says what changed since the vault was
    /// last read.
    ///
    /// # Errors
    ///
    /// The folder cannot be read; the vault is left as it was.
    pub fn refresh(&self) -> Result<Changes, vault_obsidian::LoadError> {
        let root = self.root();
        let fresh = Vault::open(&root)?;
        let mut v = self.write();
        let (old, new) = (stamps(&v), stamps(&fresh));
        let mut changes = Changes::default();
        for (path, stamp) in &new {
            match old.get(path) {
                None => changes.created.push(path.clone()),
                Some(was) if was != stamp => changes.modified.push(path.clone()),
                Some(_) => {}
            }
        }
        changes.removed = old
            .keys()
            .filter(|p| !new.contains_key(*p))
            .cloned()
            .collect();
        *v = fresh;
        drop(v);
        let after_folders = folders_in(&root);
        let mut known = self
            .folders
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        changes.folders_created = after_folders.difference(&known).cloned().collect();
        changes.folders_removed = known.difference(&after_folders).cloned().collect();
        *known = after_folders;
        Ok(changes)
    }

    /// Watches the folder: each edit made outside the app (another
    /// editor, `git pull`) reads the vault again ([`Self::refresh`]) and
    /// gives `on_change` what changed. The app's own writes are no change
    /// (the snapshot holds them), however soon an outside edit follows.
    /// Stops when the [`Watch`] is dropped.
    ///
    /// # Errors
    ///
    /// The watcher cannot start.
    pub fn watch(&self, on_change: impl Fn(Changes) + Send + 'static) -> Result<Watch, String> {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let vault = self.clone();
        let thread = std::thread::Builder::new()
            .name("vault-watch".to_owned())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                    // Every event, the app's own too: `refresh` compares each
                    // file with the snapshot its save updated, so its own
                    // writes are no change. The write guard's window (1.5s)
                    // would swallow an outside edit made while someone types.
                    let all = Arc::new(vault_obsidian::SelfWriteGuard::new());
                    let handle = match vault_obsidian::watch(vault.root(), all, tx) {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(()));
                    let mut stopped = stopped;
                    loop {
                        tokio::select! {
                            _ = &mut stopped => break,
                            event = rx.recv() => {
                                if event.is_none() {
                                    break;
                                }
                                match vault.refresh() {
                                    Ok(changes) if !changes.is_empty() => on_change(changes),
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!(error = %e, "vault.watch.refresh_failed"),
                                }
                            }
                        }
                    }
                    drop(handle);
                });
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv()
            .map_err(|e| e.to_string())
            .and_then(|r| r)?;
        Ok(Watch {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

/// A running [`OpenVault::watch`]; dropping it stops the watcher.
#[must_use = "dropping it stops the watcher"]
pub struct Watch {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watch")
    }
}
