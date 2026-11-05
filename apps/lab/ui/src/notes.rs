//! The lab's vault on disk: a folder of markdown (notes, their properties
//! and links, tags, a `.base`) that the `VAULT_*` actions read and write
//! (`vault::actions`), with the `LINKS_*` and `TAGS_*` actions over it. It is `<the lab's folder>/vault`, seeded on first
//! run from the sample in `apps/lab/vault`, or the folder
//! `$ARCHITECT_LAB_VAULT` names. Only a lab with files has one: a film's
//! headless run keeps nothing.

use std::path::{Path, PathBuf};

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

/// The folder the lab opens as its vault.
fn folder() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ARCHITECT_LAB_VAULT") {
        return Some(PathBuf::from(dir));
    }
    let dir = crate::app_dir()?.join("vault");
    seed(&dir)
        .map_err(|e| tracing::warn!(dir = %dir.display(), "the lab's vault: {e}"))
        .ok()?;
    Some(dir)
}

/// Opens the lab's vault and puts its `VAULT_*`, `LINKS_*` and `TAGS_*`
/// actions on `list`.
pub fn register(list: &ActionList) {
    let Some(dir) = folder() else { return };
    match vault::actions::OpenVault::open(&dir) {
        Ok(open) => {
            let _ = vault::actions::register(list, &open);
            let _ = links::actions::register(list, &open);
            let _ = tag::actions::register(list, &open);
        }
        Err(e) => tracing::warn!(dir = %dir.display(), "the lab's vault: {e}"),
    }
}

#[cfg(test)]
mod tests {
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
