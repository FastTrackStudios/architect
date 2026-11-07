//! A frontmatter edit written in place, as Obsidian writes one: only the
//! keys that changed are written again, each in the style it had.
//!
//! The block is cut into one run of lines per top-level key (the key's
//! line, and the indented, `- ` item, blank and comment lines under it).
//! A key whose value is the same keeps its lines byte for byte; a key
//! removed loses them; a key added goes at the end. A changed key keeps
//! its line's key as written and the shape of its value:
//!
//! - a flow list (`[a, b]`) stays a flow list, a block list stays a block
//!   list with its indent;
//! - a value or item in double quotes, single quotes or none keeps them,
//!   unless the new text cannot be written that way (a plain value that
//!   would read back as something else is double-quoted).
//!
//! Every run is read back before it is used: a changed key that cannot
//! be written in its style (a list that was text, a block scalar) is
//! written as a key added is, and a block that does not cut into runs,
//! or does not read back as the edit, is written afresh. So the edit is
//! never wrong, only, at worst, restyled.

use serde_json::Value;

use crate::obsidian_parse::{FrontmatterEntry, serialize_frontmatter};

/// The frontmatter `raw[..offset]` held as `before`, written again as
/// `after`, fences included: `""` when `after` is empty.
#[must_use]
pub fn write_frontmatter(
    raw: &str,
    offset: usize,
    before: &[FrontmatterEntry],
    after: &[FrontmatterEntry],
) -> String {
    if after.is_empty() {
        return String::new();
    }
    in_place(raw.get(..offset).unwrap_or_default(), before, after)
        .unwrap_or_else(|| serialize_frontmatter(after))
}

/// A top-level key's lines in the block.
struct Run<'a> {
    key: String,
    value: Value,
    text: &'a str,
}

fn in_place(head: &str, before: &[FrontmatterEntry], after: &[FrontmatterEntry]) -> Option<String> {
    let (open, rest) = if let Some(r) = head.strip_prefix("---\r\n") {
        ("---\r\n", r)
    } else {
        ("---\n", head.strip_prefix("---\n")?)
    };
    let nl = if open.ends_with("\r\n") { "\r\n" } else { "\n" };
    // The closing fence: the last line, `---`.
    let body = rest.trim_end_matches(['\n', '\r']);
    let yaml = body.strip_suffix("---")?;
    if !(yaml.is_empty() || yaml.ends_with('\n')) {
        return None;
    }
    let close = rest.get(yaml.len()..)?;
    let (preamble, runs) = runs(yaml)?;
    // The runs must be the block, key for key.
    if runs.len() != before.len()
        || runs
            .iter()
            .zip(before)
            .any(|(r, e)| r.key != e.key || r.value != e.value)
    {
        return None;
    }
    let mut out = String::from(open);
    out.push_str(preamble);
    for run in &runs {
        let Some(e) = after.iter().find(|e| e.key == run.key) else {
            continue;
        };
        if e.value == run.value {
            out.push_str(run.text);
        } else {
            let text = restyled(run, &e.value, nl).unwrap_or_else(|| fresh(&e.key, &e.value, nl));
            out.push_str(&text);
        }
    }
    for e in after {
        if !runs.iter().any(|r| r.key == e.key) {
            out.push_str(&fresh(&e.key, &e.value, nl));
        }
    }
    out.push_str(close);
    // The whole block, read back, is `after`.
    let (read, _) = crate::obsidian_parse::parse_frontmatter(&format!("{out}\n"));
    (read == after).then_some(out)
}

