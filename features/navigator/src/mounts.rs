//! Several sources as one: every vault the person can reach, and the
//! providers, in one navigator (#443).
//!
//! Each source is a [`Mount`] under its own first path segment, its id:
//! an org's vault is `@<org>` ([`Mount::vault`]), a provider is
//! `~<provider>` ([`Mount::provider`]). `@codywright/Bible Study/Psalms.md`
//! is the note `Bible Study/Psalms.md` of codywright's vault. [`Mounts`]
//! is a [`NavigatorSource`] over them: it lists every mount's folders and
//! files under its id, sends their changes on with the id put in front,
//! and hands a write, a read or a URL to the mount the path is in.
//!
//! Org = vault: links resolve within a vault, and a move never leaves one
//! (moving into another org's vault changes who owns the note, so it is
//! refused here; an ownership transfer is its own, confirmed, step).
//! Tag and property-key operations, which name no note, go to every vault
//! that can write.
//!
//! The navigator reads the mounts from [`NavigatorSource::mounts`] and
//! draws the left list from them (`NavigatorState::nav_rows`): the vaults'
//! top-level folders in one flat list under Vaults/, each with its org's
//! [`OrgBadge`].

use std::sync::{Arc, RwLock};

use crate::source::{FileOp, Listener, NavigatorSource, NoteFile, SourceEvent, Subscription};

/// What a mount is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountKind {
    /// An org's vault: its folders are browsed, its notes' tags and
    /// properties fill Tags/ and Properties.
    Vault,
    /// A provider's containers and items (`architect-navigator-api`).
    Provider,
}

/// Where a mount shows in the left list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Its top-level folders, flat with every other vault's, under
    /// Vaults/.
    Vaults,
    /// A row of its own at the top, with no header (Inbox, Calendar,
    /// Projects); its containers nest under it.
    Top,
    /// Its containers, flat with the other assets mounts', under Assets/.
    Assets,
    /// A folding section of its own, its title the header, its containers
    /// under it.
    Section,
    /// Not in the list (a provider turned off in it); its notes still open.
    Hidden,
}

/// Where a vault comes from, as a row shows it: the org's mark.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrgBadge {
    /// The org's id (`codywright`).
    pub org: String,
    /// Its name (`Cody Wright`): the badge's tooltip.
    pub name: String,
    /// The org's colour, a CSS colour (`#047857`): its rows are lightly
    /// washed in it ([`OrgBadge::tint`]).
    pub colour: String,
    /// Its own icon (a Lucide name, an emoji), drawn instead of the
    /// initial.
    pub icon: Option<String>,
}

impl OrgBadge {
    #[must_use]
    pub fn new(org: impl Into<String>, name: impl Into<String>, colour: impl Into<String>) -> Self {
        Self {
            org: org.into(),
            name: name.into(),
            colour: colour.into(),
            icon: None,
        }
    }

    /// The light wash its rows take: its colour mixed into transparent
    /// at the theme's `navigator.org-wash` (`--nv-org-wash`, 8% unless a
    /// theme says; 0 turns it off), so the name reads as on any row.
    #[must_use]
    pub fn tint(&self) -> String {
        format!(
            "color-mix(in srgb, {} var(--nv-org-wash, 8%), transparent)",
            self.colour.trim()
        )
    }

    /// The letter a badge without an icon shows: the name's first.
    #[must_use]
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
}

/// One source in [`Mounts`], as the left list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Its first path segment: `@<org>` or `~<provider>`.
    pub id: String,
    /// Its row's or its section's name.
    pub title: String,
    /// A Lucide icon's name for its row (a provider's manifest `icon`).
    pub icon: Option<String>,
    pub kind: MountKind,
    pub place: Place,
    /// The org it is in: a vault's, drawn as its rows' badge.
    pub org: Option<OrgBadge>,
    /// A number on its row (the Queue's captures still to process).
    pub count: Option<usize>,
    /// A top row's containers nest under it (Projects' projects); off,
    /// its row only selects it and its containers are the list's groups
    /// (the Queue's Inbox, Today, Upcoming, Waiting).
    pub nests: bool,
    /// How a folder of it lists, unless a person set its own (the Queue:
    /// with what is below, grouped by folder).
    pub list: Option<crate::Appearance>,
    /// Its folders' lists grouped by this property, its groups in this
    /// order (a project's `section`: Songs, Notes, Sub-projects, Tasks).
    pub group_by: Option<(String, Vec<String>)>,
    /// A month grid under its row (a provider's `Snapshot::calendar`,
    /// #444): its days, each the folder a click selects.
    pub calendar: Option<SectionCalendar>,
}

