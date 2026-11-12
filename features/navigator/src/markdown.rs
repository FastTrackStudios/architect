//! A note's text as the navigator shows it, without a vault.
//!
//! Its preview, frontmatter properties, tags, words and tasks, parsed from
//! the markdown alone. Wasm-clean: what an in-memory vault ([`MemorySource`],
//! a browser's) derives its [`NoteFile`]s with. The vault on disk
//! (`vault_source`, feature `vault`) parses with the vault's own parser
//! and shares the preview with this.
//!
//! [`MemorySource`]: crate::MemorySource

use crate::source::{Millis, NoteFile, PropertyValue, TaskCounts};

/// The frontmatter properties a note's feature image is read from, in
/// order (Notebook Navigator's `featureImageProperties`, and the common
/// others).
pub const FEATURE_IMAGE_PROPERTIES: &[&str] = &[
    "thumbnail",
    "featureResized",
    "feature",
    "image",
    "cover",
    "banner",
];

/// How long a preview is, at most (characters).
const PREVIEW_CHARS: usize = 300;

/// A note's text as its preview: its words, without its frontmatter, its
/// fences, headings' and tasks' marks, or links' brackets (a link's alias
/// when it has one); embeds left out.
#[must_use]
pub fn preview_of(text: &str) -> String {
    let body = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---").map(|(_, b)| b))
        .map_or(text, |b| {
            b.trim_start_matches(|c| c != '\n').trim_start_matches('\n')
        });
    let mut out = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || t.is_empty() || t.starts_with('|') {
            continue;
        }
        let t = t
            .trim_start_matches('#')
            .trim_start_matches('>')
            .trim_start();
        let t = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "]
            .iter()
            .find_map(|m| t.strip_prefix(m))
            .unwrap_or(t);
        if !out.is_empty() {
            out.push(' ');
        }
        // Emphasis and code marks are not words.
        let plain: String = unlink(t)
            .chars()
            .filter(|c| !matches!(c, '*' | '`' | '=' | '~'))
            .collect();
        out.push_str(&plain);
        if out.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    let words: Vec<&str> = out.split_whitespace().collect();
    let joined = words.join(" ");
    joined.chars().take(PREVIEW_CHARS).collect()
}

/// `[[a|b]]` as `b`, `[[a]]` as `a`, `![[x]]` and `![](x)` dropped,
/// `[t](u)` as `t`.
pub(crate) fn unlink(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(['[', '!']) {
        let (before, at) = rest.split_at_checked(i).unwrap_or((rest, ""));
        out.push_str(before);
        if let Some(inner) = at.strip_prefix("![[").and_then(|r| r.split_once("]]")) {
            rest = inner.1;
        } else if let Some((target, after)) = at.strip_prefix("[[").and_then(|r| r.split_once("]]"))
        {
            let shown = target.rsplit_once('|').map_or(target, |(_, a)| a);
            let shown = shown.split('#').next().unwrap_or(shown);
            out.push_str(shown.rsplit('/').next().unwrap_or(shown));
            rest = after;
        } else if let Some((_, after)) = at
            .strip_prefix("![")
            .and_then(|r| r.split_once("]("))
            .and_then(|(_, r)| r.split_once(')'))
        {
            rest = after;
        } else if let Some((text, after)) = at
            .strip_prefix('[')
            .and_then(|r| r.split_once("]("))
            .and_then(|(t, r)| r.split_once(')').map(|(_, a)| (t, a)))
        {
            out.push_str(text);
            rest = after;
        } else {
            let mut chars = at.chars();
            if let Some(c) = chars.next() {
                out.push(c);
            }
            rest = chars.as_str();
        }
    }
    out.push_str(rest);
    out
}

/// The targets of a text's image embeds: `![[x.png]]`, `![](x.png)`.
pub(crate) fn embeds(text: &str) -> impl Iterator<Item = String> + '_ {
    let wiki = text.match_indices("![[").filter_map(move |(i, _)| {
        let rest = text.get(i.saturating_add(3)..)?;
        rest.split_once("]]").map(|(t, _)| t.to_owned())
    });
    let md = text.match_indices("![").filter_map(move |(i, _)| {
        let rest = text.get(i.saturating_add(2)..)?;
        let (_, after) = rest.split_once("](")?;
        after.split_once(')').map(|(t, _)| t.to_owned())
    });
    wiki.chain(md)
}

/// A note at `path` from its markdown `text`, as an in-memory vault shows it.
///
/// Its frontmatter's properties, its tags (the frontmatter's `tags`
/// and the body's `#tags`, outside code), its preview, words and tasks,
/// and its feature image (the first of [`FEATURE_IMAGE_PROPERTIES`], else
/// the first image it embeds; the source's `resource_url` draws it).
#[must_use]
pub fn note_from_markdown(path: &str, text: &str, created: Millis, modified: Millis) -> NoteFile {
    let mut file = NoteFile::new(path, created, modified);
    if !has_markdown_ext(path) {
        return file;
    }
    let (front, body) = split_frontmatter(text);
    let properties = front.map(properties_of).unwrap_or_default();
    let mut tags: Vec<String> = Vec::new();
    for (key, value) in &properties {
        if key.eq_ignore_ascii_case("tags") || key.eq_ignore_ascii_case("tag") {
            let list = match value {
                PropertyValue::List(l) => l.clone(),
                PropertyValue::Text(t) => t
                    .split([',', ' '])
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect(),
                _ => Vec::new(),
            };
            for t in list {
                push_tag(&mut tags, &t);
            }
        }
    }
    let (words, tasks) = scan_body(body, &mut tags);
    let named = FEATURE_IMAGE_PROPERTIES.iter().find_map(|key| {
        properties
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropertyValue::Text(t) if !t.is_empty() => Some(t.clone()),
                PropertyValue::List(l) => l.first().cloned(),
                _ => None,
            })
    });
    file.feature_image = named.or_else(|| embeds(text).next());
    file.properties = properties;
    file.tags = tags;
    file.preview = Some(preview_of(text));
    file.words = Some(words);
    file.tasks = Some(tasks);
    file
}

