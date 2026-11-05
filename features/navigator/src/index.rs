//! The vault as the navigator holds it: folders with their child folders
//! and files, and each folder's note counts, kept current from a source's
//! [`SourceEvent`]s.
//!
//! Notebook Navigator walks Obsidian's `TFolder` tree and caches derived
//! data per path (`docs/storage-architecture.md`); counts are
//! `NoteCountInfo { current, descendants, total }`
//! (`src/utils/noteCountUtils.ts`).

use std::collections::{BTreeSet, HashMap};

use crate::source::{
    NavigatorSource, NoteFile, SourceEvent, ancestors, file_name, is_within, parent,
};

/// How many notes a folder holds: in itself, and in its folders below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteCount {
    pub current: usize,
    pub descendants: usize,
}

impl NoteCount {
    #[must_use]
    pub const fn total(self) -> usize {
        self.current.saturating_add(self.descendants)
    }
}

/// One folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folder {
    /// The paths of the folders directly in it.
    pub folders: BTreeSet<String>,
    /// The paths of the files directly in it.
    pub files: BTreeSet<String>,
    pub count: NoteCount,
}

/// The vault: every folder (the root is `""`) and every file.
#[derive(Debug, Clone, Default)]
pub struct VaultIndex {
    name: String,
    folders: HashMap<String, Folder>,
    files: HashMap<String, NoteFile>,
    /// Bumped by every change: what a view's memo keys on.
    revision: u64,
}

impl VaultIndex {
    /// Everything `source` holds now.
    #[must_use]
    pub fn read(source: &dyn NavigatorSource) -> Self {
        Self::from_parts(source.name(), source.folders(), source.files())
    }

    /// A vault of these folders and files.
    #[must_use]
    pub fn from_parts(
        name: String,
        folders: impl IntoIterator<Item = String>,
        files: impl IntoIterator<Item = NoteFile>,
    ) -> Self {
        let mut index = Self {
            name,
            ..Self::default()
        };
        index.folders.insert(String::new(), Folder::default());
        for f in folders {
            index.add_folder(&f);
        }
        for f in files {
            index.add_file(f);
        }
        index.recount();
        index
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn folder(&self, path: &str) -> Option<&Folder> {
        self.folders.get(path)
    }

    #[must_use]
    pub fn file(&self, path: &str) -> Option<&NoteFile> {
        self.files.get(path)
    }

    /// Every file, in no order.
    pub fn files(&self) -> impl Iterator<Item = &NoteFile> {
        self.files.values()
    }

    #[must_use]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Every folder path, the root (`""`) included.
    pub fn folder_paths(&self) -> impl Iterator<Item = &str> {
        self.folders.keys().map(String::as_str)
    }

    /// The files directly in `folder`, or in it and every folder below
    /// when `descendants`.
    #[must_use]
    pub fn files_in(&self, folder: &str, descendants: bool) -> Vec<&NoteFile> {
        let mut out = Vec::new();
        self.collect(folder, descendants, &mut out);
        out
    }

    fn collect<'a>(&'a self, folder: &str, descendants: bool, out: &mut Vec<&'a NoteFile>) {
        let Some(f) = self.folders.get(folder) else {
            return;
        };
        out.extend(f.files.iter().filter_map(|p| self.files.get(p)));
        if descendants {
            for child in &f.folders {
                self.collect(child, true, out);
            }
        }
    }

    fn add_folder(&mut self, path: &str) {
        if path.is_empty() || self.folders.contains_key(path) {
            return;
        }
        let up = parent(path).to_owned();
        self.add_folder(&up);
        self.folders.insert(path.to_owned(), Folder::default());
        if let Some(p) = self.folders.get_mut(&up) {
            p.folders.insert(path.to_owned());
        }
    }

    fn add_file(&mut self, file: NoteFile) {
        let folder = parent(&file.path).to_owned();
        self.add_folder(&folder);
        if let Some(f) = self.folders.get_mut(&folder) {
            f.files.insert(file.path.clone());
        }
        self.files.insert(file.path.clone(), file);
    }

    fn remove_file(&mut self, path: &str) -> Option<NoteFile> {
        let file = self.files.remove(path)?;
        if let Some(f) = self.folders.get_mut(parent(path)) {
            f.files.remove(path);
        }
        Some(file)
    }

