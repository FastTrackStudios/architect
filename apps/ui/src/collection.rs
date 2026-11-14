//! A collection note's native list in the desktop and phone apps (#668):
//! a note whose frontmatter lists media items (`sets`/`songs`,
//! `sections`/`items`: `architect_media::Collection`) draws as
//! `architect_media::ui::MediaList` at its head, as the fallback of the
//! embed its type would be claimed by: an extension that claims the type
//! draws instead (`architect_extensions::embeds::EmbedView`).
//!
//! Its rows come from the vault on disk: each link resolved among the
//! vault's notes, each item's media and cover among its files, read again
//! when the note's text or the vault changes. Covers are handed to the
//! list (and to the Now Playing card) as `data:` URLs read from the vault
//! ([`art_url`]), only for an image file inside it. The list is the app's
//! own source on the playhead, [`SOURCE`].

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use architect_media::{Collection, Item, Row, front};
use architect_navigator::VaultSource;
use dioxus::prelude::*;

/// The app's own source for a collection's list: outside the
/// extension-id grammar (`:`), so no extension can hold it.
pub const SOURCE: &str = "app:list";

/// The largest cover handed over as a `data:` URL.
const ART_MAX: u64 = 4 * 1024 * 1024;
/// The most covers kept read, and the most bytes of `data:` URLs.
const ART_KEPT: usize = 64;
const ART_KEPT_BYTES: usize = 32 * 1024 * 1024;
/// How old a cover's modification time must be before its read is kept:
/// past any file system's clock grain, so a rewrite of the same size
/// within it is never mistaken for the file read.
const ART_SETTLED: Duration = Duration::from_secs(2);

/// The list collection note `path` (its text `text`) makes in `vault`:
/// its sections and rows. `None` when it is no collection.
#[must_use]
pub fn rows(
    vault: &vault::actions::OpenVault,
    path: &str,
    text: &str,
) -> Option<Vec<(Option<String>, Vec<Row>)>> {
    let list = Collection::read(&front::fields(text))?;
    vault.with(|v| {
        let notes: Vec<&str> = v.pages.iter().map(|p| p.rel_path.as_str()).collect();
        let files: Vec<&str> = v.attachments.iter().map(|a| a.rel_path.as_str()).collect();
        let pages: HashMap<&str, &str> = v
            .pages
            .iter()
            .map(|p| (p.rel_path.as_str(), p.raw.as_str()))
            .collect();
        let resolved = list.resolve(path, notes.iter().copied());
        Some(resolved.rows(|p| {
            let raw = pages.get(p)?;
            Some(Row::of(
                Item::read(p, &front::fields(raw)),
                files.iter().copied(),
            ))
        }))
    })
}

/// A cover read: the file as it was (its modification time and size) and
/// its `data:` URL.
struct Art {
    modified: Option<SystemTime>,
    len: u64,
    url: String,
}

/// Covers read so far, the least recently used dropped first: a list draws
/// ten times a second while it plays. A file that changed is read again;
/// a miss is never kept (the file may come).
#[derive(Default)]
struct ArtCache {
    kept: HashMap<PathBuf, Art>,
    /// Least recently used first.
    order: VecDeque<PathBuf>,
    bytes: usize,
}

impl ArtCache {
    fn get(&mut self, key: &Path, modified: Option<SystemTime>, len: u64) -> Option<String> {
        let hit = self
            .kept
            .get(key)
            .filter(|a| a.modified == modified && a.len == len)?
            .url
            .clone();
        self.touch(key);
        Some(hit)
    }

    fn touch(&mut self, key: &Path) {
        if let Some(at) = self.order.iter().position(|k| k == key) {
            if let Some(k) = self.order.remove(at) {
                self.order.push_back(k);
            }
        }
    }

    fn forget(&mut self, key: &Path) {
        if let Some(old) = self.kept.remove(key) {
            self.bytes = self.bytes.saturating_sub(old.url.len());
            self.order.retain(|k| k != key);
        }
    }

    fn put(&mut self, key: PathBuf, art: Art) {
        if let Some(old) = self.kept.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.url.len());
            self.order.retain(|k| *k != key);
        }
        self.bytes = self.bytes.saturating_add(art.url.len());
        self.kept.insert(key.clone(), art);
        self.order.push_back(key);
        while self.kept.len() > ART_KEPT || self.bytes > ART_KEPT_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.kept.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(old.url.len());
            }
        }
    }
}

