//! The app's toolbars (feature `host`): every extension's `[[toolbars]]`
//! and a person's toolbar files, the files over the extensions'
//! (`docs/content/architecture/extensions.md`, "Toolbars").
//!
//! A person's files are `*.toml` in a folder the app names
//! ([`Toolbars::set_dir`]: `toolbars/` in its folder), one toolbar each
//! ([`ToolbarDecl::parse`]); [`Toolbars::poll`] reads them again when one
//! changes, so an edit shows at once. A file whose `id` is an extension
//! toolbar's key (`fts.metronome/transport`) is the person's copy of it,
//! used in its place while the file is there ([`Toolbars::reset`] takes it
//! away); another id is a toolbar of the person's own (`user/<id>`). A file
//! that does not read or check is a problem (`APP_STATUS`), the rest load.
//!
//! It keeps which action is armed, too ([`Toolbars::arm`]): a button whose
//! right-click arms its action shows it armed until it is disarmed; what
//! an armed action does next (to what is clicked) is the app's.
//!
//! The app provides toolbars at run time too ([`Toolbars::provide`]): a
//! phone's focused editor's own items are the Typing Toolbar's built-in
//! one for its context (`typing/<context>`, [`Toolbars::provide_editor`]),
//! which a person's file overrides as it would an extension's: reordered,
//! with items hidden or added. [`Toolbars::typing`] is what that bar shows.
//!
//! And the editor's state (#313): whether the toolbars are being edited
//! ([`Toolbars::set_editing`]), whose settings show ([`Toolbars::select`]),
//! and edits by item ([`Toolbars::edit`], `crate::toolbar_edit`): an edit
//! shows at once and is written [`DEBOUNCE`] after the last one
//! ([`Toolbars::flush_if`]; the `TOOLBAR_*` actions write at once). Every
//! write keeps the file it replaces: the last [`BACKUPS`] of each, in
//! `.backups/` beside them ([`Toolbars::backups`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use architect::action::Problem;

use crate::contrib::{By, Contributions};
use crate::toolbar::{TYPING, ToolItem, ToolbarDecl, ToolbarPlace};

/// Who a person's own toolbars are owned by (`user/<id>`).
pub const PERSON: &str = "user";

/// Who an editor's built-in Typing Toolbar is owned by
/// (`typing/<context>`: `typing/markdown`).
pub const EDITOR: &str = "typing";

/// How long after the last edit in the editor it is written.
pub const DEBOUNCE: Duration = Duration::from_millis(400);

/// How many earlier copies of each file are kept (`.backups/<file>.<n>.toml`,
/// 1 the newest).
pub const BACKUPS: usize = 10;

/// The folder the backups are in, inside the toolbars' folder.
pub const BACKUP_DIR: &str = ".backups";

/// Where a toolbar in force comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// An extension's manifest, as it is.
    Extension,
    /// A person's file in place of an extension's.
    Override,
    /// A person's file of their own.
    Person,
    /// The app's, at run time ([`Toolbars::provide`]): an editor's own
    /// items on the Typing Toolbar.
    Provided,
}

/// A toolbar in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolbar {
    /// `<extension>/<id>`, or `user/<id>`.
    pub key: String,
    /// The extension whose it is (whose views its widgets are), or `user`.
    pub owner: String,
    pub decl: ToolbarDecl,
    pub source: Source,
    /// The person's file it is read from.
    pub file: Option<PathBuf>,
}

#[derive(Default)]
struct State {
    dir: Option<PathBuf>,
    /// The files as last read: by key, each with its path.
    files: BTreeMap<String, (ToolbarDecl, PathBuf)>,
    /// The app's at run time, by key: each with its owner.
    provided: BTreeMap<String, (String, ToolbarDecl)>,
    problems: Vec<Problem>,
    /// What was read: each file and when it last changed.
    seen: Vec<(PathBuf, Option<SystemTime>)>,
    armed: Option<String>,
    /// The editor is on.
    editing: bool,
    /// Whose settings show: a toolbar's key, and an item's place (none:
    /// the toolbar's own).
    selected: Option<(String, Option<String>)>,
    /// Edited, not yet written: by key.
    pending: BTreeSet<String>,
    /// Moves with every edit ([`Toolbars::flush_if`]).
    edits: u64,
    /// Image files read, as `data:` URIs, by path and when they changed.
    images: BTreeMap<PathBuf, (Option<SystemTime>, Option<String>)>,
    revision: u64,
    next_listener: u64,
    listeners: Vec<(u64, Arc<dyn Fn(u64) + Send + Sync>)>,
}

/// The toolbars. Cheap to clone: every clone is the same.
#[derive(Clone, Default)]
pub struct Toolbars(Arc<Mutex<State>>);

impl std::fmt::Debug for Toolbars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("Toolbars")
            .field("dir", &s.dir)
            .field("files", &s.files.keys().collect::<Vec<_>>())
            .field("armed", &s.armed)
            .finish()
    }
}

/// Takes its listener off when dropped.
pub struct Listening {
    state: std::sync::Weak<Mutex<State>>,
    id: u64,
}

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(s) = self.state.upgrade() {
            s.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .listeners
                .retain(|(id, _)| *id != self.id);
        }
    }
}

impl std::fmt::Debug for Listening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Listening").field(&self.id).finish()
    }
}

