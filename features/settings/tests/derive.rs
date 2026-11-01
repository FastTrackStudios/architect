//! `#[derive(Settings)]`, the scopes and an org's defaults and locks, and
//! a synced setting following the person to another device.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use architect_settings::{
    Control, DocValues, OrgSettings, Registry, Scope, ScopedValues, Settings, Value, ValueError,
    Values, synced_doc_id,
};
use facet::Facet;

/// How the app looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum Theme {
    /// System
    /// Follow the computer.
    #[default]
    System,
    /// Light
    Light,
    Dark,
}

/// Input
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Input {
    /// Monitor input
    /// Hear what comes in while recording.
    pub monitor: bool,
    /// Gain
    pub gain: i32,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            monitor: true,
            gain: 0,
        }
    }
}

/// Audio
#[derive(Debug, Clone, PartialEq, Facet, Settings)]
#[settings(icon = "audio-lines", group = "Studio")]
pub struct Audio {
    /// Buffer size
    /// Smaller is quicker to play, and harder for the computer.
    #[setting(choices = [64, 128, 256, 512], unit = "samples")]
    pub buffer: u32,
    /// Theme
    /// Follow the system, or always light or dark.
    #[setting(control = cards, aliases = ["dark mode"])]
    pub theme: Theme,
    /// Text size
    #[setting(control = slider(80..=140, step = 5), unit = "%")]
    pub text_size: u32,
    /// Tracks
    #[setting(range = 1..=64)]
    pub tracks: u8,
    /// Volume
    pub volume: f64,
    /// Name
    #[setting(placeholder = "Untitled")]
    pub name: String,
    /// Record
    #[setting(control = shortcut)]
    pub record: String,
    pub reduce_motion: bool,
    /// Never on the page.
    #[setting(skip)]
    pub internal: bool,
    /// Not a kind of setting: kept as its default.
    pub recent: Vec<String>,
    /// Input
    pub input: Input,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            buffer: 256,
            theme: Theme::System,
            text_size: 100,
            tracks: 8,
            volume: 0.8,
            name: "Untitled".into(),
            record: "R".into(),
            reduce_motion: false,
            internal: true,
            recent: vec!["a".into()],
            input: Input::default(),
        }
    }
}

/// Editing (follows the person to every device)
#[derive(Debug, Clone, PartialEq, Eq, Facet, Settings)]
#[settings(id = "editing", scope = Synced)]
pub struct EditingPrefs {
    /// Tab width
    #[setting(range = 1..=8)]
    pub tab_width: u8,
    /// Wrap lines
    pub wrap: bool,
}

impl Default for EditingPrefs {
    fn default() -> Self {
        Self {
            tab_width: 4,
            wrap: false,
        }
    }
}

#[test]
fn the_derive_reads_words_controls_and_defaults_from_the_struct() {
    let s = Audio::section();
    assert_eq!(s.id, "audio");
    assert_eq!(s.title, "Audio");
    assert_eq!(s.icon, "audio-lines");
    assert_eq!(s.group.as_deref(), Some("Studio"));
    let keys: Vec<&str> = s.settings().map(|t| t.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "audio.buffer",
            "audio.theme",
            "audio.text_size",
            "audio.tracks",
            "audio.volume",
            "audio.name",
            "audio.record",
            "audio.reduce_motion",
            "audio.input.monitor",
            "audio.input.gain",
        ]
    );
    // A struct field is a titled block of its own.
    assert_eq!(s.blocks.len(), 2);
    assert_eq!(s.blocks[1].title.as_deref(), Some("Input"));

    let get = |k: &str| s.settings().find(|t| t.key == k).unwrap();
    let buffer = get("audio.buffer");
    assert_eq!(buffer.title, "Buffer size");
    assert_eq!(
        buffer.description,
        "Smaller is quicker to play, and harder for the computer."
    );
    assert_eq!(buffer.default, Value::Int(256));
    let Control::Select(choices) = &buffer.control else {
        panic!("{:?}", buffer.control)
    };
    assert_eq!(choices.len(), 4);
    assert_eq!(choices[0].label, "64 samples");

    let theme = get("audio.theme");
    assert_eq!(theme.aliases, ["dark mode"]);
    assert_eq!(theme.default, Value::Text("System".into()));
    let Control::Cards(choices) = &theme.control else {
        panic!("{:?}", theme.control)
    };
    assert_eq!(choices[0].label, "System");
    assert_eq!(choices[0].detail, "Follow the computer.");
    // A variant without a doc is its name.
    assert_eq!(choices[2].label, "Dark");

    assert_eq!(
        get("audio.text_size").control,
        Control::Slider {
            min: 80.0,
            max: 140.0,
            step: 5.0,
            unit: "%".into()
        }
    );
    // A slider holds a float, over an integer field too.
    assert_eq!(get("audio.text_size").default, Value::Float(100.0));
    assert_eq!(
        get("audio.tracks").control,
        Control::Stepper {
            min: 1,
            max: 64,
            step: 1,
            unit: String::new()
        }
    );
    assert!(matches!(
        get("audio.volume").control,
        Control::Slider { .. }
    ));
    assert_eq!(
        get("audio.name").control,
        Control::Text {
            placeholder: "Untitled".into()
        }
    );
    assert_eq!(get("audio.record").control, Control::Shortcut);
    // No doc comment: the field's name, in words.
    assert_eq!(get("audio.reduce_motion").title, "Reduce motion");
    assert_eq!(get("audio.reduce_motion").control, Control::Switch);
    assert_eq!(get("audio.input.monitor").default, Value::Bool(true));
    assert!(s.settings().all(|t| t.scope == Scope::Device));

    let e = EditingPrefs::section();
    assert_eq!(e.title, "Editing (follows the person to every device)");
    assert!(e.settings().all(|t| t.scope == Scope::Synced));
}

