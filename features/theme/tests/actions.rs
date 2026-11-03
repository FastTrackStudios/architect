//! The `THEME_*` actions, through the Action List as an agent calls them.

// Test helpers fail the test: that is their assertion.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use architect::action::{ActionList, Effect};
use architect_theme::actions::{self, ids};
use architect_theme::fs::ThemeDirs;
use architect_theme::{LayerKind, ThemeStore};

fn json(list: &ActionList, id: &str, args: &str) -> facet_value::Value {
    let out = list
        .run_json(id, args.as_bytes())
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    facet_json::from_slice(&out).unwrap()
}

fn field<'a>(v: &'a facet_value::Value, name: &str) -> &'a facet_value::Value {
    v.as_object()
        .and_then(|o| o.get(name))
        .unwrap_or_else(|| panic!("no {name} in {v:?}"))
}

fn text(v: &facet_value::Value, name: &str) -> String {
    field(v, name)
        .as_string()
        .map(|s| s.as_str().to_owned())
        .unwrap()
}

fn is_true(v: &facet_value::Value, name: &str) -> bool {
    field(v, name).as_bool().unwrap()
}

fn listed() -> (ActionList, ThemeStore) {
    let list = ActionList::new();
    let store = ThemeStore::default();
    actions::register(&list, &store).unwrap();
    (list, store)
}

#[test]
fn every_action_is_valid_and_marked() {
    let (list, _) = listed();
    list.validate().unwrap();
    list.probe_hostile_args().unwrap();
    for id in ids::ALL {
        let info = list.infos().into_iter().find(|i| i.id == *id).unwrap();
        let reads = matches!(*id, ids::LIST | ids::GET | ids::PROBLEMS | ids::INSPECT);
        assert_eq!(
            info.effect,
            if reads {
                Effect::ReadOnly
            } else {
                Effect::Mutating
            },
            "{id}"
        );
    }
}

