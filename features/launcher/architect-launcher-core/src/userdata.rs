//! Unified per-item user data: ratings, notes, hidden status, first-seen time.
//!
//! Stored as a single JSON file so export/import is trivial.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-item metadata stored by the user.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ItemData {
    /// 0 = unrated, 1-5 stars.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rating: u8,
    /// Free-text note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Hidden from results.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// When this item was first seen by the launcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
    /// User-assigned tags (on top of provider-assigned tags).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_tags: Vec<String>,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}
fn is_false(v: &bool) -> bool {
    !v
}

/// The full user data store.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserDataStore {
    /// Keyed by item ID.
    items: HashMap<String, ItemData>,
    /// Timestamp of the last scan/session, for "recently added" detection.
    #[serde(default)]
    pub last_scan: Option<DateTime<Utc>>,
    /// Variant groups: group_key -> [item_ids].
    /// Items in the same group auto-sync user tags.
    #[serde(default)]
    variant_groups: HashMap<String, Vec<String>>,
}

impl UserDataStore {
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

    fn entry(&mut self, item_id: &str) -> &mut ItemData {
        self.items.entry(item_id.to_string()).or_default()
    }

    // ── Ratings ────────────────────────────────────────────

    pub fn set_rating(&mut self, item_id: &str, rating: u8) {
        self.entry(item_id).rating = rating.min(5);
    }

    pub fn rating(&self, item_id: &str) -> u8 {
        self.items.get(item_id).map(|d| d.rating).unwrap_or(0)
    }

    /// Score boost from rating: 0-5 stars → 0-50 points.
    pub fn rating_boost(&self, item_id: &str) -> f64 {
        self.rating(item_id) as f64 * 10.0
    }

    // ── Notes ──────────────────────────────────────────────

    pub fn set_note(&mut self, item_id: &str, note: impl Into<String>) {
        self.entry(item_id).note = note.into();
    }

    pub fn note(&self, item_id: &str) -> &str {
        self.items
            .get(item_id)
            .map(|d| d.note.as_str())
            .unwrap_or("")
    }

    // ── Hidden ─────────────────────────────────────────────

    pub fn set_hidden(&mut self, item_id: &str, hidden: bool) {
        self.entry(item_id).hidden = hidden;
    }

    pub fn toggle_hidden(&mut self, item_id: &str) -> bool {
        let entry = self.entry(item_id);
        entry.hidden = !entry.hidden;
        entry.hidden
    }

    pub fn is_hidden(&self, item_id: &str) -> bool {
        self.items.get(item_id).is_some_and(|d| d.hidden)
    }

    // ── User Tags (Quick Tag) ───────────────────────────────

    /// Add a user tag to an item.
    pub fn add_tag(&mut self, item_id: &str, tag: &str) {
        let entry = self.entry(item_id);
        let tag = tag.to_string();
        if !entry.user_tags.contains(&tag) {
            entry.user_tags.push(tag);
        }
    }

    /// Remove a user tag from an item.
    pub fn remove_tag(&mut self, item_id: &str, tag: &str) {
        if let Some(entry) = self.items.get_mut(item_id) {
            entry.user_tags.retain(|t| t != tag);
        }
    }

    /// Get user tags for an item.
    pub fn user_tags(&self, item_id: &str) -> &[String] {
        self.items
            .get(item_id)
            .map(|d| d.user_tags.as_slice())
            .unwrap_or(&[])
    }

    /// Replace a tag across all items (for merge/rename).
    pub fn replace_tag_all(&mut self, old_tag: &str, new_tag: &str) {
        for entry in self.items.values_mut() {
            if let Some(pos) = entry.user_tags.iter().position(|t| t == old_tag) {
                entry.user_tags[pos] = new_tag.to_string();
            }
        }
    }

    // ── Variant Groups (Auto-Sync) ──────────────────────────

    /// Set a variant group for an item. Items in the same group
    /// auto-sync their user tags. The group key is typically
    /// the base plugin name without format suffix.
    pub fn set_variant_group(&mut self, item_id: &str, group: &str) {
        self.entry(item_id);
        self.variant_groups
            .entry(group.to_string())
            .or_default()
            .push(item_id.to_string());
        // Deduplicate
        if let Some(members) = self.variant_groups.get_mut(group) {
            members.sort();
            members.dedup();
        }
    }

    /// Get all item IDs in the same variant group.
    pub fn variant_siblings(&self, item_id: &str) -> Vec<String> {
        for members in self.variant_groups.values() {
            if members.contains(&item_id.to_string()) {
                return members
                    .iter()
                    .filter(|id| id.as_str() != item_id)
                    .cloned()
                    .collect();
            }
        }
        Vec::new()
    }

    /// Sync a tag to all variant siblings of an item.
    pub fn sync_tag_to_variants(&mut self, item_id: &str, tag: &str) {
        let siblings = self.variant_siblings(item_id);
        for sibling in siblings {
            self.add_tag(&sibling, tag);
        }
    }

    // ── First seen / recently added ────────────────────────

    /// Mark an item as seen now (if not already seen).
    pub fn mark_seen(&mut self, item_id: &str) {
        let entry = self.entry(item_id);
        if entry.first_seen.is_none() {
            entry.first_seen = Some(Utc::now());
        }
    }

    /// Check if an item was first seen after the given timestamp.
    pub fn is_new_since(&self, item_id: &str, since: DateTime<Utc>) -> bool {
        self.items
            .get(item_id)
            .and_then(|d| d.first_seen)
            .is_some_and(|seen| seen > since)
    }

    /// Check if an item was first seen after `last_scan`.
    pub fn is_recently_added(&self, item_id: &str) -> bool {
        match self.last_scan {
            Some(scan_time) => self.is_new_since(item_id, scan_time),
            None => false, // First run — nothing is "new"
        }
    }

    /// Update the scan timestamp to now. Call this at the end of provider setup.
    pub fn mark_scan_complete(&mut self) {
        self.last_scan = Some(Utc::now());
    }

    // ── Bulk / Export ──────────────────────────────────────

    pub fn all_data(&self) -> &HashMap<String, ItemData> {
        &self.items
    }

    /// Merge data from another store (for import). Existing entries are updated,
    /// new entries are added. Does NOT overwrite first_seen if already set.
    pub fn merge(&mut self, other: &UserDataStore) {
        for (id, other_data) in &other.items {
            let entry = self.entry(id);
            if other_data.rating > 0 {
                entry.rating = other_data.rating;
            }
            if !other_data.note.is_empty() {
                entry.note = other_data.note.clone();
            }
            if other_data.hidden {
                entry.hidden = true;
            }
            if entry.first_seen.is_none() {
                entry.first_seen = other_data.first_seen;
            }
        }
    }
}

pub fn default_userdata_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("dioxus-launcher").join("userdata.json")
}
