//! A theme store's layers kept as settings (`architect_settings::appearance`):
//! set, start again from the same settings, the same theme; the org's
//! default under the person's, the device's over the person's.

// Test helpers fail the test: that is their assertion.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect_settings::appearance::{self, Persist, Saved, tokens};
use architect_settings::{DocValues, OrgSettings, Scope, ScopedValues, Value, Values};
use architect_theme::{Appearance, Layer, LayerKind, Scheme, ThemeChoice, ThemeStore};

/// What a scope's store keeps, as an app would: every `Saved` put in.
#[derive(Clone, Default)]
struct Kept(Arc<Mutex<ScopedValues>>);

impl Kept {
    fn save(&self) -> impl FnMut(&Saved) + Send + 'static {
        let kept = self.0.clone();
        move |saved: &Saved| {
            kept.lock()
                .unwrap()
                .replace(saved.scope, saved.values.clone());
        }
    }

    fn values(&self) -> ScopedValues {
        self.0.lock().unwrap().clone()
    }
}

fn store() -> ThemeStore {
    let store = ThemeStore::default();
    let _ = store.set_system(Scheme::Dark);
    store
}

#[test]
fn a_restarted_store_shows_what_was_set() {
    let kept = Kept::default();
    let first = store();
    let persist = Persist::start(
        &first,
        &kept.values(),
        Duration::from_millis(20),
        kept.save(),
    );
    let _ = first.pick_in(LayerKind::Person, "nord-dark").unwrap();
    let _ = first.set_token(LayerKind::Person, tokens::DENSITY, "0.85");
    let _ = first.set_token(LayerKind::Person, "brand", "#ff6600");
    let _ = first.update(LayerKind::Device, |l| {
        l.appearance = Some(Appearance::Light);
    });
    persist.flush();
    drop(persist);

    let values = kept.values();
    assert_eq!(
        values.synced.set.get(appearance::THEME),
        Some(&Value::Text("fasttrackstudio-light,nord-dark".into()))
    );
    assert_eq!(
        values
            .synced
            .set
            .get(&appearance::token_key(tokens::DENSITY)),
        Some(&Value::Float(0.85))
    );
    assert_eq!(
        values.device.set.get(appearance::MODE),
        Some(&Value::Text("light".into()))
    );

    let again = store();
    let _persist = Persist::start(&again, &values, Duration::from_millis(20), kept.save());
    assert_eq!(again.current().tokens(), first.current().tokens());
    assert_eq!(again.current().id(), first.current().id());
    assert_eq!(again.current().id(), "fasttrackstudio-light");
    assert!((again.current().number(tokens::DENSITY) - 0.85).abs() < 1e-6);
    // Starting again is not a change to keep.
    assert_eq!(kept.values(), values);
}

#[test]
fn org_under_person_under_device() {
    let mut values = ScopedValues::new();
    values.org = OrgSettings::new()
        .default_to(
            appearance::THEME,
            Value::Text("nord-light,nord-dark".into()),
        )
        .default_to(
            appearance::token_key(tokens::RADIUS),
            Value::Text("0rem".into()),
        );
    let s = store();
    let _ = appearance::load(&s, &values);
    assert_eq!(s.current().id(), "nord-dark");
    assert_eq!(s.current().value("radius"), "0rem");

    // The person's choice sits over the org's; the org's radius stays.
    values.synced.set.insert(
        appearance::THEME.into(),
        Value::Text("dracula-light,dracula-dark".into()),
    );
    let _ = appearance::load(&s, &values);
    assert_eq!(s.current().id(), "dracula-dark");
    assert_eq!(s.current().value("radius"), "0rem");

    // The device's over the person's.
    values.device.set.insert(
        appearance::THEME.into(),
        Value::Text("catppuccin-dark".into()),
    );
    values.device.set.insert(
        appearance::token_key(tokens::RADIUS),
        Value::Text("1rem".into()),
    );
    let _ = appearance::load(&s, &values);
    assert_eq!(s.current().id(), "catppuccin-dark");
    assert_eq!(s.current().value("radius"), "1rem");

    // Reset to the org's default: the person's and the device's go.
    let _ = appearance::reset_to_org(&s);
    assert_eq!(s.current().id(), "nord-dark");
    assert_eq!(s.current().value("radius"), "0rem");
}