#[test]
fn set_get_preview_and_tokens_round_trip() {
    let (list, store) = listed();
    let all = json(&list, ids::LIST, "{}");
    assert!(field(&all, "themes").as_array().unwrap().len() >= 40);
    assert_eq!(text(&all, "showing"), "fasttrackstudio-dark");

    let set = json(&list, ids::SET, r#"{"dark":"dracula-dark"}"#);
    assert!(is_true(&set, "changed"));
    let revision = field(&set, "revision")
        .as_number()
        .unwrap()
        .to_u64()
        .unwrap();
    assert_eq!(revision, store.revision());
    let again = json(&list, ids::SET, r#"{"dark":"dracula-dark"}"#);
    assert!(
        !is_true(&again, "changed"),
        "a no-op answers changed: false"
    );

    let got = json(&list, ids::GET, r#"{"tokens":["brand","ink-*"]}"#);
    assert_eq!(text(&got, "theme"), "dracula-dark");
    assert_eq!(text(&got, "light"), "fasttrackstudio-light");
    assert_eq!(text(&got, "appearance"), "system");
    let tokens = field(&got, "tokens").as_object().unwrap();
    let names: Vec<&str> = tokens.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["brand", "ink-2", "ink-3"]);

    let set = json(&list, ids::SET, r#"{"appearance":"light"}"#);
    assert!(is_true(&set, "changed"));
    assert_eq!(store.current().id(), "fasttrackstudio-light");

    assert!(is_true(
        &json(&list, ids::PREVIEW, r#"{"id":"nord-dark"}"#),
        "changed"
    ));
    assert_eq!(store.current().id(), "nord-dark");
    assert!(is_true(&json(&list, ids::PREVIEW_END, "{}"), "changed"));
    assert_eq!(store.current().id(), "fasttrackstudio-light");

    assert!(is_true(
        &json(
            &list,
            ids::TOKEN_SET,
            r##"{"name":"primary","value":"#123456"}"##
        ),
        "changed"
    ));
    assert_eq!(store.current().get("primary"), Some("#123456"));
    assert_eq!(
        store.layer(LayerKind::Person).overrides.len(),
        1,
        "in the person's layer"
    );
    assert!(is_true(
        &json(&list, ids::TOKEN_RESET, r#"{"name":"primary"}"#),
        "changed"
    ));
    assert!(store.layer(LayerKind::Person).overrides.is_empty());

    assert!(list.run_json(ids::SET, br#"{"one":"nope"}"#).is_err());
    assert!(list.run_json(ids::SET, b"{}").is_err());
    assert!(
        list.run_json(ids::TOKEN_SET, br#"{"name":"Bad Name","value":"1"}"#)
            .is_err()
    );
    assert!(list.run_json(ids::PREVIEW, br#"{"id":"nope"}"#).is_err());
}

#[test]
fn reload_and_problems_report_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let (list, store) = listed();
    ThemeDirs::new().folder(dir.path()).install_into(&store);
    std::fs::write(dir.path().join("broken.json"), r#"{ "pallete": {} }"#).unwrap();
    let reload = json(&list, ids::RELOAD, "{}");
    assert!(!is_true(&reload, "changed"));
    let problems = json(&list, ids::PROBLEMS, "{}");
    let problems = field(&problems, "problems").as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert!(text(&problems[0], "text").contains("broken.json"));
}

#[test]
fn inspect_and_tokens_in_any_layer() {
    let (list, store) = listed();
    let set = json(
        &list,
        ids::TOKEN_SET,
        r#"{"name":"arrange.item.edge","value":"mix($border, $foreground, 0.2)","layer":"app"}"#,
    );
    assert!(is_true(&set, "changed"));
    assert!(
        store
            .layer(LayerKind::App)
            .overrides
            .contains_key("arrange.item.edge")
    );
    assert!(store.layer(LayerKind::Person).overrides.is_empty());

    let got = json(&list, ids::INSPECT, r#"{"prefix":"arrange."}"#);
    let tokens = field(&got, "tokens").as_array().unwrap();
    assert_eq!(tokens.len(), 1);
    let edge = &tokens[0];
    assert_eq!(text(edge, "name"), "arrange.item.edge");
    assert_eq!(
        text(edge, "value"),
        store.current().value("arrange.item.edge")
    );
    assert_eq!(text(edge, "origin"), "layer");
    assert_eq!(text(edge, "layer"), "app");
    assert!(is_true(edge, "reference"));
    assert_eq!(text(edge, "expression"), "mix($border, $foreground, 0.2)");

    let all = json(&list, ids::INSPECT, "{}");
    let all = field(&all, "tokens").as_array().unwrap();
    let ink = all.iter().find(|t| text(t, "name") == "ink").unwrap();
    assert_eq!(text(ink, "origin"), "derived");
    assert_eq!(text(ink, "id"), "fasttrackstudio-dark");
    assert!(!is_true(ink, "reference"));

    // An unknown reference: set, drawn magenta, and in THEME_PROBLEMS.
    json(
        &list,
        ids::TOKEN_SET,
        r#"{"name":"arrange.row.a","value":"$nope"}"#,
    );
    assert_eq!(store.current().get("arrange.row.a"), Some("#ff00ff"));
    let problems = json(&list, ids::PROBLEMS, "{}");
    let problems = field(&problems, "problems").as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        text(&problems[0], "text"),
        "person layer: overrides.arrange.row.a: `$nope` names no token"
    );

    let reset = json(&list, ids::TOKEN_RESET, r#"{"layer":"app"}"#);
    assert!(is_true(&reset, "changed"));
    assert!(store.layer(LayerKind::App).overrides.is_empty());

    for bad in [
        r#"{"name":"a.b","value":"mix($a)"}"#,
        r#"{"name":"a..b","value":"1"}"#,
        r#"{"name":"a","value":"1","layer":"builtin"}"#,
        r#"{"name":"a","value":"1","layer":"nowhere"}"#,
    ] {
        assert!(
            list.run_json(ids::TOKEN_SET, bad.as_bytes()).is_err(),
            "{bad}"
        );
    }
}
