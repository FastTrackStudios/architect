//! The person's look, kept as settings (feature `appearance`;
//! `docs/content/architecture/theming.md`, "Persistence and Appearance").
//!
//! A theme store's ([`ThemeStore`]) choices live in layers, and three of
//! them are settings: the org's default ([`LayerKind::Org`], from the
//! org's [`OrgSettings`]), the person's ([`LayerKind::Person`], the
//! [`Scope::Synced`] values, on every device) and this device's
//! ([`LayerKind::Device`], the [`Scope::Device`] values). A layer is
//! three kinds of key:
//!
//! | key | holds |
//! |---|---|
//! | [`THEME`] | the choice: one theme's id, or `light,dark` |
//! | [`MODE`] | `system`, `light` or `dark` |
//! | [`TOKEN_PREFIX`] and a name | an override of that token (`appearance.token.density`) |
//!
//! [`load`] puts the three layers in a store at start; [`Persist`] (not
//! on wasm) then writes the person's and the device's back, a moment
//! after they stop changing, whoever changed them (the Appearance page,
//! `THEME_SET`, `THEME_TOKEN_SET`). [`section`] declares the Appearance
//! section, for the settings page and its search.

use std::collections::{BTreeMap, BTreeSet};

use architect_theme::{Appearance, Layer, LayerKind, ThemeChoice, ThemeStore, valid_token};

use crate::model::{Choice, Control, Scope, Section, Setting, Value};
use crate::values::{OrgSettings, ScopedValues, Values};

/// The choice: one theme's id, or the light and the dark one, `light,dark`.
pub const THEME: &str = "appearance.theme";
/// `system`, `light` or `dark`.
pub const MODE: &str = "appearance.mode";
/// Before a token's name: an override of that token.
pub const TOKEN_PREFIX: &str = "appearance.token.";
/// The section's id.
pub const SECTION: &str = "appearance";

/// The tokens the Appearance page sets.
pub mod tokens {
    /// Text size, a factor of the designed size (`1.1`).
    pub const TEXT_SCALE: &str = "text-scale";
    /// Room around things, a factor (`0.85` compact, `1.15` roomy).
    pub const DENSITY: &str = "density";
    /// Corners, a CSS length (`0.625rem`).
    pub const RADIUS: &str = "radius";
    /// `0` as the theme draws it, `1` more.
    pub const CONTRAST: &str = "contrast";
    /// The shell's face and architect-ui's sans: a font choice sets both.
    pub const FONT: &str = "font";
    pub const FONT_SANS: &str = "font-sans";
}

/// A face the Appearance page offers: its id, its name, the CSS stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Font {
    pub id: &'static str,
    pub label: &'static str,
    pub stack: &'static str,
}

/// The faces offered. The first is the shell's own (Inter, where the
/// app bundles it).
pub const FONTS: &[Font] = &[
    Font {
        id: "inter",
        label: "Inter",
        stack: architect_theme::resolve::SHELL_FONT,
    },
    Font {
        id: "system",
        label: "System",
        stack: "-apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, sans-serif",
    },
    Font {
        id: "serif",
        label: "Serif",
        stack: "ui-serif, Georgia, 'Times New Roman', serif",
    },
    Font {
        id: "mono",
        label: "Monospace",
        stack: "ui-monospace, SFMono-Regular, Menlo, monospace",
    },
];

/// Density's steps: value, name.
pub const DENSITIES: &[(f64, &str)] = &[(0.85, "Compact"), (1.0, "Default"), (1.15, "Roomy")];
/// Corners' steps: the `radius` token, name.
pub const CORNERS: &[(&str, &str)] = &[
    ("0rem", "Square"),
    ("0.375rem", "Small"),
    ("0.625rem", "Medium"),
    ("1rem", "Large"),
];

/// The key of token `name`'s override.
#[must_use]
pub fn token_key(name: &str) -> String {
    format!("{TOKEN_PREFIX}{name}")
}

