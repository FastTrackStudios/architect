//! Filter state and presets.
//!
//! Supports Scout-style multi-tag filtering with inclusive/exclusive logic:
//! - **Include** tags: item must match ANY of these (OR)
//! - **Exclude** tags: item must NOT match any of these (NOT)
//! - **Require** tags: item must match ALL of these (AND)
//!
//! Filter presets save a named snapshot of the current filter state
//! for instant recall.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tags::Tag;

/// Active filter state. Multiple tags can be combined with different logic.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FilterState {
    /// Include tags: item matches if it has ANY of these (OR union).
    pub include: Vec<String>,
    /// Exclude tags: item is filtered out if it matches ANY of these.
    pub exclude: Vec<String>,
    /// Provider filter: only show results from this provider.
    pub provider: Option<String>,
}

impl FilterState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty() && self.provider.is_none()
    }

    pub fn clear(&mut self) {
        self.include.clear();
        self.exclude.clear();
        self.provider = None;
    }

    /// Add a tag as inclusive filter. If already included, remove it (toggle).
    pub fn toggle_include(&mut self, tag: &str) {
        let tag_str = tag.to_string();
        if let Some(pos) = self.include.iter().position(|t| t == &tag_str) {
            self.include.remove(pos);
        } else {
            // Remove from exclude if present
            self.exclude.retain(|t| t != &tag_str);
            self.include.push(tag_str);
        }
    }

    /// Add a tag as exclusive (NOT) filter. If already excluded, remove it.
    pub fn toggle_exclude(&mut self, tag: &str) {
        let tag_str = tag.to_string();
        if let Some(pos) = self.exclude.iter().position(|t| t == &tag_str) {
            self.exclude.remove(pos);
        } else {
            // Remove from include if present
            self.include.retain(|t| t != &tag_str);
            self.exclude.push(tag_str);
        }
    }

    /// Remove a tag from all filter lists.
    pub fn remove_tag(&mut self, tag: &str) {
        self.include.retain(|t| t != tag);
        self.exclude.retain(|t| t != tag);
    }

    /// Check if an item's tags pass this filter.
    pub fn matches(&self, item_tags: &crate::tags::TagSet) -> bool {
        // Check excludes first (fast reject)
        for ex in &self.exclude {
            let tag = Tag::new(ex.clone());
            if item_tags.matches(&tag) {
                return false;
            }
        }

        // If no include filters, everything passes
        if self.include.is_empty() {
            return true;
        }

        // Item must match at least one include tag (OR)
        for inc in &self.include {
            let tag = Tag::new(inc.clone());
            if item_tags.matches(&tag) {
                return true;
            }
        }

        false
    }

    /// Get a display-friendly list of active filters.
    pub fn active_labels(&self) -> Vec<FilterLabel> {
        let mut labels = Vec::new();
        for tag in &self.include {
            labels.push(FilterLabel {
                tag: tag.clone(),
                mode: FilterMode::Include,
            });
        }
        for tag in &self.exclude {
            labels.push(FilterLabel {
                tag: tag.clone(),
                mode: FilterMode::Exclude,
            });
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterMode {
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilterLabel {
    pub tag: String,
    pub mode: FilterMode,
}

/// Named filter preset — saves a FilterState for instant recall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterPreset {
    pub name: String,
    pub state: FilterState,
}

/// Preset store, persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilterPresets {
    presets: Vec<FilterPreset>,
}

impl FilterPresets {
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

    pub fn add(&mut self, name: impl Into<String>, state: FilterState) {
        let name = name.into();
        // Replace existing preset with same name
        self.presets.retain(|p| p.name != name);
        self.presets.push(FilterPreset { name, state });
    }

    pub fn remove(&mut self, name: &str) {
        self.presets.retain(|p| p.name != name);
    }

    pub fn get(&self, name: &str) -> Option<&FilterPreset> {
        self.presets.iter().find(|p| p.name == name)
    }

    pub fn list(&self) -> &[FilterPreset] {
        &self.presets
    }
}

/// Magic Words: keywords that trigger filter presets when followed by Space.
///
/// Like Scout's Magic Words: type "C" + Space → loads compressor preset.
/// Stored as keyword → preset name mapping.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MagicWords {
    words: Vec<MagicWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicWord {
    pub keyword: String,
    pub preset_name: String,
}

impl MagicWords {
    pub fn add(&mut self, keyword: impl Into<String>, preset_name: impl Into<String>) {
        let keyword = keyword.into().to_lowercase();
        self.words.retain(|w| w.keyword != keyword);
        self.words.push(MagicWord {
            keyword,
            preset_name: preset_name.into(),
        });
    }

    pub fn remove(&mut self, keyword: &str) {
        let lower = keyword.to_lowercase();
        self.words.retain(|w| w.keyword != lower);
    }

    /// Check if the query starts with a magic word followed by a space.
    /// Returns (preset_name, remaining_query) if matched.
    pub fn check<'a>(&'a self, query: &'a str) -> Option<(&'a str, &'a str)> {
        let query_lower = query.to_lowercase();
        for word in &self.words {
            let prefix = format!("{} ", word.keyword);
            if query_lower.starts_with(&prefix) {
                let remainder = &query[prefix.len()..];
                return Some((&word.preset_name, remainder));
            }
            if query_lower == word.keyword {
                return Some((&word.preset_name, ""));
            }
        }
        None
    }

    pub fn list(&self) -> &[MagicWord] {
        &self.words
    }
}

pub fn default_presets_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("dioxus-launcher").join("filter-presets.json")
}
