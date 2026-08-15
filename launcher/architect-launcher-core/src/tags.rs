//! Hierarchical tagging system.
//!
//! Tags are slash-separated paths like `audio/effects/reverb`.
//! An item tagged `audio/effects/reverb` automatically matches queries
//! for `audio/effects` and `audio` (inheritance).
//!
//! The TagRegistry holds:
//! - Known tag definitions with metadata (icon, description)
//! - Aliases that map shorthand names to full tag paths
//!
//! This is the piece Elephant was missing — it only had flat provider-based
//! categorization. Tags let domain-specific consumers (Reaper, DAWs, etc.)
//! add rich hierarchical classification that works across providers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A tag is just a slash-separated path string.
/// We use a newtype for type safety and to attach helper methods.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag(String);

impl Tag {
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        // Normalize: lowercase, trim slashes, collapse double slashes
        let normalized = path
            .to_lowercase()
            .trim_matches('/')
            .replace("//", "/")
            .to_string();
        Self(normalized)
    }

    /// The full tag path, e.g. `"audio/effects/reverb"`.
    pub fn path(&self) -> &str {
        &self.0
    }

    /// Number of segments. `"audio/effects/reverb"` has depth 3.
    pub fn depth(&self) -> usize {
        if self.0.is_empty() {
            return 0;
        }
        self.0.split('/').count()
    }

    /// The leaf segment. `"audio/effects/reverb"` -> `"reverb"`.
    pub fn leaf(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// The parent tag. `"audio/effects/reverb"` -> `Some(Tag("audio/effects"))`.
    pub fn parent(&self) -> Option<Tag> {
        self.0
            .rsplit_once('/')
            .map(|(parent, _)| Tag(parent.to_string()))
    }

    /// All ancestor tags, from most specific to root.
    /// `"audio/effects/reverb"` -> `["audio/effects", "audio"]`.
    pub fn ancestors(&self) -> Vec<Tag> {
        let mut result = Vec::new();
        let mut current = self.clone();
        while let Some(parent) = current.parent() {
            result.push(parent.clone());
            current = parent;
        }
        result
    }

    /// Check if this tag is equal to or a descendant of `other`.
    /// `"audio/effects/reverb".is_under("audio/effects")` -> true
    /// `"audio/effects/reverb".is_under("audio")` -> true
    /// `"audio/effects/reverb".is_under("audio/effects/reverb")` -> true
    /// `"audio/effects/reverb".is_under("video")` -> false
    pub fn is_under(&self, other: &Tag) -> bool {
        self.0 == other.0
            || (self.0.starts_with(&other.0) && self.0.as_bytes().get(other.0.len()) == Some(&b'/'))
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for Tag {
    fn from(s: &str) -> Self {
        Tag::new(s)
    }
}

impl From<String> for Tag {
    fn from(s: String) -> Self {
        Tag::new(s)
    }
}

/// Metadata about a registered tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    /// The tag itself.
    pub tag: Tag,
    /// Human-readable display name.
    pub display_name: String,
    /// Description of what this tag category represents.
    pub description: String,
    /// Icon identifier for UI display.
    pub icon: String,
    /// Color for UI display (CSS color string, e.g. "#89b4fa" or "rgb(137,180,250)").
    pub color: String,
}

/// Central tag registry. Holds known tags, aliases, and provides query methods.
///
/// The registry is optional — items can have tags that aren't registered.
/// Registration adds metadata (display names, icons) and enables aliases.
#[derive(Debug, Clone, Default)]
pub struct TagRegistry {
    /// Known tags with their metadata. Key is the normalized tag path.
    tags: HashMap<String, TagInfo>,
    /// Alias map: shorthand -> full tag path.
    /// e.g. "fx" -> "audio/effects", "vst" -> "audio/effects/plugin/vst"
    aliases: HashMap<String, String>,
}

impl TagRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tag with metadata.
    pub fn register(
        &mut self,
        path: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
    ) -> &mut Self {
        let tag = Tag::new(path);
        self.tags.insert(
            tag.path().to_string(),
            TagInfo {
                tag: tag.clone(),
                display_name: display_name.into(),
                description: description.into(),
                icon: String::new(),
                color: String::new(),
            },
        );

        // Auto-register ancestor tags if they don't exist
        for ancestor in tag.ancestors() {
            self.tags
                .entry(ancestor.path().to_string())
                .or_insert_with(|| {
                    let name = ancestor.leaf().to_string();
                    TagInfo {
                        tag: ancestor,
                        display_name: name.clone(),
                        description: String::new(),
                        icon: String::new(),
                        color: String::new(),
                    }
                });
        }

        self
    }

    /// Register a tag with all metadata including color.
    pub fn register_full(
        &mut self,
        path: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        icon: impl Into<String>,
        color: impl Into<String>,
    ) -> &mut Self {
        let tag = Tag::new(path);
        let tag_path = tag.path().to_string();
        self.register(tag_path.clone(), display_name, description);
        if let Some(info) = self.tags.get_mut(&tag_path) {
            info.icon = icon.into();
            info.color = color.into();
        }
        self
    }

    /// Set color on an already-registered tag.
    pub fn set_color(&mut self, path: impl Into<String>, color: impl Into<String>) -> &mut Self {
        let tag = Tag::new(path);
        if let Some(info) = self.tags.get_mut(tag.path()) {
            info.color = color.into();
        }
        self
    }

    /// Register a tag with metadata and icon.
    pub fn register_with_icon(
        &mut self,
        path: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        icon: impl Into<String>,
    ) -> &mut Self {
        let tag = Tag::new(path);
        let tag_path = tag.path().to_string();
        self.register(tag_path.clone(), display_name, description);
        if let Some(info) = self.tags.get_mut(&tag_path) {
            info.icon = icon.into();
        }
        self
    }

    /// Add an alias: a shorthand name that resolves to a full tag path.
    pub fn alias(
        &mut self,
        shorthand: impl Into<String>,
        full_path: impl Into<String>,
    ) -> &mut Self {
        let shorthand = shorthand.into().to_lowercase();
        let tag = Tag::new(full_path);
        self.aliases.insert(shorthand, tag.path().to_string());
        self
    }

    /// Resolve a tag string, checking aliases first.
    pub fn resolve(&self, input: &str) -> Tag {
        let lower = input.to_lowercase();
        if let Some(resolved) = self.aliases.get(&lower) {
            Tag::new(resolved.clone())
        } else {
            Tag::new(input)
        }
    }

    /// Get metadata for a tag, if registered.
    pub fn info(&self, tag: &Tag) -> Option<&TagInfo> {
        self.tags.get(tag.path())
    }

    /// Get mutable metadata for a tag.
    pub fn info_mut(&mut self, tag: &Tag) -> Option<&mut TagInfo> {
        self.tags.get_mut(tag.path())
    }

    /// List all registered top-level tags (depth 1).
    pub fn root_tags(&self) -> Vec<&TagInfo> {
        self.tags
            .values()
            .filter(|info| info.tag.depth() == 1)
            .collect()
    }

    /// List children of a tag.
    pub fn children_of(&self, parent: &Tag) -> Vec<&TagInfo> {
        let prefix = format!("{}/", parent.path());
        self.tags
            .values()
            .filter(|info| {
                info.tag.path().starts_with(&prefix) && info.tag.depth() == parent.depth() + 1
            })
            .collect()
    }

    /// List all tags under a parent (recursive).
    pub fn descendants_of(&self, parent: &Tag) -> Vec<&TagInfo> {
        let prefix = format!("{}/", parent.path());
        self.tags
            .values()
            .filter(|info| info.tag.path().starts_with(&prefix))
            .collect()
    }

    /// Check if any items with the given tags match a tag filter.
    pub fn matches_filter(item_tags: &[Tag], filter: &Tag) -> bool {
        item_tags.iter().any(|t| t.is_under(filter))
    }

    /// List all registered tags (flat).
    pub fn all_tags(&self) -> Vec<&TagInfo> {
        self.tags.values().collect()
    }

    /// Merge two tags: move all items tagged with `source` to `dest`.
    /// Returns the source tag path for the caller to update items.
    pub fn merge_tag(&mut self, source: &str, dest: &str) -> Option<(Tag, Tag)> {
        let src = Tag::new(source);
        let dst = Tag::new(dest);
        if self.tags.contains_key(src.path()) && self.tags.contains_key(dst.path()) {
            self.tags.remove(src.path());
            // Remove aliases pointing to source
            self.aliases.retain(|_, v| v != src.path());
            Some((src, dst))
        } else {
            None
        }
    }

    /// Rename a tag (moves metadata to new path).
    pub fn rename_tag(&mut self, old_path: &str, new_path: &str) -> bool {
        let old = Tag::new(old_path);
        let new = Tag::new(new_path);
        if let Some(mut info) = self.tags.remove(old.path()) {
            info.tag = new.clone();
            self.tags.insert(new.path().to_string(), info);
            // Update aliases
            for val in self.aliases.values_mut() {
                if val == old.path() {
                    *val = new.path().to_string();
                }
            }
            true
        } else {
            false
        }
    }

    /// Remove a tag and all its descendants.
    pub fn remove_tag(&mut self, path: &str) {
        let tag = Tag::new(path);
        let prefix = format!("{}/", tag.path());
        self.tags
            .retain(|k, _| k != tag.path() && !k.starts_with(&prefix));
        self.aliases
            .retain(|_, v| v != tag.path() && !v.starts_with(&prefix));
    }

    /// Get the color for a tag, inheriting from ancestors if not set.
    pub fn effective_color(&self, tag: &Tag) -> String {
        // Check this tag first
        if let Some(info) = self.tags.get(tag.path())
            && !info.color.is_empty()
        {
            return info.color.clone();
        }
        // Walk ancestors
        for ancestor in tag.ancestors() {
            if let Some(info) = self.tags.get(ancestor.path())
                && !info.color.is_empty()
            {
                return info.color.clone();
            }
        }
        String::new()
    }
}

