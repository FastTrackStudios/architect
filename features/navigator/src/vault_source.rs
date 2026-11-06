//! Architect's vault as a [`NavigatorSource`] (feature `vault`).
//!
//! Its folders and files, each note's properties, tags, preview, feature image
//! and tasks (what Notebook Navigator derives per file, `FileData`), and
//! the changes as they come: the app's own (a save, a new note, a rename)
//! at once, edits made outside the app as the vault's watcher hears them
//! ([`VaultSource::watch`]).

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::SystemTime;

use serde_json::Value;
use vault::actions::{Changes, OpenVault, Renamed, TagEdit, Watch, page_tags};
use vault::obsidian::{Vault, VaultPage, page_tasks, page_wordcount};

use crate::source::{
    FileOp, Listener, Millis, NavigatorSource, NoteFile, PropertyValue, SourceEvent, Subscription,
    TaskCounts,
};

/// The frontmatter properties a note's feature image is read from, in
/// order (Notebook Navigator's `featureImageProperties`, and the common
/// others).
pub const FEATURE_IMAGE_PROPERTIES: &[&str] = &[
    "thumbnail",
    "featureResized",
    "feature",
    "image",
    "cover",
    "banner",
];

/// How long a preview is, at most (characters).
const PREVIEW_CHARS: usize = 300;

/// Image files a feature image may be.
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp"];

type Shared = Arc<dyn Fn(&SourceEvent) + Send + Sync>;

struct Inner {
    vault: OpenVault,
    name: String,
    listeners: Mutex<Vec<(u64, Shared)>>,
    next: Mutex<u64>,
    watch: Mutex<Option<Watch>>,
}

/// Architect's vault, as the navigator sees it. Cheap to clone; clones
/// share it.
#[derive(Clone)]
pub struct VaultSource {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for VaultSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultSource")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

impl VaultSource {
    /// The vault open at `vault`, named for its folder.
    #[must_use]
    pub fn new(vault: OpenVault) -> Self {
        let name = vault
            .root()
            .file_name()
            .map_or_else(|| "Vault".to_owned(), |n| n.to_string_lossy().into_owned());
        Self::named(vault, name)
    }

