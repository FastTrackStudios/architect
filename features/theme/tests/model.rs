//! The model: resolving, the cascade, the store, strict files.

// Test helpers fail the test: that is their assertion.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect_theme::color::{MAGENTA, contrast};
use architect_theme::{
    Appearance, Layer, LayerKind, Registry, Rgba, Scheme, Source, Theme, ThemeChoice, ThemeStore,
    derive, names, resolve,
};

fn theme(id: &str, text: &str) -> Theme {
    Theme::parse(id, text, Source::File(format!("themes/{id}.json"))).unwrap()
}

#[test]
fn every_builtin_reads_and_resolves_the_same_twice() {
    let registry = Registry::builtin();
    assert_eq!(registry.problems(), vec![], "built-in files read");
    assert_eq!(registry.len(), architect_theme::builtin::FILES.len());
    for t in registry.iter() {
        let (a, b) = (resolve(t, &registry), resolve(t, &registry));
        assert_eq!(a, b, "{} resolves deterministically", t.id);
        let have: BTreeSet<&str> = a.tokens().keys().map(String::as_str).collect();
        for name in names::all() {
            assert!(have.contains(name), "{} has {name}", t.id);
        }
        assert_eq!(
            Some(a.scheme()),
            t.file.appearance,
            "{} is the scheme it says",
            t.id
        );
    }
}

#[test]
fn derived_ink_reads_on_every_shell_fill() {
    // Before any override: what derivation alone guarantees, for every
    // built-in palette in its own scheme.
    for t in Registry::builtin().iter() {
        let scheme = t.file.appearance.unwrap();
        let tokens = derive(&t.file.palette, scheme);
        let c = |n: &str| Rgba::parse(&tokens[n]).unwrap();
        for ground in ["desk", "sheet", "title-bar", "pill"] {
            for ink in ["ink-2", "ink-3"] {
                let ratio = contrast(c(ink), c(ground));
                assert!(ratio >= 4.5, "{}: {ink} on {ground} is {ratio:.2}", t.id);
            }
        }
        let wash = c("brand-soft").over(c("pill"));
        assert!(
            contrast(c("ink-3"), wash) >= 4.5,
            "{}: ink-3 on the brand wash",
            t.id
        );
        assert!(
            contrast(c("muted-foreground"), c("background")) >= 4.5,
            "{}",
            t.id
        );
        assert!(
            contrast(c("primary-foreground"), c("primary")) >= 3.0,
            "{}",
            t.id
        );
    }
}

#[test]
fn the_default_is_fasttrackstudio_dark_with_the_shells_colours() {
    let store = ThemeStore::default();
    let now = store.current();
    assert_eq!(now.id(), "fasttrackstudio-dark");
    assert_eq!(now.scheme(), Scheme::Dark);
    for (name, value) in [
        ("desk", "#09090b"),
        ("sheet", "#0e0e11"),
        ("title-bar", "#17171b"),
        ("pill", "#1a1a1f"),
        ("rule", "#2b2b31"),
        ("ink", "#e4e4e7"),
        ("ink-3", "#8f8f99"),
        ("brand", "#2dd4bf"),
        ("brand-ink", "#04201c"),
        ("danger", "#f87171"),
        ("primary", "#818cf8"),
    ] {
        assert_eq!(now.get(name), Some(value), "{name}");
    }
}

#[test]
fn an_unknown_token_is_magenta() {
    let now = ThemeStore::default().current();
    assert_eq!(now.value("no-such-token"), "#ff00ff");
    assert_eq!(now.color("no-such-token"), MAGENTA);
    assert_eq!(now.get("no-such-token"), None);
    // A value that is not a colour, asked for as one.
    assert_eq!(now.color("radius"), MAGENTA);
}

#[test]
fn extends_takes_the_parents_palette_and_overrides() {
    let mut registry = Registry::builtin();
    registry.insert(theme(
        "night",
        r##"{ "extends": "fasttrackstudio-dark", "palette": { "accent": "#f59e0b" }, "overrides": { "radius": "2px" } }"##,
    ));
    let r = resolve(registry.get("night").unwrap(), &registry);
    assert_eq!(r.scheme(), Scheme::Dark);
    assert_eq!(r.get("brand"), Some("#f59e0b"));
    assert_eq!(r.get("radius"), Some("2px"));
    // The parent's overrides still hold.
    assert_eq!(r.get("sheet"), Some("#0e0e11"));
}

#[test]
fn every_layer_wins_over_the_ones_below_it() {
    let mut registry = Registry::builtin();
    for (i, _) in LayerKind::ALL.iter().enumerate() {
        registry.insert(theme(
            &format!("layer-{i}"),
            r##"{ "appearance": "dark", "palette": { "background": "#101010" } }"##,
        ));
    }
    let store = ThemeStore::new(registry);
    for (i, kind) in LayerKind::ALL.into_iter().enumerate().skip(1) {
        let id = format!("layer-{i}");
        store.set(
            kind,
            Layer::default()
                .choice(ThemeChoice::One(id.clone()))
                .appearance(if i % 2 == 0 {
                    Appearance::Light
                } else {
                    Appearance::Dark
                })
                .token("radius", format!("{i}px")),
        );
        let composed = store.composed();
        assert_eq!(composed.theme, id, "{kind:?}'s choice wins");
        assert_eq!(
            store.current().get("radius"),
            Some(format!("{i}px").as_str())
        );
        assert_eq!(
            composed.appearance,
            if i % 2 == 0 {
                Appearance::Light
            } else {
                Appearance::Dark
            },
            "{kind:?}'s appearance wins"
        );
    }
    // Taking the top layers away, one at a time, shows each one below.
    for (i, kind) in LayerKind::ALL.into_iter().enumerate().skip(2).rev() {
        store.set(kind, Layer::default());
        assert_eq!(store.composed().theme, format!("layer-{}", i - 1));
    }
}

