//! A provider as a source: [`ProviderSource`] draws any
//! `architect_navigator_api::NavigatorProvider`'s containers and items
//! with the navigator built for folders and notes.
//!
//! The provider API (`architect-navigator-api`) is MIT OR Apache-2.0, so a
//! page or an extension implements it without linking this GPL crate. Here
//! its [`Snapshot`] becomes folders and files: a [`Node`] is a folder (its
//! path its ancestors' titles), an [`Item`] a note in its container's
//! folder named by its title. Everything the navigator does for a vault
//! then works the same for every provider: the dual or single pane, the
//! virtual scrolling, the keys, selection, grouping, sorting, pins, the
//! search, shortcuts and profiles.
//!
//! What it is asked to write goes back to the provider as a
//! [`ProviderRequest`] (a note dragged to a folder is a move to that
//! container; a new note is an item made in it); a path the navigator
//! opens is read back as its item ([`ProviderSource::item_at`]).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use architect_navigator_api::{Item, Node, Snapshot};

use crate::source::{
    FileOp, Listener, NavigatorSource, NoteFile, PropertyValue, SourceEvent, Subscription,
    TaskCounts, parent,
};

/// What the navigator asked of a provider: the app sends it on
/// (`NavigatorProvider::move_items`, `create`) and loads the snapshot
/// again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequest {
    /// Items dragged onto container `to` (`""`: the root).
    Move { items: Vec<String>, to: String },
    /// A new item, `title`, in `container`.
    Create { container: String, title: String },
}

/// A provider's snapshot as a [`NavigatorSource`].
#[derive(Clone, Default)]
pub struct ProviderSource {
    inner: Arc<Inner>,
}

type Requests = Box<dyn Fn(ProviderRequest) + Send + Sync>;

#[derive(Default)]
struct Inner {
    map: RwLock<Mapped>,
    listeners: Mutex<Vec<(u64, Arc<dyn Fn(&SourceEvent) + Send + Sync>)>>,
    next: AtomicU64,
    requests: Mutex<Option<Requests>>,
}

/// The snapshot, and its nodes and items as paths.
#[derive(Default)]
struct Mapped {
    snapshot: Snapshot,
    loaded: bool,
    folders: Vec<String>,
    files: Vec<NoteFile>,
    /// The nodes' order by parent folder, when the snapshot keeps it.
    order: std::collections::BTreeMap<String, Vec<String>>,
    /// Node id → folder path, and back.
    folder_of: HashMap<String, String>,
    node_at: HashMap<String, String>,
    /// Item path → its index in the snapshot; item id → path.
    item_at: HashMap<String, usize>,
    path_of: HashMap<String, String>,
}

/// A title as one path segment: no `/`, never empty.
fn segment(title: &str) -> String {
    let t = title.trim().replace('/', "∕");
    if t.is_empty() {
        "Untitled".to_owned()
    } else {
        t
    }
}

/// `name` made unique in `taken` (`Name`, `Name 2`, …), taken.
fn unique(taken: &mut HashSet<String>, dir: &str, name: &str, ext: &str) -> String {
    let at = |n: &str| {
        if dir.is_empty() {
            format!("{n}{ext}")
        } else {
            format!("{dir}/{n}{ext}")
        }
    };
    let mut path = at(name);
    let mut n = 2_u32;
    while taken.contains(&path) {
        path = at(&format!("{name} {n}"));
        n = n.saturating_add(1);
    }
    taken.insert(path.clone());
    path
}

impl Mapped {
    fn of(snapshot: Snapshot) -> Self {
        let mut m = Self {
            loaded: true,
            ..Self::default()
        };
        let mut taken = HashSet::new();
        for (up, node) in snapshot.nodes() {
            let dir = up
                .and_then(|u| m.folder_of.get(u))
                .cloned()
                .unwrap_or_default();
            let path = unique(&mut taken, &dir, &segment(&node.title), "");
            m.folder_of.insert(node.id.clone(), path.clone());
            m.node_at.insert(path.clone(), node.id.clone());
            if snapshot.ordered {
                let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                m.order.entry(dir.clone()).or_default().push(name);
            }
            m.folders.push(path);
        }
        for (i, item) in snapshot.items.iter().enumerate() {
            let dir = m
                .folder_of
                .get(&item.container)
                .cloned()
                .unwrap_or_default();
            let path = unique(&mut taken, &dir, &segment(&item.title), ".md");
            m.item_at.insert(path.clone(), i);
            m.path_of.insert(item.id.clone(), path.clone());
            m.files.push(note(&path, item));
        }
        m.snapshot = snapshot;
        m
    }
}

/// An item as the note the navigator lists.
fn note(path: &str, item: &Item) -> NoteFile {
    let mut f = NoteFile::new(path, item.created, item.modified);
    f.preview.clone_from(&item.preview);
    f.tags.clone_from(&item.tags);
    f.feature_image.clone_from(&item.thumbnail);
    f.tasks = item.progress.map(|p| TaskCounts {
        total: p.total,
        unfinished: p.total.saturating_sub(p.done),
    });
    f.properties = item
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), PropertyValue::Text(v.clone())))
        .collect();
    if let Some(icon) = &item.icon {
        f.properties
            .push(("icon".to_owned(), PropertyValue::Text(icon.clone())));
    }
    if !item.badges.is_empty() {
        f.properties.push((
            "badges".to_owned(),
            PropertyValue::List(item.badges.clone()),
        ));
    }
    f
}