    /// The vault open at `vault`, its root's row called `name`.
    #[must_use]
    pub fn named(vault: OpenVault, name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                vault,
                name: name.into(),
                listeners: Mutex::new(Vec::new()),
                next: Mutex::new(0),
                watch: Mutex::new(None),
            }),
        }
    }

    /// The vault it reads.
    #[must_use]
    pub fn vault(&self) -> &OpenVault {
        &self.inner.vault
    }

    /// Starts hearing edits made outside the app (until the last clone is
    /// dropped): each one is a [`SourceEvent`] to the subscribers.
    ///
    /// # Errors
    ///
    /// The vault's watcher cannot start.
    pub fn watch(&self) -> Result<(), String> {
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let watch = self.inner.vault.watch(move |changes| {
            if let Some(inner) = weak.upgrade() {
                Self { inner }.changed(&changes);
            }
        })?;
        *self
            .inner
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(())
    }

    /// The file at `path`, as the navigator shows it.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<NoteFile> {
        self.inner.vault.with(|v| file_of(v, path))
    }

    /// Tells the subscribers what `changes` changed (what a refresh found).
    pub fn changed(&self, changes: &Changes) {
        for f in &changes.folders_created {
            self.emit(&SourceEvent::FolderCreated(f.clone()));
        }
        for p in &changes.created {
            if let Some(f) = self.file(p) {
                self.emit(&SourceEvent::FileCreated(f));
            }
        }
        for p in &changes.modified {
            if let Some(f) = self.file(p) {
                self.emit(&SourceEvent::FileModified(f));
            }
        }
        for p in &changes.removed {
            self.emit(&SourceEvent::FileDeleted(p.clone()));
        }
        for f in &changes.folders_removed {
            self.emit(&SourceEvent::FolderDeleted(f.clone()));
        }
    }

    /// Saves a note's text (an editor's, debounced) and says so.
    ///
    /// # Errors
    ///
    /// No such note, or it cannot be written.
    pub fn save(&self, path: &str, text: &str) -> Result<(), String> {
        if self.inner.vault.text(path).as_deref() == Some(text) {
            return Ok(());
        }
        self.inner.vault.save(path, text)?;
        if let Some(f) = self.file(path) {
            self.emit(&SourceEvent::FileModified(f));
        }
        Ok(())
    }

    /// Makes a new, empty note in `folder` (`""` for the root), named as
    /// Obsidian names one (`Untitled`, `Untitled 1`, …), and says so. Its
    /// path.
    ///
    /// # Errors
    ///
    /// It cannot be written.
    pub fn create_note(&self, folder: &str) -> Result<String, String> {
        let path = self.inner.vault.untitled(folder);
        let before = self.inner.vault.folders();
        self.inner.vault.create(&path, "")?;
        for f in self.inner.vault.folders().difference(&before) {
            self.emit(&SourceEvent::FolderCreated(f.clone()));
        }
        if let Some(f) = self.file(&path) {
            self.emit(&SourceEvent::FileCreated(f));
        }
        Ok(path)
    }

    /// Renames or moves a note, the links that named it following
    /// ([`OpenVault::rename`]), and says so: the file renamed, and each
    /// note whose links changed modified.
    ///
    /// # Errors
    ///
    /// No such note, the new path is taken or not a note's.
    pub fn rename(&self, from: &str, to: &str) -> Result<Renamed, String> {
        let before = self.inner.vault.folders();
        let renamed = self.inner.vault.rename(from, to)?;
        if renamed.from == renamed.to {
            return Ok(renamed);
        }
        for f in self.inner.vault.folders().difference(&before) {
            self.emit(&SourceEvent::FolderCreated(f.clone()));
        }
        if let Some(file) = self.file(&renamed.to) {
            self.emit(&SourceEvent::FileRenamed {
                from: renamed.from.clone(),
                file,
            });
        }
        for p in &renamed.relinked {
            if let Some(f) = self.file(p) {
                self.emit(&SourceEvent::FileModified(f));
            }
        }
        Ok(renamed)
    }

    /// One of [`NavigatorSource::write`]'s ops, done on disk and said.
    fn write_one(&self, op: &FileOp) -> Result<(), String> {
        let vault = &self.inner.vault;
        let before = vault.folders();
        let new_folders = |this: &Self| {
            for f in this.inner.vault.folders().difference(&before) {
                this.emit(&SourceEvent::FolderCreated(f.clone()));
            }
        };
        match op {
            FileOp::CreateFolder(folder) => {
                if before.contains(folder) {
                    return Err(format!("{folder} is there already"));
                }
                vault.create_folder(folder)?;
                new_folders(self);
            }
            // A periodic note made (the calendar's), and its folders.
            FileOp::CreateFile { path, text } => {
                if vault.text(path).is_some() {
                    return Err(format!("{path} is there already"));
                }
                vault.create(path, text)?;
                new_folders(self);
                if let Some(f) = self.file(path) {
                    self.emit(&SourceEvent::FileCreated(f));
                }
            }
            FileOp::Move { from, to } => {
                let note = |p: &str| {
                    std::path::Path::new(p)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                };
                if note(from) && note(to) {
                    self.rename(from, to)?;
                } else {
                    vault.move_file(from, to)?;
                    new_folders(self);
                    if let Some(file) = self.file(to) {
                        self.emit(&SourceEvent::FileRenamed {
                            from: from.clone(),
                            file,
                        });
                    }
                }
            }
            FileOp::SetProperty { path, key, value } => {
                vault.set_property(path, key, json_of(value))?;
                if let Some(f) = self.file(path) {
                    self.emit(&SourceEvent::FileModified(f));
                }
            }
            // The tags, through the vault's (`OpenVault::edit_tags`): the
            // markdown changes, frontmatter and inline `#tags`.
            FileOp::AddTag { paths, tag } => {
                let changed = vault.edit_tags(paths, &TagEdit::Add(tag))?;
                self.modified(&changed);
            }
            FileOp::RemoveTag { paths, tag } => {
                let changed = vault.edit_tags(paths, &TagEdit::Remove(tag))?;
                self.modified(&changed);
            }
            FileOp::ClearTags { paths } => {
                let changed = vault.edit_tags(paths, &TagEdit::Clear)?;
                self.modified(&changed);
            }
            FileOp::RenameTag { from, to } => {
                let changed = vault.edit_tags_everywhere(&TagEdit::Rename { from, to })?;
                self.modified(&changed);
            }
            FileOp::DeleteTag(tag) => {
                let changed = vault.edit_tags_everywhere(&TagEdit::Delete(tag))?;
                self.modified(&changed);
            }
            FileOp::Copy { from, to } => {
                let changes = vault.copy_file(from, to)?;
                self.changed(&changes);
            }
            FileOp::CopyFolder { from, to } => {
                let changes = vault.copy_folder(from, to)?;
                self.changed(&changes);
            }
            // One event for the folder; the notes whose links followed
            // changed too.
            FileOp::MoveFolder { from, to } => {
                let renamed = vault.move_folder(from, to)?;
                new_folders(self);
                self.emit(&SourceEvent::FolderRenamed {
                    from: from.clone(),
                    to: to.clone(),
                });
                let relinked: Vec<String> = renamed
                    .into_iter()
                    .flat_map(|r| r.relinked)
                    .filter(|p| !p.starts_with(&format!("{to}/")))
                    .collect();
                self.modified(&relinked);
            }
            FileOp::Trash(path) => {
                let was_folder = before.contains(path);
                let (changes, _) = vault.trash(path)?;
                if was_folder {
                    self.emit(&SourceEvent::FolderDeleted(path.clone()));
                } else {
                    self.changed(&changes);
                }
            }
        }
        Ok(())
    }

    /// Says that the notes at `paths` changed.
    fn modified(&self, paths: &[String]) {
        for p in paths {
            if let Some(f) = self.file(p) {
                self.emit(&SourceEvent::FileModified(f));
            }
        }
    }

    fn emit(&self, event: &SourceEvent) {
        let listeners: Vec<Shared> = self
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
}

