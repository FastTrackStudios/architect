//! Export/import all user data for syncing between machines.
//!
//! Bundles favorites, filter presets, user data (ratings, notes, hidden),
//! and history into a single JSON file.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::favorites::Favorites;
use crate::filter::FilterPresets;
use crate::history::History;
use crate::userdata::UserDataStore;

/// Complete export bundle.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportBundle {
    pub version: u32,
    pub favorites: Favorites,
    pub presets: FilterPresets,
    pub userdata: UserDataStore,
    pub history: History,
}

impl ExportBundle {
    pub const CURRENT_VERSION: u32 = 1;

    pub fn export_to_file(
        &self,
        path: &PathBuf,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn import_from_file(
        path: &PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let data = std::fs::read_to_string(path)?;
        let bundle: Self = serde_json::from_str(&data)?;
        Ok(bundle)
    }
}