/// The Appearance section: what the settings page lists and searches.
///
/// Its rows are drawn by the Appearance page (feature `ui`,
/// `use_appearance_section`), which reads and writes the theme store;
/// the keys are the ones [`values_of`] keeps.
#[must_use]
pub fn section() -> Section {
    let mode = Control::Cards(vec![
        Choice::text("system", "System").detail("Follow the system"),
        Choice::text("light", "Light"),
        Choice::text("dark", "Dark"),
    ]);
    let synced = |s: Setting| s.scope(Scope::Synced);
    Section::new(SECTION, "Appearance").icon("palette").block(
        None,
        vec![
            synced(
                Setting::new(MODE, "Mode", mode, Value::Text("system".into()))
                    .describe("Follow the system, or always light or dark.")
                    .aliases(["dark mode", "light mode", "colour scheme", "color scheme"]),
            ),
            synced(
                Setting::new(
                    THEME,
                    "Theme",
                    Control::Text {
                        placeholder: String::new(),
                    },
                    Value::Text(String::new()),
                )
                .describe("The theme light shows, and the one dark shows.")
                .aliases([
                    "dark mode",
                    "light mode",
                    "colours",
                    "colors",
                    "palette",
                ]),
            ),
            synced(
                Setting::new(
                    token_key(tokens::TEXT_SCALE),
                    "Text size",
                    Control::Slider {
                        min: 0.8,
                        max: 1.4,
                        step: 0.05,
                        unit: "×".into(),
                    },
                    Value::Float(1.0),
                )
                .describe("How large text is everywhere.")
                .aliases(["font size", "zoom"]),
            ),
            synced(
                Setting::new(
                    token_key(tokens::DENSITY),
                    "Density",
                    Control::Segmented(
                        DENSITIES
                            .iter()
                            .map(|(v, l)| Choice::new(Value::Float(*v), *l))
                            .collect(),
                    ),
                    Value::Float(1.0),
                )
                .describe("How much room there is around things.")
                .aliases(["compact", "spacing"]),
            ),
            synced(
                Setting::new(
                    token_key(tokens::RADIUS),
                    "Corners",
                    Control::Segmented(CORNERS.iter().map(|(v, l)| Choice::text(v, *l)).collect()),
                    Value::Text("0.625rem".into()),
                )
                .describe("How round corners are.")
                .aliases(["radius", "rounded"]),
            ),
            synced(
                Setting::new(
                    token_key(tokens::CONTRAST),
                    "Contrast",
                    Control::Segmented(vec![
                        Choice::new(Value::Float(0.0), "Standard"),
                        Choice::new(Value::Float(1.0), "More"),
                    ]),
                    Value::Float(0.0),
                )
                .describe("Quieter text and edges drawn stronger.")
                .aliases(["accessibility", "legibility"]),
            ),
            synced(
                Setting::new(
                    token_key(tokens::FONT),
                    "Font",
                    Control::Select(
                        FONTS
                            .iter()
                            .map(|f| Choice::text(f.stack, f.label))
                            .collect(),
                    ),
                    Value::Text(architect_theme::resolve::SHELL_FONT.into()),
                )
                .describe("The face everything is set in.")
                .aliases(["typeface"]),
            ),
        ],
    )
}

/// `choice` as [`THEME`] keeps it.
#[must_use]
pub fn choice_text(choice: &ThemeChoice) -> String {
    match choice {
        ThemeChoice::One(id) => id.clone(),
        ThemeChoice::Pair { light, dark } => format!("{light},{dark}"),
    }
}

/// [`THEME`]'s text as a choice; `None` when it is not one.
#[must_use]
pub fn parse_choice(text: &str) -> Option<ThemeChoice> {
    let valid = |id: &str| architect_theme::valid_id(id).then(|| id.to_owned());
    match text.split_once(',') {
        Some((light, dark)) => Some(ThemeChoice::Pair {
            light: valid(light.trim())?,
            dark: valid(dark.trim())?,
        }),
        None => valid(text.trim()).map(ThemeChoice::One),
    }
}

/// `appearance` as [`MODE`] keeps it.
#[must_use]
pub const fn mode_text(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "system",
        Appearance::Light => "light",
        Appearance::Dark => "dark",
    }
}

