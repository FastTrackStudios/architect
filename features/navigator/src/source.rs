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
    /// a comma list, the first one set wins). A project's note
    /// (`<folder>/project.md`) is titled by its project: its `title`, else
    /// its folder's name ("Album", never "project").
    #[must_use]
    pub fn title<'a>(&'a self, fields: &[&str]) -> &'a str {
        if let Some(project) = self.project_name() {
            return project;
        }
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

    /// The name of the project whose note it is (`<folder>/project.md`):
    /// its `title`, else its folder's name. None for any other file.
    #[must_use]
    pub fn project_name(&self) -> Option<&str> {
        if !self.file_name().eq_ignore_ascii_case("project.md") {
            return None;
        }
        let folder = file_name(self.folder());
        if folder.is_empty() {
            return None;
        }
        match self.get("title") {
            Some(PropertyValue::Text(t)) if !t.trim().is_empty() => Some(t.trim()),
            _ => Some(folder),
        }
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
/// go, a note's place in a manual order, a periodic note made, tags and
/// properties set (`crate::tags`, `crate::properties`).
#[derive(Debug, Clone, PartialEq)]
pub enum FileOp {
    /// Make a folder (and the folders it is in).
    CreateFolder(String),
    /// Make a file holding `text`, and the folders it is in; an error when
    /// one is there already.
    CreateFile { path: String, text: String },
    /// Move (or rename) a file.
    Move { from: String, to: String },
    /// Set a frontmatter property on a note (empty text: the key alone,
    /// `null`).
    SetProperty {
        path: String,
        key: String,
        value: PropertyValue,
    },
    /// Take a frontmatter property off a note.
    RemoveProperty { path: String, key: String },
    /// Tag notes: one that has the tag or a tag above it is left; the tags
    /// below it go (`crate::tags::added`).
    AddTag { paths: Vec<String>, tag: String },
    /// Take a tag itself (any case; not the tags below it) off notes.
    RemoveTag { paths: Vec<String>, tag: String },
    /// Take every tag off notes.
    ClearTags { paths: Vec<String> },
    /// Rename a tag, and the tags below it, in every note.
    RenameTag { from: String, to: String },
    /// Take a tag, and the tags below it, off every note.
    DeleteTag(String),
    /// Rename a frontmatter key (any case) in every note that has it; a
    /// note with `to` already takes `from`'s value (`renamePropertyKeyInFile`).
    RenamePropertyKey { from: String, to: String },
    /// Take a frontmatter key (any case) off every note.
    DeletePropertyKey(String),
    /// Rename one value of a key in every note: a text value, or that
    /// element of a list.
    RenamePropertyValue {
        key: String,
        from: String,
        to: String,
    },
    /// Copy a file (a note duplicated, or dropped with the copy key held);
    /// an error when `to` is there already. Links in it stay as written.
    Copy { from: String, to: String },
    /// Move (or rename) a folder and everything in it; the links that
    /// named its notes by path follow.
    MoveFolder { from: String, to: String },
    /// Copy a folder and everything in it (a folder duplicated).
    CopyFolder { from: String, to: String },
    /// Move a file or a folder to the trash (the vault's `.trash`, where it
    /// can be got back).
    Trash(String),
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
    /// Folders' children in the order the source gives them, by parent
    /// (`""` the root), for those not by name: what shows until the
    /// person orders them by hand.
    fn child_order(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        std::collections::BTreeMap::new()
    }
    /// Every file.
    fn files(&self) -> Vec<NoteFile>;

    /// Whether the navigator browses its notes' properties (the Properties
    /// section, `property_keys`). A vault's notes have properties; a
    /// provider's items carry their own fields (a task's status, a
    /// project's due date), which are not the vault's and never show there.
    fn browses_properties(&self) -> bool {
        true
    }

    /// The sources it is made of, when it is several in one
    /// ([`crate::Mounts`]): each under its own first path segment, drawn
    /// in its place in the left list. None for one vault (the default),
    /// which draws as a folder tree.
    fn mounts(&self) -> Vec<crate::Mount> {
        Vec::new()
    }

    /// A provider's containers' org, shared mark and icon, by path
    /// (`Node::org`, `Node::shared`, `Node::icon`). None by default.
    fn folder_meta(&self) -> std::collections::BTreeMap<String, FolderMeta> {
        std::collections::BTreeMap::new()
    }
    /// Calls `listener` with each change from now on, until the
    /// [`Subscription`] is dropped.
    fn subscribe(&self, listener: Listener) -> Subscription;

    /// A URL the view can draw a file of the vault from (a feature image,
    /// an SVG, a drawing's preview); `None` when it has none.
    fn resource_url(&self, path: &str) -> Option<String> {
        let _ = path;
        None
    }

    /// Whether [`write`](Self::write) does anything (the view offers New
    /// note); `false` by default, a read-only source.
    fn can_write(&self) -> bool {
        false
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

    /// The paths of the notes whose text holds every word of `query` (the
    /// search's full-text provider, architect's own index rather than
    /// Omnisearch); `None` when the source cannot search text (the
    /// default), and the words are matched against names.
    fn search_text(&self, query: &str) -> Option<Vec<String>> {
        let _ = query;
        None
    }

    /// As [`Self::search_text`], each note with the line it matched in (as
    /// Omnisearch gives an `excerpt`); an empty one when the source has
    /// none.
    fn search_excerpts(&self, query: &str) -> Option<Vec<(String, String)>> {
        self.search_text(query)
            .map(|paths| paths.into_iter().map(|p| (p, String::new())).collect())
    }

    /// The text of the file at `path` (a template, notes to merge); `None`
    /// when there is none or the source cannot read text (the default).
    fn read_text(&self, path: &str) -> Option<String> {
        let _ = path;
        None
    }
}

/// What a provider says of one of its containers
/// ([`NavigatorSource::folder_meta`]): whose it is, and its icon.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderMeta {
    /// The org's id, when it says one.
    pub org: Option<String>,
    /// Shared from another org.
    pub shared: bool,
    /// Its own icon (a project's, a library's), when no icon is set on
    /// the folder.
    pub icon: Option<String>,
    /// Its own colour (a project's), when none is set on the folder.
    pub colour: Option<String>,
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
    /// The notes' texts, where they were given or written.
    texts: RwLock<BTreeMap<String, String>>,
    /// What went to the trash, in order.
    trash: RwLock<Vec<String>>,
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

    /// With `text` as the text of the file at `path` (what
    /// [`NavigatorSource::read_text`] reads).
    #[must_use]
    pub fn with_text(self, path: impl Into<String>, text: impl Into<String>) -> Self {
        self.texts_mut().insert(path.into(), text.into());
        self
    }

    /// What went to the trash, in order.
    #[must_use]
    pub fn trashed(&self) -> Vec<String> {
        self.inner
            .trash
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn texts_mut(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<String, String>> {
        self.inner
            .texts
            .write()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// Keeps `text` as the note at `path`'s (what [`NavigatorSource::read_text`]
    /// and the search read), and replaces its file with `file` (its
    /// preview, tags and properties made from the text, as
    /// [`crate::markdown::note_from_markdown`] does): an edit in an
    /// in-memory vault, heard by the view as one change.
    pub fn set_text(&self, file: NoteFile, text: impl Into<String>) {
        self.texts_mut().insert(file.path.clone(), text.into());
        self.put(file);
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
        let mut texts = self.texts_mut();
        if let Some(t) = texts.remove(from) {
            texts.insert(file.path.clone(), t);
        }
        drop(texts);
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

    fn can_write(&self) -> bool {
        true
    }

    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        ops.iter().try_for_each(|op| self.write_one(op))
    }

    /// Its notes' names and previews, the text it keeps.
    fn search_text(&self, query: &str) -> Option<Vec<String>> {
        let words: Vec<String> = query.split_whitespace().map(crate::search::fold).collect();
        let files = self
            .inner
            .files
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        Some(
            files
                .values()
                .filter(|f| {
                    let text = crate::search::fold(&format!(
                        "{} {}",
                        f.name(),
                        f.preview.as_deref().unwrap_or_default()
                    ));
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .map(|f| f.path.clone())
                .collect(),
        )
    }

    fn read_text(&self, path: &str) -> Option<String> {
        self.inner
            .texts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
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
                // Its frontmatter's `tags` list, and its text.
                let (tags, body) = match text
                    .strip_prefix("---\n")
                    .and_then(|t| t.split_once("\n---\n"))
                {
                    Some((fm, body)) => (
                        fm.lines()
                            .skip_while(|l| l.trim() != "tags:")
                            .skip(1)
                            .map_while(|l| l.trim().strip_prefix("- "))
                            .map(str::to_owned)
                            .collect(),
                        body,
                    ),
                    None => (Vec::new(), text.as_str()),
                };
                let body: Vec<&str> = body
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                    .collect();
                let mut file = NoteFile::new(path.clone(), now, now).preview(body.join(" "));
                file.tags = tags;
                self.texts_mut().insert(path.clone(), text.clone());
                self.put(file);
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
            FileOp::RemoveProperty { path, key } => {
                let mut file = self.file(path).ok_or_else(|| format!("no {path}"))?;
                file.properties.retain(|(k, _)| k != key);
                self.put(file);
            }
            FileOp::AddTag { paths, tag } => self.retag(paths, |t| crate::tags::added(t, tag))?,
            FileOp::RemoveTag { paths, tag } => self.retag(paths, |t| {
                let key = crate::tags::tag_key(tag);
                let kept: Vec<String> = t
                    .iter()
                    .filter(|x| crate::tags::tag_key(x) != key)
                    .cloned()
                    .collect();
                (kept.len() != t.len()).then_some(kept)
            })?,
            FileOp::ClearTags { paths } => {
                self.retag(paths, |t| (!t.is_empty()).then(Vec::new))?;
            }
            FileOp::RenameTag { from, to } => self.retag_everywhere(from, Some(to))?,
            op @ (FileOp::RenamePropertyKey { .. }
            | FileOp::DeletePropertyKey(_)
            | FileOp::RenamePropertyValue { .. }) => {
                let files = self.files();
                for each in crate::properties::expand(op, &files) {
                    self.write(std::slice::from_ref(&each))?;
                }
            }
            FileOp::DeleteTag(tag) => self.retag_everywhere(tag, None)?,
            FileOp::Copy { from, to } => {
                if self.file(to).is_some() {
                    return Err(format!("{to} is there already"));
                }
                let mut file = self.file(from).ok_or_else(|| format!("no {from}"))?;
                file.path.clone_from(to);
                let folder = parent(to);
                if !folder.is_empty() {
                    self.create_folder(folder.to_owned());
                }
                if let Some(t) = self.read_text(from) {
                    self.texts_mut().insert(to.clone(), t);
                }
                self.put(file);
            }
            FileOp::MoveFolder { from, to } => self.move_folder(from, to)?,
            FileOp::CopyFolder { from, to } => {
                if self.has_folder(to) {
                    return Err(format!("{to} is there already"));
                }
                if !self.has_folder(from) {
                    return Err(format!("no folder {from}"));
                }
                let moved = |p: &str| format!("{to}{}", p.get(from.len()..).unwrap_or(""));
                let folders: Vec<String> = self.folders_within(from);
                for f in folders {
                    self.create_folder(moved(&f));
                }
                let files: Vec<NoteFile> = self
                    .files()
                    .into_iter()
                    .filter(|f| is_within(&f.path, from))
                    .collect();
                for mut f in files {
                    let text = self.read_text(&f.path);
                    f.path = moved(&f.path);
                    if let Some(t) = text {
                        self.texts_mut().insert(f.path.clone(), t);
                    }
                    self.put(f);
                }
            }
            FileOp::Trash(path) => {
                if self.file(path).is_some() {
                    self.texts_mut().remove(path);
                    self.remove(path);
                } else if self.has_folder(path) {
                    self.texts_mut().retain(|p, _| !is_within(p, path));
                    self.delete_folder(path);
                } else {
                    return Err(format!("no {path}"));
                }
                self.inner
                    .trash
                    .write()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(path.clone());
            }
        }
        Ok(())
    }

    /// Every folder: those made, and those a file's path passes through.
    fn all_folders(&self) -> BTreeSet<String> {
        let mut all = self
            .inner
            .folders
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for p in self
            .inner
            .files
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
        {
            all.extend(
                ancestors(p)
                    .filter(|a| !a.is_empty() && *a != p)
                    .map(str::to_owned),
            );
        }
        all
    }

    fn has_folder(&self, folder: &str) -> bool {
        !folder.is_empty() && self.all_folders().contains(folder)
    }

    /// `folder` and the folders in it.
    fn folders_within(&self, folder: &str) -> Vec<String> {
        self.all_folders()
            .into_iter()
            .filter(|f| is_within(f, folder))
            .collect()
    }

    /// `from` and everything in it, at `to`: one [`SourceEvent::FolderRenamed`].
    fn move_folder(&self, from: &str, to: &str) -> Result<(), String> {
        if from.is_empty() || to.is_empty() {
            return Err("the vault's root does not move".to_owned());
        }
        if !self.has_folder(from) {
            return Err(format!("no folder {from}"));
        }
        if is_within(to, from) {
            return Err(format!("{from} cannot go into itself"));
        }
        if self.has_folder(to) || self.file(to).is_some() {
            return Err(format!("{to} is there already"));
        }
        let moved = |p: &str| format!("{to}{}", p.get(from.len()..).unwrap_or(""));
        let inside = self.folders_within(from);
        {
            let mut folders = self
                .inner
                .folders
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            for f in inside {
                folders.remove(&f);
                folders.insert(moved(&f));
            }
            for a in ancestors(to).filter(|a| !a.is_empty()) {
                folders.insert(a.to_owned());
            }
        }
        {
            let mut files = self.files_mut();
            let inside: Vec<String> = files
                .keys()
                .filter(|p| is_within(p, from))
                .cloned()
                .collect();
            for p in inside {
                if let Some(mut f) = files.remove(&p) {
                    f.path = moved(&p);
                    files.insert(f.path.clone(), f);
                }
            }
        }
        {
            let mut texts = self.texts_mut();
            let inside: Vec<String> = texts
                .keys()
                .filter(|p| is_within(p, from))
                .cloned()
                .collect();
            for p in inside {
                if let Some(t) = texts.remove(&p) {
                    texts.insert(moved(&p), t);
                }
            }
        }
        self.emit(&SourceEvent::FolderRenamed {
            from: from.to_owned(),
            to: to.to_owned(),
        });
        Ok(())
    }

    /// `tag` and the tags below it renamed to `to` (or taken off) in every
    /// note.
    fn retag_everywhere(&self, tag: &str, to: Option<&str>) -> Result<(), String> {
        use crate::tags::{is_or_below, renamed, tag_key};
        let paths: Vec<String> = self.files().into_iter().map(|f| f.path).collect();
        let key = tag_key(tag);
        self.retag(&paths, |t| {
            if !t.iter().any(|x| is_or_below(&tag_key(x), &key)) {
                return None;
            }
            Some(to.map_or_else(
                || {
                    t.iter()
                        .filter(|x| !is_or_below(&tag_key(x), &key))
                        .cloned()
                        .collect()
                },
                |to| renamed(t, tag, to),
            ))
        })
    }

    /// Each note of `paths` with its tags as `f` makes them (none: left).
    fn retag(
        &self,
        paths: &[String],
        f: impl Fn(&[String]) -> Option<Vec<String>>,
    ) -> Result<(), String> {
        for p in paths {
            let mut file = self.file(p).ok_or_else(|| format!("no {p}"))?;
            if let Some(tags) = f(&file.tags) {
                file.tags = tags;
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

#[cfg(test)]
mod project_name {
    use super::{NoteFile, PropertyValue};

    #[test]
    fn a_projects_note_is_titled_by_its_project() {
        let bare = NoteFile::new("Projects/Album/project.md", 0, 0);
        assert_eq!(bare.title(&[]), "Album", "its folder's name");
        let titled = NoteFile::new("Projects/Album/project.md", 0, 0)
            .property("title", PropertyValue::Text("The Album".to_owned()));
        assert_eq!(
            titled.title(&[]),
            "The Album",
            "its title, names from frontmatter or not"
        );
        // Any other note keeps its name; a root `project.md` has no project.
        assert_eq!(
            NoteFile::new("Projects/Album/Tracklist.md", 0, 0).title(&[]),
            "Tracklist"
        );
        assert_eq!(NoteFile::new("project.md", 0, 0).title(&[]), "project");
    }
}