impl ProviderSource {
    /// A source showing nothing until [`load`](Self::load)ed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Showing `snapshot` from the start.
    #[must_use]
    pub fn with(snapshot: Snapshot) -> Self {
        let s = Self::new();
        s.load(snapshot);
        s
    }

    /// Where its requests go (the app's: to the provider).
    #[must_use]
    pub fn on_request(self, f: impl Fn(ProviderRequest) + Send + Sync + 'static) -> Self {
        *self
            .inner
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(f));
        self
    }

    fn map(&self) -> std::sync::RwLockReadGuard<'_, Mapped> {
        self.inner
            .map
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Shows `snapshot`: the navigator reads everything again. False (and
    /// nothing said) when it is what it shows already.
    pub fn load(&self, snapshot: Snapshot) -> bool {
        {
            let mut map = self
                .inner
                .map
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            if map.loaded && map.snapshot == snapshot {
                return false;
            }
            *map = Mapped::of(snapshot);
        }
        self.emit(&SourceEvent::Reset);
        true
    }

    /// Whether a snapshot was loaded.
    #[must_use]
    pub fn loaded(&self) -> bool {
        self.map().loaded
    }

    /// The snapshot it shows.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.map().snapshot.clone()
    }

    /// The item drawn at `path` (what the navigator opens).
    #[must_use]
    pub fn item_at(&self, path: &str) -> Option<Item> {
        let map = self.map();
        map.item_at
            .get(path)
            .and_then(|i| map.snapshot.items.get(*i))
            .cloned()
    }

    /// The path item `id` is drawn at.
    #[must_use]
    pub fn path_of(&self, id: &str) -> Option<String> {
        self.map().path_of.get(id).cloned()
    }

    /// The node drawn as folder `folder` (`None` for the root).
    #[must_use]
    pub fn node_at(&self, folder: &str) -> Option<Node> {
        let map = self.map();
        let id = map.node_at.get(folder)?;
        map.snapshot
            .nodes()
            .into_iter()
            .find(|(_, n)| &n.id == id)
            .map(|(_, n)| n.clone())
    }

    /// The folder node `id` is drawn as.
    #[must_use]
    pub fn folder_of(&self, id: &str) -> Option<String> {
        self.map().folder_of.get(id).cloned()
    }

    /// Container id of folder `folder`: the root's is `""`.
    fn container(&self, folder: &str) -> Option<String> {
        if folder.is_empty() {
            return Some(String::new());
        }
        self.map().node_at.get(folder).cloned()
    }

    fn request(&self, r: ProviderRequest) -> Result<(), String> {
        let requests = self
            .inner
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(send) = requests.as_ref() {
            send(r);
            return Ok(());
        }
        drop(requests);
        Err(format!("{} is read-only", self.name()))
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
}

impl NavigatorSource for ProviderSource {
    fn name(&self) -> String {
        self.map().snapshot.title.clone()
    }

    fn folders(&self) -> Vec<String> {
        self.map().folders.clone()
    }

    fn child_order(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.map().order.clone()
    }

    fn folder_meta(&self) -> std::collections::BTreeMap<String, crate::source::FolderMeta> {
        let m = self.map();
        m.snapshot
            .nodes()
            .into_iter()
            .filter(|(_, n)| n.org.is_some() || n.shared || n.icon.is_some() || n.colour.is_some())
            .filter_map(|(_, n)| {
                let path = m.folder_of.get(&n.id)?.clone();
                Some((
                    path,
                    crate::source::FolderMeta {
                        org: n.org.clone(),
                        shared: n.shared,
                        icon: n.icon.clone(),
                        colour: n.colour.clone(),
                    },
                ))
            })
            .collect()
    }

    /// An item's fields are the provider's, not a vault's properties.
    fn browses_properties(&self) -> bool {
        false
    }

    fn files(&self) -> Vec<NoteFile> {
        self.map().files.clone()
    }

    fn subscribe(&self, listener: Listener) -> Subscription {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
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

    fn resource_url(&self, path: &str) -> Option<String> {
        // A thumbnail that is a URL is drawn from itself.
        (path.starts_with("http://") || path.starts_with("https://") || path.starts_with("data:"))
            .then(|| path.to_owned())
    }

    /// A note dragged to a folder: its item moved to that container; a new
    /// note: an item made. Nothing else is a provider's to do.
    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        let mut moves: Vec<(String, Vec<String>)> = Vec::new();
        for op in ops {
            match op {
                FileOp::Move { from, to } => {
                    let item = self
                        .item_at(from)
                        .ok_or_else(|| format!("no item at {from}"))?;
                    let target = self
                        .container(parent(to))
                        .ok_or_else(|| format!("no container at {}", parent(to)))?;
                    if target == item.container {
                        return Err(format!("{} cannot be renamed here", item.title));
                    }
                    match moves.iter_mut().find(|(t, _)| *t == target) {
                        Some((_, ids)) => ids.push(item.id),
                        None => moves.push((target, vec![item.id])),
                    }
                }
                FileOp::CreateFile { path, .. } => {
                    let container = self
                        .container(parent(path))
                        .ok_or_else(|| format!("no container at {}", parent(path)))?;
                    let title = NoteFile::new(path.clone(), 0, 0).name().to_owned();
                    self.request(ProviderRequest::Create { container, title })?;
                }
                _ => return Err(format!("{} does not do that", self.name())),
            }
        }
        for (to, items) in moves {
            self.request(ProviderRequest::Move { items, to })?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for ProviderSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSource")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}
