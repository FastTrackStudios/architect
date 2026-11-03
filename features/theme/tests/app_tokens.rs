//! An app's own tokens on top of architect's: namespaced names,
//! expressions over other tokens, numbers, provenance, misses, in-memory
//! themes, and a store a worker thread reads.

// Test helpers fail the test: that is their assertion.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use architect_theme::color::MAGENTA;
use architect_theme::{
    FromResolved, Layer, LayerKind, Origin, Projected, Registry, Resolved, Rgba, Scheme, Source,
    Theme, ThemeChoice, ThemeStore, css_name, resolve, valid_token,
};

fn file(id: &str, text: &str) -> Theme {
    Theme::parse(id, text, Source::File(format!("themes/{id}.json"))).unwrap()
}

/// What `mix($a, $b, t)` should give, from the theme's own values.
fn mixed(r: &Resolved, a: &str, b: &str, t: f32) -> String {
    r.color(a).mix(r.color(b), t).to_css()
}

/// Studio's shape: its defaults, shipped as an app layer of expressions.
fn studio_defaults() -> Layer {
    Layer::default()
        .token("arrange.item.edge", "mix($border, $foreground, 0.2)")
        .token("arrange.item.body-alpha", "0.42")
        .token("arrange.grid.bar", "alpha($ink, 0.18)")
        .token("arrange.cursor.play", "$warn")
}

#[test]
fn dotted_names_are_token_names() {
    for ok in ["ink-2", "arrange.item.edge", "editor.pitch-0", "a.b-c.d1"] {
        assert!(valid_token(ok), "{ok}");
    }
    for bad in ["", ".a", "a.", "a..b", "Arrange.x", "a.b_c", "a b", "$a"] {
        assert!(!valid_token(bad), "{bad}");
    }
    let t = file(
        "night",
        r##"{ "appearance": "dark", "overrides": { "arrange.item.edge": "#123456" } }"##,
    );
    assert_eq!(
        resolve(&t, &Registry::builtin()).get("arrange.item.edge"),
        Some("#123456")
    );
    let bad = Theme::parse(
        "bad",
        r##"{ "overrides": { "arrange..edge": "#fff" } }"##,
        Source::Builtin,
    )
    .unwrap_err();
    assert_eq!(bad[0].field.as_deref(), Some("overrides.arrange..edge"));
    assert!(bad[0].message.contains("separated by `.`"), "{}", bad[0]);
}

#[test]
fn css_keeps_architects_names_and_namespaces_are_opt_in() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::App,
        studio_defaults()
            .token("mixer.strip", "#222222")
            .token("ink.2", "#ffffff"),
    );
    let r = store.current();
    assert!(r.css_tokens().all(|(k, _)| !k.contains('.')));
    assert!(r.css_tokens().any(|(k, _)| k == "ink-2"));
    let css: BTreeMap<String, &str> = r.css_namespaced(&["arrange", "ink"]).into_iter().collect();
    assert_eq!(
        css.get("arrange-item-edge").copied(),
        r.get("arrange.item.edge")
    );
    assert!(!css.contains_key("mixer-strip"), "not asked for");
    assert!(
        !css.contains_key("ink-2"),
        "architect's `ink-2` wins over an app's `ink.2`"
    );
    assert_eq!(css_name("arrange.item.edge"), "arrange-item-edge");
}

#[test]
fn app_defaults_follow_the_palette() {
    let store = ThemeStore::default();
    store.set(LayerKind::App, studio_defaults());
    for theme in ["fasttrackstudio-dark", "nord-dark", "solarized-light"] {
        store.set(
            LayerKind::Pick,
            Layer::default().choice(ThemeChoice::One(theme.into())),
        );
        let r = store.current();
        assert_eq!(r.id(), theme);
        assert_eq!(
            r.get("arrange.item.edge").unwrap(),
            mixed(&r, "border", "foreground", 0.2),
            "{theme}"
        );
        assert_eq!(r.get("arrange.cursor.play"), r.get("warn"), "{theme}");
        assert_eq!(
            r.get("arrange.grid.bar").unwrap(),
            r.color("ink").alpha(0.18).to_css(),
            "{theme}"
        );
        assert!(store.problems().is_empty(), "{:?}", store.problems());
    }
}

