//! A note's parsed metadata, by line: its headings, list items and tasks,
//! block ids (`^id`) and inline fields (`key:: value`), what Dataview and
//! Tasks read from a page (Obsidian's `CachedMetadata`). `VAULT_METADATA`
//! answers it (`vault::actions`).
//!
//! One pass over the text, frontmatter and fenced code left out. Lines
//! are 1-based, as the outline's and the tasks' are.

use serde::Serialize;

/// A heading: `#` to `######`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetaHeading {
    /// 1 to 6.
    pub level: u8,
    /// Its text, the `#`s and a trailing `^id` left out.
    pub text: String,
    pub line: usize,
}

/// A list item, and a task when it has a `[ ]` box.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetaListItem {
    pub line: usize,
    /// Its text, the bullet, the box and a trailing `^id` left out.
    pub text: String,
    /// How deep it is: 0 at the left margin, one a level of nesting.
    pub depth: u32,
    /// The line of the item it is nested under, if any.
    pub parent: Option<usize>,
    /// Whether it is numbered (`1.`).
    pub ordered: bool,
    /// The box's character when it is a task: `' '` open, `x` done,
    /// `/`, `-` and the rest as a theme draws them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<char>,
    /// Whether the task is done (`x` or `X`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    /// Its block id, when it ends with `^id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// The heading it is under, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

/// A block id: a line that ends with `^id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetaBlock {
    pub id: String,
    pub line: usize,
}

/// An inline field: a line `key:: value`, or `[key:: value]` /
/// `(key:: value)` inside one (Dataview's).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MetaField {
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// Everything [`metadata`] finds in a note.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NoteMetadata {
    pub headings: Vec<MetaHeading>,
    pub list_items: Vec<MetaListItem>,
    pub blocks: Vec<MetaBlock>,
    pub fields: Vec<MetaField>,
}

impl NoteMetadata {
    /// Its list items that are tasks.
    pub fn tasks(&self) -> impl Iterator<Item = &MetaListItem> {
        self.list_items.iter().filter(|i| i.task.is_some())
    }
}

/// A fence's opening run: its character and length, when `line` opens
/// one (CommonMark: up to three spaces, then three or more of one of
/// `` ` `` or `~`; a backtick fence's info string holds no backtick).
fn fence_open(line: &str) -> Option<(char, usize)> {
    let lead = line.len() - line.trim_start_matches(' ').len();
    if lead > 3 {
        return None;
    }
    let rest = &line[lead..];
    let c = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = rest.chars().take_while(|x| *x == c).count();
    if run < 3 {
        return None;
    }
    let info = &rest[run..];
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, run))
}

/// Whether `line` closes a fence opened by `c` × `run`: up to three
/// spaces, at least `run` of `c`, then only spaces.
fn fence_closes(line: &str, c: char, run: usize) -> bool {
    let lead = line.len() - line.trim_start_matches(' ').len();
    if lead > 3 {
        return false;
    }
    let rest = &line[lead..];
    let n = rest.chars().take_while(|x| *x == c).count();
    n >= run && rest[n..].trim().is_empty()
}

/// Where the text after frontmatter starts: past a `---` first line and
/// the `---` (or `...`) that closes it; none when it never closes (then
/// the first line is a thematic break, not frontmatter).
fn body_start(lines: &[&str]) -> usize {
    if lines.first().map(|l| l.trim_end_matches('\r').trim_end()) != Some("---") {
        return 0;
    }
    lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| matches!(l.trim_end_matches('\r').trim_end(), "---" | "..."))
        .map_or(0, |(n, _)| n.saturating_add(1))
}

/// What a line is inside of, that holds no metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    /// A fenced code block, by its run.
    Fence(char, usize),
    /// An Obsidian comment, `%%` to `%%`.
    Comment,
    /// A math block, `$$` to `$$`.
    Math,
}

