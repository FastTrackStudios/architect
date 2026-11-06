//! The lab's vault on disk: a folder of markdown (notes, their properties
//! and links, tags, bases, images) that the `VAULT_*` actions read and
//! write (`vault::actions`), with the `LINKS_*` and `TAGS_*` actions over
//! it, and that the Vault's navigator and editor show and edit
//! (`crate::vault`). It is the folder `$ARCHITECT_LAB_VAULT` names (`just
//! lab` and `devenv up` name a scratch one under devenv's state, seeded
//! from `apps/lab/demo-vault`, which `$ARCHITECT_LAB_SEED` names, when it
//! is missing), else `<the lab's folder>/vault`, seeded on first run from
//! the sample in `apps/lab/vault`, which backs the actions only (the
//! Vault's own notes are in memory then, as tests and films expect). Only
//! a lab with files has one: a film's headless run keeps nothing.
//! `APP_STATUS`'s `vault` section says which folder is open.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use architect::action::ActionList;

/// The sample vault, as `(vault path, text)`.
const SAMPLE: &[(&str, &str)] = &[
    ("Welcome.md", include_str!("../../vault/Welcome.md")),
    (
        "Notes/The app shell.md",
        include_str!("../../vault/Notes/The app shell.md"),
    ),
    (
        "Notes/The workspace.md",
        include_str!("../../vault/Notes/The workspace.md"),
    ),
    (
        "Notes/Actions everywhere.md",
        include_str!("../../vault/Notes/Actions everywhere.md"),
    ),
    (
        "Songs/Amazing Grace.md",
        include_str!("../../vault/Songs/Amazing Grace.md"),
    ),
    (
        "Bases/Notes by status.base",
        include_str!("../../vault/Bases/Notes by status.base"),
    ),
];

/// Writes the sample into `dir` when `dir` does not exist yet.
fn seed(dir: &Path) -> std::io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    for (path, text) in SAMPLE {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, text)?;
    }
    Ok(())
}

/// Lists the folders to make empty in a copy of the demo vault (git keeps
/// no empty folder): one vault path a line.
const EMPTY_FOLDERS: &str = ".empty-folders";

/// Copies the vault at `from` into `dir` when `dir` does not exist yet,
/// with the empty folders `from`'s [`EMPTY_FOLDERS`] lists.
///
/// # Errors
///
/// A file cannot be read or written.
pub fn seed_from(dir: &Path, from: &Path) -> std::io::Result<bool> {
    if dir.exists() {
        return Ok(false);
    }
    let mut todo = vec![(from.to_path_buf(), dir.to_path_buf())];
    while let Some((src, dst)) = todo.pop() {
        std::fs::create_dir_all(&dst)?;
        for e in std::fs::read_dir(&src)? {
            let e = e?;
            let name = e.file_name();
            if name == EMPTY_FOLDERS {
                continue;
            }
            if e.file_type()?.is_dir() {
                todo.push((e.path(), dst.join(&name)));
            } else {
                std::fs::copy(e.path(), dst.join(&name))?;
            }
        }
    }
    if let Ok(list) = std::fs::read_to_string(from.join(EMPTY_FOLDERS)) {
        for line in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
            std::fs::create_dir_all(dir.join(line))?;
        }
    }
    Ok(true)
}

/// The folder the lab opens as its vault.
fn folder() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ARCHITECT_LAB_VAULT") {
        let dir = PathBuf::from(dir);
        if let Some(seed) = std::env::var_os("ARCHITECT_LAB_SEED") {
            match seed_from(&dir, Path::new(&seed)) {
                Ok(true) => tracing::info!(dir = %dir.display(), "lab.vault.seeded"),
                Ok(false) => {}
                Err(e) => tracing::warn!(dir = %dir.display(), "the lab's vault: {e}"),
            }
        }
        return Some(dir);
    }
    let dir = crate::app_dir()?.join("vault");
    seed(&dir)
        .map_err(|e| tracing::warn!(dir = %dir.display(), "the lab's vault: {e}"))
        .ok()?;
    Some(dir)
}

/// The vault the lab opened, for the Vault's navigator and editor.
static OPEN: OnceLock<vault::actions::OpenVault> = OnceLock::new();

/// The vault on disk the Vault's navigator and editor show: the one
/// `$ARCHITECT_LAB_VAULT` names (`just lab`, `devenv up`). The lab's own
/// sample (`<its folder>/vault`) backs the `VAULT_*` actions only, so a
/// test run or film keeps its notes in memory and looks as it always did.
pub fn open_vault() -> Option<vault::actions::OpenVault> {
    std::env::var_os("ARCHITECT_LAB_VAULT")?;
    OPEN.get().cloned()
}

/// Opens the lab's vault, puts its `VAULT_*`, `LINKS_*` and `TAGS_*`
/// actions on `list`, and says which folder it is in `APP_STATUS`
/// (`vault`).
pub fn register(list: &ActionList) {
    let Some(dir) = folder() else { return };
    match vault::actions::OpenVault::open(&dir) {
        Ok(open) => {
            let _ = vault::actions::register(list, &open);
            let _ = links::actions::register(list, &open);
            let _ = tag::actions::register(list, &open);
            let shown = open.clone();
            let seeded = std::env::var("ARCHITECT_LAB_SEED").ok();
            list.status().section("vault", move || {
                let notes = shown.with(|v| v.pages.len());
                serde_json::json!({
                    "folder": shown.root().display().to_string(),
                    "seeded_from": seeded,
                    "notes": notes,
                })
                .to_string()
            });
            let _ = OPEN.set(open);
        }
        Err(e) => tracing::warn!(dir = %dir.display(), "the lab's vault: {e}"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_demo_vault_seeds_a_scratch_copy_with_its_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("scratch");
        let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../demo-vault");
        assert!(super::seed_from(&dir, &demo).unwrap());
        assert!(dir.join("Welcome.md").exists());
        assert!(dir.join("Archive/Empty").is_dir(), "an empty folder");
        assert!(!dir.join(super::EMPTY_FOLDERS).exists());
        let open = vault::actions::OpenVault::open(&dir).unwrap();
        assert!(
            open.with(|v| v.pages.len()) > 300,
            "the scroll test's notes"
        );
        // Seeded once: a second run leaves the copy as it is.
        std::fs::write(dir.join("Welcome.md"), "mine").unwrap();
        assert!(!super::seed_from(&dir, &demo).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.join("Welcome.md")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn the_sample_seeds_a_vault_the_actions_read() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("vault");
        super::seed(&dir).unwrap();
        let list = architect::action::ActionList::new();
        let open = vault::actions::OpenVault::open(&dir).unwrap();
        vault::actions::register(&list, &open).unwrap();
        let out = list.run_json("VAULT_STATUS", b"").unwrap();
        let status: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(status["notes"], 5, "{status}");
        // A second seed leaves what is there alone.
        std::fs::write(dir.join("Welcome.md"), "mine").unwrap();
        super::seed(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("Welcome.md")).unwrap(),
            "mine"
        );
    }
}
