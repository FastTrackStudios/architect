//! Usage history tracking with time-decayed scoring.
//!
//! Ported from Elephant's `pkg/common/history/history.go`.
//!
//! Tracks per-query, per-item usage data and computes a time-decayed score
//! that boosts frequently-used items in search results.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single history entry tracking usage of one item for one query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// How many times this item was activated for this query.
    pub count: u32,
    /// When it was last activated.
    pub last_used: DateTime<Utc>,
}

/// Full history store. Keyed by `(query, item_id)`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct History {
    /// Map of "query" -> "item_id" -> entry.
    entries: HashMap<String, HashMap<String, HistoryEntry>>,
}

impl History {
    /// Load history from a JSON file, or return empty if it doesn't exist.
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    /// Save history to a JSON file.
    pub fn save(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir)?;
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Record that an item was activated for a given query.
    pub fn record(&mut self, query: &str, item_id: &str) {
        let entry = self
            .entries
            .entry(query.to_string())
            .or_default()
            .entry(item_id.to_string())
            .or_insert(HistoryEntry {
                count: 0,
                last_used: Utc::now(),
            });
        entry.count = entry.count.saturating_add(1);
        entry.last_used = Utc::now();
    }

    /// Compute a usage score for an item given a query.
    ///
    /// Mirrors Elephant's `CalcUsageScore`:
    /// - base = 10 - days_since_last_use (clamped to 0)
    /// - score = base * min(count, 10)
    ///
    /// This means recent, frequently-used items get a boost of up to 100,
    /// which decays to 0 after 10 days without use.
    pub fn usage_score(&self, query: &str, item_id: &str) -> f64 {
        let Some(query_entries) = self.entries.get(query) else {
            return 0.0;
        };
        let Some(entry) = query_entries.get(item_id) else {
            return 0.0;
        };

        let days_since = (Utc::now() - entry.last_used).num_days().max(0) as f64;
        let base = (10.0 - days_since).max(0.0);
        let count = (entry.count as f64).min(10.0);
        base * count
    }

    /// Erase all history for a specific item.
    pub fn erase_item(&mut self, item_id: &str) {
        for query_entries in self.entries.values_mut() {
            query_entries.remove(item_id);
        }
    }

    /// Erase all history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Default history file path.
pub fn default_history_path() -> PathBuf {
    dirs_path("history.json")
}

fn dirs_path(filename: &str) -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("dioxus-launcher").join(filename)
}