/// The metadata of a note's text.
#[must_use]
pub fn metadata(text: &str) -> NoteMetadata {
    let mut out = NoteMetadata::default();
    let all: Vec<&str> = text.split('\n').collect();
    let start = body_start(&all);
    let mut skip: Option<Skip> = None;
    let mut section: Option<String> = None;
    // The open list items: (indent width, line).
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut blank_before = true;
    for (i, raw) in all.iter().enumerate().skip(start) {
        let line = i.saturating_add(1);
        let l = raw.trim_end_matches('\r');
        let trimmed = l.trim();
        match skip {
            Some(Skip::Fence(c, run)) => {
                if fence_closes(l, c, run) {
                    skip = None;
                }
                continue;
            }
            Some(Skip::Comment) => {
                if trimmed.ends_with("%%") {
                    skip = None;
                }
                continue;
            }
            Some(Skip::Math) => {
                if trimmed.ends_with("$$") {
                    skip = None;
                }
                continue;
            }
            None => {}
        }
        if let Some((c, run)) = fence_open(l) {
            skip = Some(Skip::Fence(c, run));
            continue;
        }
        // A block comment or math block: `%%` / `$$` opening a line, not
        // closed on it.
        for (mark, kind) in [("%%", Skip::Comment), ("$$", Skip::Math)] {
            if let Some(rest) = trimmed.strip_prefix(mark)
                && !rest.contains(mark)
            {
                skip = Some(kind);
            }
        }
        if skip.is_some() {
            continue;
        }
        // Indented code: four spaces (or a tab) after a blank line, not a
        // list's continuation.
        let indented = l.starts_with('\t') || l.starts_with("    ");
        if indented && blank_before && stack.is_empty() && !trimmed.is_empty() {
            continue;
        }
        blank_before = trimmed.is_empty();
        // An inline comment's text is no metadata either.
        let uncommented = strip_inline(l, "%%");
        let (content, block) = block_id(&uncommented);
        if let Some(id) = &block {
            out.blocks.push(MetaBlock {
                id: id.clone(),
                line,
            });
        }
        if let Some((level, text)) = heading(content) {
            let (text, _) = block_id(text);
            section = Some(text.trim().to_owned());
            out.headings.push(MetaHeading {
                level,
                text: text.trim().to_owned(),
                line,
            });
            stack.clear();
            continue;
        }
        if let Some(item) = list_item(content) {
            while stack.last().is_some_and(|(w, _)| *w >= item.indent) {
                stack.pop();
            }
            let parent = stack.last().map(|(_, l)| *l);
            let depth = u32::try_from(stack.len()).unwrap_or(u32::MAX);
            stack.push((item.indent, line));
            fields_in(item.text, line, &mut out.fields);
            out.list_items.push(MetaListItem {
                line,
                text: item.text.trim().to_owned(),
                depth,
                parent,
                ordered: item.ordered,
                task: item.task,
                done: item.task.map(|c| matches!(c, 'x' | 'X')),
                block,
                section: section.clone(),
            });
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        // A line that is not a list item's continuation ends the list.
        if !l.starts_with([' ', '\t']) {
            stack.clear();
        }
        fields_in(content, line, &mut out.fields);
    }
    out
}

/// `line` with every `mark`…`mark` span on it taken out.
fn strip_inline(line: &str, mark: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(mark) {
        let after = &rest[at.saturating_add(mark.len())..];
        let Some(end) = after.find(mark) else { break };
        out.push_str(&rest[..at]);
        rest = &after[end.saturating_add(mark.len())..];
    }
    out.push_str(rest);
    out
}

/// `line` without a trailing `^id`, and the id.
fn block_id(line: &str) -> (&str, Option<String>) {
    let t = line.trim_end();
    if let Some(at) = t.rfind('^') {
        let id = &t[at.saturating_add(1)..];
        let before = &t[..at];
        let ok_id = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if ok_id && (before.is_empty() || before.ends_with([' ', '\t'])) {
            return (before.trim_end(), Some(id.to_owned()));
        }
    }
    (line, None)
}

/// A heading's level and text.
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some((u8::try_from(hashes).ok()?, rest.trim().trim_end_matches('#').trim_end()))
}

struct Item<'a> {
    indent: usize,
    ordered: bool,
    task: Option<char>,
    text: &'a str,
}

/// A list item: `- `, `* `, `+ ` or `1. ` / `1) `, then maybe `[m] `.
fn list_item(line: &str) -> Option<Item<'_>> {
    let rest = line.trim_start_matches([' ', '\t']);
    let lead = &line[..line.len().saturating_sub(rest.len())];
    let indent = lead.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
    let (ordered, after) = if let Some(r) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        (false, r)
    } else if matches!(rest, "-" | "*" | "+") {
        (false, "")
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let r = &rest[digits..];
        (
            true,
            r.strip_prefix(". ").or_else(|| r.strip_prefix(") "))?,
        )
    };
    let mut task = None;
    let mut text = after;
    let mut chars = after.chars();
    if chars.next() == Some('[')
        && let Some(m) = chars.next()
        && chars.next() == Some(']')
    {
        let tail = chars.as_str();
        if tail.is_empty() || tail.starts_with([' ', '\t']) {
            task = Some(m);
            text = tail.trim_start();
        }
    }
    Some(Item {
        indent,
        ordered,
        task,
        text,
    })
}

/// The inline fields in `text`: `key:: value` as the whole line, else
/// each `[key:: value]` and `(key:: value)`.
fn fields_in(text: &str, line: usize, out: &mut Vec<MetaField>) {
    let t = text.trim();
    if let Some((key, value)) = t.split_once("::")
        && is_key(key)
        && !key.contains(['[', '('])
    {
        out.push(MetaField {
            key: key.trim().to_owned(),
            value: value.trim().to_owned(),
            line,
        });
        return;
    }
    for (open, close) in [('[', ']'), ('(', ')')] {
        let mut rest = t;
        while let Some(at) = rest.find(open) {
            let after = &rest[at.saturating_add(1)..];
            let Some(end) = after.find(close) else { break };
            let inner = &after[..end];
            if let Some((key, value)) = inner.split_once("::")
                && is_key(key)
            {
                out.push(MetaField {
                    key: key.trim().to_owned(),
                    value: value.trim().to_owned(),
                    line,
                });
            }
            rest = &after[end..];
        }
    }
}

