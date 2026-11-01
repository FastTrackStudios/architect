//! Values on disk (feature `store`), through `architect::settings::Store`:
//! the device's in the app's settings directory, a project's in the
//! project's.

use architect::settings::{SettingsError, Store};

use crate::values::Values;

/// Where this device keeps its settings' values.
#[derive(Debug)]
pub struct DeviceValues {
    store: Store<Values>,
}

impl DeviceValues {
    /// The app's store (`<app>/settings.json`).
    ///
    /// # Errors
    ///
    /// The settings directory cannot be found.
    pub fn open(app: &str) -> Result<Self, SettingsError> {
        Ok(Self {
            store: Store::open(app, "settings")?,
        })
    }

    /// What was kept; nothing set when there is none yet.
    #[must_use]
    pub fn load(&self) -> Values {
        self.store.load().unwrap_or_default()
    }

    /// Keeps `values`.
    ///
    /// # Errors
    ///
    /// It could not be written.
    pub fn save(&self, values: &Values) -> Result<(), SettingsError> {
        self.store.save(values).map(|_| ())
    }
}

/// A project's values (`Scope::Project`).
///
/// One JSON file in a directory the app picks for the project (its
/// folder, or a per-project directory of the app's). A project kept in a synced document can keep them there
/// instead (feature `synced`, `DocValues`).
#[derive(Debug)]
pub struct ProjectValues {
    store: Store<Values>,
}

impl ProjectValues {
    /// The values of the project whose settings live in `dir`
    /// (`<dir>/settings.json`). Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn in_dir(app: &str, dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            store: Store::in_dir(app, "settings", dir),
        }
    }

    /// What was kept; nothing set when there is none yet.
    #[must_use]
    pub fn load(&self) -> Values {
        self.store.load().unwrap_or_default()
    }

    /// Keeps `values`.
    ///
    /// # Errors
    ///
    /// It could not be written.
    pub fn save(&self, values: &Values) -> Result<(), SettingsError> {
        self.store.save(values).map(|_| ())
    }
}
