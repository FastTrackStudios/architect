//! A collection note's native list in the desktop and phone apps (#668):
//! a note whose frontmatter lists media items (`sets`/`songs`,
//! `sections`/`items`: `architect_media::Collection`) draws as
//! `architect_media::ui::MediaList` at its head, as the fallback of the
//! embed its type would be claimed by: an extension that claims the type
//! draws instead (`architect_extensions::embeds::EmbedView`).
//!
//! Its rows come from the vault on disk: each link resolved among the
//! vault's notes, each item's media and cover among its files. Covers are
//! handed to the list (and to the Now Playing card) as `data:` URLs read
//! from the vault ([`art_url`]), only for an image file inside it. The
//! list is the app's own source on the playhead, [`SOURCE`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use architect_media::{Collection, Item, Row, front};
use dioxus::prelude::*;

/// The app's own source for a collection's list: outside the
/// extension-id grammar (`:`), so no extension can hold it.
pub const SOURCE: &str = "app:list";

/// The largest cover handed over as a `data:` URL.
const ART_MAX: u64 = 4 * 1024 * 1024;

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
        let resolved = list.resolve(path, notes.iter().copied());
        Some(resolved.rows(|p| {
            let page = v.pages.iter().find(|page| page.rel_path == p)?;
            Some(Row::of(
                Item::read(p, &front::fields(&page.raw)),
                files.iter().copied(),
            ))
        }))
    })
}

thread_local! {
    /// Covers read so far: a list draws ten times a second while it plays.
    static ART: RefCell<HashMap<PathBuf, Option<String>>> = RefCell::new(HashMap::new());
}

/// The image at vault path `path` in the vault at `root`, as a `data:`
/// URL: only a vault path (`architect_playhead::model::vault_path`) to an
/// image file inside the vault (not through a link out of it), at most
/// [`ART_MAX`].
#[must_use]
pub fn art_url(root: &Path, path: &str) -> Option<String> {
    use base64::Engine as _;
    if !architect_playhead::model::vault_path(path) {
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
    let file = root.join(path);
    ART.with(|cache| {
        if let Some(hit) = cache.borrow().get(&file) {
            return hit.clone();
        }
        let read = (|| {
            let real = file.canonicalize().ok()?;
            let inside = root.canonicalize().ok()?;
            if !real.starts_with(&inside) {
                return None;
            }
            let meta = std::fs::metadata(&real).ok()?;
            if !meta.is_file() || meta.len() > ART_MAX {
                return None;
            }
            let bytes = std::fs::read(&real).ok()?;
            let data = base64::engine::general_purpose::STANDARD.encode(bytes);
            Some(format!("data:{mime};base64,{data}"))
        })();
        cache.borrow_mut().insert(file, read.clone());
        read
    })
}

/// The core list of collection note `path` (its text `text`, its title
/// `title`) in `vault`. `on_open` opens a row's note.
#[component]
pub fn CollectionHead(
    vault: VaultHandle,
    path: String,
    title: String,
    text: String,
    on_open: EventHandler<String>,
) -> Element {
    let Some(sections) = rows(&vault.0, &path, &text) else {
        return rsx! {};
    };
    let root = vault.0.root();
    let art = use_callback(move |p: String| art_url(&root, &p));
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

/// A vault on disk, as a prop.
#[derive(Clone)]
pub struct VaultHandle(pub vault::actions::OpenVault);

impl PartialEq for VaultHandle {
    fn eq(&self, other: &Self) -> bool {
        self.0.root() == other.0.root()
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
}