/// A set of tags on an item. Provides convenience methods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TagSet {
    tags: Vec<Tag>,
}

impl TagSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_tags(tags: Vec<Tag>) -> Self {
        Self { tags }
    }

    /// Create from string slices.
    pub fn from_strs(tags: &[&str]) -> Self {
        Self {
            tags: tags.iter().map(|&s| Tag::new(s)).collect(),
        }
    }

    pub fn add(&mut self, tag: impl Into<Tag>) {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    pub fn remove(&mut self, tag: &Tag) {
        self.tags.retain(|t| t != tag);
    }

    pub fn contains(&self, tag: &Tag) -> bool {
        self.tags.contains(tag)
    }

    /// Check if any tag in this set is under the given filter tag.
    pub fn matches(&self, filter: &Tag) -> bool {
        TagRegistry::matches_filter(&self.tags, filter)
    }

    /// Check if this set contains a tag or any of its descendants.
    pub fn matches_any(&self, filters: &[Tag]) -> bool {
        filters.iter().any(|f| self.matches(f))
    }

    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tags.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_hierarchy() {
        let tag = Tag::new("audio/effects/reverb");
        assert_eq!(tag.path(), "audio/effects/reverb");
        assert_eq!(tag.depth(), 3);
        assert_eq!(tag.leaf(), "reverb");
        assert_eq!(tag.parent().unwrap().path(), "audio/effects");

        let ancestors = tag.ancestors();
        assert_eq!(ancestors.len(), 2);
        assert_eq!(ancestors[0].path(), "audio/effects");
        assert_eq!(ancestors[1].path(), "audio");
    }

    #[test]
    fn tag_is_under() {
        let reverb = Tag::new("audio/effects/reverb");
        let effects = Tag::new("audio/effects");
        let audio = Tag::new("audio");
        let video = Tag::new("video");

        assert!(reverb.is_under(&effects));
        assert!(reverb.is_under(&audio));
        assert!(reverb.is_under(&reverb));
        assert!(!reverb.is_under(&video));
        assert!(!effects.is_under(&reverb));
    }

    #[test]
    fn tag_normalization() {
        let tag = Tag::new("Audio/Effects//Reverb/");
        assert_eq!(tag.path(), "audio/effects/reverb");
    }

    #[test]
    fn registry_aliases() {
        let mut reg = TagRegistry::new();
        reg.register("audio/effects", "Effects", "Audio effects");
        reg.alias("fx", "audio/effects");
        reg.alias("vst", "audio/effects/plugin/vst");

        let resolved = reg.resolve("fx");
        assert_eq!(resolved.path(), "audio/effects");

        let resolved = reg.resolve("vst");
        assert_eq!(resolved.path(), "audio/effects/plugin/vst");

        // Unknown alias returns as-is
        let resolved = reg.resolve("unknown");
        assert_eq!(resolved.path(), "unknown");
    }

    #[test]
    fn tagset_filtering() {
        let tags = TagSet::from_strs(&["audio/effects/reverb", "reaper/fx-chain"]);
        let filter_effects = Tag::new("audio/effects");
        let filter_audio = Tag::new("audio");
        let filter_video = Tag::new("video");

        assert!(tags.matches(&filter_effects));
        assert!(tags.matches(&filter_audio));
        assert!(!tags.matches(&filter_video));
    }

    #[test]
    fn registry_children() {
        let mut reg = TagRegistry::new();
        reg.register("audio/effects/reverb", "Reverb", "Reverb effects");
        reg.register("audio/effects/delay", "Delay", "Delay effects");
        reg.register("audio/instruments/synth", "Synth", "Synthesizers");

        let audio = Tag::new("audio");
        let children = reg.children_of(&audio);
        assert_eq!(children.len(), 2); // effects, instruments

        let effects = Tag::new("audio/effects");
        let children = reg.children_of(&effects);
        assert_eq!(children.len(), 2); // reverb, delay
    }
}
