//! The registry (the app's and extensions' sections), values and search.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use architect_settings::{
    Choice, Control, Owner, Registry, RegistryError, Section, Setting, Value, ValueError, Values,
};

fn appearance() -> Section {
    Section::new("appearance", "Appearance")
        .icon("palette")
        .block(
            None,
            vec![
                Setting::new(
                    "appearance.theme",
                    "Theme",
                    Control::Cards(vec![
                        Choice::text("system", "System"),
                        Choice::text("light", "Light"),
                        Choice::text("dark", "Dark"),
                    ]),
                    Value::Text("system".into()),
                )
                .describe("Follow the system, or always light or dark.")
                .aliases(["dark mode", "colour"]),
                Setting::new(
                    "appearance.text_size",
                    "Text size",
                    Control::Stepper {
                        min: 80,
                        max: 140,
                        step: 5,
                        unit: "%".into(),
                    },
                    Value::Int(100),
                ),
                Setting::new(
                    "appearance.reduce_motion",
                    "Reduce motion",
                    Control::Switch,
                    Value::Bool(false),
                )
                .describe("Drawers and menus appear without moving."),
            ],
        )
}

fn metronome() -> Section {
    Section::new("metronome", "Metronome")
        .group("Extensions")
        .owned_by("fts.metronome")
        .block(
            None,
            vec![Setting::new(
                "fts.metronome.accent",
                "Accent the downbeat",
                Control::Switch,
                Value::Bool(true),
            )],
        )
}

#[test]
fn an_extension_registers_its_section_and_takes_it_away() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    r.register(metronome()).unwrap();
    assert_eq!(r.sections().len(), 2);
    let groups = r.grouped();
    assert_eq!(groups[0].0, None);
    assert_eq!(groups[1].0.as_deref(), Some("Extensions"));
    // Reloading the extension replaces its section.
    r.register(metronome()).unwrap();
    assert_eq!(r.sections().len(), 2);
    r.remove_owner(&Owner::Extension("fts.metronome".into()));
    assert_eq!(r.sections().len(), 1);
}

#[test]
fn an_extension_cannot_take_the_apps_section_or_keys() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    let thief = Section::new("appearance", "Mine").owned_by("bad");
    assert_eq!(
        r.register(thief),
        Err(RegistryError::SectionTaken("appearance".into()))
    );
    let dup = Section::new("other", "Other").owned_by("bad").block(
        None,
        vec![Setting::new(
            "appearance.theme",
            "Theme",
            Control::Switch,
            Value::Bool(true),
        )],
    );
    assert!(matches!(
        r.register(dup),
        Err(RegistryError::KeyTaken { .. })
    ));
}

#[test]
fn values_read_defaults_and_keep_only_what_they_may() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    let mut v = Values::new();
    assert_eq!(
        v.get(&r, "appearance.theme"),
        Some(Value::Text("system".into()))
    );
    v.set(&r, "appearance.theme", Value::Text("dark".into()))
        .unwrap();
    assert!(v.is_changed("appearance.theme"));
    assert!(matches!(
        v.set(&r, "appearance.theme", Value::Text("purple".into())),
        Err(ValueError::Refused { .. })
    ));
    // Numbers stay in range and on their step.
    v.set(&r, "appearance.text_size", Value::Int(200)).unwrap();
    assert_eq!(v.get(&r, "appearance.text_size"), Some(Value::Int(140)));
    v.set(&r, "appearance.text_size", Value::Int(103)).unwrap();
    assert_eq!(v.get(&r, "appearance.text_size"), Some(Value::Int(105)));
    // Setting the default clears it.
    v.set(&r, "appearance.reduce_motion", Value::Bool(false))
        .unwrap();
    assert!(!v.is_changed("appearance.reduce_motion"));
    assert!(matches!(
        v.set(&r, "nope", Value::Bool(true)),
        Err(ValueError::UnknownKey(_))
    ));
}

#[test]
fn restore_defaults_resets_one_section() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    r.register(metronome()).unwrap();
    let mut v = Values::new();
    v.set(&r, "appearance.theme", Value::Text("light".into()))
        .unwrap();
    v.set(&r, "appearance.text_size", Value::Int(120)).unwrap();
    v.set(&r, "fts.metronome.accent", Value::Bool(false))
        .unwrap();
    assert_eq!(v.changed_in(&r, "appearance").len(), 2);
    assert_eq!(v.restore_section(&r, "appearance"), 2);
    assert!(v.is_changed("fts.metronome.accent"));
}

#[test]
fn values_round_trip_and_an_unloaded_extensions_value_waits() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    r.register(metronome()).unwrap();
    let mut v = Values::new();
    v.set(&r, "fts.metronome.accent", Value::Bool(false))
        .unwrap();
    let back = Values::from_json(&v.to_json().unwrap()).unwrap();
    assert_eq!(back, v);
    r.remove_owner(&Owner::Extension("fts.metronome".into()));
    assert_eq!(back.get(&r, "fts.metronome.accent"), None);
    r.register(metronome()).unwrap();
    assert_eq!(
        back.get(&r, "fts.metronome.accent"),
        Some(Value::Bool(false))
    );
}

#[test]
fn search_finds_a_setting_by_title_alias_or_description() {
    let mut r = Registry::new();
    r.register(appearance()).unwrap();
    assert_eq!(r.search("motion")[0].key, "appearance.reduce_motion");
    assert_eq!(r.search("dark mode")[0].key, "appearance.theme");
    assert_eq!(r.search("menus appear")[0].key, "appearance.reduce_motion");
    assert!(r.search("").is_empty());
}