#[test]
fn a_later_literal_wins_and_a_reference_sees_the_final_value() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::App,
        Layer::default()
            .token("arrange.bg", "$border")
            .token("arrange.row.a", "#111111")
            .token("arrange.row.b", "mix($arrange.bg, #ffffff, 50%)"),
    );
    // A higher layer's literal over a lower one's expression, and over a
    // token the expressions read.
    store.set(
        LayerKind::Person,
        Layer::default()
            .token("border", "#123456")
            .token("arrange.row.a", "#222222"),
    );
    let r = store.current();
    assert_eq!(r.get("arrange.bg"), Some("#123456"), "the person's border");
    assert_eq!(r.get("arrange.row.a"), Some("#222222"));
    assert_eq!(
        r.get("arrange.row.b").unwrap(),
        Rgba::parse("#123456")
            .unwrap()
            .mix(Rgba::parse("#ffffff").unwrap(), 0.5)
            .to_css(),
        "nested references see the final values"
    );
    // A higher layer's expression over a lower one's literal.
    store.set_token(LayerKind::Device, "arrange.row.a", "$arrange.bg");
    assert_eq!(store.current().get("arrange.row.a"), Some("#123456"));
}

#[test]
fn an_unknown_name_or_a_cycle_is_magenta_and_a_problem() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::App,
        Layer::default()
            .token("arrange.a", "$nope")
            .token("arrange.b", "mix($arrange.c, #ffffff, 0.5)")
            .token("arrange.c", "$arrange.b")
            .token("arrange.d", "alpha($arrange.c, 0.5)")
            .token("arrange.e", "mix($radius, #ffffff, 0.5)")
            .token("arrange.f", "alpha($ink, 2)")
            .token("arrange.g", "$arrange.g"),
    );
    let r = store.current();
    for name in [
        "arrange.a",
        "arrange.b",
        "arrange.c",
        "arrange.d",
        "arrange.e",
        "arrange.f",
        "arrange.g",
    ] {
        assert_eq!(r.color(name), MAGENTA, "{name}");
    }
    let problems = store.problems();
    let about = |name: &str| {
        problems
            .iter()
            .find(|p| p.field.as_deref() == Some(&format!("overrides.{name}")))
            .unwrap_or_else(|| panic!("no problem for {name}: {problems:?}"))
            .to_string()
    };
    assert_eq!(
        about("arrange.a"),
        "app layer: overrides.arrange.a: `$nope` names no token"
    );
    assert!(
        about("arrange.b").contains("a cycle: $arrange.b → $arrange.c → $arrange.b"),
        "{}",
        about("arrange.b")
    );
    assert!(
        about("arrange.c").contains("a cycle"),
        "{}",
        about("arrange.c")
    );
    assert!(
        about("arrange.d").contains("`$arrange.c` does not resolve"),
        "{}",
        about("arrange.d")
    );
    assert!(
        about("arrange.e").contains("\"0.625rem\" is not a colour"),
        "{}",
        about("arrange.e")
    );
    assert!(
        about("arrange.f").contains("not between 0 and 1"),
        "{}",
        about("arrange.f")
    );
    assert!(
        about("arrange.g").contains("a cycle: $arrange.g → $arrange.g"),
        "{}",
        about("arrange.g")
    );
    assert_eq!(problems.len(), 7, "{problems:?}");

    // Fixing it clears the problems.
    store.set(LayerKind::App, studio_defaults());
    assert!(store.problems().is_empty());
}

#[test]
fn a_theme_files_expressions_resolve_and_bad_ones_are_problems() {
    let mut registry = Registry::builtin();
    registry.insert(file(
        "night",
        r#"{ "extends": "fasttrackstudio-dark", "overrides": { "arrange.item.edge": "mix($rule, $brand, 0.5)" } }"#,
    ));
    let r = resolve(registry.get("night").unwrap(), &registry);
    assert_eq!(
        r.get("arrange.item.edge").unwrap(),
        mixed(&r, "rule", "brand", 0.5)
    );

    let bad = Theme::parse(
        "bad",
        r#"{ "overrides": { "arrange.item.edge": "mix($rule, $brand)" } }"#,
        Source::File("themes/bad.json".into()),
    )
    .unwrap_err();
    assert_eq!(bad.len(), 1);
    assert!(
        bad[0]
            .to_string()
            .starts_with("themes/bad.json: overrides.arrange.item.edge: `mix($rule, $brand)`: mix takes three arguments"),
        "{}",
        bad[0]
    );
}