impl NavigatorSource for VaultSource {
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    fn folders(&self) -> Vec<String> {
        self.inner.vault.folders().into_iter().collect()
    }

    fn files(&self) -> Vec<NoteFile> {
        self.inner.vault.with(|v| {
            let mut out: Vec<NoteFile> = v.pages.iter().map(|p| note_of(v, p)).collect();
            out.extend(v.bases.iter().map(|b| plain(&v.root, &b.rel_path)));
            out.extend(v.attachments.iter().map(|a| plain(&v.root, &a.rel_path)));
            out
        })
    }

    fn resource_url(&self, path: &str) -> Option<String> {
        self.inner
            .vault
            .with(|v| resolve_resource(v, path))
            .map(|abs| file_url(&abs))
    }

    fn can_write(&self) -> bool {
        true
    }

    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        ops.iter().try_for_each(|op| self.write_one(op))
    }

    /// The vault's full-text search (`OpenVault::search_text`).
    fn search_text(&self, query: &str) -> Option<Vec<String>> {
        Some(
            self.inner
                .vault
                .search_text(query, usize::MAX)
                .into_iter()
                .map(|h| h.path)
                .collect(),
        )
    }

    fn read_text(&self, path: &str) -> Option<String> {
        self.inner.vault.text(path)
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

/// The file at `path`: a note with all it derives, else a base or an
/// attachment with its times.
fn file_of(v: &Vault, path: &str) -> Option<NoteFile> {
    if let Some(p) = v.page(path) {
        return Some(note_of(v, p));
    }
    let other = v.bases.iter().any(|b| b.rel_path == path)
        || v.attachments.iter().any(|a| a.rel_path == path);
    other.then(|| plain(&v.root, path))
}

fn millis(t: SystemTime) -> Millis {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|d| Millis::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// When the file at `path` was made and last changed.
fn times(root: &Path, path: &str) -> (Millis, Millis) {
    let meta = std::fs::metadata(root.join(path)).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map_or(0, millis);
    let created = meta
        .as_ref()
        .and_then(|m| m.created().ok())
        .map_or(modified, millis);
    (created.min(modified), modified)
}

/// A file with nothing derived from it: a base, an image, a PDF.
fn plain(root: &Path, path: &str) -> NoteFile {
    let (created, modified) = times(root, path);
    NoteFile::new(path, created, modified)
}

/// A note, with its properties, tags, preview, feature image and tasks.
fn note_of(v: &Vault, page: &VaultPage) -> NoteFile {
    let (created, modified) = times(&v.root, &page.rel_path);
    let mut file = derived(page, created, modified);
    // A path in the vault: the view draws it from `resource_url`.
    file.feature_image = feature_image(v, page).and_then(|abs| {
        abs.strip_prefix(&v.root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    });
    file
}

/// What a page's text gives: its properties, tags, preview, words and
/// tasks.
fn derived(page: &VaultPage, created: Millis, modified: Millis) -> NoteFile {
    let mut file = NoteFile::new(page.rel_path.clone(), created, modified);
    file.properties = page
        .parsed
        .frontmatter
        .iter()
        .map(|e| (e.key.clone(), property(&e.value)))
        .collect();
    file.tags = page_tags(page);
    file.preview = Some(preview_of(&page.raw));
    file.words = Some(u32::try_from(page_wordcount(page).words).unwrap_or(u32::MAX));
    let tasks = page_tasks(page);
    let total = u32::try_from(tasks.len()).unwrap_or(u32::MAX);
    let open = u32::try_from(tasks.iter().filter(|t| t.is_open()).count()).unwrap_or(u32::MAX);
    file.tasks = Some(TaskCounts {
        total,
        unfinished: open,
    });
    file
}

/// A note at `path` as [`VaultSource`] shows one, from a text the caller
/// holds.
///
/// For an in-memory vault built from the same files (the lab's film
/// vault): its properties, tags, preview, words and tasks, and its feature
/// image as its frontmatter names it (the first of
/// [`FEATURE_IMAGE_PROPERTIES`], else the first image embedded; the
/// source's `resource_url` draws it).
#[must_use]
pub fn note_from_text(path: &str, text: &str, created: Millis, modified: Millis) -> NoteFile {
    let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
    let page = VaultPage {
        rel_path: path.to_owned(),
        basename: name.strip_suffix(".md").unwrap_or(name).to_owned(),
        folder: folder.to_owned(),
        raw: text.to_owned(),
        parsed: vault::obsidian::obsidian_parse::parse_page(text),
        mtime: SystemTime::UNIX_EPOCH,
    };
    let mut file = derived(&page, created, modified);
    let named = FEATURE_IMAGE_PROPERTIES.iter().find_map(|key| {
        page.parsed
            .frontmatter
            .iter()
            .find(|e| e.key == *key)
            .and_then(|e| e.value.as_str().map(str::to_owned))
    });
    file.feature_image = named.or_else(|| embeds(text).next());
    file
}

/// A frontmatter value as the navigator keeps one.
fn property(v: &Value) -> PropertyValue {
    match v {
        Value::String(s) => PropertyValue::Text(s.clone()),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| PropertyValue::Text(n.to_string()), PropertyValue::Number),
        Value::Bool(b) => PropertyValue::Bool(*b),
        Value::Array(a) => PropertyValue::List(
            a.iter()
                .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_owned))
                .collect(),
        ),
        Value::Null => PropertyValue::Text(String::new()),
        Value::Object(_) => PropertyValue::Text(v.to_string()),
    }
}