thread_local! {
    static ART: RefCell<ArtCache> = RefCell::new(ArtCache::default());
}

/// The image at vault path `path` in the vault at `root`, as a `data:`
/// URL: only a vault path (`architect_playhead::model::vault_path`, no
/// `.` or `..` segment) to an image file of the vault's own, opened
/// without following a link out of it (`vault::obsidian::open_own`), at
/// most [`ART_MAX`]. Read again once the file changes.
#[must_use]
pub fn art_url(root: &Path, path: &str) -> Option<String> {
    use base64::Engine as _;
    use std::io::Read as _;
    if !architect_playhead::model::vault_path(path)
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return None;
    }
    let mime = match path.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    let file = vault::obsidian::open_own(root, path).ok()?;
    // What was opened, not what the path names now.
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > ART_MAX {
        return None;
    }
    let (modified, len) = (meta.modified().ok(), meta.len());
    let key = root.join(path);
    // A file changed within the clock's grain of now may change again
    // with the same time and size: read it, keep nothing of it (as git's
    // racy index does), so a cover rewritten in place never shows stale.
    let settled = modified.is_some_and(|m| {
        SystemTime::now()
            .duration_since(m)
            .is_ok_and(|age| age >= ART_SETTLED)
    });
    if let Some(hit) = ART
        .with(|cache| cache.borrow_mut().get(&key, modified, len))
        .filter(|_| settled)
    {
        return Some(hit);
    }
    let mut bytes = Vec::new();
    file.take(ART_MAX).read_to_end(&mut bytes).ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    let url = format!("data:{mime};base64,{data}");
    ART.with(|cache| {
        if !settled {
            cache.borrow_mut().forget(&key);
            return;
        }
        cache.borrow_mut().put(
            key,
            Art {
                modified,
                len,
                url: url.clone(),
            },
        );
    });
    Some(url)
}

/// The core list of collection note `path` (its text `text`, its title
/// `title`) in `vault`. `on_open` opens a row's note. Its rows are read
/// once per text and per change to the vault, not per draw.
#[component]
pub fn CollectionHead(
    vault: VaultHandle,
    path: String,
    title: String,
    text: String,
    on_open: EventHandler<String>,
) -> Element {
    // Keyed by the vault's root: another vault is another list, heard
    // from its own subscription.
    let root = vault.0.vault().root().display().to_string();
    rsx! {
        VaultList { key: "{root}", vault, path, title, text, on_open }
    }
}

/// [`CollectionHead`] for one vault: subscribed to its changes once.
#[component]
fn VaultList(
    vault: VaultHandle,
    path: String,
    title: String,
    text: String,
    on_open: EventHandler<String>,
) -> Element {
    // Bumped by every change the vault hears (a song's file edited, a
    // recording added): the rows are read again.
    let mut changed = use_signal(|| 0_u64);
    let source = vault.0.clone();
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
        let sub = architect_navigator::source::NavigatorSource::subscribe(
            &source,
            Box::new(move |_| {
                let _ = tx.unbounded_send(());
            }),
        );
        spawn(async move {
            use futures_util::StreamExt as _;
            while rx.next().await.is_some() {
                let n = changed.peek().wrapping_add(1);
                changed.set(n);
            }
        });
        std::rc::Rc::new(sub)
    });
    let sections = use_memo(use_reactive!(|vault, path, text| {
        let _ = changed();
        rows(vault.0.vault(), &path, &text)
    }));
    let root = vault.0.vault().root();
    let art = use_callback(move |p: String| art_url(&root, &p));
    let Some(sections) = sections() else {
        return rsx! {};
    };
    rsx! {
        architect_media::ui::MediaList {
            source: SOURCE.to_owned(),
            name: title,
            sections,
            art: Some(art),
            on_open,
        }
    }
}

/// A vault on disk, as a prop: what the navigator reads, heard for
/// changes.
#[derive(Clone)]
pub struct VaultHandle(pub VaultSource);

