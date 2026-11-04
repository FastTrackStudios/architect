//! `theme` for scripts: a frozen snapshot of the theme showing, keyed by
//! its revision and made again only when the revision moves; a live
//! script's subscribers told once per change.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use architect::action::{ActionCaller, ActionList};
use architect_scripting::luau::Runtime;
use architect_scripting::theme::{ThemeCell, ThemeModule};
use architect_theme::{LayerKind, Rgba, ThemeStore};
use futures::executor::block_on;
use serde_json::{Value, json};

fn runtime(store: &ThemeStore) -> Runtime {
    Runtime::new(ActionList::new()).with_module(Arc::new(ThemeModule::new(store.clone())))
}

#[test]
fn a_script_reads_colours_numbers_and_the_scheme() {
    let store = ThemeStore::default();
    store.set_token(LayerKind::App, "lab.alpha", "0.42");
    let brand = store.current().color("brand").packed();
    let report = block_on(runtime(&store).run(
        &ActionCaller::app(),
        r"return {
            brand = theme.color('brand'),
            missing = theme.color('no.such'),
            alpha = theme.number('lab.alpha'),
            not_a_number = theme.number('brand'),
            css = theme.value('brand'),
            scheme = theme.scheme,
            id = theme.id,
            revision = theme.revision,
            same = require('theme').snapshot() == theme.snapshot(),
        }",
        Value::Null,
        "S",
    ));
    let out = report.result.unwrap();
    assert_eq!(out["brand"], json!(brand));
    assert_eq!(out["missing"], json!(Rgba::rgb(1.0, 0.0, 1.0).packed()));
    assert!((out["alpha"].as_f64().unwrap() - 0.42).abs() < 1e-9);
    assert_eq!(out["not_a_number"], Value::Null);
    assert_eq!(out["css"], json!(store.current().value("brand")));
    assert_eq!(out["scheme"], json!("dark"));
    assert_eq!(out["id"], json!("fasttrackstudio-dark"));
    assert_eq!(out["revision"], json!(store.revision()));
    assert_eq!(out["same"], json!(true));
}

#[test]
fn the_snapshot_changes_only_when_the_revision_does() {
    let store = ThemeStore::default();
    let source = r"
local first = theme.snapshot()
local function loop()
  print(theme.revision, tostring(theme.snapshot() == first), theme.scheme)
  defer(loop)
end
defer(loop)
";
    let (report, live) =
        block_on(runtime(&store).start(&ActionCaller::app(), source, Value::Null, "S"));
    report.result.unwrap();
    let live = live.unwrap();
    let next_line = || {
        let report = block_on(live.frame());
        report.result.unwrap();
        report.log.last().cloned().unwrap()
    };
    let at = store.revision();
    assert_eq!(next_line(), format!("{at}\ttrue\tdark"));
    // A write that changes nothing that shows: no new revision, the same
    // frozen table.
    assert!(!store.set_system(architect_theme::Scheme::Dark));
    assert_eq!(next_line(), format!("{at}\ttrue\tdark"));
    // A real change: a new snapshot on the next frame, and only one.
    store.update(LayerKind::Pick, |l| {
        l.appearance = Some(architect_theme::Appearance::Light);
    });
    let now = store.revision();
    assert_eq!(now, at + 1);
    assert_eq!(next_line(), format!("{now}\tfalse\tlight"));
    assert_eq!(next_line(), format!("{now}\tfalse\tlight"));
}

#[test]
fn a_live_scripts_subscriber_is_told_once_per_change() {
    let cell = ThemeCell::new(ThemeStore::default().current());
    let runtime =
        Runtime::new(ActionList::new()).with_module(Arc::new(ThemeModule::new(cell.clone())));
    let source = r"
local told, last = 0, nil
local first = theme.snapshot()
theme.subscribe(function(s) told += 1; last = s.scheme end)
local function loop()
  print(told, last or '-', tostring(theme.snapshot() == first), theme.scheme)
  defer(loop)
end
defer(loop)
";
    let (report, live) = block_on(runtime.start(&ActionCaller::app(), source, Value::Null, "S"));
    report.result.unwrap();
    let live = live.unwrap();
    let next_line = |live: &architect_scripting::luau::Live| {
        let report = block_on(live.frame());
        report.result.unwrap();
        report.log.last().cloned().unwrap()
    };
    assert_eq!(next_line(&live), "0\t-\ttrue\tdark");
    // The same theme again: nothing moves.
    assert!(!cell.set(&ThemeStore::default().current()));
    assert_eq!(next_line(&live), "0\t-\ttrue\tdark");
    // Another: the next frame sees it, once.
    let light = ThemeStore::default();
    light.set_system(architect_theme::Scheme::Light);
    assert!(cell.set(&light.current()));
    assert_eq!(next_line(&live), "1\tlight\tfalse\tlight");
    assert_eq!(next_line(&live), "1\tlight\tfalse\tlight");
}
