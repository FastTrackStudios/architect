//! The demo vault, built in.
//!
//! `apps/ui/demo-vault` (made by `apps/ui/demo-vault.py`): what
//! `/app/?demo` ([`crate::DEMO`]) opens in the tab's memory. The desktop app's first run offers the same vault
//! (`apps/ui/src/vaults.rs` `demo`), written out to a folder there.
//!
//! `/app/?showcase` ([`crate::SHOWCASE`]) opens the extension showcase's
//! vault (`apps/ui/showcase-vault`, #710) the same way instead: [`choose`]
//! picks which one before the app mounts.

use std::sync::atomic::{AtomicBool, Ordering};

use include_dir::{Dir, DirEntry, include_dir};

static DEMO: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../ui/demo-vault");
static SHOWCASE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../ui/showcase-vault");

/// Whether this tab opened the showcase's vault rather than the demo's.
static SHOWING: AtomicBool = AtomicBool::new(false);

/// Which built-in vault the demo opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vault {
    /// `apps/ui/demo-vault`.
    Demo,
    /// `apps/ui/showcase-vault`: one note per extension capability (#710).
    Showcase,
}

/// Picks the vault the demo opens: once, before the app mounts.
pub fn choose(vault: Vault) {
    SHOWING.store(vault == Vault::Showcase, Ordering::Relaxed);
}

/// The vault the demo opens.
#[must_use]
pub fn chosen() -> Vault {
    if SHOWING.load(Ordering::Relaxed) {
        Vault::Showcase
    } else {
        Vault::Demo
    }
}

fn dir(vault: Vault) -> &'static Dir<'static> {
    match vault {
        Vault::Demo => &DEMO,
        Vault::Showcase => &SHOWCASE,
    }
}

/// Lists the folders git keeps no file in (the vault's own list).
const EMPTY_FOLDERS: &str = ".empty-folders";

/// The demo vault's name, as the navigator shows its root.
pub const NAME: &str = "Demo vault";

/// The showcase vault's name, as the navigator shows its root.
pub const SHOWCASE_NAME: &str = "Showcase";

/// The chosen vault's name, as the navigator shows its root.
#[must_use]
pub fn name() -> &'static str {
    match chosen() {
        Vault::Demo => NAME,
        Vault::Showcase => SHOWCASE_NAME,
    }
}

/// One file of the demo vault: its path from the root (`/`-separated) and
/// its bytes.
#[derive(Debug, Clone, Copy)]
pub struct File {
    pub path: &'static str,
    pub bytes: &'static [u8],
}

impl File {
    /// Its text, when it is a note or another text file the app reads
    /// (`.md`, `.base`, `.canvas`, `.svg`).
    #[must_use]
    pub fn text(&self) -> Option<&'static str> {
        let ext = ext(self.path);
        ["md", "base", "canvas", "svg"]
            .iter()
            .any(|t| ext.eq_ignore_ascii_case(t))
            .then(|| std::str::from_utf8(self.bytes).ok())
            .flatten()
    }

    /// Its media type, when it is an image or a PDF the view draws from a
    /// `data:` URL.
    #[must_use]
    pub fn media_type(&self) -> Option<&'static str> {
        match ext(self.path).to_ascii_lowercase().as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "svg" => Some("image/svg+xml"),
            "pdf" => Some("application/pdf"),
            _ => None,
        }
    }
}

/// The most bytes an image of the demo vault may have to be handed as an
/// item's art: a cover is small (the demo's are SVGs of about 2 KB).
pub const ART_MAX: usize = 32 * 1024;

/// The demo vault's image at vault path `path`, as a `data:` URL.
///
/// How an item's art (`architect.playhead`'s `art`) is shown: on the Now
/// Playing card and in a frame's props (`img-src data:` on the app's page
/// and in an island's frame, so no policy changes). None for a path the playhead
/// would refuse (`vault_path`), a file the vault does not have, one that is
/// not an image (a PDF is not art), or one over [`ART_MAX`].
///
/// Only the demo vault, which is compiled in and public: a signed-in
/// vault's art waits for #628 and the rules of `super::app::audio`'s
/// `media_url` (the source's note must link the path).
#[must_use]
pub fn image_url(path: &str) -> Option<String> {
    use base64::Engine as _;
    if !crate::ext_host::vault_path(path) {
        return None;
    }
    let file = dir(chosen()).get_file(path)?;
    let f = File {
        path: file.path().to_str()?,
        bytes: file.contents(),
    };
    let mime = f.media_type().filter(|m| m.starts_with("image/"))?;
    if f.bytes.len() > ART_MAX {
        return None;
    }
    let data = base64::engine::general_purpose::STANDARD.encode(f.bytes);
    Some(format!("data:{mime};base64,{data}"))
}

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Every file of the chosen vault, by path.
#[must_use]
pub fn files() -> Vec<File> {
    files_of(chosen())
}

/// Every file of `vault`, by path.
#[must_use]
pub fn files_of(vault: Vault) -> Vec<File> {
    let mut out = Vec::new();
    walk(dir(vault), &mut out);
    out.sort_by_key(|f| f.path);
    out
}

fn walk(dir: &'static Dir<'static>, out: &mut Vec<File>) {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(d) => walk(d, out),
            DirEntry::File(f) => {
                let Some(path) = f.path().to_str() else {
                    continue;
                };
                if path == EMPTY_FOLDERS || path.ends_with(".DS_Store") {
                    continue;
                }
                out.push(File {
                    path,
                    bytes: f.contents(),
                });
            }
        }
    }
}

/// Every folder of the chosen vault, its empty ones too (which git keeps
/// no file in: the vault lists them).
#[must_use]
pub fn folders() -> Vec<&'static str> {
    folders_of(chosen())
}