/// The block's lines before its first key, and its keys' runs.
fn runs(yaml: &str) -> Option<(&str, Vec<Run<'_>>)> {
    let mut starts = Vec::new();
    let mut at = 0usize;
    for line in yaml.split_inclusive('\n') {
        let first = line.chars().next();
        let starts_key = first.is_some_and(|c| !c.is_whitespace() && c != '#' && c != '-');
        if starts_key {
            starts.push(at);
        }
        at = at.checked_add(line.len())?;
    }
    let preamble = yaml.get(..starts.first().copied().unwrap_or(yaml.len()))?;
    let mut out = Vec::new();
    for (i, &from) in starts.iter().enumerate() {
        let to = starts.get(i.checked_add(1)?).copied().unwrap_or(yaml.len());
        let text = yaml.get(from..to)?;
        let (key, value) = one_entry(text)?;
        out.push(Run { key, value, text });
    }
    Some((preamble, out))
}

/// `text` read as a mapping of one key.
fn one_entry(text: &str) -> Option<(String, Value)> {
    let (mut fm, _) = crate::obsidian_parse::parse_frontmatter(&format!("---\n{text}\n---\n"));
    if fm.len() != 1 {
        return None;
    }
    let e = fm.pop()?;
    Some((e.key, e.value))
}

/// How a value was written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Plain,
    Single,
    Double,
}

impl Quote {
    fn of(text: &str) -> Self {
        match text.trim_start().chars().next() {
            Some('"') => Self::Double,
            Some('\'') => Self::Single,
            _ => Self::Plain,
        }
    }
}

/// A changed key written in its run's style.
fn restyled(run: &Run<'_>, value: &Value, nl: &str) -> Option<String> {
    let first = run.text.split_inclusive('\n').next()?;
    let colon = key_end(first)?;
    let key_text = first.get(..colon)?;
    let shown = first
        .get(colon.checked_add(1)?..)?
        .trim_end_matches(['\n', '\r'])
        .trim();
    // The run's last blank and comment lines stay with it.
    let tail_len: usize = run
        .text
        .split_inclusive('\n')
        .skip(1)
        .collect::<Vec<_>>()
        .iter()
        .rev()
        .take_while(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with('#')
        })
        .map(|l| l.len())
        .sum();
    let tail = run.text.get(run.text.len().checked_sub(tail_len)?..)?;
    let text = match value {
        Value::Array(items) if shown.starts_with('[') && shown.ends_with(']') => {
            let old = flow_items(shown.get(1..shown.len().checked_sub(1)?)?);
            let parts = list(items, &old, true)?;
            format!("{key_text}: [{}]{nl}", parts.join(", "))
        }
        Value::Array(items) if shown.is_empty() => {
            let item_line = run.text.split_inclusive('\n').nth(1)?;
            let indent_len = item_line.len().checked_sub(item_line.trim_start().len())?;
            let indent = item_line.get(..indent_len)?;
            let old: Vec<&str> = run
                .text
                .lines()
                .skip(1)
                .filter_map(|l| l.trim().strip_prefix('-'))
                .map(str::trim)
                .collect();
            let mut out = format!("{key_text}:{nl}");
            for part in list(items, &old, false)? {
                out.push_str(indent);
                out.push_str("- ");
                out.push_str(&part);
                out.push_str(nl);
            }
            out
        }
        Value::Array(_) | Value::Object(_) => return None,
        Value::Null if shown.is_empty() => format!("{key_text}:{nl}"),
        _ if shown.is_empty() || shown.starts_with(['[', '{', '|', '>']) => return None,
        v => format!("{key_text}: {}{nl}", scalar(v, Quote::of(shown), false)?),
    };
    let out = format!("{text}{tail}");
    let (key, read) = one_entry(&out)?;
    (key == run.key && read == *value).then_some(out)
}

/// A list's items: one that was there as it was written, a new one in
/// the first old item's quotes.
fn list(items: &[Value], old: &[&str], in_flow: bool) -> Option<Vec<String>> {
    let q = old.first().map_or(Quote::Plain, |t| Quote::of(t));
    let was: Vec<(Option<Value>, &str)> = old.iter().map(|t| (read_item(t, in_flow), *t)).collect();
    items
        .iter()
        .map(
            |item| match was.iter().find(|(v, _)| v.as_ref() == Some(item)) {
                Some((_, text)) => Some((*text).to_owned()),
                None => scalar(item, q, in_flow),
            },
        )
        .collect()
}

