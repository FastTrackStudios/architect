//! The navigator's view of a vault: [`NavigatorSource`].
//!
//! Notebook Navigator reads Obsidian's vault (`TFile`, `TFolder`) and keeps
//! what it derives per file in its own cache (`FileData`:
//! `src/storage/indexeddb/fileData.ts`): tags, properties, the preview, a
//! feature image, task counts. Here a source hands all of that over as a
//! [`NoteFile`], and says what changed as [`SourceEvent`]s. A field a
//! source has not worked out yet is `None` (Notebook Navigator's `null`,
//! "not generated yet"); the row draws without it and a
//! [`SourceEvent::FileModified`] fills it in later.
//!
//! [`MemorySource`] keeps a vault in memory: the tests', the lab's.
//! Architect's vault becomes a source through an adapter in this crate.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

/// Milliseconds since the Unix epoch, UTC.
pub type Millis = i64;

/// A frontmatter property's value.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
}

/// A note's tasks: how many, and how many are still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TaskCounts {
    pub total: u32,
    pub unfinished: u32,
}

/// One file of the vault, as the navigator shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NoteFile {
    /// Its path from the vault's root, `/`-separated, with its extension:
    /// `Projects/Alpha/Plan.md`.
    pub path: String,
    /// When it was made and last changed ([`Millis`]).
    pub created: Millis,
    pub modified: Millis,
    /// Its frontmatter, in order.
    pub properties: Vec<(String, PropertyValue)>,
    /// Its tags, without `#`.
    pub tags: Vec<String>,
    /// The start of its text, as plain text; `None` until worked out.
    pub preview: Option<String>,
    /// An image to show beside it (a path in the vault or a URL).
    pub feature_image: Option<String>,
    /// Its tasks; `None` until counted.
    pub tasks: Option<TaskCounts>,
    /// How many words its text has; `None` until counted (Notebook
    /// Navigator's `textCountDisplay`).
    pub words: Option<u32>,
}

impl NoteFile {
    /// A file at `path`, made and changed at `modified`.
    #[must_use]
    pub fn new(path: impl Into<String>, created: Millis, modified: Millis) -> Self {
        Self {
            path: path.into(),
            created,
            modified,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn preview(mut self, text: impl Into<String>) -> Self {
        self.preview = Some(text.into());
        self
    }

    #[must_use]
    pub fn property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties.push((key.into(), value));
        self
    }

    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    #[must_use]
    pub fn feature_image(mut self, image: impl Into<String>) -> Self {
        self.feature_image = Some(image.into());
        self
    }

    #[must_use]
    pub const fn tasks(mut self, total: u32, unfinished: u32) -> Self {
        self.tasks = Some(TaskCounts { total, unfinished });
        self
    }

    #[must_use]
    pub const fn words(mut self, words: u32) -> Self {
        self.words = Some(words);
        self
    }

    /// Its file name with the extension: `Plan.md`.
    #[must_use]
    pub fn file_name(&self) -> &str {
        file_name(&self.path)
    }

    /// Its name without the extension (Obsidian's `basename`): `Plan`.
    #[must_use]
    pub fn name(&self) -> &str {
        let n = self.file_name();
        n.rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map_or(n, |(stem, _)| stem)
    }

    /// Its extension, without the dot: `md`.
    #[must_use]
    pub fn extension(&self) -> &str {
        let n = self.file_name();
        n.rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map_or("", |(_, ext)| ext)
    }

    /// The folder it is in (`""` for the vault's root).
    #[must_use]
    pub fn folder(&self) -> &str {
        parent(&self.path)
    }

    /// A property's value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&PropertyValue> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Its title: the first of `fields` (frontmatter keys) with a text
    /// value, else its name (Notebook Navigator's `frontmatterNameField`,
    /// a comma list, the first one set wins).
    #[must_use]
    pub fn title<'a>(&'a self, fields: &[&str]) -> &'a str {
        for key in fields {
            if let Some(PropertyValue::Text(t)) = self.get(key) {
                let t = t.trim();
                if !t.is_empty() {
                    return t;
                }
            }
        }
        self.name()
    }

    /// Whether it is a Markdown note.
    #[must_use]
    pub fn is_markdown(&self) -> bool {
        self.extension().eq_ignore_ascii_case("md")
    }
}

/// The last segment of a path.
#[must_use]
pub fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, n)| n)
}

/// The folder a path is in: `""` (the vault's root) for a top-level one.
#[must_use]
pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Every folder a path is inside, nearest first, the root (`""`) last.
pub fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    let mut at = Some(path);
    std::iter::from_fn(move || {
        let p = at?;
        if p.is_empty() {
            at = None;
            return None;
        }
        let up = parent(p);
        at = Some(up);
        Some(up)
    })
}