#[test]
fn the_typed_value_reads_and_writes_through_the_values() {
    let mut r = Registry::new();
    r.register(Audio::section()).unwrap();
    let mut v = ScopedValues::new();
    assert_eq!(Audio::read(&r, &v), Audio::default());

    v.set(&r, "audio.buffer", Value::Int(512)).unwrap();
    v.set(&r, "audio.theme", Value::Text("Dark".into()))
        .unwrap();
    v.set(&r, "audio.text_size", Value::Float(122.0)).unwrap();
    v.set(&r, "audio.input.gain", Value::Int(-6)).unwrap();
    assert_eq!(
        v.set(&r, "audio.buffer", Value::Int(100)),
        Err(ValueError::Refused {
            key: "audio.buffer".into(),
            why: "not one of its choices".into()
        })
    );
    let a = Audio::read(&r, &v);
    assert_eq!(a.buffer, 512);
    assert_eq!(a.theme, Theme::Dark);
    // Snapped to the slider's step.
    assert_eq!(a.text_size, 120);
    assert_eq!(a.input.gain, -6);
    // What is not on the page keeps its default.
    assert!(a.internal);
    assert_eq!(a.recent, ["a"]);

    // A changed struct's entries are what to set.
    let mut b = a;
    b.tracks = 16;
    b.input.monitor = false;
    for (k, value) in b.entries() {
        v.set(&r, &k, value).unwrap();
    }
    assert_eq!(Audio::read(&r, &v), b);
    assert!(v.is_changed(&r, "audio.tracks"));
    // Every changed one lives on this device.
    assert_eq!(v.device.set.len(), 6);
    assert!(v.synced.set.is_empty());
}

#[test]
fn each_setting_lives_in_its_scope() {
    let mut r = Registry::new();
    r.register(Audio::section()).unwrap();
    r.register(EditingPrefs::section()).unwrap();
    let mut v = ScopedValues::new();
    assert_eq!(
        v.set(&r, "editing.tab_width", Value::Int(2)),
        Ok(Scope::Synced)
    );
    assert_eq!(v.set(&r, "audio.tracks", Value::Int(3)), Ok(Scope::Device));
    assert!(v.synced.set.contains_key("editing.tab_width"));
    assert!(!v.device.set.contains_key("editing.tab_width"));

    // Another device's values arrive: read as they are.
    let mut theirs = Values::new();
    theirs.set.insert("editing.wrap".into(), Value::Bool(true));
    v.replace(Scope::Synced, theirs);
    assert!(EditingPrefs::read(&r, &v).wrap);
    assert_eq!(EditingPrefs::read(&r, &v).tab_width, 4);

    assert_eq!(v.restore_section(&r, "editing"), [Scope::Synced]);
    assert!(v.synced.set.is_empty());
    assert_eq!(v.device.set.len(), 1);
}

#[test]
fn an_org_sets_defaults_and_locks() {
    let mut r = Registry::new();
    r.register(Audio::section()).unwrap();
    let mut v = ScopedValues::new();
    v.set(&r, "audio.tracks", Value::Int(12)).unwrap();
    v.org = OrgSettings::new()
        .default_to("audio.buffer", Value::Int(512))
        .default_to("audio.tracks", Value::Int(32))
        .default_to("audio.name", Value::Int(3))
        .lock("audio.tracks");

    // The org's default replaces the declared one.
    assert_eq!(v.get(&r, "audio.buffer"), Some(Value::Int(512)));
    // One the setting would not take is ignored.
    assert_eq!(
        v.get(&r, "audio.name"),
        Some(Value::Text("Untitled".into()))
    );
    // A locked one reads the org's, whatever the member had.
    assert_eq!(v.get(&r, "audio.tracks"), Some(Value::Int(32)));
    assert!(!v.is_changed(&r, "audio.tracks"));
    assert_eq!(
        v.set(&r, "audio.tracks", Value::Int(2)),
        Err(ValueError::Locked("audio.tracks".into()))
    );
    // Choosing the org's default is not a change.
    v.set(&r, "audio.buffer", Value::Int(512)).unwrap();
    assert!(!v.is_changed(&r, "audio.buffer"));
    v.set(&r, "audio.buffer", Value::Int(64)).unwrap();
    assert!(v.is_changed(&r, "audio.buffer"));
    v.reset(&r, "audio.buffer");
    assert_eq!(Audio::read(&r, &v).buffer, 512);
    assert_eq!(Audio::read(&r, &v).tracks, 32);
}