/// A day of a mount's month grid ([`Mount::calendar`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SectionDay {
    /// The folder a click on it selects (`~calendar:calendar/2026/…`).
    pub folder: String,
    /// How many things are on it: its dots.
    pub count: u32,
    /// Its word (the day's template) and the mark's colour.
    pub label: Option<String>,
    pub colour: Option<String>,
}

/// A mount's month grid: its days by date.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SectionCalendar {
    pub days: std::collections::BTreeMap<chrono::NaiveDate, SectionDay>,
}

impl Mount {
    /// The vault of org `badge.org`, under `@<org>`, in Vaults/.
    #[must_use]
    pub fn vault(badge: OrgBadge) -> Self {
        Self {
            id: format!("@{}", badge.org),
            title: badge.name.clone(),
            icon: None,
            kind: MountKind::Vault,
            place: Place::Vaults,
            org: Some(badge),
            count: None,
            nests: true,
            list: None,
            group_by: None,
            calendar: None,
        }
    }

    /// Provider `key` (`tasks/queue`), under `~<key>`, shown at `place`.
    #[must_use]
    pub fn provider(key: &str, title: impl Into<String>, place: Place) -> Self {
        Self {
            id: format!("~{}", key.replace('/', ":")),
            title: title.into(),
            icon: None,
            kind: MountKind::Provider,
            place,
            org: None,
            count: None,
            nests: true,
            list: None,
            group_by: None,
            calendar: None,
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Its containers do not nest under its row ([`Self::nests`]).
    #[must_use]
    pub const fn flat(mut self) -> Self {
        self.nests = false;
        self
    }

    /// Its folders' lists grouped by property `key`, `order` first.
    #[must_use]
    pub fn grouped_by(mut self, key: impl Into<String>, order: Vec<String>) -> Self {
        self.group_by = Some((key.into(), order));
        self
    }

    /// Its folders list as `look` does (its `target` is not read).
    #[must_use]
    pub fn listed(mut self, look: crate::Appearance) -> Self {
        self.list = Some(look);
        self
    }

    #[must_use]
    pub fn org(mut self, badge: OrgBadge) -> Self {
        self.org = Some(badge);
        self
    }

    /// `rest` within this mount: `<id>/<rest>` (`<id>` for an empty one).
    #[must_use]
    pub fn path(&self, rest: &str) -> String {
        join(&self.id, rest)
    }

    /// Whether it is an org's vault.
    #[must_use]
    pub fn is_vault(&self) -> bool {
        self.kind == MountKind::Vault
    }
}

fn join(id: &str, rest: &str) -> String {
    if rest.is_empty() {
        id.to_owned()
    } else {
        format!("{id}/{rest}")
    }
}

/// The mount id a path is in and the rest of it (`@a/B/c.md` is
/// `("@a", "B/c.md")`); `None` for a path in no mount (`@`/`~` first).
#[must_use]
pub fn split(path: &str) -> Option<(&str, &str)> {
    if !(path.starts_with('@') || path.starts_with('~')) {
        return None;
    }
    Some(path.split_once('/').unwrap_or((path, "")))
}

type Entry = (Mount, Arc<dyn NavigatorSource>);

/// Several sources as one, each under its [`Mount`]'s id.
#[derive(Clone, Default)]
pub struct Mounts {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    name: RwLock<String>,
    entries: RwLock<Vec<Entry>>,
    /// Who listens, each with what it listens to in every mount (dropped
    /// with the mounts when they are set again).
    listeners: RwLock<Vec<Listening>>,
}

struct Listening {
    id: u64,
    listener: Arc<Listener>,
    subs: Vec<Subscription>,
}

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl std::fmt::Debug for Mounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mounts")
            .field("mounts", &self.mounts_now())
            .finish()
    }
}