/// A key as a file name: `fts.metronome/transport` → `fts.metronome--transport.toml`.
fn file_name(key: &str) -> String {
    let safe: String = key
        .chars()
        .map(|c| match c {
            '/' => '-',
            c if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') => c,
            _ => '_',
        })
        .collect();
    format!("{safe}.toml")
}

/// Backup `n` (1 the newest) of the file at `path`.
fn backup_path(dir: &Path, path: &Path, n: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "toolbar".into(), |s| s.to_string_lossy().into_owned());
    dir.join(BACKUP_DIR).join(format!("{stem}.{n}.toml"))
}

/// Keeps the file at `path` as its newest backup, the older ones moving
/// down one and the oldest past [`BACKUPS`] going.
fn back_up(dir: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let shown = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    std::fs::create_dir_all(dir.join(BACKUP_DIR)).map_err(|e| shown(dir, e))?;
    let oldest = backup_path(dir, path, BACKUPS);
    if oldest.exists() {
        std::fs::remove_file(&oldest).map_err(|e| shown(&oldest, e))?;
    }
    for n in (1..BACKUPS).rev() {
        let from = backup_path(dir, path, n);
        if from.exists() {
            let to = backup_path(dir, path, n.saturating_add(1));
            std::fs::rename(&from, &to).map_err(|e| shown(&from, e))?;
        }
    }
    let newest = backup_path(dir, path, 1);
    std::fs::copy(path, &newest).map_err(|e| shown(&newest, e))?;
    Ok(())
}

/// Writes `text` at `path`, the file it replaces kept as a backup (none
/// when it says the same).
fn write_kept(dir: &Path, path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|was| was == text) {
        return Ok(());
    }
    back_up(dir, path)?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Each widget of `owner`'s toolbar that names its view alone, named
/// whole (`meter` → `<owner>/meter`), as a person's file names them.
fn qualify_views(t: &mut ToolbarDecl, owner: &str) {
    fn walk(items: &mut [crate::toolbar::ToolItem], owner: &str) {
        for i in items {
            if !i.view.is_empty() && !i.view.contains('/') {
                i.view = format!("{owner}/{}", i.view);
            }
            walk(&mut i.items, owner);
        }
    }
    walk(&mut t.items, owner);
}

/// What a person's file checks against: any action (theirs to run), a
/// widget's view named whole (`<extension>/<view>`); it may be empty.
fn check_file(t: &ToolbarDecl) -> Result<(), String> {
    t.check_draft(
        |_| Ok(()),
        |view| {
            if view
                .split_once('/')
                .is_some_and(|(e, v)| !e.is_empty() && !v.is_empty())
            {
                Ok(())
            } else {
                Err(format!(
                    "{view:?}: a person's widget names <extension>/<view>"
                ))
            }
        },
    )
}

impl Toolbars {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the listeners, after the lock.
    fn changed(mut s: MutexGuard<'_, State>) {
        s.revision = s.revision.wrapping_add(1);
        let r = s.revision;
        let listeners: Vec<_> = s.listeners.iter().map(|(_, l)| Arc::clone(l)).collect();
        drop(s);
        for l in listeners {
            l(r);
        }
    }

    /// Calls `listener` with the revision after every change (a file read
    /// again, an action armed).
    pub fn subscribe(&self, listener: impl Fn(u64) + Send + Sync + 'static) -> Listening {
        let mut s = self.lock();
        let id = s.next_listener;
        s.next_listener = id.wrapping_add(1);
        s.listeners.push((id, Arc::new(listener)));
        drop(s);
        Listening {
            state: Arc::downgrade(&self.0),
            id,
        }
    }

    /// Moves on with every change.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Reads a person's toolbar files from `dir` (made when it is not
    /// there), and again whenever [`Self::poll`] sees one change.
    pub fn set_dir(&self, dir: impl Into<PathBuf>) {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        self.lock().dir = Some(dir);
        self.reload();
    }

    /// The folder a person's files are in.
    #[must_use]
    pub fn dir(&self) -> Option<PathBuf> {
        self.lock().dir.clone()
    }

    /// The files and when each last changed.
    fn stamps(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
        let mut out: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .map(|p| {
                let at = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                (p, at)
            })
            .collect();
        out.sort();
        out
    }

    /// Reads the files again when one was added, changed or removed since
    /// the last read: whether it did.
    #[must_use = "whether the files changed"]
    pub fn poll(&self) -> bool {
        let Some(dir) = self.dir() else {
            return false;
        };
        if Self::stamps(&dir) == self.lock().seen {
            return false;
        }
        self.reload();
        true
    }

