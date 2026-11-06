//! A note's tags edited as text: added, removed, cleared, renamed and
//! deleted, in its frontmatter (`tags`/`tag`, and `aliases`/`alias`
//! entries written as `#tag`) and in its body (inline `#tags`, as
//! [`TAG_REGEX`] reads them: not in code, not in links).
//!
//! A tag is compared by its key ([`tag_key`]): no `#`, no `/` at either
//! end, any case. A tag holds the tags below it: `project` holds
//! `project/alpha`.
//!
//! - [`TagEdit::Add`] skips a note that has the tag or one above it, and
//!   drops the ones below it first; it writes the tag, as given, into the
//!   frontmatter's `tags` (or `tag`) list, never inline.
//! - [`TagEdit::Remove`] removes the tag itself, not the ones below it.
//! - [`TagEdit::Clear`] removes the `tags`/`tag` property and every inline
//!   tag.
//! - [`TagEdit::Rename`] renames the tag and the ones below it, keeping
//!   the rest of each below as written (`a/b` to `x`: `#A/B/Child` becomes
//!   `#x/Child`); whether an entry had a `#` is kept.
//! - [`TagEdit::Delete`] removes the tag and the ones below it.
//!
//! A frontmatter value that is a list stays a list; one written as text
//! (`tags: a, b`) is split on commas and spaces, and written back as text
//! when one tag is left, as a list when more are. A property left empty
//! goes. The frontmatter is written again only when it changed, and the
//! body only where a tag was.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::mutate::{MutateError, has_yaml_content};
use crate::obsidian_parse::{TAG_REGEX, parse_frontmatter, serialize_frontmatter};

/// A change to a note's tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagEdit<'a> {
    Add(&'a str),
    Remove(&'a str),
    Clear,
    Rename { from: &'a str, to: &'a str },
    Delete(&'a str),
}

/// A tag's key: no `#`, no `/` at either end, lower case.
#[must_use]
pub fn tag_key(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase()
}

/// Whether the tag keyed `key` is `want` or below it.
#[must_use]
pub fn is_or_under(key: &str, want: &str) -> bool {
    key == want || under(key, want)
}

/// Whether the tag keyed `key` is below `want`.
#[must_use]
pub fn under(key: &str, want: &str) -> bool {
    key.len() > want.len() && key.starts_with(want) && key.as_bytes().get(want.len()) == Some(&b'/')
}

/// Whether `tag` can be a tag's name: not empty, no spaces, no `#`, no
/// `/` at either end or twice in a row; letters, numbers, `_`, `-`, `/`,
/// and the symbols past ASCII (emoji) (Notebook Navigator's
/// `hasValidTagCharacters`).
#[must_use]
pub fn valid_tag(tag: &str) -> bool {
    let t = tag.trim().strip_prefix('#').unwrap_or(tag.trim());
    !t.is_empty()
        && !t.starts_with('/')
        && !t.ends_with('/')
        && !t.contains("//")
        && t.chars().any(char::is_alphanumeric)
        && t.chars().all(|c| {
            c.is_alphanumeric()
                || matches!(c, '_' | '-' | '/')
                || (!c.is_ascii() && !c.is_whitespace())
        })
}

fn tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(TAG_REGEX).expect("TAG_REGEX is valid"))
}

fn skip_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // Inline code, and wikilinks and embeds (`[[Page#heading]]`).
    R.get_or_init(|| Regex::new(r"`[^`\n]+`|!?\[\[[^\]\n]*\]\]").expect("valid"))
}