impl PartialEq for VaultHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.vault().root() == other.0.vault().root()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_setlist_lists_its_two_sets_with_their_songs() {
        let dir = tempfile::tempdir().unwrap();
        crate::vaults::write_demo(&dir.path().join("v")).unwrap();
        let open = vault::actions::OpenVault::open(&dir.path().join("v")).unwrap();
        let text =
            std::fs::read_to_string(dir.path().join("v/Setlists/Merry & Bright.md")).unwrap();
        let rows = rows(&open, "Setlists/Merry & Bright.md", &text).unwrap();
        let names: Vec<Option<&str>> = rows.iter().map(|(n, _)| n.as_deref()).collect();
        assert_eq!(names, vec![Some("Opening Set"), Some("Concert Set")]);
        let all: Vec<&Row> = rows.iter().flat_map(|(_, r)| r).collect();
        assert_eq!(all.len(), 9);
        let greyed: Vec<&str> = all
            .iter()
            .filter(|r| !r.plays())
            .map(|r| r.item.title.as_str())
            .collect();
        assert_eq!(
            greyed,
            vec![
                "Feliz Navidad",
                "What Christmas Means",
                "Rockin' Around the Christmas Tree",
                "Last Christmas"
            ]
        );
        assert!(all.iter().all(|r| r.art.is_some()), "every song's cover");
        let url = art_url(&open.root(), all[0].art.as_deref().unwrap()).unwrap();
        assert!(url.starts_with("data:image/svg+xml;base64,"));
        // Nothing outside the vault, nor a path that climbs out.
        assert_eq!(art_url(&open.root(), "../outside.png"), None);
        assert_eq!(art_url(&open.root(), "Setlists/Merry & Bright.md"), None);
    }

    #[test]
    fn a_cover_is_read_again_once_it_changes_and_a_miss_is_never_kept() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A miss, then the file comes: found.
        assert_eq!(art_url(root, "Art/a.png"), None);
        std::fs::create_dir_all(root.join("Art")).unwrap();
        std::fs::write(root.join("Art/a.png"), b"one").unwrap();
        let one = art_url(root, "Art/a.png").unwrap();
        assert!(one.ends_with(&base64_of(b"one")), "{one}");
        // Changed: read again, not the old cover.
        std::fs::write(root.join("Art/a.png"), b"two two").unwrap();
        let two = art_url(root, "Art/a.png").unwrap();
        assert!(two.ends_with(&base64_of(b"two two")), "{two}");
        // A dot segment or a link out of the vault: nothing.
        assert_eq!(art_url(root, "Art/./a.png"), None);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.png"), b"outside").unwrap();
        std::os::unix::fs::symlink(outside.path().join("x.png"), root.join("Art/out.png")).unwrap();
        assert_eq!(art_url(root, "Art/out.png"), None);
    }

    #[test]
    fn the_cover_cache_keeps_the_most_recent_within_its_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..ART_KEPT + 10 {
            write_settled(
                &root.join(format!("{i}.png")),
                format!("png {i}").as_bytes(),
            );
            assert!(art_url(root, &format!("{i}.png")).is_some());
        }
        ART.with(|c| {
            let c = c.borrow();
            assert_eq!(c.kept.len(), ART_KEPT);
            assert_eq!(c.order.len(), ART_KEPT);
            assert!(
                !c.kept.contains_key(&root.join("0.png")),
                "the oldest dropped"
            );
            assert!(
                c.kept
                    .contains_key(&root.join(format!("{}.png", ART_KEPT + 9)))
            );
        });
    }

    #[test]
    fn a_cover_rewritten_with_the_same_time_and_size_is_never_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        // Rewritten in place within the clock's grain: the same time, the
        // same size. Read fresh each time while it may still change.
        let now = SystemTime::now();
        for bytes in [b"one".as_slice(), b"two".as_slice()] {
            std::fs::write(&path, bytes).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now)
                .unwrap();
            let url = art_url(dir.path(), "a.png").unwrap();
            assert!(url.ends_with(&base64_of(bytes)), "{url}");
        }
        ART.with(|c| {
            assert!(
                !c.borrow().kept.contains_key(&path),
                "a racy read is not kept"
            );
        });
        // Settled: kept.
        write_settled(&path, b"six");
        assert!(
            art_url(dir.path(), "a.png")
                .unwrap()
                .ends_with(&base64_of(b"six"))
        );
        ART.with(|c| assert!(c.borrow().kept.contains_key(&path)));
    }

    /// `bytes` at `path`, its modification time past [`ART_SETTLED`].
    fn write_settled(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
        let old = SystemTime::now()
            .checked_sub(ART_SETTLED.saturating_add(Duration::from_secs(5)))
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    fn base64_of(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
}