/// Every folder of `vault`, its empty ones too.
#[must_use]
pub fn folders_of(vault: Vault) -> Vec<&'static str> {
    let mut out = Vec::new();
    collect_dirs(dir(vault), &mut out);
    if let Some(list) = dir(vault)
        .get_file(EMPTY_FOLDERS)
        .and_then(|f| f.contents_utf8())
    {
        out.extend(list.lines().map(str::trim).filter(|l| !l.is_empty()));
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn collect_dirs(dir: &'static Dir<'static>, out: &mut Vec<&'static str>) {
    for d in dir.dirs() {
        if let Some(p) = d.path().to_str() {
            out.push(p);
        }
        collect_dirs(d, out);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_demo_vault_is_built_in_with_its_notes_images_and_empty_folders() {
        let files = super::files();
        assert!(files.len() > 400, "{}", files.len());
        let welcome = files.iter().find(|f| f.path == "Welcome.md");
        assert!(welcome.and_then(super::File::text).is_some());
        assert!(
            files
                .iter()
                .any(|f| f.media_type() == Some("image/png") && f.text().is_none())
        );
        assert!(!files.iter().any(|f| f.path.ends_with(".empty-folders")));
        let folders = super::folders();
        assert!(folders.contains(&"Archive/Empty"), "{folders:?}");
        assert!(folders.contains(&"Notes"));
    }

    #[test]
    fn a_demo_image_is_art_as_a_data_url_and_nothing_else_is() {
        let cover = super::image_url("Songs/Christmas/Sleigh Ride/cover.svg");
        assert!(
            cover
                .as_deref()
                .is_some_and(|u| u.starts_with("data:image/svg+xml;base64,")),
            "{cover:?}"
        );
        assert!(
            super::image_url("Attachments/sun.jpg")
                .is_some_and(|u| u.starts_with("data:image/jpeg;base64,"))
        );
        // Not an image, too big, not there, or not a vault path.
        assert_eq!(super::image_url("Attachments/Brief.pdf"), None);
        assert_eq!(super::image_url("Attachments/plasma-one.png"), None);
        assert_eq!(super::image_url("Welcome.md"), None);
        assert_eq!(super::image_url("Songs/Christmas/nothing.svg"), None);
        assert_eq!(
            super::image_url("../Songs/Christmas/Sleigh Ride/cover.svg"),
            None
        );
        assert_eq!(super::image_url("/Attachments/sun.jpg"), None);
        assert_eq!(super::image_url("https://evil.test/a.svg"), None);
    }

    /// Every SVG of the demo vault can be handed to a frame as art: no
    /// script, handler, embedded document or outside reference in any.
    #[test]
    fn no_demo_svg_carries_script_or_an_outside_reference() {
        let svgs: Vec<_> = super::files()
            .into_iter()
            .filter(|f| super::ext(f.path).eq_ignore_ascii_case("svg"))
            .collect();
        assert!(!svgs.is_empty());
        for f in svgs {
            let text = f.text().unwrap_or_default().to_ascii_lowercase();
            for bad in [
                "<script",
                "foreignobject",
                "href",
                " on",
                "javascript:",
                "url(http",
                "url(//",
                "<iframe",
                "<object",
                "<embed",
            ] {
                assert!(!text.contains(bad), "{}: {bad}", f.path);
            }
        }
    }

    /// The showcase's vault (#710): a note per capability id, each named
    /// without a colon, and the samples its notes point at.
    #[test]
    fn the_showcase_vault_has_a_note_per_capability() {
        use super::{Vault, files_of, folders_of};
        let files = files_of(Vault::Showcase);
        let ids = (1..=7)
            .map(|n| format!("R{n} "))
            .chain((1..=4).map(|n| format!("L{n} ")))
            .chain((1..=13).map(|n| format!("C{n} ")))
            .chain((1..=16).map(|n| format!("H{n} ")))
            .chain((1..=5).map(|n| format!("U{n} ")))
            .chain((1..=2).map(|n| format!("W{n} ")));
        for id in ids {
            let note = files.iter().find(|f| {
                f.path
                    .rsplit('/')
                    .next()
                    .is_some_and(|n| n.starts_with(&id) && super::ext(n) == "md")
            });
            let text = note.and_then(super::File::text).unwrap_or_default();
            assert!(text.contains("## Try"), "{id}: {note:?}");
        }
        assert!(!files.iter().any(|f| f.path.contains(':')));
        for sample in [
            "Showcase.md",
            "Samples/Tables and lists.md",
            "Samples/Dataview-lite demo.md",
            "Bases/Showcase tasks.base",
        ] {
            assert!(files.iter().any(|f| f.path == sample), "{sample}");
        }
        assert!(folders_of(Vault::Showcase).contains(&"Inbox"));
    }

    #[test]
    fn the_christmas_setlist_is_in_the_demo_with_a_cover_a_song() {
        let files = super::files();
        let setlist = files
            .iter()
            .find(|f| f.path == "Setlists/Merry & Bright.md")
            .and_then(super::File::text)
            .unwrap_or_default();
        assert!(setlist.contains("type: setlist"));
        let rows: Vec<&str> = setlist
            .lines()
            .filter(|l| l.split(" | ").count() > 2)
            .collect();
        assert_eq!(rows.len(), 9, "{rows:?}");
        for row in rows {
            let cover = row.rsplit(" | ").next().unwrap_or_default();
            assert!(super::image_url(cover).is_some(), "{cover}");
            let note = row.split(" | ").nth(2).unwrap_or_default();
            assert!(files.iter().any(|f| f.path == note), "{note}");
        }
    }
}
