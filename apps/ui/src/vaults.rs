//! The Architect app's vault: the folder of markdown it opens, chosen on
//! its first run ([`crate::first_run`]) and remembered in its folder
//! (`<app dir>/vaults.json`), so the next start opens it again.
//!
//! The first run offers three: a folder the person has, a new vault
//! ([`create`], `~/Documents/Architect`), or the demo vault ([`demo`]: the
//! made-up vault in `apps/ui/demo-vault`, built into the app and written
//! out to `<app dir>/Demo vault` to explore and edit).

use std::path::{Path, PathBuf};

/// Where the chosen vault is kept, in the app's folder.
const KEPT: &str = "vaults.json";

/// The demo vault, as built into the app (`apps/ui/demo-vault.py` makes it).
static DEMO: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/demo-vault");

/// The vault the app opened last, while its folder is there.
#[must_use]
pub fn remembered() -> Option<PathBuf> {
    let text = std::fs::read_to_string(crate::app_dir()?.join(KEPT)).ok()?;
    let kept: serde_json::Value = serde_json::from_str(&text).ok()?;
    let dir = PathBuf::from(kept.get("current")?.as_str()?);
    dir.is_dir().then_some(dir)
}

/// Remembers `dir` as the vault to open from now on.
///
/// # Errors
///
/// The app has no folder, or the file cannot be written.
pub fn remember(dir: &Path) -> std::io::Result<()> {
    let app = crate::app_dir().ok_or_else(|| std::io::Error::other("the app has no folder"))?;
    std::fs::create_dir_all(&app)?;
    let kept = serde_json::json!({ "current": dir.display().to_string() });
    std::fs::write(app.join(KEPT), kept.to_string())
}

/// The demo vault, written out to `<app dir>/Demo vault` when it is not
/// there yet (an edit made to it stays; delete the folder for a fresh one).
///
/// # Errors
///
/// The app has no folder, or a file cannot be written.
pub fn demo() -> std::io::Result<PathBuf> {
    let app = crate::app_dir().ok_or_else(|| std::io::Error::other("the app has no folder"))?;
    let dir = app.join("Demo vault");
    write_demo(&dir)?;
    Ok(dir)
}

/// Writes the built-in demo vault into `dir` when `dir` does not exist,
/// with the empty folders its `.empty-folders` lists (git keeps none).
///
/// # Errors
///
/// A file cannot be written.
pub fn write_demo(dir: &Path) -> std::io::Result<bool> {
    if dir.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    DEMO.extract(dir)?;
    let empty = dir.join(crate::notes::EMPTY_FOLDERS);
    if let Ok(list) = std::fs::read_to_string(&empty) {
        for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
            std::fs::create_dir_all(dir.join(line))?;
        }
        std::fs::remove_file(empty)?;
    }
    Ok(true)
}

/// A new, empty vault: `~/Documents/Architect` (`Architect 2`, … when
/// that is taken; the home folder when there is no Documents), with a
/// welcome note.
///
/// # Errors
///
/// There is no home folder, or the folder cannot be made.
pub fn create() -> std::io::Result<PathBuf> {
    let home = std::env::home_dir().ok_or_else(|| std::io::Error::other("no home folder"))?;
    let documents = home.join("Documents");
    let parent = if documents.is_dir() { documents } else { home };
    create_in(&parent)
}

/// [`create`] under `parent`.
///
/// # Errors
///
/// The folder cannot be made.
pub fn create_in(parent: &Path) -> std::io::Result<PathBuf> {
    let dir = (1..=999)
        .map(|n: u32| {
            parent.join(if n == 1 {
                "Architect".to_owned()
            } else {
                format!("Architect {n}")
            })
        })
        .find(|d| !d.exists())
        .ok_or_else(|| std::io::Error::other("no free folder name"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("Welcome.md"), WELCOME)?;
    Ok(dir)
}

/// A new vault's first note.
const WELCOME: &str = "# Welcome\n\nThis is your vault: a folder of markdown files on this device. \
Every note is a file you can open in any editor, and Architect reads the folder as it is.\n\n\
Start writing here, or make a new note from the file list.\n";

#[cfg(test)]
mod tests {
    #[test]
    fn the_built_in_demo_vault_is_written_with_its_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Demo vault");
        assert!(super::write_demo(&dir).unwrap());
        assert!(dir.join("Welcome.md").is_file());
        assert!(dir.join("Archive/Empty").is_dir(), "an empty folder");
        assert!(!dir.join(crate::notes::EMPTY_FOLDERS).exists());
        assert!(!super::write_demo(&dir).unwrap(), "an edited copy is kept");
    }

    #[test]
    fn a_new_vault_takes_the_next_free_name() {
        let tmp = tempfile::tempdir().unwrap();
        let first = super::create_in(tmp.path()).unwrap();
        let second = super::create_in(tmp.path()).unwrap();
        assert_eq!(first, tmp.path().join("Architect"));
        assert_eq!(second, tmp.path().join("Architect 2"));
        assert!(second.join("Welcome.md").is_file());
    }
}