/// `file` of mount `id`, as the whole lists it: its path, and its feature
/// image when that is a vault path, under the mount.
fn mounted(id: &str, mut file: NoteFile) -> NoteFile {
    file.path = join(id, &file.path);
    if let Some(img) = file.feature_image.as_mut()
        && !img.contains("://")
        && !img.is_empty()
    {
        *img = join(id, img);
    }
    file
}

fn mounted_event(id: &str, e: &SourceEvent) -> SourceEvent {
    use SourceEvent as E;
    match e {
        E::FolderCreated(p) => E::FolderCreated(join(id, p)),
        E::FolderDeleted(p) => E::FolderDeleted(join(id, p)),
        E::FolderRenamed { from, to } => E::FolderRenamed {
            from: join(id, from),
            to: join(id, to),
        },
        E::FileCreated(f) => E::FileCreated(mounted(id, f.clone())),
        E::FileModified(f) => E::FileModified(mounted(id, f.clone())),
        E::FileDeleted(p) => E::FileDeleted(join(id, p)),
        E::FileRenamed { from, file } => E::FileRenamed {
            from: join(id, from),
            file: mounted(id, file.clone()),
        },
        E::Reset => E::Reset,
    }
}

impl Mounts {
    /// None yet, named `name` (the root's name, never shown in the list).
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let m = Self::default();
        *write(&m.inner.name) = name.into();
        m
    }

    /// With `source` mounted as `mount`.
    #[must_use]
    pub fn with(self, mount: Mount, source: Arc<dyn NavigatorSource>) -> Self {
        self.mount(mount, source);
        self
    }

    /// Mounts `source` as `mount` (in its place when one has its id), and
    /// tells every listener to read everything again.
    pub fn mount(&self, mount: Mount, source: Arc<dyn NavigatorSource>) {
        {
            let mut entries = write(&self.inner.entries);
            if let Some(e) = entries.iter_mut().find(|(m, _)| m.id == mount.id) {
                *e = (mount, source);
            } else {
                entries.push((mount, source));
            }
        }
        self.relisten();
    }

    /// Changes how mount `id` shows (its count, its place); a no-op for
    /// an id not mounted. Says so to the listeners when it changed.
    pub fn describe(&self, id: &str, f: impl FnOnce(&mut Mount)) {
        let mut entries = write(&self.inner.entries);
        let Some((m, _)) = entries.iter_mut().find(|(m, _)| m.id == id) else {
            return;
        };
        let before = m.clone();
        f(m);
        let changed = *m != before;
        drop(entries);
        if changed {
            self.tell(&SourceEvent::Reset);
        }
    }

    /// Takes mount `id` away.
    pub fn unmount(&self, id: &str) {
        write(&self.inner.entries).retain(|(m, _)| m.id != id);
        self.relisten();
    }

    /// The mounts, in order.
    #[must_use]
    pub fn mounts_now(&self) -> Vec<Mount> {
        read(&self.inner.entries)
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// Mount `id`'s source.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<Arc<dyn NavigatorSource>> {
        read(&self.inner.entries)
            .iter()
            .find(|(m, _)| m.id == id)
            .map(|(_, s)| Arc::clone(s))
    }

    /// The mount `path` is in, its source, and the path within it.
    #[must_use]
    pub fn resolve(&self, path: &str) -> Option<(Mount, Arc<dyn NavigatorSource>, String)> {
        let (id, rest) = split(path)?;
        read(&self.inner.entries)
            .iter()
            .find(|(m, _)| m.id == id)
            .map(|(m, s)| (m.clone(), Arc::clone(s), rest.to_owned()))
    }

    fn tell(&self, e: &SourceEvent) {
        let listeners: Vec<Arc<Listener>> = read(&self.inner.listeners)
            .iter()
            .map(|l| Arc::clone(&l.listener))
            .collect();
        for l in listeners {
            l(e);
        }
    }

    /// Every listener listens to the mounts as they are now, then reads
    /// everything again.
    fn relisten(&self) {
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        {
            let mut listeners = write(&self.inner.listeners);
            for l in listeners.iter_mut() {
                l.subs = subscribe_all(&entries, &l.listener);
            }
        }
        self.tell(&SourceEvent::Reset);
    }

    /// The mount every path of `op` is in, and `op` within it; `Err` when
    /// they are in two (a move to another org's vault).
    fn route(op: &FileOp) -> Result<Option<(String, FileOp)>, String> {
        use FileOp as O;
        let one = |p: &str| -> Result<(String, String), String> {
            split(p)
                .map(|(id, rest)| (id.to_owned(), rest.to_owned()))
                .ok_or_else(|| format!("{p} is in no vault"))
        };
        let two = |from: &str, to: &str| -> Result<(String, String, String), String> {
            let (a, from) = one(from)?;
            let (b, to) = one(to)?;
            if a == b {
                Ok((a, from, to))
            } else {
                Err("moving to another org's vault changes who owns it: \
                     move it within its vault, or transfer it"
                    .to_owned())
            }
        };
        let many = |paths: &[String]| -> Result<(String, Vec<String>), String> {
            let mut id = None::<String>;
            let mut out = Vec::new();
            for p in paths {
                let (m, rest) = one(p)?;
                if id.as_ref().is_some_and(|i| *i != m) {
                    return Err("the notes are in more than one vault: do one at a time".to_owned());
                }
                id = Some(m);
                out.push(rest);
            }
            id.map(|i| (i, out)).ok_or_else(|| "no notes".to_owned())
        };
        Ok(Some(match op {
            O::CreateFolder(p) => {
                let (id, p) = one(p)?;
                (id, O::CreateFolder(p))
            }
            O::CreateFile { path, text } => {
                let (id, path) = one(path)?;
                (
                    id,
                    O::CreateFile {
                        path,
                        text: text.clone(),
                    },
                )
            }
            O::Move { from, to } => {
                let (id, from, to) = two(from, to)?;
                (id, O::Move { from, to })
            }
            O::Copy { from, to } => {
                let (id, from, to) = two(from, to)?;
                (id, O::Copy { from, to })
            }
            O::MoveFolder { from, to } => {
                let (id, from, to) = two(from, to)?;
                (id, O::MoveFolder { from, to })
            }
            O::CopyFolder { from, to } => {
                let (id, from, to) = two(from, to)?;
                (id, O::CopyFolder { from, to })
            }
            O::SetProperty { path, key, value } => {
                let (id, path) = one(path)?;
                (
                    id,
                    O::SetProperty {
                        path,
                        key: key.clone(),
                        value: value.clone(),
                    },
                )
            }
            O::RemoveProperty { path, key } => {
                let (id, path) = one(path)?;
                (
                    id,
                    O::RemoveProperty {
                        path,
                        key: key.clone(),
                    },
                )
            }
            O::AddTag { paths, tag } => {
                let (id, paths) = many(paths)?;
                (
                    id,
                    O::AddTag {
                        paths,
                        tag: tag.clone(),
                    },
                )
            }
            O::RemoveTag { paths, tag } => {
                let (id, paths) = many(paths)?;
                (
                    id,
                    O::RemoveTag {
                        paths,
                        tag: tag.clone(),
                    },
                )
            }
            O::ClearTags { paths } => {
                let (id, paths) = many(paths)?;
                (id, O::ClearTags { paths })
            }
            O::Trash(p) => {
                let (id, p) = one(p)?;
                (id, O::Trash(p))
            }
            // Vault-wide: every vault that writes does it.
            O::RenameTag { .. }
            | O::DeleteTag(_)
            | O::RenamePropertyKey { .. }
            | O::DeletePropertyKey(_)
            | O::RenamePropertyValue { .. } => return Ok(None),
        }))
    }
}