    fn remove_folder(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        self.folders.retain(|p, _| !is_within(p, path));
        self.files.retain(|p, _| !is_within(p, path));
        if let Some(up) = self.folders.get_mut(parent(path)) {
            up.folders.remove(path);
        }
    }

    /// Takes in one change. Returns the paths that moved (`from`, `to`),
    /// for the selection and the expanded folders to follow.
    pub fn apply(&mut self, event: &SourceEvent) -> Vec<(String, String)> {
        let mut moved = Vec::new();
        match event {
            SourceEvent::FolderCreated(p) => self.add_folder(p),
            SourceEvent::FolderDeleted(p) => self.remove_folder(p),
            SourceEvent::FolderRenamed { from, to } => {
                let files: Vec<NoteFile> = self
                    .files
                    .values()
                    .filter(|f| is_within(&f.path, from))
                    .cloned()
                    .collect();
                let folders: Vec<String> = self
                    .folders
                    .keys()
                    .filter(|p| is_within(p, from))
                    .cloned()
                    .collect();
                self.remove_folder(from);
                for f in folders {
                    let new = rebase(&f, from, to);
                    self.add_folder(&new);
                    moved.push((f, new));
                }
                for mut f in files {
                    let new = rebase(&f.path, from, to);
                    moved.push((std::mem::replace(&mut f.path, new), f.path.clone()));
                    self.add_file(f);
                }
            }
            SourceEvent::FileCreated(f) | SourceEvent::FileModified(f) => {
                self.remove_file(&f.path);
                self.add_file(f.clone());
            }
            SourceEvent::FileDeleted(p) => {
                self.remove_file(p);
            }
            SourceEvent::FileRenamed { from, file } => {
                self.remove_file(from);
                self.add_file(file.clone());
                moved.push((from.clone(), file.path.clone()));
            }
            SourceEvent::Reset => {}
        }
        self.recount();
        moved
    }

    /// Every folder's counts again (bottom up: deepest paths first).
    fn recount(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let mut paths: Vec<String> = self.folders.keys().cloned().collect();
        paths.sort_by_key(|p| std::cmp::Reverse(depth(p)));
        let mut below: HashMap<String, usize> = HashMap::new();
        for p in paths {
            let Some(f) = self.folders.get_mut(&p) else {
                continue;
            };
            let current = f.files.len();
            let descendants = below.remove(&p).unwrap_or(0);
            f.count = NoteCount {
                current,
                descendants,
            };
            if !p.is_empty() {
                let total = current.saturating_add(descendants);
                let up = below.entry(parent(&p).to_owned()).or_insert(0);
                *up = up.saturating_add(total);
            }
        }
    }

    /// The folder's display name: the vault's for the root.
    #[must_use]
    pub fn folder_name<'a>(&'a self, path: &'a str) -> &'a str {
        if path.is_empty() {
            &self.name
        } else {
            file_name(path)
        }
    }

    /// Whether `folder` has folders in it.
    #[must_use]
    pub fn has_child_folders(&self, folder: &str) -> bool {
        self.folders
            .get(folder)
            .is_some_and(|f| !f.folders.is_empty())
    }

    /// Every folder that holds a folder: what "expand all" opens.
    #[must_use]
    pub fn expandable(&self) -> Vec<String> {
        self.folders
            .iter()
            .filter(|(_, f)| !f.folders.is_empty())
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Whether every folder of `path`'s chain exists.
    #[must_use]
    pub fn has_folder(&self, path: &str) -> bool {
        self.folders.contains_key(path)
    }

    /// `path`'s folders up to the root, nearest first.
    #[must_use]
    pub fn chain(path: &str) -> Vec<String> {
        ancestors(path).map(str::to_owned).collect()
    }
}

/// How deep a folder is: the root 0, `A` 1, `A/B` 2.
#[must_use]
pub fn depth(path: &str) -> usize {
    if path.is_empty() {
        0
    } else {
        path.matches('/').count().saturating_add(1)
    }
}

/// `path`, from inside `from` to inside `to`.
fn rebase(path: &str, from: &str, to: &str) -> String {
    path.strip_prefix(from)
        .map_or_else(|| path.to_owned(), |rest| format!("{to}{rest}"))
}
