//! Saved scripts in a settings file (`architect::settings::Store`):
//! `scripts.json` in the app's directory on a desk, `localStorage` on the
//! web.

use architect::settings::Store;
use facet::Facet;

use crate::{Script, ScriptStore};

/// What the file holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Facet)]
pub struct Saved {
    pub scripts: Vec<Script>,
}

/// One scope's scripts in a settings file.
#[derive(Debug)]
pub struct SettingsStore {
    store: Store<Saved>,
}

impl SettingsStore {
    /// The scripts kept in `store`.
    #[must_use]
    pub const fn new(store: Store<Saved>) -> Self {
        Self { store }
    }

    /// `app`'s `<name>.json` (`scripts` for the person's own).
    ///
    /// # Errors
    ///
    /// When the app's directory cannot be found.
    pub fn open(app: &str, name: &str) -> Result<Self, architect::settings::SettingsError> {
        Store::open(app, name).map(Self::new)
    }
}

impl ScriptStore for SettingsStore {
    fn load(&self) -> Result<Vec<Script>, String> {
        self.store
            .load()
            .map(|saved| saved.scripts)
            .map_err(|e| e.to_string())
    }

    fn save(&self, scripts: &[Script]) -> Result<(), String> {
        self.store
            .save(&Saved {
                scripts: scripts.to_vec(),
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