fn subscribe_all(entries: &[Entry], listener: &Arc<Listener>) -> Vec<Subscription> {
    entries
        .iter()
        .map(|(m, s)| {
            let id = m.id.clone();
            let l = Arc::clone(listener);
            s.subscribe(Box::new(move |e: &SourceEvent| l(&mounted_event(&id, e))))
        })
        .collect()
}

impl NavigatorSource for Mounts {
    fn name(&self) -> String {
        read(&self.inner.name).clone()
    }

    fn folders(&self) -> Vec<String> {
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        let mut out = Vec::new();
        for (m, s) in &entries {
            out.push(m.id.clone());
            out.extend(s.folders().into_iter().map(|f| join(&m.id, &f)));
        }
        out
    }

    fn child_order(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        let mut out = std::collections::BTreeMap::new();
        // The root's children: the mounts, as they were mounted.
        out.insert(
            String::new(),
            entries.iter().map(|(m, _)| m.id.clone()).collect(),
        );
        for (m, s) in &entries {
            for (parent, names) in s.child_order() {
                out.insert(join(&m.id, &parent), names);
            }
        }
        out
    }

    fn files(&self) -> Vec<NoteFile> {
        read(&self.inner.entries)
            .iter()
            .flat_map(|(m, s)| {
                let id = m.id.clone();
                s.files().into_iter().map(move |f| mounted(&id, f))
            })
            .collect()
    }

