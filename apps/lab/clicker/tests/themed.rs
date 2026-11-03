//! The clicker as the lab's themed extension: its package's theme (Clicker
//! Neon) is in `THEME_LIST` and picked by `THEME_SET`, its `clicker.*`
//! tokens follow the palette (and its own theme restyles them), and its
//! `ImGui` window (`window.luau`) draws in the theme, following a switch.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use architect::action::{ActionCaller, ActionList};
use architect_canvas::imgui::Input;
use architect_canvas::luau::Session;
use architect_canvas::{Color, Shape, Size};
use architect_extensions::Package;
use architect_extensions::host::theme_contribution;
use architect_scripting::luau::Runtime;
use architect_theme::ThemeStore;
use futures::executor::block_on;
use serde_json::Value;

fn clicker() -> Package {
    Package::read(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn run(list: &ActionList, id: &str, args: &str) -> Value {
    let out = block_on(list.run_json_async(id, args.as_bytes())).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn its_theme_is_in_theme_list_and_theme_set_shows_it() {
    let store = ThemeStore::default();
    let list = ActionList::new();
    architect_theme::actions::register(&list, &store).unwrap();
    let package = clicker();
    let contribution = theme_contribution(&package);
    assert!(contribution.failed.is_empty(), "{:?}", contribution.failed);
    store.contribute(&package.manifest.id, contribution);

    let listed = run(&list, "THEME_LIST", "");
    let neon = listed["themes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "clicker-neon")
        .expect("Clicker Neon is listed");
    assert_eq!(neon["label"], "Clicker Neon");
    assert_eq!(neon["source"], "extension:lab.clicker");
    assert_eq!(neon["problems"], serde_json::json!([]));

    // Under the default theme, the clicker's tokens are its defaults.
    let r = store.current();
    assert_eq!(r.get("clicker.count"), r.get("brand"));
    assert!((r.number("clicker.radius") - 10.0).abs() < f32::EPSILON);

    run(&list, "THEME_SET", r#"{"one":"clicker-neon"}"#);
    let r = store.current();
    assert_eq!(r.id(), "clicker-neon");
    assert_eq!(r.get("brand"), Some("#ff2bd6"));
    // Its theme says the count is the info colour.
    assert_eq!(r.get("clicker.count"), r.get("info"));
    assert!(store.problems().is_empty(), "{:?}", store.problems());
}

#[test]
fn its_imgui_window_draws_in_the_theme_and_follows_a_switch() {
    let store = ThemeStore::default();
    store.contribute("lab.clicker", theme_contribution(&clicker()));
    let source =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("window.luau")).unwrap();
    let (report, session) = block_on(Session::start_themed(
        &Runtime::new(ActionList::new()),
        &ActionCaller::app(),
        &source,
        Value::Null,
        "Clicker window",
        &store.current(),
    ));
    report.result.unwrap();
    let mut session = session.unwrap();
    let input = Input {
        sizes: [("clicker/Window".to_owned(), Size::new(320.0, 200.0))].into(),
        ..Input::default()
    };
    let colours = |s: &Session| {
        let w = s.output().window("clicker/Window").unwrap().clone();
        let shapes = w.draw.shapes().to_vec();
        let ground = match &shapes[0] {
            Shape::Rect { color, .. } => *color,
            other => panic!("the ground first: {other:?}"),
        };
        let count = shapes
            .iter()
            .find_map(|s| match s {
                Shape::Text { text, color, .. } if text.ends_with("presses") => Some(*color),
                _ => None,
            })
            .unwrap();
        let has = move |c: Color| {
            shapes
                .iter()
                .any(|s| matches!(s, Shape::Rect { color, .. } if *color == c))
        };
        (ground, count, has)
    };
    let packed = |r: &architect_theme::Resolved, name: &str| Color(r.color(name).packed());

    block_on(session.frame(input.clone())).result.unwrap();
    let dark = store.current();
    let (ground, count, has) = colours(&session);
    assert_eq!(ground, packed(&dark, "sheet"));
    assert_eq!(count, packed(&dark, "clicker.count"));
    assert!(
        has(packed(&dark, "clicker.face")),
        "its button is the clicker's face"
    );

    store.pick("clicker-neon").unwrap();
    let neon = store.current();
    assert!(session.set_theme(&neon));
    block_on(session.frame(input)).result.unwrap();
    let (ground, count, has) = colours(&session);
    assert_eq!(ground, packed(&neon, "sheet"));
    assert_ne!(ground, packed(&dark, "sheet"));
    assert_eq!(count, packed(&neon, "info"));
    assert!(has(packed(&neon, "clicker.face")));
}