/// Whether `key` is a field's key: words, `-` and `_`, not empty.
fn is_key(key: &str) -> bool {
    let k = key.trim();
    !k.is_empty()
        && k
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const NOTE: &str = "---\ntitle: x\n---\n# Plan ^top\nstatus:: active\n\n## Today\n- [ ] Call Sam [due:: 2026-11-20] ^call\n  - [x] Find the number\n  - note (who:: Sam)\n1. First\n\n```\n- [ ] not a task\n# not a heading\n```\nSee ^here\n";

    #[test]
    fn it_finds_headings_items_tasks_blocks_and_fields() {
        let m = metadata(NOTE);
        assert_eq!(
            m.headings,
            [
                MetaHeading { level: 1, text: "Plan".into(), line: 4 },
                MetaHeading { level: 2, text: "Today".into(), line: 7 },
            ]
        );
        let items = &m.list_items;
        assert_eq!(items.len(), 4, "{items:#?}");
        assert_eq!(items[0].text, "Call Sam [due:: 2026-11-20]");
        assert_eq!(items[0].task, Some(' '));
        assert_eq!(items[0].done, Some(false));
        assert_eq!(items[0].block.as_deref(), Some("call"));
        assert_eq!(items[0].section.as_deref(), Some("Today"));
        assert_eq!((items[1].depth, items[1].parent), (1, Some(8)));
        assert_eq!(items[1].done, Some(true));
        assert_eq!((items[2].task, items[2].parent), (None, Some(8)));
        assert!(items[3].ordered);
        assert_eq!((items[3].depth, items[3].parent), (0, None));
        assert_eq!(m.tasks().count(), 2);
        assert_eq!(
            m.blocks.iter().map(|b| (b.id.as_str(), b.line)).collect::<Vec<_>>(),
            [("top", 4), ("call", 8), ("here", 17)]
        );
        assert_eq!(
            m.fields
                .iter()
                .map(|f| (f.key.as_str(), f.value.as_str(), f.line))
                .collect::<Vec<_>>(),
            [
                ("status", "active", 5),
                ("due", "2026-11-20", 8),
                ("who", "Sam", 10),
            ]
        );
    }

    #[test]
    fn a_link_or_a_url_is_not_a_field() {
        let m = metadata("See [[a::b]] and https://x.y::z\n");
        assert!(m.fields.is_empty(), "{:?}", m.fields);
    }

    fn heads(m: &NoteMetadata) -> Vec<&str> {
        m.headings.iter().map(|h| h.text.as_str()).collect()
    }

    #[test]
    fn a_fence_closes_only_on_its_own_run() {
        // A longer opening run, a shorter one inside, a ~~~ inside ```.
        let m = metadata("````\n```\n# no\n~~~\n````\n# yes\n");
        assert_eq!(heads(&m), ["yes"]);
        // A closing run with text after it does not close.
        let m = metadata("```\n``` not closed\n# no\n```\n# yes\n");
        assert_eq!(heads(&m), ["yes"]);
        // Inline code is no fence.
        let m = metadata("```inline```\n# yes\n");
        assert_eq!(heads(&m), ["yes"]);
    }

    #[test]
    fn frontmatter_counts_only_when_it_closes() {
        let m = metadata("---\n# a heading\n- [ ] a task\n");
        assert_eq!(heads(&m), ["a heading"]);
        assert_eq!(m.tasks().count(), 1);
        let m = metadata("---\ntitle: x\n---\n# body\n");
        assert_eq!((heads(&m), m.headings[0].line), (vec!["body"], 4));
    }

    #[test]
    fn comments_math_and_indented_code_hold_no_metadata() {
        let note = "%%\n# hidden\n- [ ] hidden\n%%\n$$\nx:: 1\n$$\n\n    # code\n    - [ ] code\n\nkept:: yes %%gone:: no%%\n- [ ] real\n    - [ ] nested, not code\n";
        let m = metadata(note);
        assert!(m.headings.is_empty(), "{:?}", m.headings);
        let tasks: Vec<&str> = m.tasks().map(|t| t.text.as_str()).collect();
        assert_eq!(tasks, ["real", "nested, not code"]);
        let fields: Vec<&str> = m.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(fields, ["kept"]);
    }

    /// A large note (20 000 lines: headings, tasks, fields, code) parses
    /// well inside a second, even unoptimised.
    #[test]
    fn a_large_note_parses_quickly() {
        let mut note = String::from("---\ntitle: big\n---\n");
        for i in 0..2000 {
            note.push_str(&format!(
                "## Day {i}\n- [ ] task {i} [due:: 2026-11-20] ^t{i}\n  - [x] sub {i}\nnote:: {i}\n```\n# not\n```\n\ntext text text\n"
            ));
        }
        let started = std::time::Instant::now();
        let m = metadata(&note);
        let took = started.elapsed();
        assert_eq!(m.headings.len(), 2000);
        assert_eq!(m.tasks().count(), 4000);
        assert_eq!(m.blocks.len(), 2000);
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");
        eprintln!("metadata of a {}-byte note: {took:?}", note.len());
    }
}