    fn mounts(&self) -> Vec<Mount> {
        self.mounts_now()
    }

    fn folder_meta(&self) -> std::collections::BTreeMap<String, crate::source::FolderMeta> {
        read(&self.inner.entries)
            .iter()
            .flat_map(|(m, s)| {
                let id = m.id.clone();
                s.folder_meta()
                    .into_iter()
                    .map(move |(p, o)| (join(&id, &p), o))
            })
            .collect()
    }

    fn subscribe(&self, listener: Listener) -> Subscription {
        let listener = Arc::new(listener);
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        write(&self.inner.listeners).push(Listening {
            id,
            subs: subscribe_all(&entries, &listener),
            listener,
        });
        let inner = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = inner.upgrade() {
                write(&inner.listeners).retain(|l| l.id != id);
            }
        })
    }

    fn resource_url(&self, path: &str) -> Option<String> {
        let (_, s, rest) = self.resolve(path)?;
        s.resource_url(&rest)
    }

    fn can_write(&self) -> bool {
        read(&self.inner.entries).iter().any(|(_, s)| s.can_write())
    }

    fn write(&self, ops: &[FileOp]) -> Result<(), String> {
        // Routed first, so a refused op writes nothing.
        let mut routed: Vec<(String, FileOp)> = Vec::new();
        let mut wide: Vec<FileOp> = Vec::new();
        for op in ops {
            match Self::route(op)? {
                Some(r) => routed.push(r),
                None => wide.push(op.clone()),
            }
        }
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        for (id, op) in routed {
            let (_, s) = entries
                .iter()
                .find(|(m, _)| m.id == id)
                .ok_or_else(|| format!("{id} is not mounted"))?;
            s.write(std::slice::from_ref(&op))?;
        }
        if !wide.is_empty() {
            for (m, s) in &entries {
                if m.is_vault() && s.can_write() {
                    s.write(&wide)?;
                }
            }
        }
        Ok(())
    }

    fn search_text(&self, query: &str) -> Option<Vec<String>> {
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        let mut any = false;
        let mut out = Vec::new();
        for (m, s) in &entries {
            if let Some(paths) = s.search_text(query) {
                any = true;
                out.extend(paths.into_iter().map(|p| join(&m.id, &p)));
            }
        }
        any.then_some(out)
    }

    fn search_excerpts(&self, query: &str) -> Option<Vec<(String, String)>> {
        let entries: Vec<Entry> = read(&self.inner.entries).clone();
        let mut any = false;
        let mut out = Vec::new();
        for (m, s) in &entries {
            if let Some(hits) = s.search_excerpts(query) {
                any = true;
                out.extend(hits.into_iter().map(|(p, e)| (join(&m.id, &p), e)));
            }
        }
        any.then_some(out)
    }

    fn read_text(&self, path: &str) -> Option<String> {
        let (_, s, rest) = self.resolve(path)?;
        s.read_text(&rest)
    }
}
