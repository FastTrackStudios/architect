//! An extension's `[[themes]]` and `[tokens]` go into the app's theme
//! store as it loads (`Host::with_themes`), as a diff on a reload, and
//! out as it unloads; a broken theme file is a theme problem, never a
//! failed load; a development copy's saved theme file shows at once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::{Manifest, ManifestError, Origin, Package};
use architect_settings::Registry;
use architect_theme::{LayerKind, ThemeStore};

const MANIFEST: &str = r#"
id = "lab.glow"
name = "Glow"
version = "0.1.0"
kind = "data"

[[themes]]
path = "themes/glow-night.json"

[tokens]
"glow.face" = "mix($card, $brand, 0.12)"
"glow.count" = "$brand"
"#;

const NIGHT: &str = r##"{
  "label": "Glow Night",
  "appearance": "dark",
  "palette": { "background": "#0b0420", "accent": "#ff2bd6" }
}"##;

fn host() -> (Host, ThemeStore, ActionList) {
    let store = ThemeStore::default();
    let list = ActionList::new();
    architect_theme::actions::register(&list, &store).unwrap();
    let host = Host::new(list.clone(), SharedSections::new(Registry::default()))
        .with_themes(store.clone());
    (host, store, list)
}

fn theme_list(list: &ActionList) -> serde_json::Value {
    let out = futures::executor::block_on(list.run_json_async("THEME_LIST", b"")).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn its_themes_and_tokens_load_and_unload_with_it() {
    let (host, store, list) = host();
    let package = Package::embedded(MANIFEST, &[("themes/glow-night.json", NIGHT)]).unwrap();
    let before = store.revision();
    host.load(&package, Origin::Installed).unwrap();

    let listed = theme_list(&list);
    let glow = listed["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "glow-night")
        .expect("THEME_LIST has the extension's theme");
    assert_eq!(glow["label"], "Glow Night");
    assert_eq!(glow["source"], "extension:lab.glow");
    assert_eq!(glow["appearance"], "dark");
    // Its tokens resolve in the theme showing: one real change.
    let r = store.current();
    assert_eq!(r.get("glow.count"), r.get("brand"));
    assert_eq!(
        r.source("glow.face").unwrap().layer,
        Some(LayerKind::Extension)
    );
    assert_eq!(store.revision(), before + 1);

    // Loaded again, the same: the store does not move.
    let at = store.revision();
    host.load(&package, Origin::Installed).unwrap();
    assert_eq!(store.revision(), at);

    // THEME_SET picks it like any other.
    futures::executor::block_on(list.run_json_async("THEME_SET", br#"{"one":"glow-night"}"#))
        .unwrap();
    assert_eq!(store.current().id(), "glow-night");

    assert!(host.unload("lab.glow"));
    assert!(store.themes().iter().all(|t| t.id != "glow-night"));
    assert_eq!(store.current().get("glow.face"), None);
    assert_eq!(store.current().id(), "fasttrackstudio-dark");
}

#[test]
fn a_broken_theme_file_is_a_problem_not_a_failed_load() {
    let (host, store, list) = host();
    let package = Package::embedded(
        MANIFEST,
        &[(
            "themes/glow-night.json",
            r#"{ "palette": { "accent": "teal-ish" } }"#,
        )],
    )
    .unwrap();
    host.load(&package, Origin::Installed).unwrap();
    assert!(host.list().info("THEME_LIST").is_some());
    let out = futures::executor::block_on(list.run_json_async("THEME_PROBLEMS", b"")).unwrap();
    let problems: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let found = problems["problems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["theme"] == "glow-night")
        .expect("the broken file is a problem");
    assert_eq!(found["field"], "palette.accent");
    assert_eq!(found["source"], "lab.glow: themes/glow-night.json");
    // Its tokens still went in.
    assert!(store.current().get("glow.face").is_some());
}

#[test]
fn a_development_copys_saved_theme_shows_at_once() {
    let (host, store, _) = host();
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("glow");
    std::fs::create_dir_all(pkg.join("themes")).unwrap();
    std::fs::write(pkg.join("extension.toml"), MANIFEST).unwrap();
    std::fs::write(pkg.join("themes/glow-night.json"), NIGHT).unwrap();
    let extensions = Extensions::new(host, Installs::new(dir.path().join("installs")));
    let _ = extensions.dev(&pkg);
    assert!(store.pick("glow-night").unwrap());
    assert_eq!(store.current().get("brand"), Some("#ff2bd6"));
    // First poll remembers the folder.
    let _ = extensions.reload_changed();

    let at = store.revision();
    std::fs::write(
        pkg.join("themes/glow-night.json"),
        NIGHT.replace("#ff2bd6", "#22d3ee"),
    )
    .unwrap();
    let events = extensions.reload_changed();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(store.current().get("brand"), Some("#22d3ee"));
    assert_eq!(store.revision(), at + 1);

    // Saved broken: the last good theme stays showing, the problem said.
    std::fs::write(pkg.join("themes/glow-night.json"), "{ not json").unwrap();
    let _ = extensions.reload_changed();
    assert_eq!(store.current().get("brand"), Some("#22d3ee"));
    assert!(store.problems().iter().any(|p| p.theme == "glow-night"));
}

#[test]
fn a_token_outside_its_namespace_or_unreadable_is_refused() {
    let bad = |tokens: &str| {
        Manifest::parse(&format!(
            "id = \"lab.glow\"\nname = \"G\"\nversion = \"0.1.0\"\nkind = \"data\"\n[tokens]\n{tokens}"
        ))
    };
    assert_eq!(
        bad("\"glow.face\" = \"$card\"").unwrap().token_namespace(),
        "glow"
    );
    assert!(matches!(
        bad("\"other.face\" = \"$card\""),
        Err(ManifestError::BadToken { .. })
    ));
    assert!(matches!(
        bad("\"glow.Face\" = \"$card\""),
        Err(ManifestError::BadToken { .. })
    ));
    assert!(matches!(
        bad("\"glow.face\" = \"mix($card\""),
        Err(ManifestError::BadToken { .. })
    ));
}
