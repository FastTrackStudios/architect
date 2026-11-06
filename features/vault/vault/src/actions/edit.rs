//! Editing an open vault from an app: a note's text read and saved, notes
//! made and renamed (the links that named them following), its folders,
//! and what changed on disk ([`Changes`]), read again on request or as a
//! watcher hears it ([`OpenVault::watch`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use vault_live::relink::{PageName, relink};
use vault_obsidian::tag_edit::{TagEdit, edit_tags, is_or_under, tag_key, valid_tag};
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
    pub const fn is_empty(&self) -> bool {
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

/// A length as a file's size.
fn size(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Every file the snapshot holds, by path: when it last changed and its
/// size (an attachment has no text to compare).
fn stamps(v: &Vault) -> BTreeMap<String, (SystemTime, u64)> {
    let mut out = BTreeMap::new();
    for p in &v.pages {
        out.insert(p.rel_path.clone(), (p.mtime, size(p.raw.len())));
    }
    for b in &v.bases {
        out.insert(b.rel_path.clone(), (b.mtime, size(b.raw.len())));
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
        || !Path::new(to)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if bad {
        return Err(format!(
            "{to:?} is not a note's path (a vault path ending in .md)"
        ));
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
        drop(known);
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
        drop(v);
        Ok(Renamed {
            from: from.to_owned(),
            to: to.to_owned(),
            relinked,
        })
    }

    /// Makes `folder` (and the folders it is in).
    ///
    /// # Errors
    ///
    /// It is not a path inside the vault, or it cannot be made.
    pub fn create_folder(&self, folder: &str) -> Result<(), String> {
        let bad = folder.is_empty()
            || folder.starts_with('/')
            || folder
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..");
        if bad {
            return Err(format!("{folder:?} is not a folder's path in the vault"));
        }
        std::fs::create_dir_all(self.root().join(folder)).map_err(|e| e.to_string())?;
        self.learn_folders(&format!("{folder}/x"));
        Ok(())
    }

    /// Moves (or renames) any file of the vault: a note through
    /// [`Self::rename`] (the links that named it follow), anything else (an
    /// image, a PDF, a base) on disk, the vault read again after. What
    /// changed, for a file that is not a note.
    ///
    /// # Errors
    ///
    /// No file at `from`, `to` is taken or not inside the vault, or the
    /// move fails.
    pub fn move_file(&self, from: &str, to: &str) -> Result<Changes, String> {
        let is_note = |p: &str| {
            Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        };
        if is_note(from) && is_note(to) {
            self.rename(from, to)?;
            return Ok(Changes::default());
        }
        let bad = to.is_empty()
            || to.starts_with('/')
            || to.split('/').any(|s| s.is_empty() || s == "." || s == "..");
        if bad {
            return Err(format!("{to:?} is not a path inside the vault"));
        }
        let root = self.root();
        let (src, dst) = (root.join(from), root.join(to));
        if !src.is_file() {
            return Err(format!("no file at {from}"));
        }
        if dst.exists() {
            return Err(format!("{to} is there already"));
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&src, &dst).map_err(|e| e.to_string())?;
        self.refresh().map_err(|e| e.to_string())
    }

    /// Sets one frontmatter property of the note at `path` (the rest of
    /// the file left as written).
    ///
    /// # Errors
    ///
    /// No such note, or it cannot be written.
    pub fn set_property(
        &self,
        path: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let guard = Arc::clone(&self.guard);
        mutate::set_property(&mut self.write(), path, key, value, &guard).map_err(|e| e.to_string())
    }

    /// Removes one frontmatter property of the note at `path`.
    ///
    /// # Errors
    ///
    /// No such note, or it cannot be written.
    pub fn remove_property(&self, path: &str, key: &str) -> Result<(), String> {
        let guard = Arc::clone(&self.guard);
        mutate::remove_property(&mut self.write(), path, key, &guard).map_err(|e| e.to_string())
    }

    /// Does `edit` to the tags of the notes at `paths`
    /// (`vault_obsidian::tag_edit`), and says which notes changed: a tag
    /// added or removed, or every tag cleared.
    ///
    /// # Errors
    ///
    /// A tag that cannot be one ([`valid_tag`]), a path that is no note,
    /// or a note that cannot be written; the notes before it stay done.
    pub fn edit_tags(&self, paths: &[String], edit: &TagEdit<'_>) -> Result<Vec<String>, String> {
        check_tag_edit(edit)?;
        let guard = Arc::clone(&self.guard);
        let mut v = self.write();
        let mut changed = Vec::new();
        for path in paths {
            let raw = v
                .page(path)
                .ok_or_else(|| format!("no note at {path}"))?
                .raw
                .clone();
            if let Some(text) = edit_tags(&raw, edit).map_err(|e| format!("{path}: {e}"))? {
                v.save_page(path, &text, &guard)
                    .map_err(|e| format!("{path}: {e}"))?;
                changed.push(path.clone());
            }
        }
        Ok(changed)
    }

    /// Does `edit` to every note that has the tag: a tag renamed (and the
    /// tags below it) or deleted. Says which notes changed.
    ///
    /// # Errors
    ///
    /// A rename to a name that cannot be a tag, to the same name, or into
    /// the tag itself or below it; or a note that cannot be written (the
    /// notes before it stay done).
    pub fn edit_tags_everywhere(&self, edit: &TagEdit<'_>) -> Result<Vec<String>, String> {
        check_tag_edit(edit)?;
        let paths: Vec<String> = self.with(|v| {
            v.pages
                .iter()
                .filter(|p| p.rel_path.to_ascii_lowercase().ends_with(".md"))
                .map(|p| p.rel_path.clone())
                .collect()
        });
        self.edit_tags(&paths, edit)
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
        drop(known);
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
        ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r)?;
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

/// Whether `edit` can be done: the tag it writes can be one, and a rename
/// goes somewhere else, not into the tag itself or below it.
fn check_tag_edit(edit: &TagEdit<'_>) -> Result<(), String> {
    match *edit {
        TagEdit::Add(tag) if !valid_tag(tag) => Err(format!("{tag:?} cannot be a tag")),
        TagEdit::Rename { from, to } => {
            if !valid_tag(to) {
                return Err(format!("{to:?} cannot be a tag"));
            }
            let (from_key, to_key) = (tag_key(from), tag_key(to));
            if from.trim().trim_start_matches('#') == to.trim().trim_start_matches('#') {
                return Err(format!("#{from} is called that already"));
            }
            if to_key != from_key && is_or_under(&to_key, &from_key) {
                return Err(format!("#{from} cannot move inside itself"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