#[test]
fn search_finds_a_derived_setting_by_its_words() {
    let mut r = Registry::new();
    r.register(Audio::section()).unwrap();
    let hits = r.search("dark mode");
    assert_eq!(hits[0].key, "audio.theme");
    assert_eq!(hits[0].section, "audio");
}

/// One device: its own document, as the app's sync would keep it.
fn device() -> (loro::LoroDoc, DocValues) {
    let doc = loro::LoroDoc::new();
    let values = DocValues::new(&doc);
    (doc, values)
}

fn sync(from: &loro::LoroDoc, to: &loro::LoroDoc) {
    let bytes = from.export(loro::ExportMode::all_updates()).unwrap();
    to.import(&bytes).unwrap();
}

#[test]
fn a_synced_setting_follows_the_person_to_another_device() {
    let mut r = Registry::new();
    r.register(EditingPrefs::section()).unwrap();
    let (laptop_doc, laptop) = device();
    let (phone_doc, phone) = device();

    let heard = Arc::new(AtomicUsize::new(0));
    let h = heard.clone();
    let _sub = phone.on_remote_change(move || {
        h.fetch_add(1, Ordering::SeqCst);
    });

    // On the laptop, the person sets a tab width.
    let mut on_laptop = ScopedValues::new();
    on_laptop
        .set(&r, "editing.tab_width", Value::Int(2))
        .unwrap();
    laptop
        .save_keys(&on_laptop.synced, &["editing.tab_width"])
        .unwrap();
    assert_eq!(heard.load(Ordering::SeqCst), 0);
    sync(&laptop_doc, &phone_doc);
    // The phone hears it arrive.
    assert_eq!(heard.load(Ordering::SeqCst), 1);

    // The phone reads it.
    let mut on_phone = ScopedValues::new();
    on_phone.replace(Scope::Synced, phone.load());
    assert_eq!(EditingPrefs::read(&r, &on_phone).tab_width, 2);

    // Offline, each changes a different one; both survive the meeting.
    on_phone.set(&r, "editing.wrap", Value::Bool(true)).unwrap();
    phone
        .save_keys(&on_phone.synced, &["editing.wrap"])
        .unwrap();
    on_laptop
        .set(&r, "editing.tab_width", Value::Int(8))
        .unwrap();
    laptop
        .save_keys(&on_laptop.synced, &["editing.tab_width"])
        .unwrap();
    sync(&laptop_doc, &phone_doc);
    sync(&phone_doc, &laptop_doc);
    for doc in [&laptop, &phone] {
        let mut v = ScopedValues::new();
        v.replace(Scope::Synced, doc.load());
        assert_eq!(
            EditingPrefs::read(&r, &v),
            EditingPrefs {
                tab_width: 8,
                wrap: true
            }
        );
    }

    // Back to the default on one: gone from both.
    on_laptop.reset(&r, "editing.tab_width");
    laptop
        .save_keys(&on_laptop.synced, &["editing.tab_width"])
        .unwrap();
    sync(&laptop_doc, &phone_doc);
    assert!(!phone.load().set.contains_key("editing.tab_width"));
}

#[test]
fn the_synced_document_has_one_id_on_every_device() {
    let a = synced_doc_id("studio", "person-1");
    assert_eq!(a, synced_doc_id("studio", "person-1"));
    assert_ne!(a, synced_doc_id("studio", "person-2"));
    assert_ne!(a, synced_doc_id("task", "person-1"));
}

#[test]
fn a_project_keeps_its_own_values() {
    let dir =
        std::env::temp_dir().join(format!("architect-settings-project-{}", std::process::id()));
    let p = architect_settings::ProjectValues::in_dir("lab", &dir);
    let mut v = Values::new();
    v.set.insert("song.tempo".into(), Value::Int(120));
    p.save(&v).unwrap();
    assert_eq!(
        architect_settings::ProjectValues::in_dir("lab", &dir).load(),
        v
    );
    let _ = std::fs::remove_dir_all(&dir);
}