#[test]
fn an_org_lock_beats_the_person_and_the_device() {
    let mut values = ScopedValues::new();
    values.org = OrgSettings::new()
        .default_to(appearance::THEME, Value::Text("nord-dark".into()))
        .lock(appearance::THEME);
    values
        .synced
        .set
        .insert(appearance::THEME.into(), Value::Text("dracula-dark".into()));
    values.device.set.insert(
        appearance::THEME.into(),
        Value::Text("catppuccin-dark".into()),
    );
    let s = store();
    let _ = appearance::load(&s, &values);
    assert_eq!(s.current().id(), "nord-dark");
}

#[test]
fn a_burst_of_changes_is_written_once() {
    let writes = Arc::new(Mutex::new(Vec::<Saved>::new()));
    let w = writes.clone();
    let s = store();
    let persist = Persist::start(
        &s,
        &ScopedValues::new(),
        Duration::from_millis(150),
        move |saved: &Saved| w.lock().unwrap().push(saved.clone()),
    );
    for scale in ["1.05", "1.1", "1.15", "1.2"] {
        let _ = s.set_token(LayerKind::Person, tokens::TEXT_SCALE, scale);
    }
    std::thread::sleep(Duration::from_millis(600));
    {
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(writes[0].scope, Scope::Synced);
        assert_eq!(
            writes[0].keys,
            vec![appearance::token_key(tokens::TEXT_SCALE)]
        );
    }
    // A layer nobody keeps (the pick, a preview) is never written.
    let _ = s.pick("dracula-dark").unwrap();
    persist.flush();
    assert_eq!(writes.lock().unwrap().len(), 1);
}

/// The person's layer through the synced document: written on one
/// device, it arrives on another and shows there, and the other's own
/// device layer stays over it.
#[test]
fn the_person_layer_syncs_between_devices() {
    let (doc_a, doc_b) = (loro::LoroDoc::new(), loro::LoroDoc::new());
    let (a, b) = (DocValues::new(&doc_a), DocValues::new(&doc_b));

    let store_a = store();
    let a2 = a;
    let persist_a = Persist::start(
        &store_a,
        &ScopedValues::new(),
        Duration::from_millis(10),
        move |saved: &Saved| {
            if saved.scope == Scope::Synced {
                a2.save_keys(&saved.values, &saved.keys).unwrap();
            }
        },
    );
    let _ = store_a.pick_in(LayerKind::Person, "gruvbox-dark").unwrap();
    persist_a.flush();

    let store_b = store();
    let mut b_values = ScopedValues::new();
    b_values
        .device
        .set
        .insert(appearance::MODE.into(), Value::Text("light".into()));
    let persist_b = Persist::start(&store_b, &b_values, Duration::from_millis(10), |_| {});
    doc_b
        .import(&doc_a.export(loro::ExportMode::Snapshot).unwrap())
        .unwrap();
    persist_b.replace(Scope::Synced, &b.load());
    persist_b.flush();
    assert_eq!(
        store_b.layer(LayerKind::Person).choice,
        Some(ThemeChoice::pair("fasttrackstudio-light", "gruvbox-dark"))
    );
    // B's device says light: B shows the light half of A's choice.
    assert_eq!(store_b.current().id(), "fasttrackstudio-light");
    store_b.set(LayerKind::Device, Layer::default());
    assert_eq!(store_b.current().id(), "gruvbox-dark");
}

#[test]
fn values_round_trip_a_layer() {
    let layer = Layer::default()
        .choice(ThemeChoice::One("nord-dark".into()))
        .appearance(Appearance::Dark)
        .token("density", "1.15")
        .token("arrange.item.edge", "mix($border, $foreground, 0.2)");
    let values: Values = appearance::values_of(&layer);
    assert_eq!(
        appearance::layer_of(&values.set, &std::collections::BTreeSet::new()),
        layer
    );
    // What does not read is left out.
    let mut bad = Values::new();
    bad.set
        .insert(appearance::THEME.into(), Value::Text("Not An Id".into()));
    bad.set
        .insert(appearance::MODE.into(), Value::Text("sepia".into()));
    bad.set
        .insert(appearance::token_key("Bad Name"), Value::Text("1".into()));
    assert!(appearance::layer_of(&bad.set, &std::collections::BTreeSet::new()).is_empty());
}

#[test]
fn the_section_is_searchable() {
    let mut registry = architect_settings::Registry::new();
    registry.register(appearance::section()).unwrap();
    let hit = registry.search("density");
    assert_eq!(
        hit.first().map(|h| h.key.as_str()),
        Some(appearance::token_key(tokens::DENSITY).as_str())
    );
}