/// The inline tags of `body` (a note's text after its frontmatter): each
/// `#tag`'s bytes, `#` included, and the tag without it. Not in fenced
/// code, inline code or links, nor a link's `#anchor`.
#[must_use]
pub fn inline_tags(body: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let text = line.trim_end_matches(['\r', '\n']);
        let ts = text.trim_start();
        if let Some(f) = fence {
            let c = f.chars().next().unwrap_or('`');
            if ts.starts_with(f) && ts.chars().all(|x| x == c || x.is_whitespace()) {
                fence = None;
            }
            continue;
        }
        if ts.starts_with("```") || ts.starts_with("~~~") {
            fence = Some(if ts.starts_with("```") { "```" } else { "~~~" });
            continue;
        }
        let skip: Vec<Range<usize>> = skip_re().find_iter(text).map(|m| m.range()).collect();
        for caps in tag_re().captures_iter(text) {
            let Some(name) = caps.get(1) else { continue };
            let hash = name.start().saturating_sub(1);
            let span = hash..name.end();
            if skip
                .iter()
                .any(|s| s.start <= span.start && span.end <= s.end)
            {
                continue;
            }
            let before = text[..hash].trim_end();
            if before.ends_with("]:") || before.ends_with("](") {
                continue;
            }
            out.push((
                start + span.start..start + span.end,
                name.as_str().to_owned(),
            ));
        }
    }
    out
}

/// Every tag of the note `raw`, as written (without `#`): its
/// frontmatter's, then its inline ones.
#[must_use]
pub fn note_tags(raw: &str) -> Vec<String> {
    let (fm, offset) = parse_frontmatter(raw);
    let mut out = Vec::new();
    for e in &fm {
        if let Some(alias) = field(&e.key) {
            for entry in entries(&e.value) {
                if let Some(t) = tag_entry(&entry, alias) {
                    out.push(t.to_owned());
                }
            }
        }
    }
    out.extend(inline_tags(&raw[offset..]).into_iter().map(|(_, t)| t));
    out
}

/// Whether a frontmatter key holds tags: `Some(false)` for `tags`/`tag`,
/// `Some(true)` for `aliases`/`alias` (whose `#` entries are tags).
fn field(key: &str) -> Option<bool> {
    match key.to_lowercase().as_str() {
        "tags" | "tag" => Some(false),
        "aliases" | "alias" => Some(true),
        _ => None,
    }
}

/// A field's entries: a list's text items, or text split on commas and
/// spaces.
fn entries(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Value::String(s) => split(s),
        _ => Vec::new(),
    }
}

