//! The app's vault on disk: a folder of markdown (notes, their properties
//! and links, tags, bases, images) that the `VAULT_*` actions read and
//! write (`vault::actions`), with the `LINKS_*`, `TAGS_*` and `BASES_*` (`architect_bases::actions`)
//! actions over it, and that the Vault's navigator and editor show and edit
//! (`crate::vault`). It is the folder `$ARCHITECT_VAULT` names (`just
//! sandbox` and `devenv up` name a scratch one under devenv's state, seeded
//! from `apps/ui/demo-vault`, which `$ARCHITECT_VAULT_SEED` names, when it
//! is missing; the lab's `$ARCHITECT_LAB_VAULT` and `$ARCHITECT_LAB_SEED`
//! are still read). Else the Architect app opens the vault the person
//! chose on its first run (`crate::vaults`), and the sandbox
//! `<its folder>/vault`, seeded on first run from the sample in
//! `apps/sandbox/vault`, which backs the actions only (the Vault's own
//! notes are in memory then, as tests and films expect). Only an app with
//! files has one: a film's headless run keeps nothing. `APP_STATUS`'s
//! `vault` section says which folder is open.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use architect::action::ActionList;
use architect_bases::model::ViewTypes;

/// The sample vault, as `(vault path, text)`.
const SAMPLE: &[(&str, &str)] = &[
    ("Welcome.md", include_str!("../../sandbox/vault/Welcome.md")),
    (
        "Notes/The app shell.md",
        include_str!("../../sandbox/vault/Notes/The app shell.md"),
    ),
    (
        "Notes/The workspace.md",
        include_str!("../../sandbox/vault/Notes/The workspace.md"),
    ),
    (
        "Notes/Actions everywhere.md",
        include_str!("../../sandbox/vault/Notes/Actions everywhere.md"),
    ),
    (
        "Songs/Amazing Grace.md",
        include_str!("../../sandbox/vault/Songs/Amazing Grace.md"),
    ),
    (
        "Bases/Notes by status.base",
        include_str!("../../sandbox/vault/Bases/Notes by status.base"),
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
pub const EMPTY_FOLDERS: &str = ".empty-folders";

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

/// `$new`, else the lab's `$old`, when set and not empty.
fn env_either(new: &str, old: &str) -> Option<std::ffi::OsString> {
    [new, old]
        .into_iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
}

/// The vault folder the environment names (`$ARCHITECT_VAULT`), seeded
/// from `$ARCHITECT_VAULT_SEED` when it is missing.
fn named_folder() -> Option<PathBuf> {
    let dir = PathBuf::from(env_either("ARCHITECT_VAULT", "ARCHITECT_LAB_VAULT")?);
    if let Some(seed) = env_either("ARCHITECT_VAULT_SEED", "ARCHITECT_LAB_SEED") {
        match seed_from(&dir, Path::new(&seed)) {
            Ok(true) => tracing::info!(dir = %dir.display(), "app.vault.seeded"),
            Ok(false) => {}
            Err(e) => tracing::warn!(dir = %dir.display(), "the app's vault: {e}"),
        }
    }
    Some(dir)
}

/// The folder the app opens as its vault.
fn folder() -> Option<PathBuf> {
    if let Some(dir) = named_folder() {
        return Some(dir);
    }
    if !crate::distribution().is_sandbox() {
        return crate::vaults::remembered();
    }
    let dir = crate::app_dir()?.join("vault");
    seed(&dir)
        .map_err(|e| tracing::warn!(dir = %dir.display(), "the sandbox's vault: {e}"))
        .ok()?;
    Some(dir)
}

/// Whether the app has a vault to open now: the Architect app's first run
/// asks for one when it has none (`crate::first_run`).
#[must_use]
pub fn has_vault() -> bool {
    OPEN.get().is_some() || named_folder().is_some() || crate::vaults::remembered().is_some()
}

/// The vault the app opened, for the Vault's navigator and editor.
static OPEN: OnceLock<vault::actions::OpenVault> = OnceLock::new();

/// The vault on disk the Vault's navigator and editor show: the Architect
/// app's, or the one `$ARCHITECT_VAULT` names. The sandbox's own sample
/// (`<its folder>/vault`) backs the `VAULT_*` actions only, so a test run
/// or film keeps its notes in memory and looks as it always did.
#[must_use]
pub fn open_vault() -> Option<vault::actions::OpenVault> {
    if crate::distribution().is_sandbox() {
        env_either("ARCHITECT_VAULT", "ARCHITECT_LAB_VAULT")?;
    }
    OPEN.get().cloned()
}

/// The app's vault as the navigator, its editors and the extensions'
/// documents see it: one source, so each hears what the others change.
static SOURCE: OnceLock<architect_navigator::VaultSource> = OnceLock::new();

/// What waits for the app's vault to open ([`on_source`]).
type Waiting = Vec<Box<dyn FnOnce(&architect_navigator::VaultSource) + Send>>;
static WAITING: std::sync::Mutex<Waiting> = std::sync::Mutex::new(Vec::new());

/// The app's vault ([`open_vault`]) as a watched navigator source, the
/// same one for the run: the Vault's navigator and editors read and save
/// through it, and the extension host's documents
/// (`architect_navigator::documents::VaultDocuments`) change it, so an
/// open note merges an extension's edit into its typing and the person's
/// own saves are document events (#706).
#[must_use]
pub fn source() -> Option<architect_navigator::VaultSource> {
    if let Some(s) = SOURCE.get() {
        return Some(s.clone());
    }
    let open = open_vault()?;
    let mut fresh = false;
    let s = SOURCE
        .get_or_init(|| {
            fresh = true;
            // The sandbox's is the demo vault; the Architect app's is the
            // person's, by its folder's name.
            let name = if crate::distribution().is_sandbox() {
                "Demo vault".to_owned()
            } else {
                open.root().file_name().map_or_else(
                    || "Vault".to_owned(),
                    |n| n.to_string_lossy().into_owned(),
                )
            };
            crate::vault::watched(open, name)
        })
        .clone();
    if fresh {
        let waiting = std::mem::take(
            &mut *WAITING
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for f in waiting {
            f(&s);
        }
    }
    Some(s)
}

/// Runs `f` with the app's vault's [`source`] once there is one: now, or
/// when the first run's choice opens it.
pub fn on_source(f: impl FnOnce(&architect_navigator::VaultSource) + Send + 'static) {
    let mut waiting = WAITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(s) = SOURCE.get() {
        drop(waiting);
        f(s);
        return;
    }
    waiting.push(Box::new(f));
    drop(waiting);
    let _ = source();
}

/// The base view types the lab's extensions add, for `BASES_VIEW_TYPES`:
/// filled from the extension host once it is up (`crate::extensions`).
pub fn view_types() -> &'static ViewTypes {
    static TYPES: OnceLock<ViewTypes> = OnceLock::new();
    TYPES.get_or_init(ViewTypes::new)
}

/// Opens the app's vault, puts its `VAULT_*`, `LINKS_*`, `TAGS_*` and
/// `BASES_*` actions on `list`, and says which folder it is in
/// `APP_STATUS` (`vault`). The vault opens once a process; a later list (a
/// film's next run, its own app) gets the same vault's actions.
pub fn register(list: &Arc<ActionList>) {
    let open = if let Some(open) = OPEN.get() {
        open.clone()
    } else {
        let Some(dir) = folder() else { return };
        match vault::actions::OpenVault::open(&dir) {
            Ok(open) => OPEN.get_or_init(|| open).clone(),
            Err(e) => {
                tracing::warn!(dir = %dir.display(), "the app's vault: {e}");
                return;
            }
        }
    };
    // Its source, for whatever waits on it (the extensions' documents).
    let _ = source();
    if list.info(vault::actions::ids::READ).is_some() {
        return;
    }
    let _ = vault::actions::register(list, &open);
    let _ = links::actions::register(list, &open);
    let _ = tag::actions::register(list, &open);
    let _ = architect_bases::actions::register(list, &open, view_types());
    crate::publish::register(list, &open);
    let shown = open;
    let seeded = env_either("ARCHITECT_VAULT_SEED", "ARCHITECT_LAB_SEED")
        .map(|s| s.to_string_lossy().into_owned());
    list.status().section("vault", move || {
        let notes = shown.with(|v| v.pages.len());
        serde_json::json!({
            "folder": shown.root().display().to_string(),
            "seeded_from": seeded,
            "notes": notes,
        })
        .to_string()
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_demo_vault_seeds_a_scratch_copy_with_its_empty_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("scratch");
        let demo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("demo-vault");
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
