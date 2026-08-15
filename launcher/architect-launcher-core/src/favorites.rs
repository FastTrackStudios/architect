//! Favorites system.
//!
//! Persisted set of item IDs that the user has explicitly favorited.
//! Favorites get a large score boost and appear in a dedicated section
//! when the query is empty.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Favorites {
    items: HashSet<String>,
}

impl Favorites {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn add(&mut self, item_id: &str) {
        self.items.insert(item_id.to_string());
    }

    pub fn remove(&mut self, item_id: &str) {
        self.items.remove(item_id);
    }

    pub fn toggle(&mut self, item_id: &str) -> bool {
        if self.items.contains(item_id) {
            self.items.remove(item_id);
            false
        } else {
            self.items.insert(item_id.to_string());
            true
        }
    }

    pub fn is_favorite(&self, item_id: &str) -> bool {
        self.items.contains(item_id)
    }

    pub fn all(&self) -> &HashSet<String> {
        &self.items
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Score boost for favorites. Applied on top of match + history score.
    pub fn score_boost(&self, item_id: &str) -> f64 {
        if self.items.contains(item_id) {
            500.0
        } else {
            0.0
        }
    }
}

pub fn default_favorites_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("dioxus-launcher").join("favorites.json")
}