#[test]
fn appearance_picks_the_half_and_a_pick_replaces_only_its_own() {
    let store = ThemeStore::default();
    assert_eq!(store.current().id(), "fasttrackstudio-dark");
    assert!(store.set_system(Scheme::Light));
    assert_eq!(store.current().id(), "fasttrackstudio-light");
    assert!(store.pick("nord-dark").unwrap());
    // Light still shows the light half.
    assert_eq!(store.current().id(), "fasttrackstudio-light");
    assert_eq!(
        store.composed().choice,
        ThemeChoice::pair("fasttrackstudio-light", "nord-dark")
    );
    assert!(store.set_system(Scheme::Dark));
    assert_eq!(store.current().id(), "nord-dark");
    assert!(store.pick("missing").is_err());
}

#[test]
fn set_is_idempotent_and_tells_listeners_once() {
    let store = ThemeStore::default();
    let heard = Arc::new(AtomicUsize::new(0));
    let h = heard.clone();
    let sub = store.subscribe(move |_| {
        h.fetch_add(1, Ordering::SeqCst);
    });
    let r0 = store.revision();
    let layer = Layer::default().choice(ThemeChoice::One("dracula-dark".into()));
    assert!(store.set(LayerKind::Person, layer.clone()));
    assert_eq!(store.revision(), r0 + 1);
    assert!(
        !store.set(LayerKind::Person, layer),
        "the same again changes nothing"
    );
    assert!(!store.set_system(Scheme::Dark), "already dark");
    assert!(!store.set_registry(Registry::builtin()), "the same themes");
    assert_eq!(store.revision(), r0 + 1);
    assert_eq!(heard.load(Ordering::SeqCst), 1);
    // A layer that changes nothing that shows: no revision either.
    assert!(!store.set(LayerKind::Device, Layer::default()));
    drop(sub);
    assert!(store.set_token(LayerKind::Person, "radius", "1px"));
    assert_eq!(heard.load(Ordering::SeqCst), 1, "unsubscribed");
}

#[test]
fn preview_shows_over_everything_and_ends() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::Pick,
        Layer::default().choice(ThemeChoice::One("gruvbox-dark".into())),
    );
    assert!(store.preview("solarized-light").unwrap());
    assert_eq!(store.current().id(), "solarized-light");
    assert!(store.end_preview());
    assert_eq!(store.current().id(), "gruvbox-dark");
}

#[test]
fn a_chosen_theme_that_is_not_there_falls_back_and_says_so() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::Person,
        Layer::default().choice(ThemeChoice::One("gone".into())),
    );
    assert_eq!(store.current().id(), "fasttrackstudio-dark");
    let problems = store.problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].theme, "gone");
}

#[test]
fn strict_parsing_names_the_file_and_the_field() {
    let source = Source::File("themes/bad.json".into());
    let unknown = Theme::parse("bad", r#"{ "palete": {} }"#, source.clone()).unwrap_err();
    assert_eq!(unknown.len(), 1);
    let text = unknown[0].to_string();
    assert!(text.starts_with("themes/bad.json"), "{text}");
    assert!(text.contains("palete"), "{text}");

    let deep = Theme::parse(
        "bad",
        r##"{ "palette": { "acent": "#fff" } }"##,
        source.clone(),
    )
    .unwrap_err();
    assert!(deep[0].to_string().contains("acent"), "{}", deep[0]);

    let colour = Theme::parse(
        "bad",
        r#"{ "palette": { "accent": "teal-ish" }, "overrides": { "Bad Name": "1" } }"#,
        source.clone(),
    )
    .unwrap_err();
    let fields: Vec<_> = colour.iter().filter_map(|p| p.field.as_deref()).collect();
    assert_eq!(fields, ["palette.accent", "overrides.Bad Name"]);
    assert_eq!(
        colour[0].to_string(),
        "themes/bad.json: palette.accent: \"teal-ish\" is not a colour"
    );

    let scheme = Theme::parse("bad", r#"{ "appearance": "dim" }"#, source.clone()).unwrap_err();
    assert!(scheme[0].to_string().contains("dim"), "{}", scheme[0]);

    assert!(Theme::parse("Bad_Id", "{}", source.clone()).is_err());
    assert!(
        Theme::parse("ok", "// a comment\n{}", source).is_ok(),
        "comments are allowed"
    );
}

#[test]
fn a_missing_or_looping_parent_is_a_problem() {
    let mut registry = Registry::empty();
    registry.insert(theme("a", r#"{ "extends": "b" }"#));
    registry.insert(theme("b", r#"{ "extends": "a" }"#));
    registry.insert(theme("c", r#"{ "extends": "nowhere" }"#));
    let problems = registry.problems();
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems.iter().any(|p| p.message.contains("nowhere")));
    // Still resolves (the loop broken, the parent left out).
    let _ = resolve(registry.get("a").unwrap(), &registry);
}
