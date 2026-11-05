//! Navigator as a built-in: its manifest declares what its value builds
//! (its view, its actions), its settings section is `NavigatorSettings`'
//! keys, defaults and controls as they are, and it loads linked into a
//! host, where its actions answer — saying so when no navigator is on
//! show. (Run as a process, the same value: `tests/dev_loop.rs`.)

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect::action::ActionList;
use architect_extensions::Origin;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::linked::Builder;
use architect_extensions::manifest::{Manifest, decl_of};
use architect_navigator::NavigatorSettings;
use architect_navigator_extension::{ACTIONS, ID, MANIFEST, NavigatorExtension};
use architect_settings::{Registry, Settings as _};
use futures::executor::block_on;

#[derive(serde::Serialize)]
struct Sections {
    sections: Vec<architect_extensions::manifest::SectionDecl>,
}

#[test]
fn its_section_is_navigator_settings() {
    let m = Manifest::parse(MANIFEST).unwrap();
    let mut want = decl_of(&NavigatorSettings::section(), false);
    want.group = Some(String::new());
    let toml = toml::to_string(&Sections {
        sections: vec![want.clone()],
    })
    .unwrap();
    let got = m.sections.first().cloned();
    assert_eq!(
        got.as_ref(),
        Some(&want),
        "extension.toml's [[sections]] is not NavigatorSettings; put this in its place:\n{toml}"
    );
    assert_eq!(m.name, "Navigator");
    assert_eq!(want.title, "Navigator");
}

#[test]
fn it_builds_what_its_manifest_declares() {
    let m = Manifest::parse(MANIFEST).unwrap();
    let b = Builder::of(&NavigatorExtension);
    let mut declared: Vec<String> = m.actions.iter().map(|a| a.id.clone()).collect();
    let mut built = b.action_ids();
    declared.sort();
    built.sort();
    assert_eq!(declared, built);
    let mut listed: Vec<String> = ACTIONS.iter().map(|a| (*a).to_owned()).collect();
    listed.sort();
    assert_eq!(listed, built);
    assert_eq!(b.view_ids(), ["files"]);
    assert!(
        m.views
            .iter()
            .any(|v| v.id == "files" && v.title == "Navigator")
    );
}

#[test]
fn linked_in_its_actions_answer_and_say_when_no_navigator_shows() {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let sections = SharedSections::new(Registry::new());
    let host = Host::new(list.clone(), sections.clone());
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")))
        .with_linked(&NavigatorExtension)
        .unwrap();
    let _ = ext.sync();
    assert_eq!(ext.host().get(ID).unwrap().origin, Origin::Builtin);
    assert!(ext.problems().is_empty(), "{:?}", ext.problems());
    // Its section is on the settings page, as the struct's.
    let registry = sections.registry();
    let section = registry.section("navigator").expect("its section");
    assert_eq!(section.title, "Navigator");
    assert!(registry.setting("navigator.grouping").is_some());
    assert!(registry.setting("navigator.folder_notes").is_some());
    // No view on show: it says so.
    let err = block_on(list.run_json_async("NAVIGATOR_STATE", b"")).unwrap_err();
    assert!(err.to_string().contains("not on show"), "{err}");
}