/// [`MODE`]'s text as an appearance.
#[must_use]
pub fn parse_mode(text: &str) -> Option<Appearance> {
    match text {
        "system" => Some(Appearance::System),
        "light" => Some(Appearance::Light),
        "dark" => Some(Appearance::Dark),
        _ => None,
    }
}

/// A kept value as a token's text: a number as it was written (`1.15`).
fn token_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(t) if !t.trim().is_empty() => Some(t.trim().to_owned()),
        Value::Float(f) if f.is_finite() => Some(f.to_string()),
        Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

/// A token's text as it is kept: a plain number as a number (so the
/// settings' controls read it), anything else as text.
fn token_value(text: &str) -> Value {
    match text.parse::<f64>() {
        Ok(f) if f.is_finite() => Value::Float(f),
        _ => Value::Text(text.to_owned()),
    }
}

/// Layer `layer` as the values that keep it: its choice, its mode and
/// every token it overrides.
#[must_use]
pub fn values_of(layer: &Layer) -> Values {
    let mut out = Values::new();
    if let Some(choice) = &layer.choice {
        out.set
            .insert(THEME.to_owned(), Value::Text(choice_text(choice)));
    }
    if let Some(appearance) = layer.appearance {
        out.set
            .insert(MODE.to_owned(), Value::Text(mode_text(appearance).into()));
    }
    for (name, value) in &layer.overrides {
        out.set.insert(token_key(name), token_value(value));
    }
    out
}

/// The layer `values` keep. A key in `locked` (the org has set it for
/// everyone) is left out, as is anything that does not read: a choice
/// naming no valid id, an unknown mode, a token name that is not one.
#[must_use]
pub fn layer_of(values: &BTreeMap<String, Value>, locked: &BTreeSet<String>) -> Layer {
    let mut layer = Layer::default();
    for (key, value) in values {
        if locked.contains(key) {
            continue;
        }
        if key == THEME {
            layer.choice = value.as_text().and_then(parse_choice);
        } else if key == MODE {
            layer.appearance = value.as_text().and_then(parse_mode);
        } else if let Some(name) = key.strip_prefix(TOKEN_PREFIX) {
            if let (true, Some(text)) = (valid_token(name), token_text(value)) {
                layer.overrides.insert(name.to_owned(), text);
            }
        }
    }
    layer
}

