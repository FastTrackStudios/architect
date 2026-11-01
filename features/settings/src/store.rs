//! The device's values on disk (feature `store`): one JSON file in the
//! app's settings directory, through `architect::settings::Store`.

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
