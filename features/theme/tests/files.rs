//! Theme files: last good, the watcher, the shipped-theme install rule.

// Test helpers fail the test: that is their assertion.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::time::{Duration, Instant};

use architect_theme::fs::{Installed, Shipped, ThemeDirs, install, watch};
use architect_theme::{Layer, LayerKind, Registry, ThemeChoice, ThemeStore};

const GOOD: &str =
    r##"{ "appearance": "dark", "palette": { "background": "#101820", "accent": "#ff8800" } }"##;
const BETTER: &str =
    r##"{ "appearance": "dark", "palette": { "background": "#101820", "accent": "#00aaff" } }"##;
const BROKEN: &str = r##"{ "appearance": "dark", "palette": { "accent": "#00aaff" "# }"##;

fn showing_mine(dir: &std::path::Path) -> ThemeStore {
    let store = ThemeStore::default();
    ThemeDirs::new().folder(dir).install_into(&store);
    store.set(
        LayerKind::Person,
        Layer::default().choice(ThemeChoice::One("mine".into())),
    );
    store
}

#[test]
fn a_broken_file_keeps_its_last_good_theme() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mine.json");
    std::fs::write(&file, GOOD).unwrap();
    let store = showing_mine(dir.path());
    assert_eq!(store.current().get("brand"), Some("#ff8800"));
    assert!(store.problems().is_empty());

    let before = store.revision();
    std::fs::write(&file, BROKEN).unwrap();
    assert!(!store.reload(), "nothing that shows changed");
    assert_eq!(store.revision(), before);
    assert_eq!(store.current().get("brand"), Some("#ff8800"), "last good");
    let problems = store.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].source.ends_with("mine.json"));

    // Fixed: the new colours show, the problem goes.
    std::fs::write(&file, BETTER).unwrap();
    assert!(store.reload());
    assert_eq!(store.current().get("brand"), Some("#00aaff"));
    assert!(store.problems().is_empty());
}

#[test]
fn a_file_overrides_a_builtin_of_the_same_id_and_extensions_add_theirs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nord-dark.json"), GOOD).unwrap();
    let ext = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ext.path().join("themes")).unwrap();
    std::fs::write(ext.path().join("themes/studio-night.json"), BETTER).unwrap();
    let registry = ThemeDirs::new()
        .folder(dir.path())
        .extension(
            "studio-pack",
            ext.path(),
            vec!["themes/studio-night.json".into()],
        )
        .load(&Registry::empty());
    assert_eq!(registry.get("nord-dark").unwrap().source.kind(), "file");
    let night = registry.get("studio-night").unwrap();
    assert_eq!(night.source.kind(), "extension");
    assert_eq!(
        night.source.to_string(),
        "studio-pack: themes/studio-night.json"
    );
}

#[test]
fn the_watcher_reloads_the_theme_showing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mine.json");
    std::fs::write(&file, GOOD).unwrap();
    let store = showing_mine(dir.path());
    let _watch = watch(&store, &ThemeDirs::new().folder(dir.path())).unwrap();
    std::fs::write(&file, BETTER).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while store.current().get("brand") != Some("#00aaff") {
        assert!(Instant::now() < deadline, "the watcher reloaded");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shipped_themes_upgrade_only_an_untouched_earlier_version() {
    const V1: &str = "{ \"label\": \"Lab\" }\n";
    const V2: &str = "{ \"label\": \"Lab 2\" }\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lab.json");
    let v1 = [Shipped {
        id: "lab",
        text: V1,
        previous: &[],
    }];
    let v2 = [Shipped {
        id: "lab",
        text: V2,
        previous: &[V1],
    }];

    assert_eq!(
        install(dir.path(), &v1).unwrap(),
        [("lab", Installed::Written)]
    );
    assert_eq!(
        install(dir.path(), &v1).unwrap(),
        [("lab", Installed::Current)]
    );
    assert_eq!(
        install(dir.path(), &v2).unwrap(),
        [("lab", Installed::Upgraded)]
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), V2);

    // The person's edit of v1 is theirs: v2 leaves it.
    std::fs::write(&path, V1).unwrap();
    let edited = "{ \"label\": \"Mine\" }\n";
    std::fs::write(&path, edited).unwrap();
    assert_eq!(
        install(dir.path(), &v2).unwrap(),
        [("lab", Installed::Kept)]
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}
