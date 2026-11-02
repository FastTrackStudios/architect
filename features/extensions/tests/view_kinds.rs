//! An extension's `[[views]] kind` is checked when it loads
//! (`docs/content/architecture/embedded-views.md`): a kind the app has no
//! provider for refuses the load with its reason, through the same
//! `Host::load` diff as everything else it adds, and nothing of it lands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::ActionList;
use architect_extensions::host::{Host, LoadError, SharedSections};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use architect_workspace::{KindError, KindSpec, ViewHost, ViewKinds};

const MANIFEST: &str = r#"
id = "test.panel"
name = "Panel"
version = "0.1.0"
kind = "data"

[[actions]]
id = "TEST_PANEL_OPEN"
name = "Open the panel"
preset = { action = "TEST_PANEL_OPEN" }

[[views]]
id = "panel"
title = "Panel"
kind = "frame.surface"
"#;

fn host(kinds: Option<ViewKinds>) -> Host {
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::default()));
    match kinds {
        Some(k) => host.with_view_kinds(k),
        None => host,
    }
}

#[test]
fn a_view_of_an_unknown_kind_refuses_the_load_and_says_why() {
    let host = host(None);
    let package = Package::embedded(MANIFEST, &[]).unwrap();
    let err = host.load(&package, Origin::Installed).unwrap_err();
    let LoadError::ViewKind { view, why } = &err else {
        panic!("not refused for its view kind: {err}");
    };
    assert_eq!(view, "panel");
    assert!(matches!(why, KindError::Unknown { .. }));
    assert_eq!(
        err.to_string(),
        "its view panel: no provider for the view kind “frame.surface” in this app (it has: dioxus)"
    );
    // Nothing of it landed.
    assert!(host.ids().is_empty());
    assert!(host.list().info("TEST_PANEL_OPEN").is_none());
}

#[test]
fn a_view_of_a_provided_kind_loads() {
    let mut kinds = ViewKinds::new();
    kinds
        .register(KindSpec::new("frame.surface", "frame-view").host(ViewHost::Blitz))
        .unwrap();
    let host = host(Some(kinds));
    let package = Package::embedded(MANIFEST, &[]).unwrap();
    let change = host.load(&package, Origin::Installed).unwrap();
    assert_eq!(change.added, ["TEST_PANEL_OPEN"]);
}

#[test]
fn a_view_naming_no_kind_is_dioxus() {
    let manifest = MANIFEST.replace("kind = \"frame.surface\"\n", "");
    let package = Package::embedded(&manifest, &[]).unwrap();
    assert_eq!(package.manifest.views[0].kind, "dioxus");
    host(None).load(&package, Origin::Installed).unwrap();
}