    /// Reads every file in the folder.
    pub fn reload(&self) {
        let Some(dir) = self.dir() else {
            return;
        };
        let seen = Self::stamps(&dir);
        let mut files = BTreeMap::new();
        let mut problems = Vec::new();
        for (path, _) in &seen {
            let shown = path.display().to_string();
            let read = std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|text| ToolbarDecl::parse(&text))
                .and_then(|t| check_file(&t).map(|()| t));
            match read {
                Ok(t) => {
                    let key = if t.id.contains('/') {
                        t.id.clone()
                    } else {
                        format!("{PERSON}/{}", t.id)
                    };
                    files.insert(key, (t, path.clone()));
                }
                Err(why) => {
                    problems.push(
                        Problem::new("toolbars", format!("{shown}: {why}")).file(shown.clone()),
                    );
                }
            }
        }
        let mut s = self.lock();
        // What is edited and not yet written stays as it is.
        for key in s.pending.clone() {
            match s.files.get(&key).cloned() {
                Some(mine) => {
                    files.insert(key, mine);
                }
                None => {
                    files.remove(&key);
                }
            }
        }
        s.files = files;
        s.problems = problems;
        s.seen = seen;
        Self::changed(s);
    }

    /// What is wrong with the person's files now: one problem a file.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        self.lock().problems.clone()
    }

    /// Every toolbar in force: each extension's, then the app's provided
    /// ones (a person's copy in the place of either), then the person's
    /// own.
    #[must_use]
    pub fn all(&self, contributions: &Contributions) -> Vec<Toolbar> {
        let (files, provided) = {
            let s = self.lock();
            (s.files.clone(), s.provided.clone())
        };
        let based = contributions
            .toolbars()
            .into_iter()
            .map(|By { owner, item }| (owner, item, Source::Extension))
            .chain(
                provided
                    .into_values()
                    .map(|(owner, item)| (owner, item, Source::Provided)),
            );
        let mut out: Vec<Toolbar> = based
            .map(|(owner, item, source)| {
                let key = format!("{owner}/{}", item.id);
                match files.get(&key) {
                    Some((mine, path)) => Toolbar {
                        decl: ToolbarDecl {
                            id: item.id,
                            ..mine.clone()
                        },
                        key,
                        owner,
                        source: Source::Override,
                        file: Some(path.clone()),
                    },
                    None => Toolbar {
                        key,
                        owner,
                        decl: item,
                        source,
                        file: None,
                    },
                }
            })
            .collect();
        for (key, (decl, path)) in &files {
            if let Some(id) = key.strip_prefix(&format!("{PERSON}/")) {
                out.push(Toolbar {
                    key: key.clone(),
                    owner: PERSON.to_owned(),
                    decl: ToolbarDecl {
                        id: id.to_owned(),
                        ..decl.clone()
                    },
                    source: Source::Person,
                    file: Some(path.clone()),
                });
            }
        }
        out
    }

    /// Provides `decl` as `owner`'s toolbar at run time
    /// (`<owner>/<id>`), in place of what it provided under that key
    /// before: whether that changed it. A person's file of the same key
    /// is used in its place, as for an extension's.
    #[must_use]
    pub fn provide(&self, owner: &str, decl: ToolbarDecl) -> bool {
        let key = format!("{owner}/{}", decl.id);
        let mut s = self.lock();
        if s.provided
            .get(&key)
            .is_some_and(|(o, d)| o == owner && *d == decl)
        {
            return false;
        }
        s.provided.insert(key, (owner.to_owned(), decl));
        Self::changed(s);
        true
    }

    /// Takes away what was provided under `key`: whether there was one.
    #[must_use]
    pub fn withdraw(&self, key: &str) -> bool {
        let mut s = self.lock();
        if s.provided.remove(key).is_none() {
            return false;
        }
        Self::changed(s);
        true
    }

    /// Provides a phone's editor's own items, its editing actions by id in
    /// its order, as the Typing Toolbar's built-in toolbar for its context
    /// (`typing/<context>`, for `editor:<context>`): its key.
    #[must_use]
    pub fn provide_editor(&self, context: &str, actions: &[(String, String)]) -> String {
        let decl = ToolbarDecl {
            id: context.to_owned(),
            title: format!("Typing: {context}"),
            at: TYPING.to_owned(),
            when: format!("editor:{context}"),
            items: actions
                .iter()
                .map(|(action, label)| ToolItem {
                    action: action.clone(),
                    label: label.clone(),
                    ..ToolItem::default()
                })
                .collect(),
            ..ToolbarDecl::default()
        };
        let key = format!("{EDITOR}/{context}");
        let _ = self.provide(EDITOR, decl);
        key
    }

    /// What a phone's Typing Toolbar shows for `contexts` (the focused
    /// editor's `editor:<context>`, the pages and views the focus is in):
    /// the editor's own toolbar (`typing/<editor>`, a person's copy in its
    /// place) first, then every other of place `typing` whose `when` is in
    /// force, in order.
    #[must_use]
    pub fn typing(
        &self,
        contributions: &Contributions,
        editor: Option<&str>,
        contexts: &[String],
    ) -> Vec<Toolbar> {
        let own = editor.map(|e| format!("{EDITOR}/{e}"));
        let mut out: Vec<Toolbar> = self
            .all(contributions)
            .into_iter()
            .filter(|t| t.decl.place() == Some(ToolbarPlace::Typing))
            .filter(|t| {
                if t.owner == EDITOR {
                    // An editor's own: only its, while it is typed in.
                    own.as_deref() == Some(t.key.as_str())
                } else {
                    t.decl.applies_to_any(contexts)
                }
            })
            .collect();
        // Stable: the editor's own first, the rest as they were.
        out.sort_by_key(|t| own.as_deref() != Some(t.key.as_str()));
        out
    }

    /// The toolbar in force under `key`.
    #[must_use]
    pub fn get(&self, contributions: &Contributions, key: &str) -> Option<Toolbar> {
        self.all(contributions).into_iter().find(|t| t.key == key)
    }

    /// The toolbars in the header of a pane of view kind `kind`.
    #[must_use]
    pub fn in_header_of(&self, contributions: &Contributions, kind: &str) -> Vec<Toolbar> {
        self.all(contributions)
            .into_iter()
            .filter(|t| t.decl.in_header_of(kind))
            .collect()
    }

    /// Writes `decl` as the person's toolbar under `key` (an extension's
    /// key: their copy of it; `user/<id>` or a bare id: their own), and
    /// reads the folder again: its file.
    ///
    /// # Errors
    ///
    /// No folder, a toolbar that does not check, a file that cannot be
    /// written.
    pub fn save(&self, key: &str, decl: &ToolbarDecl) -> Result<PathBuf, String> {
        let dir = self
            .dir()
            .ok_or("no folder for toolbars (Toolbars::set_dir)")?;
        let key = if key.contains('/') {
            key.to_owned()
        } else {
            format!("{PERSON}/{key}")
        };
        let mut decl = decl.clone();
        decl.id = key
            .strip_prefix(&format!("{PERSON}/"))
            .map_or_else(|| key.clone(), str::to_owned);
        check_file(&decl)?;
        let path = self
            .lock()
            .files
            .get(&key)
            .map_or_else(|| dir.join(file_name(&key)), |(_, p)| p.clone());
        write_kept(&dir, &path, &decl.to_toml()?)?;
        self.lock().pending.remove(&key);
        self.reload();
        Ok(path)
    }

    /// Takes away the person's file under `key` (their copy: the
    /// extension's comes back; their own: it goes), kept as a backup:
    /// whether there was one.
    ///
    /// # Errors
    ///
    /// The file cannot be removed.
    pub fn reset(&self, key: &str) -> Result<bool, String> {
        let (path, dir) = {
            let mut s = self.lock();
            s.pending.remove(key);
            (s.files.get(key).map(|(_, p)| p.clone()), s.dir.clone())
        };
        let Some(path) = path else {
            return Ok(false);
        };
        // Only edited, never written: the edit goes.
        if path.exists() {
            if let Some(dir) = dir {
                back_up(&dir, &path)?;
            }
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        self.reload();
        Ok(true)
    }

    /// Edits the toolbar in force under `key` with `edit`: the person's
    /// copy of an extension's, or their own. Shown at once, checked first
    /// (a person's may be empty); written by [`Self::flush`] or
    /// [`Self::flush_if`]. What `edit` answered, and the edit's number.
    ///
    /// # Errors
    ///
    /// No folder; no toolbar under `key`; what `edit` refuses; a toolbar
    /// that does not check after it.
    // The lock is handed to `changed`, which lets it go before it tells.
    #[allow(clippy::significant_drop_tightening)]
    pub fn edit<T>(
        &self,
        contributions: &Contributions,
        key: &str,
        edit: impl FnOnce(&mut ToolbarDecl) -> Result<T, String>,
    ) -> Result<(T, u64), String> {
        let dir = self
            .dir()
            .ok_or("no folder for toolbars (Toolbars::set_dir)")?;
        let now = self
            .get(contributions, key)
            .ok_or_else(|| format!("no toolbar {key:?} (TOOLBAR_LIST lists them)"))?;
        let mut decl = now.decl.clone();
        // A person's file names its widgets' views whole.
        qualify_views(&mut decl, &now.owner);
        let answer = edit(&mut decl)?;
        decl.id = key
            .strip_prefix(&format!("{PERSON}/"))
            .map_or_else(|| key.to_owned(), str::to_owned);
        check_file(&decl)?;
        let path = now.file.unwrap_or_else(|| dir.join(file_name(key)));
        let mut s = self.lock();
        s.files.insert(key.to_owned(), (decl, path));
        s.pending.insert(key.to_owned());
        s.edits = s.edits.wrapping_add(1);
        let n = s.edits;
        Self::changed(s);
        Ok((answer, n))
    }

    /// The last edit's number ([`Self::edit`], [`Self::flush_if`]).
    #[must_use]
    pub fn last_edit(&self) -> u64 {
        self.lock().edits
    }

    /// Whether an edit is not yet written.
    #[must_use]
    pub fn pending(&self) -> bool {
        !self.lock().pending.is_empty()
    }

    /// Writes every edit not yet written (each file it replaces kept as
    /// a backup): the files.
    ///
    /// # Errors
    ///
    /// No folder; a file that cannot be written (the rest are).
    pub fn flush(&self) -> Result<Vec<PathBuf>, String> {
        let (dir, todo) = {
            let mut s = self.lock();
            let todo: Vec<(ToolbarDecl, PathBuf)> = std::mem::take(&mut s.pending)
                .into_iter()
                .filter_map(|k| s.files.get(&k).cloned())
                .collect();
            (s.dir.clone(), todo)
        };
        if todo.is_empty() {
            return Ok(Vec::new());
        }
        let dir = dir.ok_or("no folder for toolbars (Toolbars::set_dir)")?;
        let mut written = Vec::new();
        let mut failed = None;
        for (decl, path) in todo {
            match decl
                .to_toml()
                .and_then(|text| write_kept(&dir, &path, &text))
            {
                Ok(()) => written.push(path),
                Err(why) => failed = Some(why),
            }
        }
        self.reload();
        failed.map_or(Ok(written), Err)
    }

    /// [`Self::flush`], when edit `n` ([`Self::edit`]) is still the last:
    /// what a pause after an edit writes (a later edit waits its own).
    ///
    /// # Errors
    ///
    /// As [`Self::flush`].
    pub fn flush_if(&self, n: u64) -> Result<Vec<PathBuf>, String> {
        if self.lock().edits != n {
            return Ok(Vec::new());
        }
        self.flush()
    }

    /// A new toolbar of the person's own, empty: `user/<id>` (`id` from
    /// `title` when none, made unique), written at once.
    ///
    /// # Errors
    ///
    /// No folder; a place there is not; the file cannot be written.
    pub fn create(
        &self,
        contributions: &Contributions,
        id: Option<&str>,
        title: &str,
        at: &str,
    ) -> Result<String, String> {
        let title = if title.trim().is_empty() {
            "Toolbar"
        } else {
            title.trim()
        };
        let id = self.free_id(
            contributions,
            id.map_or_else(|| crate::toolbar_edit::slug(title), str::to_owned),
        );
        let key = format!("{PERSON}/{id}");
        let decl = ToolbarDecl {
            id,
            title: title.to_owned(),
            at: at.to_owned(),
            ..ToolbarDecl::default()
        };
        self.save(&key, &decl)?;
        Ok(key)
    }

    /// `base`, or `base-2`, `base-3`… : one no toolbar of the person's has.
    fn free_id(&self, contributions: &Contributions, base: String) -> String {
        let taken: BTreeSet<String> = self
            .all(contributions)
            .into_iter()
            .filter_map(|t| t.key.strip_prefix(&format!("{PERSON}/")).map(str::to_owned))
            .collect();
        if !taken.contains(&base) {
            return base;
        }
        (2_u32..10_000)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !taken.contains(id))
            .unwrap_or(base)
    }

    /// A copy of the toolbar under `key` as a toolbar of the person's own
    /// (`title` or "<its title> copy", hidden until it is shown): its key.
    ///
    /// # Errors
    ///
    /// No toolbar under `key`; as [`Self::save`].
    pub fn duplicate(
        &self,
        contributions: &Contributions,
        key: &str,
        id: Option<&str>,
        title: Option<&str>,
    ) -> Result<String, String> {
        let from = self
            .get(contributions, key)
            .ok_or_else(|| format!("no toolbar {key:?} (TOOLBAR_LIST lists them)"))?;
        let title = title.map_or_else(|| format!("{} copy", from.decl.title), str::to_owned);
        let id = self.free_id(
            contributions,
            id.map_or_else(|| crate::toolbar_edit::slug(&title), str::to_owned),
        );
        let mut decl = from.decl;
        // Its widgets name their views whole, as a person's file does.
        qualify_views(&mut decl, &from.owner);
        decl.title = title;
        "hidden".clone_into(&mut decl.at);
        let key = format!("{PERSON}/{id}");
        self.save(&key, &decl)?;
        Ok(key)
    }

    /// Shows the toolbar under `key` at `at` in place of whichever showed
    /// there (each now `hidden`): switches which toolbar a place shows.
    ///
    /// # Errors
    ///
    /// No toolbar under `key`; a place there is not; as [`Self::edit`].
    pub fn show(
        &self,
        contributions: &Contributions,
        key: &str,
        at: &str,
    ) -> Result<Vec<String>, String> {
        let probe = ToolbarDecl {
            at: at.to_owned(),
            ..ToolbarDecl::default()
        };
        if probe.place().is_none() {
            return Err(format!("{at:?} is not a place"));
        }
        let mut hidden = Vec::new();
        // The Typing Toolbar shows one a context: another of the same
        // `when` gives way, not every other.
        let when = (probe.place() == Some(ToolbarPlace::Typing))
            .then(|| self.get(contributions, key).map(|t| t.decl.when))
            .flatten();
        if probe.place() != Some(ToolbarPlace::Hidden) {
            for other in self.all(contributions) {
                let same_context = when.as_ref().is_none_or(|w| *w == other.decl.when);
                if other.key != key && other.decl.at == at && same_context {
                    self.edit(contributions, &other.key, |t| {
                        "hidden".clone_into(&mut t.at);
                        Ok(())
                    })?;
                    hidden.push(other.key);
                }
            }
        }
        self.edit(contributions, key, |t| {
            at.clone_into(&mut t.at);
            Ok(())
        })?;
        Ok(hidden)
    }

    /// Moves item `from` of toolbar `key` to place `to` of toolbar `into`
    /// (another open toolbar): its new place.
    ///
    /// # Errors
    ///
    /// As [`crate::toolbar_edit::move_item`] and [`Self::edit`].
    pub fn move_between(
        &self,
        contributions: &Contributions,
        key: &str,
        from: crate::toolbar_edit::At,
        into: &str,
        to: crate::toolbar_edit::At,
    ) -> Result<crate::toolbar_edit::At, String> {
        use crate::toolbar_edit as edit;
        if key == into {
            return self
                .edit(contributions, key, |t| edit::move_item(t, from, to))
                .map(|(at, _)| at);
        }
        let from_bar = self
            .get(contributions, key)
            .ok_or_else(|| format!("no toolbar {key:?}"))?;
        let mut item = edit::get(&from_bar.decl, from)
            .cloned()
            .ok_or_else(|| format!("no item at {from}"))?;
        // A widget of its own extension names it whole where it goes.
        let mut one = ToolbarDecl {
            items: vec![item],
            ..ToolbarDecl::default()
        };
        qualify_views(&mut one, &from_bar.owner);
        item = one.items.remove(0);
        // Into the other first: a refusal there leaves both as they were.
        let (at, _) = self.edit(contributions, into, |t| edit::insert(t, Some(to), item))?;
        self.edit(contributions, key, |t| edit::remove(t, from).map(|_| ()))?;
        Ok(at)
    }

    /// The backups of the person's file for `key`, the newest first.
    #[must_use]
    pub fn backups(&self, key: &str) -> Vec<PathBuf> {
        let (dir, path) = {
            let s = self.lock();
            (s.dir.clone(), s.files.get(key).map(|(_, p)| p.clone()))
        };
        let Some(dir) = dir else {
            return Vec::new();
        };
        let path = path.unwrap_or_else(|| dir.join(file_name(key)));
        (1..=BACKUPS)
            .map(|n| backup_path(&dir, &path, n))
            .take_while(|p| p.exists())
            .collect()
    }

    /// Turns the editor on or off (off: whose settings showed, too).
    pub fn set_editing(&self, on: bool) {
        let mut s = self.lock();
        if s.editing == on {
            return;
        }
        s.editing = on;
        if !on {
            s.selected = None;
        }
        Self::changed(s);
    }

    /// Whether the editor is on.
    #[must_use]
    pub fn editing(&self) -> bool {
        self.lock().editing
    }

    /// Shows the settings of toolbar `key`'s item at `at` (none: the
    /// toolbar's own), or of nothing.
    pub fn select(&self, selected: Option<(String, Option<String>)>) {
        let mut s = self.lock();
        if s.selected == selected {
            return;
        }
        s.selected = selected;
        Self::changed(s);
    }

    /// Whose settings show.
    #[must_use]
    pub fn selected(&self) -> Option<(String, Option<String>)> {
        self.lock().selected.clone()
    }

    /// Image file `image` as a `data:` URI (read once, again when it
    /// changes): `image` absolute, or in `beside` (the person's folder
    /// when none). None when it is not there or not an image.
    #[must_use]
    pub fn image(&self, image: &str, beside: Option<&Path>) -> Option<String> {
        use base64::Engine as _;
        let mime = crate::toolbar::image_type(image)?;
        let path = Path::new(image);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            beside
                .map(Path::to_path_buf)
                .or_else(|| self.dir())?
                .join(path)
        };
        let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if let Some((at, uri)) = self.lock().images.get(&path)
            && *at == stamp
        {
            return uri.clone();
        }
        let uri = std::fs::read(&path).ok().map(|bytes| {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )
        });
        self.lock().images.insert(path, (stamp, uri.clone()));
        uri
    }

    /// Arms `action` (one at a time): its buttons show it armed.
    pub fn arm(&self, action: &str) {
        let mut s = self.lock();
        if s.armed.as_deref() == Some(action) {
            return;
        }
        s.armed = Some(action.to_owned());
        Self::changed(s);
    }

    /// Disarms whatever is armed: what was.
    // What was armed is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn disarm(&self) -> Option<String> {
        let mut s = self.lock();
        let was = s.armed.take();
        if was.is_some() {
            Self::changed(s);
        }
        was
    }

    /// The action armed now.
    #[must_use]
    pub fn armed(&self) -> Option<String> {
        self.lock().armed.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::contrib::Contribution;
    use crate::toolbar::ToolItem;

    fn bar(id: &str, action: &str) -> ToolbarDecl {
        ToolbarDecl {
            id: id.into(),
            title: "T".into(),
            at: "dock:right".into(),
            items: vec![ToolItem {
                action: action.into(),
                ..ToolItem::default()
            }],
            ..ToolbarDecl::default()
        }
    }

    #[test]
    fn a_persons_file_overrides_an_extensions_and_adds_their_own() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Contributions::new();
        reg.put(
            "lab.x",
            Contribution {
                toolbars: vec![bar("t", "LAB_X_A")],
                ..Contribution::default()
            },
        );
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        assert_eq!(bars.all(&reg)[0].source, Source::Extension);

        // Their copy, saved: in its place, the same key.
        let mut mine = bar("t", "LAB_X_B");
        mine.title = "Mine".into();
        let path = bars.save("lab.x/t", &mine).unwrap();
        assert!(path.ends_with("lab.x-t.toml"), "{}", path.display());
        let all = bars.all(&reg);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, Source::Override);
        assert_eq!(all[0].decl.title, "Mine");
        assert_eq!(all[0].decl.id, "t");
        // One of their own, written by hand: read on the next poll.
        std::fs::write(
            tmp.path().join("own.toml"),
            bar("own", "ANYTHING").to_toml().unwrap(),
        )
        .unwrap();
        assert!(bars.poll());
        assert!(!bars.poll(), "nothing changed since");
        let keys: Vec<String> = bars.all(&reg).into_iter().map(|t| t.key).collect();
        assert_eq!(keys, ["lab.x/t", "user/own"]);
        // A broken file is a problem; the rest stay.
        std::fs::write(tmp.path().join("broken.toml"), "id = ").unwrap();
        assert!(bars.poll());
        assert_eq!(bars.problems().len(), 1);
        assert!(bars.problems()[0].message.contains("broken.toml"));
        assert_eq!(bars.all(&reg).len(), 2);
        // Reset: the extension's comes back.
        assert!(bars.reset("lab.x/t").unwrap());
        assert_eq!(bars.all(&reg)[0].source, Source::Extension);
        assert!(!bars.reset("lab.x/t").unwrap());
    }

    #[test]
    fn an_edit_shows_at_once_is_written_after_the_last_and_backups_rotate() {
        use crate::toolbar_edit::{self as edit, At};
        let tmp = tempfile::tempdir().unwrap();
        let reg = Contributions::new();
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        let key = bars.create(&reg, None, "My tools", "dock:right").unwrap();
        assert_eq!(key, "user/my-tools");
        assert_eq!(
            bars.create(&reg, None, "My tools", "float").unwrap(),
            "user/my-tools-2"
        );
        let file = tmp.path().join("user-my-tools.toml");
        assert!(file.exists());
        // Two edits: shown at once, not written until the last one's pause.
        let (_, first) = bars
            .edit(&reg, &key, |t| {
                edit::insert(
                    t,
                    None,
                    ToolItem {
                        action: "A".into(),
                        ..ToolItem::default()
                    },
                )
            })
            .unwrap();
        let (_, second) = bars
            .edit(&reg, &key, |t| {
                edit::insert(
                    t,
                    Some(At::top(0)),
                    ToolItem {
                        action: "B".into(),
                        ..ToolItem::default()
                    },
                )
            })
            .unwrap();
        assert_eq!(bars.get(&reg, &key).unwrap().decl.items.len(), 2);
        assert!(bars.pending());
        // A poll meanwhile keeps them.
        let _ = bars.poll();
        assert_eq!(bars.get(&reg, &key).unwrap().decl.items.len(), 2);
        assert!(
            bars.flush_if(first).unwrap().is_empty(),
            "a later edit waits its own"
        );
        assert!(!std::fs::read_to_string(&file).unwrap().contains("\"B\""));
        assert_eq!(bars.flush_if(second).unwrap(), vec![file.clone()]);
        assert!(!bars.pending());
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"B\""));
        // A new folder (a restart) reads what was written.
        let again = Toolbars::new();
        again.set_dir(tmp.path());
        let read = again.get(&reg, &key).unwrap().decl;
        assert_eq!(
            read.items
                .iter()
                .map(|i| i.action.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"]
        );
        // The write kept what it replaced: the empty one.
        let kept = bars.backups(&key);
        assert_eq!(kept.len(), 1);
        assert!(
            !std::fs::read_to_string(&kept[0])
                .unwrap()
                .contains("items = [{")
        );
        // Each write keeps one more, the oldest past BACKUPS going.
        for n in 0..(BACKUPS + 3) {
            bars.edit(&reg, &key, |t| {
                t.title = format!("Take {n}");
                Ok(())
            })
            .unwrap();
            bars.flush().unwrap();
        }
        let kept = bars.backups(&key);
        assert_eq!(kept.len(), BACKUPS);
        let newest = std::fs::read_to_string(&kept[0]).unwrap();
        assert!(
            newest.contains(&format!("Take {}", BACKUPS + 1)),
            "{newest}"
        );
        let oldest = std::fs::read_to_string(&kept[BACKUPS - 1]).unwrap();
        assert!(oldest.contains("Take 2"), "{oldest}");
        // Deleting keeps it too.
        assert!(bars.reset(&key).unwrap());
        assert!(bars.get(&reg, &key).is_none());
        assert!(
            std::fs::read_to_string(&bars.backups(&key)[0])
                .unwrap()
                .contains(&format!("Take {}", BACKUPS + 2))
        );
    }

    #[test]
    fn a_place_shows_one_toolbar_and_a_copy_is_the_persons_own() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Contributions::new();
        let mut ext = bar("t", "LAB_X_A");
        ext.items.push(ToolItem {
            kind: crate::toolbar::ToolKind::Widget,
            view: "meter".into(),
            ..ToolItem::default()
        });
        reg.put(
            "lab.x",
            Contribution {
                toolbars: vec![ext],
                ..Contribution::default()
            },
        );
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        let copy = bars.duplicate(&reg, "lab.x/t", None, None).unwrap();
        assert_eq!(copy, "user/t-copy");
        let c = bars.get(&reg, &copy).unwrap();
        assert_eq!(
            (c.decl.at.as_str(), c.decl.title.as_str()),
            ("hidden", "T copy")
        );
        assert_eq!(
            c.decl.items[1].view, "lab.x/meter",
            "its widget named whole"
        );
        // Shown in the dock: the extension's goes hidden (the person's copy of it).
        assert_eq!(bars.show(&reg, &copy, "dock:right").unwrap(), ["lab.x/t"]);
        bars.flush().unwrap();
        let all = bars.all(&reg);
        let at = |k: &str| all.iter().find(|t| t.key == k).unwrap().decl.at.clone();
        assert_eq!(
            (at("lab.x/t"), at(&copy)),
            ("hidden".to_owned(), "dock:right".to_owned())
        );
        assert!(bars.show(&reg, &copy, "dock:top").is_err());
        // Its item, between toolbars.
        bars.move_between(
            &reg,
            &copy,
            crate::toolbar_edit::At::top(1),
            "lab.x/t",
            crate::toolbar_edit::At::top(0),
        )
        .unwrap();
        assert_eq!(
            bars.get(&reg, "lab.x/t").unwrap().decl.items[0].view,
            "lab.x/meter"
        );
        assert_eq!(bars.get(&reg, &copy).unwrap().decl.items.len(), 1);
    }

    #[test]
    fn an_image_is_a_data_uri_read_again_when_it_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        std::fs::create_dir_all(tmp.path().join("icons")).unwrap();
        std::fs::write(tmp.path().join("icons/a.svg"), "<svg/>").unwrap();
        let uri = bars.image("icons/a.svg", None).unwrap();
        assert_eq!(uri, "data:image/svg+xml;base64,PHN2Zy8+");
        assert_eq!(bars.image("icons/none.svg", None), None);
        assert_eq!(bars.image("icons/a.txt", None), None);
    }

    #[test]
    fn the_typing_toolbar_is_the_editor_s_own_then_those_for_the_focus() {
        use crate::toolbar_edit::{self as edit, At};
        let tmp = tempfile::tempdir().unwrap();
        let reg = Contributions::new();
        let typing = |id: &str, when: &str, action: &str| ToolbarDecl {
            at: TYPING.into(),
            when: when.into(),
            ..bar(id, action)
        };
        reg.put(
            "lab.board",
            Contribution {
                toolbars: vec![
                    typing("chords", "view:lab.board/chart", "LAB_BOARD_CHORD_C"),
                    bar("dock", "LAB_BOARD_PIN"),
                ],
                ..Contribution::default()
            },
        );
        let bars = Toolbars::new();
        bars.set_dir(tmp.path());
        let actions: Vec<(String, String)> = ["TAG", "BOLD", "CLEAR"]
            .into_iter()
            .map(|a| (a.to_owned(), a.to_lowercase()))
            .collect();
        let key = bars.provide_editor("capture", &actions);
        assert_eq!(key, "typing/capture");
        let r = bars.revision();
        let _ = bars.provide_editor("capture", &actions);
        assert_eq!(bars.revision(), r, "the same again changes nothing");
        let keys = |editor: Option<&str>, contexts: &[&str]| -> Vec<String> {
            let contexts: Vec<String> = contexts.iter().map(|c| (*c).to_owned()).collect();
            bars.typing(&reg, editor, &contexts)
                .into_iter()
                .map(|t| t.key)
                .collect()
        };
        // The capture field: its own items alone.
        assert_eq!(
            keys(Some("capture"), &["editor:capture"]),
            ["typing/capture"]
        );
        // The chart's chord line: the Board's, not the capture's.
        assert_eq!(keys(None, &["view:lab.board/chart"]), ["lab.board/chords"]);
        assert!(keys(None, &["view:note"]).is_empty());
        let own = bars.get(&reg, &key).unwrap();
        assert_eq!(own.source, Source::Provided);
        assert_eq!(own.decl.when, "editor:capture");

        // A person's edit: CLEAR first, TAG hidden; written, it stays the
        // bar's in place of the editor's own.
        bars.edit(&reg, &key, |t| edit::move_item(t, At::top(2), At::top(0)))
            .unwrap();
        bars.edit(&reg, &key, |t| edit::remove(t, At::top(1)))
            .unwrap();
        bars.flush().unwrap();
        let again = Toolbars::new();
        again.set_dir(tmp.path());
        let _ = again.provide_editor("capture", &actions);
        let mine = again.get(&reg, &key).unwrap();
        assert_eq!(mine.source, Source::Override);
        let order: Vec<&str> = mine.decl.items.iter().map(|i| i.action.as_str()).collect();
        assert_eq!(order, ["CLEAR", "BOLD"]);
        // Reset: the editor's own come back.
        assert!(again.reset(&key).unwrap());
        assert_eq!(again.get(&reg, &key).unwrap().decl.items.len(), 3);

        // Showing another typing toolbar for the chart hides only the
        // one for that context.
        let other = bars
            .create(&reg, Some("chords-2"), "Chords 2", TYPING)
            .unwrap();
        bars.edit(&reg, &other, |t| {
            "view:lab.board/chart".clone_into(&mut t.when);
            Ok(())
        })
        .unwrap();
        let hidden = bars.show(&reg, &other, TYPING).unwrap();
        assert_eq!(hidden, ["lab.board/chords"]);
        assert_eq!(bars.get(&reg, &key).unwrap().decl.at, TYPING, "kept");
        assert!(bars.withdraw(&key));
        assert!(!bars.withdraw(&key));
    }

    #[test]
    fn one_action_is_armed_at_a_time() {
        let bars = Toolbars::new();
        let heard = Arc::new(Mutex::new(0_u32));
        let h = Arc::clone(&heard);
        let _l = bars.subscribe(move |_| *h.lock().unwrap() += 1);
        bars.arm("A");
        bars.arm("A");
        bars.arm("B");
        assert_eq!(bars.armed().as_deref(), Some("B"));
        assert_eq!(bars.disarm().as_deref(), Some("B"));
        assert_eq!(bars.disarm(), None);
        assert_eq!(
            *heard.lock().unwrap(),
            3,
            "a change each time, none for the same"
        );
    }
}
