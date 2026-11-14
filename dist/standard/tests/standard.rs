//! The standard distribution: the Architect app with the core extensions
//! the list names, and its packaged folders checked against their locks.

use std::path::Path;

use architect_standard::packaged::{self, Island, Packaged, PackagedError};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

fn ids(dist: &architect_distribution::Distribution) -> Vec<String> {
    dist.describe()["extensions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn it_is_the_architect_app_with_the_linked_core_extensions() {
    let dist = architect_standard::distribution_in(None);
    let described = dist.describe();
    assert_eq!(described["name"], "Architect");
    assert_eq!(described["bundle_id"], "app.fasttrackstudio.architect");
    let ids = ids(&dist);
    // The Architect app's own …
    assert!(ids.iter().any(|i| i == "calendar"), "{ids:?}");
    // … and every linked core extension.
    assert!(ids.iter().any(|i| i == "studio-setlists"), "{ids:?}");
    // The window's alone (the desk's and the iOS app's), never the node's.
    assert_eq!(
        ids.iter().any(|i| i == "studio-views"),
        cfg!(feature = "gui"),
        "{ids:?}"
    );
    assert!(dist.islands().is_empty(), "no core folder, no island");
}

fn lock_of(files: &[(&str, &[u8])]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|(name, bytes)| {
            format!(
                "\"{name}\": \"sha256-{}\"",
                base64::engine::general_purpose::STANDARD.encode(Sha256::digest(bytes))
            )
        })
        .collect();
    format!(
        "{{\"lockfileVersion\": 1, \"files\": {{{}}}}}",
        entries.join(", ")
    )
}

const ISLAND: Packaged = Packaged {
    id: "example.island",
    version: "1.0.0",
    island: Some(Island {
        progress: Some("progress"),
        transport: None,
        lyrics: None,
        stylesheet: Some("island.css"),
    }),
};

fn write_island(core: &Path) -> std::path::PathBuf {
    let dir = core.join(ISLAND.id);
    std::fs::create_dir_all(&dir).unwrap();
    let files: [(&str, &[u8]); 2] = [("island.css", b"p{}"), ("island.js", b"export {}")];
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(dir.join(packaged::LOCK), lock_of(&files)).unwrap();
    dir
}

#[test]
fn a_packaged_island_that_matches_its_lock_is_served() {
    let core = tempfile::tempdir().unwrap();
    write_island(core.path());
    let island = packaged::island(core.path(), &ISLAND).expect("served");
    assert_eq!(island.id(), "example.island");
    assert_eq!(island.file("island.js"), Some(&b"export {}"[..]));
    assert!(island.file(packaged::LOCK).is_some());
}

#[test]
fn a_packaged_folder_edited_after_the_fetch_is_refused_whole() {
    let core = tempfile::tempdir().unwrap();
    let dir = write_island(core.path());
    std::fs::write(dir.join("island.js"), b"alert(1)").unwrap();
    assert!(matches!(
        packaged::files(&dir, Some("island.css")),
        Err(PackagedError::Altered(name)) if name == "island.js"
    ));
    assert!(packaged::island(core.path(), &ISLAND).is_none());
}

#[cfg(unix)]
#[test]
fn a_link_in_a_packaged_folder_is_refused_not_followed() {
    let core = tempfile::tempdir().unwrap();
    let dir = write_island(core.path());
    let outside = core.path().join("outside.js");
    std::fs::write(&outside, b"export {}").unwrap();
    std::fs::remove_file(dir.join("island.js")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("island.js")).unwrap();
    assert!(matches!(
        packaged::files(&dir, None),
        Err(PackagedError::NotAFile(_))
    ));
}

#[test]
fn a_lock_naming_a_path_or_missing_its_stylesheet_is_refused() {
    let core = tempfile::tempdir().unwrap();
    let dir = write_island(core.path());
    assert!(matches!(
        packaged::files(&dir, Some("other.css")),
        Err(PackagedError::Unlocked(_))
    ));
    std::fs::write(
        dir.join(packaged::LOCK),
        "{\"files\": {\"../escape.js\": \"sha256-x\"}}",
    )
    .unwrap();
    assert!(matches!(
        packaged::files(&dir, None),
        Err(PackagedError::Path(_))
    ));
}

#[test]
fn no_folder_is_no_island_and_no_refusal() {
    let core = tempfile::tempdir().unwrap();
    assert!(matches!(
        packaged::files(&core.path().join("absent"), None),
        Err(PackagedError::Absent(_))
    ));
    assert!(packaged::island(core.path(), &ISLAND).is_none());
}

#[test]
fn the_list_s_packaged_entries_are_compiled_in() {
    assert!(
        architect_standard::PACKAGED
            .iter()
            .any(|p| p.id == "studio.transport" && p.island.is_some())
    );
}
