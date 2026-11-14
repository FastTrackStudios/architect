//! What a note says about itself: its frontmatter, read the vault's way
//! (YAML between `---`, which the vault has already parsed) or Zola's
//! and Hugo's (TOML between `+++`, which the vault leaves as text).

use serde_json::{Map, Value};
use vault::obsidian::VaultPage;
use vault::obsidian::obsidian_parse::parse_frontmatter;

/// The fields a site reads from a note's frontmatter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// `title:`.
    pub title: Option<String>,
    /// `description:`, else `summary:`, else `blurb:`.
    pub description: Option<String>,
    /// `weight:` (Zola) or `order:`: lower first; none sorts last.
    pub weight: Option<i64>,
    /// `tags:` (YAML or TOML), or Zola's `[taxonomies] tags`.
    pub tags: Vec<String>,
    /// `draft: true` or `publish: false`: left out of the site.
    pub hidden: bool,
    /// `art:` (else `cover:`), as written: the link to its image
    /// ([`crate::art`]).
    pub art: Option<String>,
    /// `media:` (else `recording:`), as written: the link to what it
    /// plays (a media item's, `architect_media::Item`).
    pub media: Option<String>,
}

impl Meta {
    /// `page`'s frontmatter, whichever way it is written.
    #[must_use]
    pub fn of(page: &VaultPage) -> Self {
        let fields = toml_block(&page.raw).map_or_else(
            || {
                page.parsed
                    .frontmatter
                    .iter()
                    .map(|e| (e.key.clone(), e.value.clone()))
                    .collect::<Map<String, Value>>()
            },
            |(fields, _)| fields,
        );
        Self::from_fields(&fields)
    }

    fn from_fields(fields: &Map<String, Value>) -> Self {
        let text = |key: &str| {
            fields
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        };
        let weight = ["weight", "order"]
            .iter()
            .find_map(|key| fields.get(*key).and_then(Value::as_i64));
        let mut tags = Vec::new();
        let taxonomy = fields
            .get("taxonomies")
            .and_then(|t| t.get("tags"))
            .into_iter();
        for value in fields.get("tags").into_iter().chain(taxonomy) {
            match value {
                Value::String(s) => tags.extend(s.split([',', ' ']).map(str::to_owned)),
                Value::Array(list) => {
                    tags.extend(list.iter().filter_map(Value::as_str).map(str::to_owned));
                }
                _ => {}
            }
        }
        for tag in &mut tags {
            *tag = tag.trim().trim_start_matches('#').to_lowercase();
        }
        tags.retain(|t| !t.is_empty());
        let flag = |key: &str| fields.get(key).and_then(Value::as_bool);
        Self {
            title: text("title"),
            description: text("description")
                .or_else(|| text("summary"))
                .or_else(|| text("blurb")),
            weight,
            tags,
            hidden: flag("draft") == Some(true) || flag("publish") == Some(false),
            art: crate::art::art_link(fields),
            media: architect_media::item::MEDIA.iter().find_map(|k| text(k)),
        }
    }
}

/// The note's text after its frontmatter, of either kind.
#[must_use]
pub fn body(raw: &str) -> &str {
    if let Some((_, rest)) = toml_block(raw) {
        return rest;
    }
    let (_, offset) = parse_frontmatter(raw);
    raw.get(offset..).unwrap_or(raw)
}

/// A leading `+++` TOML block: its fields as JSON, and the text after it.
/// `None` when the note does not start with one or it does not read.
fn toml_block(raw: &str) -> Option<(Map<String, Value>, &str)> {
    let rest = raw
        .strip_prefix("+++\n")
        .or_else(|| raw.strip_prefix("+++\r\n"))?;
    let mut offset = 0_usize;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "+++" {
            let block = rest.get(..offset)?;
            let after = rest.get(offset.saturating_add(line.len())..)?;
            let table: toml::Table = toml::from_str(block).ok()?;
            let Value::Object(fields) = serde_json::to_value(table).ok()? else {
                return None;
            };
            return Some((fields, after));
        }
        offset = offset.saturating_add(line.len());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_frontmatter_reads_like_yaml() {
        let raw = "+++\ntitle = \"The shell\"\nweight = 20\n[taxonomies]\ntags = [\"ui\"]\n+++\n\nBody.\n";
        let (fields, rest) = toml_block(raw).expect("a toml block");
        let meta = Meta::from_fields(&fields);
        assert_eq!(meta.title.as_deref(), Some("The shell"));
        assert_eq!(meta.weight, Some(20));
        assert_eq!(meta.tags, vec!["ui"]);
        assert_eq!(rest, "\nBody.\n");
        assert_eq!(body(raw), "\nBody.\n");
    }

    #[test]
    fn yaml_body_and_flags() {
        assert_eq!(body("---\ntitle: x\n---\nHi\n"), "Hi\n");
        assert_eq!(body("No frontmatter"), "No frontmatter");
        let mut fields = Map::new();
        fields.insert("publish".into(), Value::Bool(false));
        fields.insert("tags".into(), Value::String("a, b".into()));
        let meta = Meta::from_fields(&fields);
        assert!(meta.hidden);
        assert_eq!(meta.tags, vec!["a", "b"]);
    }
}