/// A note's text as its preview: its words, without its frontmatter, its
/// fences, headings' and tasks' marks, or links' brackets (a link's alias
/// when it has one); embeds left out.
#[must_use]
pub fn preview_of(text: &str) -> String {
    let body = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(_, b)| b))
        .map_or(text, |b| {
            b.trim_start_matches(|c| c != '\n').trim_start_matches('\n')
        });
    let mut out = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || t.is_empty() || t.starts_with('|') {
            continue;
        }
        let t = t
            .trim_start_matches('#')
            .trim_start_matches('>')
            .trim_start();
        let t = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "]
            .iter()
            .find_map(|m| t.strip_prefix(m))
            .unwrap_or(t);
        if !out.is_empty() {
            out.push(' ');
        }
        // Emphasis and code marks are not words.
        let plain: String = unlink(t)
            .chars()
            .filter(|c| !matches!(c, '*' | '`' | '=' | '~'))
            .collect();
        out.push_str(&plain);
        if out.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let joined = words.join(" ");
    joined.chars().take(PREVIEW_CHARS).collect()
}

/// `[[a|b]]` as `b`, `[[a]]` as `a`, `![[x]]` and `![](x)` dropped,
/// `[t](u)` as `t`.
fn unlink(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(['[', '!']) {
        let (before, at) = rest.split_at_checked(i).unwrap_or((rest, ""));
        out.push_str(before);
        if let Some(inner) = at.strip_prefix("![[").and_then(|r| r.split_once("]]")) {
            rest = inner.1;
        } else if let Some((target, after)) = at.strip_prefix("[[").and_then(|r| r.split_once("]]"))
        {
            let shown = target.rsplit_once('|').map_or(target, |(_, a)| a);
            let shown = shown.split('#').next().unwrap_or(shown);
            out.push_str(shown.rsplit('/').next().unwrap_or(shown));
            rest = after;
        } else if let Some((_, after)) = at
            .strip_prefix("![")
            .and_then(|r| r.split_once("]("))
            .and_then(|(_, r)| r.split_once(')'))
        {
            rest = after;
        } else if let Some((text, after)) = at
            .strip_prefix('[')
            .and_then(|r| r.split_once("]("))
            .and_then(|(t, r)| r.split_once(')').map(|(_, a)| (t, a)))
        {
            out.push_str(text);
            rest = after;
        } else {
            let mut chars = at.chars();
            if let Some(c) = chars.next() {
                out.push(c);
            }
            rest = chars.as_str();
        }
    }
    out.push_str(rest);
    out
}