/// Whether `path` is `folder` or inside it (the root holds everything).
#[must_use]
pub fn is_within(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// What changed in a source.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceEvent {
    FolderCreated(String),
    /// The folder and everything in it.
    FolderDeleted(String),
    FolderRenamed {
        from: String,
        to: String,
    },
    FileCreated(NoteFile),
    /// Its text or what is derived from it (a preview worked out, tags).
    FileModified(NoteFile),
    FileDeleted(String),
    FileRenamed {
        from: String,
        file: NoteFile,
    },
    /// Read everything again.
    Reset,
}

/// A change the navigator asks of a source: a folder note made or let
/// go, a note's place in a manual order, a periodic note made.
#[derive(Debug, Clone, PartialEq)]
pub enum FileOp {
    /// Make a folder (and the folders it is in).
    CreateFolder(String),
    /// Make a file holding `text`, and the folders it is in; an error when
    /// one is there already.
    CreateFile { path: String, text: String },
    /// Move (or rename) a file.
    Move { from: String, to: String },
    /// Set a frontmatter property on a note.
    SetProperty {
        path: String,
        key: String,
        value: PropertyValue,
    },
}

/// Kept while the subscriber wants events; dropping it stops them.
#[must_use = "dropping it unsubscribes"]
pub struct Subscription(Option<Box<dyn FnOnce() + Send + Sync>>);

impl Subscription {
    /// Runs `stop` when dropped.
    pub fn new(stop: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self(Some(Box::new(stop)))
    }

    /// A subscription to a source that never changes.
    pub fn none() -> Self {
        Self(None)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Subscription")
    }
}

/// What a subscriber is called with, on whichever thread the change
/// happened.
pub type Listener = Box<dyn Fn(&SourceEvent) + Send + Sync>;

/// The navigator's view of a vault.
///
/// Folders are paths from the root (`Projects/Alpha`; the root itself is
/// `""` and is never listed). A file's folders need not be listed: the
/// navigator makes every folder a path passes through.
pub trait NavigatorSource: Send + Sync {
    /// The vault's name: the root folder's row.
    fn name(&self) -> String;
    /// Every folder, the root left out.
    fn folders(&self) -> Vec<String>;
    /// Every file.
    fn files(&self) -> Vec<NoteFile>;
    /// Calls `listener` with each change from now on, until the
    /// [`Subscription`] is dropped.
    fn subscribe(&self, listener: Listener) -> Subscription;

    /// A URL the view can draw a file of the vault from (a feature image,
    /// an SVG, a drawing's preview); `None` when it has none.
    fn resource_url(&self, path: &str) -> Option<String> {
        let _ = path;
        None
    }

    /// Does `ops`, in order, and says what changed through its events.
    ///
    /// # Errors
    ///
    /// The source is read-only (the default), or an op failed: the ones
    /// before it stay done.
    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        let _ = ops;
        Err("this vault is read-only".to_owned())
    }
}

/// A vault kept in memory.
#[derive(Clone, Default)]
pub struct MemorySource {
    inner: Arc<MemoryInner>,
}

#[derive(Default)]
struct MemoryInner {
    name: RwLock<String>,
    folders: RwLock<BTreeSet<String>>,
    files: RwLock<BTreeMap<String, NoteFile>>,
    resources: RwLock<BTreeMap<String, String>>,
    listeners: Mutex<Vec<(u64, Arc<dyn Fn(&SourceEvent) + Send + Sync>)>>,
    next: Mutex<u64>,
}

