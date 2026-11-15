//! A note as a base's row (`bases::BaseRow`): its file's facts, its
//! frontmatter, its tags (frontmatter and inline `#tag`) and its links
//! (frontmatter and body `[[wikilinks]]`), so `file.tags`, `file.links`,
//! `file.hasLink()` and link-follow (`note.song.key`) work alike over a
//! vault on disk and a replica in a browser.

use uuid::Uuid;

use crate::bases::BaseRow;
use crate::obsidian_parse::{ParsedPage, parse_page};

/// Rows from `(vault-relative path, markdown)` pairs — for callers that
/// hold note bytes rather than a filesystem vault (the MCP lane reads
/// them through `VaultSync`). Non-`.md` paths are skipped.
pub fn rows_from_markdown<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<BaseRow> {
    files
        .into_iter()
        .filter(|(path, _)| path.ends_with(".md"))
        .map(|(path, raw)| {
            let (folder, basename) = split_rel(path);
            row_of(path, &basename, &folder, &parse_page(raw))
        })
        .collect()
}

/// `Music/Charts.md` → (`Music`, `Charts`).
pub fn split_rel(rel: &str) -> (String, String) {
    let (folder, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let basename = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    (folder.to_string(), basename.to_string())
}

/// One parsed note as a row.
#[must_use]
pub fn row_of(rel_path: &str, basename: &str, folder: &str, parsed: &ParsedPage) -> BaseRow {
    // The frontmatter goes to the row as a map, not as a JSON string
    // for `BaseRow::from_parts_full` to parse back (zero-copy audit B1:
    // that was two more copies of every page's frontmatter per query).
    // It is still collected through a `serde_json::Map` so the row sees
    // what it saw before: last write wins on a duplicate key (how
    // Obsidian collapses them), and keys in the Map's order, which is
    // sorted (serde_json is built without `preserve_order`). The test
    // `rows_match_the_json_round_trip` pins the two readings.
    let mut obj = serde_json::Map::with_capacity(parsed.frontmatter.len());
    for entry in &parsed.frontmatter {
        obj.insert(entry.key.clone(), entry.value.clone());
    }
    let frontmatter: indexmap::IndexMap<String, serde_json::Value> = obj.into_iter().collect();

    // Inline `#tag` markers get merged into `file.tags` so
    // `file.tags.contains("chart")` filters match the way Obsidian's
    // bases engine does (frontmatter ∪ inline body tags). Body
    // `[[wikilinks]]` join the frontmatter links behind `file.links`.
    let mut inline_tags: Vec<String> = Vec::new();
    let mut body_links: Vec<&str> = Vec::new();
    for block in &parsed.blocks {
        for r in &block.refs {
            match r {
                crate::refs::Ref::Tag(t) => {
                    let joined = t.path.join("/");
                    if !joined.is_empty() && !inline_tags.iter().any(|x| x == &joined) {
                        inline_tags.push(joined);
                    }
                }
                crate::refs::Ref::Link(l) => body_links.push(&l.target_linkpath),
                _ => {}
            }
        }
    }

    let ext = rel_path
        .rsplit_once('.')
        .map(|(_, e)| e.to_string())
        .unwrap_or_default();

    BaseRow::from_frontmatter(
        Uuid::new_v4(),
        basename,
        rel_path,
        folder,
        ext,
        frontmatter,
        &inline_tags,
    )
    .with_links(body_links)
}