fn has_markdown_ext(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
}

/// The frontmatter (between the first two `---` lines) and the rest.
fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let mut at = 0_usize;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let front = rest.get(..at).unwrap_or_default();
            let body = rest
                .get(at.saturating_add(line.len())..)
                .unwrap_or_default();
            return (Some(front), body);
        }
        at = at.saturating_add(line.len());
    }
    (None, text)
}

/// The frontmatter's top-level keys, in order: a scalar, or a list of
/// them. A frontmatter that is not a YAML map has none.
fn properties_of(front: &str) -> Vec<(String, PropertyValue)> {
    let Ok(serde_yaml::Value::Mapping(map)) = serde_yaml::from_str::<serde_yaml::Value>(front)
    else {
        return Vec::new();
    };
    map.into_iter()
        .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), property(&v))))
        .collect()
}

fn scalar(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => String::new(),
        other => serde_yaml::to_string(other)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default(),
    }
}

fn property(v: &serde_yaml::Value) -> PropertyValue {
    match v {
        serde_yaml::Value::Bool(b) => PropertyValue::Bool(*b),
        serde_yaml::Value::Number(n) => n
            .as_f64()
            .map_or_else(|| PropertyValue::Text(n.to_string()), PropertyValue::Number),
        serde_yaml::Value::Sequence(s) => PropertyValue::List(s.iter().map(scalar).collect()),
        other => PropertyValue::Text(scalar(other)),
    }
}

/// A tag as the navigator keeps one: no `#`, once (by its case-folded
/// form).
fn push_tag(tags: &mut Vec<String>, raw: &str) {
    let t = raw.trim().trim_start_matches('#');
    if t.is_empty() || tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
        return;
    }
    tags.push(t.to_owned());
}

/// The body's words and tasks, and its `#tags` (outside fences and
/// inline code) into `tags`.
fn scan_body(body: &str, tags: &mut Vec<String>) -> (u32, TaskCounts) {
    let (mut words, mut total, mut open) = (0_u32, 0_u32, 0_u32);
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let item = ["- ", "* ", "+ "].iter().find_map(|m| t.strip_prefix(m));
        if let Some(rest) = item {
            if rest.starts_with("[ ]") {
                total = total.saturating_add(1);
                open = open.saturating_add(1);
            } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
                total = total.saturating_add(1);
            }
        }
        let mut in_code = false;
        for word in t.split_whitespace() {
            if word.matches('`').count() % 2 == 1 {
                in_code = !in_code;
            }
            let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
            if !bare.is_empty() && !word.starts_with('#') {
                words = words.saturating_add(1);
            }
            if !in_code && let Some(tag) = word.strip_prefix('#') {
                let tag = tag.trim_end_matches(|c: char| {
                    !(c.is_alphanumeric() || matches!(c, '/' | '-' | '_'))
                });
                if !tag.is_empty()
                    && !tag.chars().all(|c| c.is_ascii_digit())
                    && tag.chars().next().is_some_and(char::is_alphanumeric)
                {
                    push_tag(tags, tag);
                }
            }
        }
    }
    (
        words,
        TaskCounts {
            total,
            unfinished: open,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_its_frontmatter_tags_preview_words_and_tasks() {
        let text = "---\ntitle: Welcome\ntags: [lab/demo, start]\nstatus: active\ncount: 3\ndone: false\ncover: \"[[Attachments/a.png]]\"\n---\n# Welcome\n\nSome words here #inline and `#not` a tag.\n\n- [ ] one\n- [x] two\n\n```\n#nope\n```\n";
        let f = note_from_markdown("Notes/Welcome.md", text, 1, 2);
        assert_eq!(f.tags, vec!["lab/demo", "start", "inline"]);
        assert!(f.properties.contains(&(
            "status".to_owned(),
            PropertyValue::Text("active".to_owned())
        )));
        assert!(
            f.properties
                .contains(&("count".to_owned(), PropertyValue::Number(3.0)))
        );
        assert!(
            f.properties
                .contains(&("done".to_owned(), PropertyValue::Bool(false)))
        );
        assert_eq!(
            f.tasks,
            Some(TaskCounts {
                total: 2,
                unfinished: 1
            })
        );
        assert_eq!(f.feature_image.as_deref(), Some("[[Attachments/a.png]]"));
        assert!(
            f.preview
                .as_deref()
                .unwrap_or_default()
                .starts_with("Welcome Some words here")
        );
        assert!(f.words.unwrap_or_default() > 5);
    }

    #[test]
    fn a_note_without_frontmatter_or_markdown_is_plain() {
        let f = note_from_markdown("a.md", "just text", 0, 0);
        assert!(f.properties.is_empty() && f.tags.is_empty());
        assert_eq!(f.preview.as_deref(), Some("just text"));
        let png = note_from_markdown("a.png", "x", 0, 0);
        assert_eq!(png.preview, None);
    }
}