/// The note's feature image on disk: the first of
/// [`FEATURE_IMAGE_PROPERTIES`] that names an image in the vault, else the
/// first image its text embeds.
fn feature_image(v: &Vault, page: &VaultPage) -> Option<std::path::PathBuf> {
    for key in FEATURE_IMAGE_PROPERTIES {
        let value = page
            .parsed
            .frontmatter
            .iter()
            .find(|e| e.key == *key)
            .and_then(|e| match &e.value {
                Value::String(s) => Some(s.clone()),
                Value::Array(a) => a.first().and_then(Value::as_str).map(str::to_owned),
                _ => None,
            });
        if let Some(found) = value.as_deref().and_then(|s| resolve_image(v, page, s)) {
            return Some(found);
        }
    }
    embeds(&page.raw).find_map(|target| resolve_image(v, page, &target))
}

/// The targets of a text's image embeds: `![[x.png]]`, `![](x.png)`.
fn embeds(text: &str) -> impl Iterator<Item = String> + '_ {
    let wiki = text.match_indices("![[").filter_map(move |(i, _)| {
        let rest = text.get(i.saturating_add(3)..)?;
        rest.split_once("]]").map(|(t, _)| t.to_owned())
    });
    let md = text.match_indices("![").filter_map(move |(i, _)| {
        let rest = text.get(i.saturating_add(2)..)?;
        let (_, after) = rest.split_once("](")?;
        after.split_once(')').map(|(t, _)| t.to_owned())
    });
    wiki.chain(md)
}

/// An image a property or embed names (`[[Attachments/x.png|alt]]`,
/// `x.png`, `Attachments/x.png`, a path beside the note), on disk.
fn resolve_image(v: &Vault, page: &VaultPage, raw: &str) -> Option<std::path::PathBuf> {
    let t = raw.trim().trim_start_matches('!');
    let t = t
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(t);
    let t = t.split('|').next().unwrap_or(t).trim();
    if t.is_empty() || t.contains("://") {
        return None;
    }
    let ext = t.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    if !IMAGES.contains(&ext.as_str()) {
        return None;
    }
    let beside = if page.folder.is_empty() {
        t.to_owned()
    } else {
        format!("{}/{t}", page.folder)
    };
    for candidate in [t.to_owned(), beside] {
        let abs = v.root.join(&candidate);
        if abs.is_file() {
            return Some(abs);
        }
    }
    let name = t.rsplit('/').next().unwrap_or(t);
    v.attachments
        .iter()
        .find(|a| a.rel_path.rsplit('/').next() == Some(name))
        .map(|a| v.root.join(&a.rel_path))
}

/// A file the view asks for by its vault path (`Attachments/x.png`), or by
/// a name an embed gives (`x.png`, `[[x.png|alt]]`): on disk, when it is.
fn resolve_resource(v: &Vault, raw: &str) -> Option<std::path::PathBuf> {
    let t = raw.trim().trim_start_matches('!');
    let t = t
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(t);
    let t = t.split('|').next().unwrap_or(t).trim();
    if t.is_empty() || t.contains("://") || t.split('/').any(|s| s == "..") {
        return None;
    }
    let abs = v.root.join(t);
    if abs.is_file() {
        return Some(abs);
    }
    let name = t.rsplit('/').next().unwrap_or(t);
    v.attachments
        .iter()
        .find(|a| a.rel_path.rsplit('/').next() == Some(name))
        .map(|a| v.root.join(&a.rel_path))
}

/// A property's value as frontmatter holds it.
fn json_of(value: &PropertyValue) -> Value {
    match value {
        // The key alone: `key:` (`null`).
        PropertyValue::Text(t) if t.is_empty() => Value::Null,
        PropertyValue::Text(t) => Value::String(t.clone()),
        PropertyValue::Number(n) => {
            serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number)
        }
        PropertyValue::Bool(b) => Value::Bool(*b),
        PropertyValue::List(l) => Value::Array(l.iter().cloned().map(Value::String).collect()),
    }
}

/// A file on disk as a `file:` URL, its path's spaces and the like
/// escaped.
fn file_url(abs: &Path) -> String {
    let mut out = String::from("file://");
    for c in abs.to_string_lossy().chars() {
        match c {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '%' => out.push_str("%25"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{preview_of, unlink};

    #[test]
    fn a_preview_is_the_notes_words() {
        let text = "---\ntitle: X\ntags: [a]\n---\n# Heading\nSee [[Other note|the other]] and **[[Plain]]**.\n\
                    ![[img.png]]\n```kf\n| C | G |\n```\n- [ ] A task\n> Quoted";
        assert_eq!(
            preview_of(text),
            "Heading See the other and Plain. A task Quoted"
        );
        assert_eq!(unlink("a [link](http://x) b ![i](x.png) c"), "a link b  c");
    }
}