impl MemorySource {
    /// An empty vault called `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let s = Self::default();
        *s.inner.name.write().unwrap_or_else(PoisonError::into_inner) = name.into();
        s
    }

    /// With `folder` (and the folders it is in).
    #[must_use]
    pub fn with_folder(self, folder: impl Into<String>) -> Self {
        self.create_folder(folder);
        self
    }

    /// With `file`.
    #[must_use]
    pub fn with_file(self, file: NoteFile) -> Self {
        self.put(file);
        self
    }

    /// With `url` as what the view draws the file at `path` from.
    #[must_use]
    pub fn with_resource(self, path: impl Into<String>, url: impl Into<String>) -> Self {
        self.inner
            .resources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.into(), url.into());
        self
    }

    /// With every file of `files`, at once (one [`SourceEvent::Reset`]).
    #[must_use]
    pub fn with_files(self, files: impl IntoIterator<Item = NoteFile>) -> Self {
        {
            let mut all = self.files_mut();
            for f in files {
                all.insert(f.path.clone(), f);
            }
        }
        self.emit(&SourceEvent::Reset);
        self
    }

    fn files_mut(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, NoteFile>> {
        self.inner
            .files
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, event: &SourceEvent) {
        let listeners: Vec<_> = self
            .inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, l)| Arc::clone(l))
            .collect();
        for l in listeners {
            l(event);
        }
    }

    /// Makes `folder`; says so when it is new.
    pub fn create_folder(&self, folder: impl Into<String>) {
        let folder = folder.into();
        let new = self
            .inner
            .folders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(folder.clone());
        if new {
            self.emit(&SourceEvent::FolderCreated(folder));
        }
    }

    /// Adds `file`, or replaces the one at its path.
    pub fn put(&self, file: NoteFile) {
        let old = self.files_mut().insert(file.path.clone(), file.clone());
        self.emit(&if old.is_some() {
            SourceEvent::FileModified(file)
        } else {
            SourceEvent::FileCreated(file)
        });
    }

    /// Removes the file at `path`.
    pub fn remove(&self, path: &str) {
        if self.files_mut().remove(path).is_some() {
            self.emit(&SourceEvent::FileDeleted(path.to_owned()));
        }
    }

    /// Moves the file at `from` to `to`.
    pub fn rename(&self, from: &str, to: impl Into<String>) {
        let file = {
            let mut files = self.files_mut();
            let Some(mut file) = files.remove(from) else {
                return;
            };
            file.path = to.into();
            files.insert(file.path.clone(), file.clone());
            file
        };
        self.emit(&SourceEvent::FileRenamed {
            from: from.to_owned(),
            file,
        });
    }

    /// Deletes `folder` and everything in it.
    pub fn delete_folder(&self, folder: &str) {
        self.inner
            .folders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|f| !is_within(f, folder));
        self.files_mut().retain(|p, _| !is_within(p, folder));
        self.emit(&SourceEvent::FolderDeleted(folder.to_owned()));
    }

    /// The file at `path`.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<NoteFile> {
        self.inner
            .files
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
    }

    /// A made-up vault of `notes` notes spread over `folders` folders (two
    /// levels: `Area N/Topic M`), each a minute older than the one before
    /// it, from `now`: a load to test and measure with.
    #[must_use]
    pub fn synthetic(name: &str, folders: usize, notes: usize, now: Millis) -> Self {
        let folders = folders.max(1);
        let names: Vec<String> = (0..folders)
            .map(|i| format!("Area {}/Topic {}", i / 10, i % 10))
            .collect();
        let s = Self::new(name);
        {
            let mut f = s
                .inner
                .folders
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            for n in &names {
                f.insert(n.clone());
            }
        }
        let mut minute: Millis = 0;
        let files = (0..notes).map(|i| {
            let folder = names
                .get(i.checked_rem(folders).unwrap_or(0))
                .map_or("", String::as_str);
            minute = minute.saturating_add(60_000);
            let at = now.saturating_sub(minute);
            NoteFile::new(format!("{folder}/Note {i}.md"), at, at)
                .preview(format!("The text of note {i}, as it starts."))
        });
        s.with_files(files)
    }
}

impl NavigatorSource for MemorySource {
    fn name(&self) -> String {
        self.inner
            .name
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn folders(&self) -> Vec<String> {
        self.inner
            .folders
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn files(&self) -> Vec<NoteFile> {
        self.inner
            .files
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    fn resource_url(&self, path: &str) -> Option<String> {
        self.inner
            .resources
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
    }

    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        ops.iter().try_for_each(|op| self.write_one(op))
    }

    fn subscribe(&self, listener: Listener) -> Subscription {
        let id = {
            let mut n = self
                .inner
                .next
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *n = n.saturating_add(1);
            *n
        };
        self.inner
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, Arc::from(listener)));
        let inner = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner
                    .listeners
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .retain(|(i, _)| *i != id);
            }
        })
    }
}

impl MemorySource {
    fn write_one(&self, op: &FileOp) -> Result<(), String> {
        match op {
            FileOp::CreateFolder(f) => {
                if self
                    .inner
                    .folders
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(f)
                {
                    return Err(format!("{f} is there already"));
                }
                self.create_folder(f.clone());
            }
            FileOp::CreateFile { path, text } => {
                if self.file(path).is_some() {
                    return Err(format!("{path} is there already"));
                }
                let folders: Vec<&str> = ancestors(path).filter(|f| !f.is_empty()).collect();
                for folder in folders.into_iter().rev() {
                    self.create_folder(folder);
                }
                let now = chrono::Utc::now().timestamp_millis();
                let body: Vec<&str> = text
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                    .collect();
                self.put(NoteFile::new(path.clone(), now, now).preview(body.join(" ")));
            }
            FileOp::Move { from, to } => {
                if self.file(to).is_some() {
                    return Err(format!("{to} is there already"));
                }
                if self.file(from).is_none() {
                    return Err(format!("no {from}"));
                }
                let folder = parent(to);
                if !folder.is_empty() {
                    self.create_folder(folder.to_owned());
                }
                self.rename(from, to.clone());
            }
            FileOp::SetProperty { path, key, value } => {
                let mut file = self.file(path).ok_or_else(|| format!("no {path}"))?;
                match file.properties.iter_mut().find(|(k, _)| k == key) {
                    Some((_, v)) => *v = value.clone(),
                    None => file.properties.push((key.clone(), value.clone())),
                }
                self.put(file);
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for MemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemorySource")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}
