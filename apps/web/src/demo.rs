//! The demo vault, built in.
//!
//! `apps/ui/demo-vault` (made by `apps/ui/demo-vault.py`): what
//! `/app/?demo` ([`crate::DEMO`]) opens in the tab's memory. The desktop app's first run offers the same vault
//! (`apps/ui/src/vaults.rs` `demo`), written out to a folder there.

use include_dir::{Dir, DirEntry, include_dir};

static VAULT: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../ui/demo-vault");

/// Lists the folders git keeps no file in (the vault's own list).
const EMPTY_FOLDERS: &str = ".empty-folders";

/// The demo vault's name, as the navigator shows its root.
pub const NAME: &str = "Demo vault";

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

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Every file of the demo vault, by path.
#[must_use]
pub fn files() -> Vec<File> {
    let mut out = Vec::new();
    walk(&VAULT, &mut out);
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

/// Every folder of the demo vault, its empty ones too (which git keeps no
/// file in: the vault lists them).
#[must_use]
pub fn folders() -> Vec<&'static str> {
    let mut out = Vec::new();
    collect_dirs(&VAULT, &mut out);
    if let Some(list) = VAULT
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
}