/// Only the appearance keys of `values`.
#[must_use]
pub fn only_appearance(values: &Values) -> Values {
    Values {
        set: values
            .set
            .iter()
            .filter(|(k, _)| is_appearance_key(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

/// `values`' appearance keys as [`values_of`] writes them (a number
/// kept as text read as a number; what does not read left out), so a
/// layer read back compares equal to what it was read from.
#[must_use]
pub fn normalized(values: &Values) -> Values {
    values_of(&layer_of(&values.set, &BTreeSet::new()))
}

/// Whether `key` is one this module keeps.
#[must_use]
pub fn is_appearance_key(key: &str) -> bool {
    key == THEME || key == MODE || key.starts_with(TOKEN_PREFIX)
}

/// The org's layer: its defaults for the appearance keys.
#[must_use]
pub fn org_layer(org: &OrgSettings) -> Layer {
    layer_of(&org.defaults, &BTreeSet::new())
}

/// The three layers `values` keep: the org's, the person's (synced) and
/// this device's. What the org locked reads the org's in each.
#[must_use]
pub fn layers_of(values: &ScopedValues) -> [(LayerKind, Layer); 3] {
    let locked = &values.org.locked;
    [
        (LayerKind::Org, org_layer(&values.org)),
        (LayerKind::Person, layer_of(&values.synced.set, locked)),
        (LayerKind::Device, layer_of(&values.device.set, locked)),
    ]
}

/// Puts the org's, the person's and the device's layers in `store`, as
/// `values` keep them (at start, or when they change elsewhere). True
/// when what shows changed.
// A courtesy, as the store's own writes.
#[allow(clippy::must_use_candidate)]
pub fn load(store: &ThemeStore, values: &ScopedValues) -> bool {
    let mut changed = false;
    for (kind, layer) in layers_of(values) {
        changed |= store.set(kind, layer);
    }
    changed
}

/// The layer a scope keeps: the person's for [`Scope::Synced`], the
/// device's for [`Scope::Device`].
#[must_use]
pub const fn layer_kind(scope: Scope) -> Option<LayerKind> {
    match scope {
        Scope::Synced => Some(LayerKind::Person),
        Scope::Device => Some(LayerKind::Device),
        Scope::Project => None,
    }
}

/// Back to the org's default: the person's and this device's layers
/// emptied. True when what shows changed.
#[allow(clippy::must_use_candidate)]
pub fn reset_to_org(store: &ThemeStore) -> bool {
    let a = store.set(LayerKind::Person, Layer::default());
    let b = store.set(LayerKind::Device, Layer::default());
    a || b
}

/// A write for a scope's store: what it holds now, and the keys that
/// changed (a synced store writes only those: `DocValues::save_keys`).
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub scope: Scope,
    /// Only the appearance keys: keep them where nothing else writes the
    /// whole file over them (a device file of their own, or the synced
    /// document's keys).
    pub values: Values,
    pub keys: Vec<String>,
}

/// What `store`'s person and device layers hold that `kept` does not:
/// one [`Saved`] per scope that changed, `kept` updated to match.
pub fn pending(store: &ThemeStore, kept: &mut ScopedValues) -> Vec<Saved> {
    let mut out = Vec::new();
    for scope in [Scope::Synced, Scope::Device] {
        let Some(kind) = layer_kind(scope) else {
            continue;
        };
        let mut now = values_of(&store.layer(kind));
        let before = kept.of(scope).clone();
        // What the org locked is not the member's: keep what was there.
        for key in &kept.org.locked {
            now.set.remove(key);
            if let Some(v) = before.set.get(key) {
                now.set.insert(key.clone(), v.clone());
            }
        }
        let keys: Vec<String> = before
            .set
            .keys()
            .chain(now.set.keys())
            .filter(|k| before.set.get(*k) != now.set.get(*k))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if keys.is_empty() {
            continue;
        }
        kept.replace(scope, now.clone());
        out.push(Saved {
            scope,
            values: now,
            keys,
        });
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub use persist::Persist;

#[cfg(not(target_arch = "wasm32"))]
mod persist {
    use std::sync::mpsc::{self, RecvTimeoutError, Sender};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use architect_theme::{Subscription, ThemeStore};

    use super::{Saved, layer_kind, layers_of, normalized, pending};
    use crate::model::Scope;
    use crate::values::{OrgSettings, ScopedValues, Values};

    enum Msg {
        Changed,
        Replace(Scope, Values),
        Org(OrgSettings),
        Flush(Sender<()>),
        Stop,
    }

    /// Keeps a store's person and device layers as settings.
    ///
    /// It loads them (and the org's) at start, then hands each change to `save`
    /// `delay` after the layers stop changing, from a thread of its own.
    /// Dropping it writes what is waiting and stops.
    pub struct Persist {
        tx: Sender<Msg>,
        subscription: Option<Subscription>,
        worker: Option<JoinHandle<()>>,
    }

    impl std::fmt::Debug for Persist {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Persist").finish_non_exhaustive()
        }
    }

    impl Persist {
        /// Loads `values` (their appearance keys) into `store`, and keeps
        /// every later change of its person and device layers through
        /// `save`. `save` runs on the persist thread.
        #[must_use]
        pub fn start(
            store: &ThemeStore,
            values: &ScopedValues,
            delay: Duration,
            save: impl FnMut(&Saved) + Send + 'static,
        ) -> Self {
            let mut kept = ScopedValues {
                device: normalized(&values.device),
                synced: normalized(&values.synced),
                project: Values::new(),
                org: values.org.clone(),
            };
            super::load(store, &kept);
            // What the store held over the loaded layers is not a change.
            let _ = pending(store, &mut kept);
            let (tx, rx) = mpsc::channel();
            let ping = tx.clone();
            let subscription = store.subscribe_layers(move |_| {
                let _ = ping.send(Msg::Changed);
            });
            let store = store.clone();
            let worker = std::thread::Builder::new()
                .name("appearance-persist".into())
                .spawn(move || run(&store, kept, &rx, delay, save))
                .ok();
            Self {
                tx,
                subscription: Some(subscription),
                worker,
            }
        }

        /// An app's look kept on this machine alone: this device's layer in
        /// `<app dir>/appearance.json` (`DeviceValues::open_named`), the
        /// person's in a synced settings document kept as a file,
        /// `<app dir>/person.loro` (`DocValues`), for an app with no
        /// account to sync it with yet. Loads them (and they show), then
        /// keeps every change `delay` after the layers stop changing.
        /// `None` when the app has no settings directory
        /// (`architect::settings::app_dir`).
        #[cfg(all(feature = "store", feature = "synced"))]
        #[must_use]
        pub fn local(store: &ThemeStore, app: &str, delay: Duration) -> Option<Self> {
            use crate::{DeviceValues, DocValues};

            let dir = architect::settings::app_dir(app).ok()?;
            let device = DeviceValues::open_named(app, "appearance").ok()?;
            let doc = loro::LoroDoc::new();
            let file = dir.join("person.loro");
            if let Ok(bytes) = std::fs::read(&file) {
                let _ = doc.import(&bytes);
            }
            let person = DocValues::new(&doc);
            let values = ScopedValues {
                device: device.load(),
                synced: person.load(),
                ..ScopedValues::default()
            };
            let save = move |saved: &Saved| match saved.scope {
                Scope::Device => {
                    let _ = device.save(&saved.values);
                }
                Scope::Synced => {
                    if person.save_keys(&saved.values, &saved.keys).is_ok() {
                        if let Ok(bytes) = doc.export(loro::ExportMode::Snapshot) {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::fs::write(&file, bytes);
                        }
                    }
                }
                Scope::Project => {}
            };
            Some(Self::start(store, &values, delay, save))
        }

        /// `scope`'s values as its store has them now (another device's
        /// change arrived): its layer set from them, nothing written.
        pub fn replace(&self, scope: Scope, values: &Values) {
            let _ = self.tx.send(Msg::Replace(scope, normalized(values)));
        }

        /// The org's settings changed (the org switched, an admin set a
        /// default or a lock): its layer set again, and what it locked
        /// taken out of the others.
        pub fn set_org(&self, org: OrgSettings) {
            let _ = self.tx.send(Msg::Org(org));
        }

        /// Writes what is waiting now, and returns once it is written.
        pub fn flush(&self) {
            let (done, wait) = mpsc::channel();
            if self.tx.send(Msg::Flush(done)).is_ok() {
                let _ = wait.recv();
            }
        }
    }

    impl Drop for Persist {
        fn drop(&mut self) {
            drop(self.subscription.take());
            let _ = self.tx.send(Msg::Stop);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn run(
        store: &ThemeStore,
        mut kept: ScopedValues,
        rx: &mpsc::Receiver<Msg>,
        delay: Duration,
        mut save: impl FnMut(&Saved),
    ) {
        let mut write = |kept: &mut ScopedValues| {
            for saved in pending(store, kept) {
                save(&saved);
            }
        };
        let mut dirty = false;
        loop {
            let msg = if dirty {
                match rx.recv_timeout(delay) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => {
                        write(&mut kept);
                        dirty = false;
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => Msg::Stop,
                }
            } else {
                rx.recv().unwrap_or(Msg::Stop)
            };
            match msg {
                Msg::Changed => dirty = true,
                Msg::Replace(scope, values) => {
                    // What waits is this device's, newer than what came.
                    if dirty {
                        write(&mut kept);
                        dirty = false;
                    }
                    kept.replace(scope, values);
                    if let Some(kind) = layer_kind(scope) {
                        let layers = layers_of(&kept);
                        if let Some((_, layer)) = layers.into_iter().find(|(k, _)| *k == kind) {
                            let _ = store.set(kind, layer);
                        }
                    }
                }
                Msg::Org(org) => {
                    if dirty {
                        write(&mut kept);
                        dirty = false;
                    }
                    kept.org = org;
                    super::load(store, &kept);
                }
                Msg::Flush(done) => {
                    write(&mut kept);
                    dirty = false;
                    let _ = done.send(());
                }
                Msg::Stop => {
                    write(&mut kept);
                    return;
                }
            }
        }
    }
}