#[test]
fn numbers_read_through_references() {
    let store = ThemeStore::default();
    store.set(
        LayerKind::App,
        studio_defaults()
            .token("arrange.item.radius", "4")
            .token("arrange.item.edge-width", "$arrange.item.radius"),
    );
    let r = store.current();
    assert!((r.number("arrange.item.body-alpha") - 0.42).abs() < f32::EPSILON);
    assert!((r.number("arrange.item.edge-width") - 4.0).abs() < f32::EPSILON);
    assert!(
        r.number("radius").is_nan(),
        "0.625rem is not a plain number"
    );
    assert!(r.number("arrange.nothing").is_nan());
    assert_eq!(r.misses(), ["arrange.nothing"], "absent only is a miss");
}

#[test]
fn misses_are_the_names_asked_for_and_not_there() {
    let r = ThemeStore::default().current();
    assert!(r.misses().is_empty());
    let _ = r.color("arrange.zzz");
    let _ = r.value("arrange.aaa");
    let _ = r.color("arrange.zzz");
    let _ = r.color("ink");
    assert_eq!(r.get("arrange.not-asked"), None, "`get` is the Option API");
    assert_eq!(r.misses(), ["arrange.aaa", "arrange.zzz"]);
    // A clone shares them; a new revision starts again.
    let copy = r.clone();
    let _ = copy.color("arrange.bbb");
    assert_eq!(r.misses().len(), 3);
    assert!(r.at_revision(9).misses().is_empty());
}

#[test]
fn source_says_who_set_each_value() {
    let mut registry = Registry::builtin();
    registry.insert(file(
        "night",
        r#"{ "extends": "fasttrackstudio-dark", "overrides": { "radius": "2px" } }"#,
    ));
    let store = ThemeStore::new(registry);
    store.set(
        LayerKind::Person,
        Layer::default().choice(ThemeChoice::One("night".into())),
    );
    store.set(LayerKind::App, studio_defaults().token("rule", "#333333"));
    let r = store.current();

    let derived = r.source("ink").unwrap();
    assert_eq!(derived.origin, Origin::Derived);
    assert_eq!(derived.id, "night");
    assert_eq!(
        derived.layer,
        Some(LayerKind::Person),
        "the person chose it"
    );
    assert!(!derived.is_reference());

    let own = r.source("radius").unwrap();
    assert_eq!((own.origin, own.id.as_str()), (Origin::Theme, "night"));
    let parents = r.source("primary").unwrap();
    assert_eq!(
        (parents.origin, parents.id.as_str()),
        (Origin::Theme, "fasttrackstudio-dark"),
        "a parent's override names the parent"
    );

    let edge = r.source("arrange.item.edge").unwrap();
    assert_eq!(edge.origin, Origin::Layer);
    assert_eq!(edge.layer, Some(LayerKind::App));
    assert_eq!(edge.id, "app");
    assert_eq!(
        edge.expression.as_deref(),
        Some("mix($border, $foreground, 0.2)")
    );
    assert!(edge.is_reference());
    assert!(!r.source("rule").unwrap().is_reference());
    assert_eq!(r.source("arrange.none"), None);
}

#[derive(Debug, PartialEq)]
struct Arrange {
    edge: Rgba,
    body_alpha: f32,
}

impl FromResolved for Arrange {
    fn from_resolved(r: &Resolved) -> Self {
        Self {
            edge: r.color("arrange.item.edge"),
            body_alpha: r.number("arrange.item.body-alpha"),
        }
    }
}

#[test]
fn a_projection_reads_the_apps_tokens_and_says_its_revision() {
    let store = ThemeStore::default();
    store.set(LayerKind::App, studio_defaults());
    let mut arrange = Projected::<Arrange>::new(&store.current());
    assert_eq!(arrange.revision(), store.revision());
    assert!((arrange.get().body_alpha - 0.42).abs() < f32::EPSILON);

    // Another slice's token: a new revision, the same slice.
    store.set_token(LayerKind::Person, "radius", "1px");
    assert!(!arrange.update(&store.current()));
    assert_eq!(arrange.revision(), store.revision());
    // What it reads, through the expression: it changes.
    store.set_token(LayerKind::Person, "border", "#ff0000");
    assert!(arrange.update(&store.current()));
}

