//! Settings in a browser: the settings page's values, kept in this
//! browser's `localStorage` (device and synced scopes alike: nothing syncs
//! them yet), read at start. A per-viewer convenience: a private window or
//! cleared site data starts from the defaults.

use architect_settings::{Registry, Scope, ScopedValues, SettingsChange, SettingsHandle, Values};
use dioxus::prelude::*;

/// Where each scope's values are kept.
const fn key(scope: Scope) -> Option<&'static str> {
    match scope {
        Scope::Device => Some("architect.settings.device"),
        Scope::Synced => Some("architect.settings.synced"),
        Scope::Project => None,
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn load(scope: Scope) -> Values {
    key(scope)
        .and_then(|k| storage()?.get_item(k).ok().flatten())
        .and_then(|text| Values::from_json(&text).ok())
        .unwrap_or_default()
}

fn save(change: &SettingsChange) {
    let (Some(k), Some(s)) = (key(change.scope), storage()) else {
        return;
    };
    if let Ok(text) = change.values.to_json() {
        let _ = s.set_item(k, &text);
    }
}

/// The settings in context, their values this browser's.
pub fn use_web_settings() -> SettingsHandle {
    architect_settings::use_settings_provider(
        Registry::new,
        || ScopedValues {
            device: load(Scope::Device),
            synced: load(Scope::Synced),
            ..Default::default()
        },
        Some(Callback::new(move |change: SettingsChange| save(&change))),
    )
}