fn split(s: &str) -> Vec<String> {
    s.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The tag an entry is, without `#`: in an alias field only one written
/// with `#`.
fn tag_entry(entry: &str, alias: bool) -> Option<&str> {
    let t = entry.trim();
    if alias {
        t.strip_prefix('#')
    } else {
        Some(t.strip_prefix('#').unwrap_or(t))
    }
    .filter(|t| !t.is_empty())
}

/// What becomes of one tag.
enum Fate {
    Keep,
    Drop,
    Become(String),
}

/// `tag` (as written, no `#`) under `edit`.
fn fate(tag: &str, edit: &TagEdit<'_>) -> Fate {
    let key = tag_key(tag);
    match *edit {
        TagEdit::Add(want) => {
            if under(&key, &tag_key(want)) {
                Fate::Drop
            } else {
                Fate::Keep
            }
        }
        TagEdit::Remove(want) => {
            if key == tag_key(want) {
                Fate::Drop
            } else {
                Fate::Keep
            }
        }
        TagEdit::Clear => Fate::Drop,
        TagEdit::Delete(want) => {
            if is_or_under(&key, &tag_key(want)) {
                Fate::Drop
            } else {
                Fate::Keep
            }
        }
        TagEdit::Rename { from, to } => {
            let from = tag_key(from);
            if !is_or_under(&key, &from) {
                return Fate::Keep;
            }
            let to = to.trim().trim_start_matches('#').trim_matches('/');
            let tail: String = tag.chars().skip(from.chars().count()).collect();
            Fate::Become(format!("{to}{tail}"))
        }
    }
}

/// `edit` done to the note `raw`: its new text, or `None` when nothing
/// changed (an [`TagEdit::Add`] to a note that has the tag or one above
/// it).
///
/// # Errors
///
/// [`MutateError::InvalidFrontmatter`] when the note's frontmatter block
/// is not a YAML mapping.
pub fn edit_tags(raw: &str, edit: &TagEdit<'_>) -> Result<Option<String>, MutateError> {
    let (mut fm, offset) = parse_frontmatter(raw);
    if fm.is_empty() && offset > 0 && has_yaml_content(&raw[..offset]) {
        return Err(MutateError::InvalidFrontmatter(
            "the frontmatter block isn't a YAML mapping; fix it by hand first".into(),
        ));
    }
    if let TagEdit::Add(want) = edit {
        let want = tag_key(want);
        let has = note_tags(raw).iter().any(|t| {
            let k = tag_key(t);
            k == want || under(&want, &k)
        });
        if has {
            return Ok(None);
        }
    }
    let before = fm.clone();
    // The frontmatter.
    for e in &mut fm {
        let Some(alias) = field(&e.key) else { continue };
        if matches!(edit, TagEdit::Clear) {
            if !alias {
                e.value = Value::Null;
            }
            continue;
        }
        let was_list = matches!(e.value, Value::Array(_));
        let items = entries(&e.value);
        let mut changed = false;
        let mut kept: Vec<String> = Vec::new();
        for item in &items {
            let Some(tag) = tag_entry(item, alias) else {
                kept.push(item.clone());
                continue;
            };
            let hash = item.trim().starts_with('#');
            match fate(tag, edit) {
                Fate::Keep => kept.push(item.clone()),
                Fate::Drop => changed = true,
                Fate::Become(new) => {
                    changed = true;
                    kept.push(if hash { format!("#{new}") } else { new });
                }
            }
        }
        if !changed {
            continue;
        }
        e.value = match (kept.len(), was_list) {
            (0, _) => Value::Null,
            (1, false) => Value::String(kept.remove(0)),
            _ => Value::Array(kept.into_iter().map(Value::String).collect()),
        };
    }
    // A property emptied goes (a `null` set above: cleared or emptied).
    fm.retain(|e| !(field(&e.key).is_some() && e.value.is_null()));
    if let TagEdit::Add(want) = edit {
        let want = want
            .trim()
            .trim_start_matches('#')
            .trim_matches('/')
            .to_owned();
        let at = fm
            .iter()
            .position(|e| e.key.eq_ignore_ascii_case("tags"))
            .or_else(|| fm.iter().position(|e| e.key.eq_ignore_ascii_case("tag")));
        match at.and_then(|i| fm.get_mut(i)) {
            Some(e) => {
                let mut list: Vec<String> = match &e.value {
                    Value::Array(items) => items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    Value::String(s) => s
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    _ => Vec::new(),
                };
                list.push(want);
                let mut seen = std::collections::BTreeSet::new();
                list.retain(|t| seen.insert(tag_key(t)));
                e.value = Value::Array(list.into_iter().map(Value::String).collect());
            }
            None => fm.push(crate::obsidian_parse::FrontmatterEntry {
                key: "tags".to_owned(),
                value: Value::Array(vec![Value::String(want)]),
            }),
        }
    }
    // The body: last first, so the earlier spans hold.
    let body = &raw[offset..];
    let mut text = body.to_owned();
    let mut body_changed = false;
    for (span, tag) in inline_tags(body).into_iter().rev() {
        match fate(&tag, edit) {
            Fate::Keep => {}
            Fate::Drop => {
                // With the spaces before it; or, when it starts its
                // line, the spaces after it.
                let head = &text[..span.start];
                let before = head.len() - head.trim_end_matches([' ', '\t']).len();
                let starts_line = head[..head.len() - before].is_empty()
                    || head[..head.len() - before].ends_with('\n');
                let (from, to) = if starts_line {
                    let tail = &text[span.end..];
                    let after = tail.len() - tail.trim_start_matches([' ', '\t']).len();
                    (span.start, span.end + after)
                } else {
                    (span.start - before, span.end)
                };
                text.replace_range(from..to, "");
                body_changed = true;
            }
            Fate::Become(new) => {
                text.replace_range(span, &format!("#{new}"));
                body_changed = true;
            }
        }
    }
    let fm_changed = fm != before;
    if !fm_changed && !body_changed {
        return Ok(None);
    }
    let head = if fm_changed {
        serialize_frontmatter(&fm)
    } else {
        raw[..offset].to_owned()
    };
    Ok(Some(format!("{head}{text}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(raw: &str, e: TagEdit<'_>) -> String {
        edit_tags(raw, &e)
            .unwrap()
            .unwrap_or_else(|| raw.to_owned())
    }

    #[test]
    fn inline_tags_skip_code_links_and_anchors() {
        let body = "#one and #two/three.\n`#code` [[Page#four]]\n```\n#fenced\n```\n[a](#anchor) x #five\n";
        let tags: Vec<String> = inline_tags(body).into_iter().map(|(_, t)| t).collect();
        assert_eq!(tags, ["one", "two/three", "five"]);
        let (span, _) = &inline_tags(body)[1];
        assert_eq!(&body[span.clone()], "#two/three");
    }

    #[test]
    fn add_writes_the_list_skips_a_tag_above_and_drops_the_ones_below() {
        assert_eq!(
            edit("Hi\n", TagEdit::Add("work")),
            "---\ntags:\n- work\n---\nHi\n"
        );
        let raw = "---\ntitle: x\ntags: [a, project/alpha]\n---\nText #project/beta here\n";
        let out = edit(raw, TagEdit::Add("Project"));
        assert_eq!(
            out,
            "---\ntitle: x\ntags:\n- a\n- Project\n---\nText here\n"
        );
        assert!(
            edit_tags(&out, &TagEdit::Add("project/gamma"))
                .unwrap()
                .is_none()
        );
        assert!(
            edit_tags("Body #work\n", &TagEdit::Add("WORK"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn remove_takes_only_the_tag_itself_in_any_case() {
        let raw = "---\ntags: [Work, work/meeting]\naliases: ['#work', Other]\n---\nA #WORK b #work/meeting\n";
        let out = edit(raw, TagEdit::Remove("work"));
        assert_eq!(
            out,
            "---\ntags:\n- work/meeting\naliases:\n- Other\n---\nA b #work/meeting\n"
        );
    }

    #[test]
    fn clear_takes_the_property_and_every_inline_tag() {
        let raw = "---\ntitle: x\ntags: [a, b]\n---\n#a starts. Then #b/c.\n";
        assert_eq!(
            edit(raw, TagEdit::Clear),
            "---\ntitle: x\n---\nstarts. Then.\n"
        );
    }

    #[test]
    fn rename_moves_the_tag_and_those_below_keeping_their_rest() {
        let raw = "---\ntags: [a/b, A/B/Child, other]\n---\nSee #A/B/Child and #a/bc and #a/b.\n";
        let out = edit(
            raw,
            TagEdit::Rename {
                from: "a/b",
                to: "x",
            },
        );
        assert_eq!(
            out,
            "---\ntags:\n- x\n- x/Child\n- other\n---\nSee #x/Child and #a/bc and #x.\n"
        );
        // Text frontmatter untouched when only the body has the tag.
        let raw = "---\ntags: [keep]\n---\n#old here\n";
        assert_eq!(
            edit(
                raw,
                TagEdit::Rename {
                    from: "old",
                    to: "new"
                }
            ),
            "---\ntags: [keep]\n---\n#new here\n"
        );
    }

    #[test]
    fn a_text_value_left_with_one_tag_is_text_again() {
        let raw = "---\ntags: a, b\n---\nx\n";
        assert_eq!(edit(raw, TagEdit::Remove("a")), "---\ntags: b\n---\nx\n");
    }

    #[test]
    fn delete_takes_the_tag_and_those_below() {
        let raw = "---\ntags: [music, music/guitar, musical]\n---\nPlay #music/theory now\n";
        assert_eq!(
            edit(raw, TagEdit::Delete("music")),
            "---\ntags:\n- musical\n---\nPlay now\n"
        );
    }

    #[test]
    fn a_tag_name_is_checked() {
        assert!(valid_tag("work/meeting"));
        assert!(valid_tag("#café-2"));
        assert!(!valid_tag("two words"));
        assert!(!valid_tag("a//b"));
        assert!(!valid_tag("/a"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("a.b"));
    }
}
