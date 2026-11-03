//! What an owner other than the app (an extension) contributes: themes
//! and its own tokens' defaults, in while it is loaded, out when it goes,
//! as a diff that moves the revision only when what shows changes.

// Test helpers fail the test: that is their assertion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use architect_theme::{Contribution, Layer, LayerKind, Origin, Problem, Source, Theme, ThemeStore};

fn neon() -> Theme {
    Theme::parse(
        "clicker-neon",
        r##"{ "label": "Clicker Neon", "appearance": "dark",
              "palette": { "background": "#0b0420", "accent": "#ff2bd6" } }"##,
        Source::Extension {
            id: "lab.clicker".to_owned(),
            file: "themes/clicker-neon.json".to_owned(),
        },
    )
    .unwrap()
}

fn tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "clicker.face".to_owned(),
            "mix($card, $brand, 0.12)".to_owned(),
        ),
        ("clicker.count".to_owned(), "$brand".to_owned()),
    ])
}

fn clicker() -> Contribution {
    Contribution {
        themes: vec![neon()],
        failed: Vec::new(),
        tokens: tokens(),
    }
}

#[test]
fn a_contributed_theme_is_listed_and_picked_and_goes_with_its_owner() {
    let store = ThemeStore::default();
    let before = store.revision();
    // A theme nobody shows, and tokens: the theme's id is known, and the
    // tokens resolve in the theme showing, so the revision moves once.
    assert!(store.contribute("lab.clicker", clicker()));
    let listed = store.themes();
    let info = listed.iter().find(|t| t.id == "clicker-neon").unwrap();
    assert_eq!(info.label, "Clicker Neon");
    assert_eq!(info.source.label(), "extension:lab.clicker");
    assert_eq!(store.revision(), before + 1);
    let r = store.current();
    assert_eq!(
        r.get("clicker.count"),
        r.get("brand"),
        "a default follows the palette"
    );
    assert_eq!(
        r.source("clicker.face").unwrap().layer,
        Some(LayerKind::Extension)
    );
    assert_eq!(r.source("clicker.face").unwrap().origin, Origin::Layer);

    // The same contribution again: nothing changed, the revision stays.
    let at = store.revision();
    assert!(!store.contribute("lab.clicker", clicker()));
    assert_eq!(store.revision(), at);

    // Picked, it shows.
    assert!(store.pick("clicker-neon").unwrap());
    assert_eq!(store.current().id(), "clicker-neon");
    assert_eq!(store.current().get("brand"), Some("#ff2bd6"));

    // The owner goes: its theme and tokens with it; the choice falls back
    // to the built-in default, with the problem said.
    assert!(store.withdraw("lab.clicker"));
    assert!(store.themes().iter().all(|t| t.id != "clicker-neon"));
    assert_eq!(store.current().id(), "fasttrackstudio-dark");
    assert_eq!(store.current().get("clicker.face"), None);
    assert!(
        store
            .problems()
            .iter()
            .any(|p| p.theme == "clicker-neon" && p.field.as_deref() == Some("choice"))
    );
    // Withdrawing again changes nothing.
    let at = store.revision();
    assert!(!store.withdraw("lab.clicker"));
    assert_eq!(store.revision(), at);
}

#[test]
fn a_reload_is_a_diff_and_a_broken_file_keeps_the_last_good_theme() {
    let store = ThemeStore::default();
    store.contribute("lab.clicker", clicker());
    assert!(store.pick("clicker-neon").unwrap());
    let shown = store.current();

    // The file broke on a save: a problem, and the theme stays as it was.
    let broken = Contribution {
        themes: Vec::new(),
        failed: vec![(
            "clicker-neon".to_owned(),
            vec![
                Problem::new(
                    "clicker-neon",
                    &neon().source,
                    "\"teal-ish\" is not a colour",
                )
                .at("palette.accent"),
            ],
        )],
        tokens: tokens(),
    };
    assert!(!store.contribute("lab.clicker", broken));
    assert_eq!(store.current(), shown);
    assert!(
        store
            .problems()
            .iter()
            .any(|p| p.field.as_deref() == Some("palette.accent"))
    );

    // A token dropped in the next version goes; one kept stays.
    let fewer = Contribution {
        themes: vec![neon()],
        failed: Vec::new(),
        tokens: BTreeMap::from([("clicker.count".to_owned(), "$brand".to_owned())]),
    };
    assert!(store.contribute("lab.clicker", fewer));
    assert_eq!(store.current().get("clicker.face"), None);
    assert!(store.current().get("clicker.count").is_some());
    assert!(
        store
            .problems()
            .iter()
            .all(|p| p.field.as_deref() != Some("palette.accent"))
    );
}

#[test]
fn the_app_and_the_person_say_the_last_word_over_an_extensions_defaults() {
    let store = ThemeStore::default();
    store.contribute("lab.clicker", clicker());
    store.set(
        LayerKind::App,
        Layer::default().token("clicker.count", "$danger"),
    );
    let r = store.current();
    assert_eq!(r.get("clicker.count"), r.get("danger"));
    store.set_token(LayerKind::Person, "clicker.count", "#123456");
    assert_eq!(store.current().get("clicker.count"), Some("#123456"));
}

#[test]
fn an_id_or_a_token_another_has_is_left_out_and_said() {
    let store = ThemeStore::default();
    // A built-in id: the app's stays.
    let mut taken = neon();
    taken.id = "nord-dark".to_owned();
    store.contribute(
        "lab.other",
        Contribution {
            themes: vec![taken],
            failed: Vec::new(),
            tokens: BTreeMap::from([("clicker.face".to_owned(), "#000000".to_owned())]),
        },
    );
    store.contribute("lab.clicker", clicker());
    let nord = store.registry().get("nord-dark").cloned().unwrap();
    assert_eq!(nord.source, Source::Builtin);
    let problems = store.problems();
    assert!(
        problems
            .iter()
            .any(|p| p.theme == "nord-dark" && p.field.as_deref() == Some("id"))
    );
    // Owners go in by id: `lab.clicker` first, so its token holds.
    assert!(problems.iter().any(|p| p.theme == "lab.other"
        && p.field.as_deref() == Some("tokens.clicker.face")
        && p.message.contains("lab.clicker")));
    assert_ne!(store.current().get("clicker.face"), Some("#000000"));
}

#[test]
fn a_reload_of_the_app_files_keeps_the_contributions() {
    let store = ThemeStore::default();
    store.contribute("lab.clicker", clicker());
    store.set_reloader(std::sync::Arc::new(|_| {
        architect_theme::Registry::builtin()
    }));
    store.reload();
    assert!(store.registry().contains("clicker-neon"));
}