#[test]
fn setting_the_same_expression_again_changes_nothing() {
    let store = ThemeStore::default();
    assert!(store.set(LayerKind::App, studio_defaults()));
    let r0 = store.revision();
    assert!(!store.set(LayerKind::App, studio_defaults()));
    assert!(!store.set_token(LayerKind::App, "arrange.cursor.play", "$warn"));
    assert_eq!(store.revision(), r0);
}

#[test]
fn the_store_is_send_and_sync_and_a_reader_sees_one_whole_theme() {
    fn shared<T: Send + Sync>() {}
    shared::<ThemeStore>();
    shared::<Resolved>();

    let store = ThemeStore::default();
    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (store, done) = (store.clone(), done.clone());
        std::thread::spawn(move || {
            let mut last = 0;
            let mut reads = 0u32;
            while !done.load(Ordering::Acquire) || reads == 0 {
                let polled = store.revision();
                assert!(polled >= last, "the revision only grows");
                last = polled;
                let r = store.current();
                assert!(
                    r.revision() >= polled,
                    "current is at least what was polled"
                );
                // Written together, so seen together.
                assert_eq!(r.get("arrange.a"), r.get("arrange.b"));
                reads = reads.saturating_add(1);
            }
        })
    };
    for i in 0..300u32 {
        let v = format!("#{:06x}", i.saturating_mul(4099) % 0xff_ffff);
        store.set(
            LayerKind::Person,
            Layer::default()
                .token("arrange.a", v.clone())
                .token("arrange.b", v),
        );
    }
    done.store(true, Ordering::Release);
    reader.join().unwrap();
    assert_eq!(store.revision(), store.current().revision());
}

#[test]
fn an_in_memory_theme_is_added_and_replaced_while_the_app_runs() {
    let tokens = |edge: &str| {
        BTreeMap::from([
            ("background".to_owned(), "#202020".to_owned()),
            ("arrange.item.edge".to_owned(), edge.to_owned()),
        ])
    };
    let imported = Theme::from_tokens("reaper-import", Scheme::Dark, tokens("#abcdef")).unwrap();
    assert_eq!(imported.source, Source::Memory);
    let store = ThemeStore::default();
    assert!(
        !store.insert_theme(imported.clone()),
        "added, but not showing: nothing that shows changed"
    );
    store.set(
        LayerKind::Pick,
        Layer::default().choice(ThemeChoice::One("reaper-import".into())),
    );
    let r = store.current();
    assert_eq!(r.id(), "reaper-import");
    assert_eq!(r.get("background"), Some("#202020"));
    assert_eq!(r.get("arrange.item.edge"), Some("#abcdef"));

    let r0 = store.revision();
    assert!(!store.insert_theme(imported), "the same again: no change");
    assert_eq!(store.revision(), r0);
    let next = Theme::from_tokens(
        "reaper-import",
        Scheme::Dark,
        tokens("mix($background, #ffffff, 0.5)"),
    )
    .unwrap();
    assert!(store.insert_theme(next));
    assert_eq!(store.revision(), r0 + 1);
    let r = store.current();
    assert_eq!(
        r.get("arrange.item.edge").unwrap(),
        r.color("background")
            .mix(architect_theme::color::WHITE, 0.5)
            .to_css()
    );

    // A reload (files read again) keeps what was built in memory.
    store.set_reloader(Arc::new(|_: &Registry| Registry::builtin()));
    assert!(!store.reload());
    assert_eq!(store.current().id(), "reaper-import");

    assert!(store.remove_theme("reaper-import"));
    assert_eq!(store.current().id(), "fasttrackstudio-dark", "falls back");

    let bad = Theme::from_tokens(
        "Bad Id",
        Scheme::Dark,
        BTreeMap::from([
            ("Bad".to_owned(), "#fff".to_owned()),
            ("a.b".to_owned(), "alpha($x)".to_owned()),
        ]),
    )
    .unwrap_err();
    let fields: Vec<_> = bad.iter().filter_map(|p| p.field.as_deref()).collect();
    assert_eq!(fields, ["id", "overrides.Bad", "overrides.a.b"]);
}