/// An item's text read as YAML.
fn read_item(text: &str, in_flow: bool) -> Option<Value> {
    let probe = if in_flow {
        format!("k: [{text}]")
    } else {
        format!("k: {text}")
    };
    match one_entry(&probe)?.1 {
        Value::Array(mut a) if in_flow && a.len() == 1 => a.pop(),
        v if !in_flow => Some(v),
        _ => None,
    }
}

/// A flow list's items as written, split on the commas outside quotes.
fn flow_items(inner: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut quote, mut from) = (None::<char>, 0usize);
    for (i, c) in inner.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, ',') => {
                out.push(inner.get(from..i).unwrap_or_default().trim());
                from = i.saturating_add(1);
            }
            _ => {}
        }
    }
    let last = inner.get(from..).unwrap_or_default().trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// The `:` that ends the key on a key's line.
fn key_end(line: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut prev_colon: Option<usize> = None;
    for (i, c) in line.char_indices() {
        if let Some(q) = prev_colon.take() {
            if c.is_whitespace() {
                return Some(q);
            }
        }
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') if i == 0 => quote = Some(c),
            (None, ':') => prev_colon = Some(i),
            _ => {}
        }
    }
    prev_colon
}

/// A scalar in `q`, or the nearest way that reads back as it.
fn scalar(v: &Value, q: Quote, in_flow: bool) -> Option<String> {
    let s = match v {
        Value::String(s) => s,
        Value::Null => return Some(String::new()),
        Value::Bool(b) => return Some(b.to_string()),
        Value::Number(n) => return Some(n.to_string()),
        Value::Array(_) | Value::Object(_) => return None,
    };
    let reads = |text: &str| read_item(text, in_flow).as_ref() == Some(v);
    let double = serde_json::to_string(s).ok()?;
    let single = format!("'{}'", s.replace('\'', "''"));
    let tries: [&str; 3] = match q {
        Quote::Plain => [s, &double, &single],
        Quote::Single => [&single, &double, s],
        Quote::Double => [&double, &single, s],
    };
    tries.into_iter().find(|t| reads(t)).map(str::to_owned)
}

/// A key added: written as the block is written afresh; an empty one as
/// Obsidian writes it, `key:` (not `key: null`).
fn fresh(key: &str, value: &Value, nl: &str) -> String {
    if value.is_null() && !key.is_empty() && !key.contains([':', '#', '\n']) {
        return format!("{key}:{nl}");
    }
    let one = serialize_frontmatter(&[FrontmatterEntry {
        key: key.to_owned(),
        value: value.clone(),
    }]);
    let inner = one
        .strip_prefix("---\n")
        .and_then(|s| s.strip_suffix("---\n"))
        .unwrap_or_default();
    if nl == "\n" {
        inner.to_owned()
    } else {
        inner.replace('\n', nl)
    }
}

#[cfg(test)]
mod tests {
    use crate::mutate::edit_frontmatter;
    use crate::tag_edit::{TagEdit, edit_tags};
    use serde_json::json;

    const MIXED: &str = "---\n\
# a comment kept\n\
title: \"My note\"\n\
tags: [tomato, basil, sage]\n\
aliases:\n  - 'First'\n  - Second\n\
status: 'todo'\n\
rating: 3\n\
cssclasses:\n- wide\n\
\"spaced key\": plain text # trailing comment\n\
---\n\
Body #tomato here.\n";

    fn lines(s: &str) -> Vec<&str> {
        s.lines().collect()
    }

    /// Every line of `a` not in `changed` is in `b`, in the same order.
    fn untouched(a: &str, b: &str, changed: &[&str]) {
        let keep: Vec<&str> = lines(a)
            .into_iter()
            .filter(|l| !changed.iter().any(|c| l.starts_with(c)))
            .collect();
        let got: Vec<&str> = lines(b).into_iter().filter(|l| keep.contains(l)).collect();
        assert_eq!(keep, got, "{b}");
    }

    #[test]
    fn an_empty_key_added_is_written_as_obsidian_writes_it() {
        let raw = "---\ntitle: A\n---\nbody\n";
        let out = edit_frontmatter(raw, &[("rating".into(), serde_json::Value::Null)], &[])
            .expect("edits")
            .expect("changed");
        assert_eq!(out, "---\ntitle: A\nrating:\n---\nbody\n");
    }

    #[test]
    fn a_set_an_add_tag_and_a_rename_touch_only_their_key() {
        // A set.
        let set = edit_frontmatter(MIXED, &[("status".into(), json!("done"))], &[])
            .unwrap()
            .unwrap();
        assert!(set.contains("\nstatus: 'done'\n"), "{set}");
        untouched(MIXED, &set, &["status:"]);
        assert_eq!(set.lines().count(), MIXED.lines().count());

        // An add-tag: the flow list stays flow.
        let tagged = edit_tags(&set, &TagEdit::Add("thyme")).unwrap().unwrap();
        assert!(
            tagged.contains("\ntags: [tomato, basil, sage, thyme]\n"),
            "{tagged}"
        );
        untouched(&set, &tagged, &["tags:"]);

        // A rename: frontmatter and body, nothing else.
        let renamed = edit_tags(
            &tagged,
            &TagEdit::Rename {
                from: "tomato",
                to: "red/tomato",
            },
        )
        .unwrap()
        .unwrap();
        assert!(
            renamed.contains("\ntags: [red/tomato, basil, sage, thyme]\n"),
            "{renamed}"
        );
        assert!(renamed.ends_with("Body #red/tomato here.\n"));
        untouched(&tagged, &renamed, &["tags:", "Body"]);

        // A block list keeps its indent and its quotes.
        let aliased = edit_frontmatter(
            &renamed,
            &[("aliases".into(), json!(["First", "Second", "Third"]))],
            &[],
        )
        .unwrap()
        .unwrap();
        assert!(
            aliased.contains("\naliases:\n  - 'First'\n  - Second\n  - 'Third'\nstatus:"),
            "{aliased}"
        );
        untouched(&renamed, &aliased, &["aliases:", "  - "]);
    }

    #[test]
    fn quotes_and_keys_are_kept_and_a_value_that_needs_quotes_gets_them() {
        let out = edit_frontmatter(
            MIXED,
            &[
                ("title".into(), json!("Other")),
                ("spaced key".into(), json!("yes")),
            ],
            &[],
        )
        .unwrap()
        .unwrap();
        assert!(out.contains("\ntitle: \"Other\"\n"), "{out}");
        // Plain `yes` reads as a string in YAML 1.2, so it stays plain;
        // the trailing comment goes with the old value.
        assert!(out.contains("\n\"spaced key\": yes\n"), "{out}");
        let out = edit_frontmatter(MIXED, &[("rating".into(), json!("4: five"))], &[])
            .unwrap()
            .unwrap();
        assert!(out.contains("\nrating: \"4: five\"\n"), "{out}");
    }

    #[test]
    fn a_removed_key_takes_its_lines_and_an_added_one_goes_last() {
        let out = edit_frontmatter(
            MIXED,
            &[("due".into(), json!("2026-11-01"))],
            &["aliases".into()],
        )
        .unwrap()
        .unwrap();
        assert!(!out.contains("aliases"));
        assert!(!out.contains("First"));
        assert!(
            out.contains("trailing comment\ndue: 2026-11-01\n---\n"),
            "{out}"
        );
        untouched(MIXED, &out, &["aliases:", "  - "]);
    }

    #[test]
    fn crlf_is_kept() {
        let raw = "---\r\na: 1\r\ntags: [x]\r\n---\r\nbody\r\n";
        let out = edit_tags(raw, &TagEdit::Add("y")).unwrap().unwrap();
        assert_eq!(out, "---\r\na: 1\r\ntags: [x, y]\r\n---\r\nbody\r\n");
    }
}
