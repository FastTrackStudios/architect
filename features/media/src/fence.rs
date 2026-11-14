//! A listing fence ([`crate::is_listing_fence`]) cut to what a reader may
//! see: what a share link's node hands its guest of a collection's
//! words, and what a site builds into its pages (#668).
//!
//! An extension writes its richer layer of a list as a fence of rows, one
//! line each, its fields split at `|` (`\|` a `|` of a field's own; a `|`
//! inside `[[…]]` or `[…]` is the link's), one of them its note's vault
//! path (Studio's `<n> | <label> | <note>[#<arr>] | <summary> |
//! <recording> | …`). Core knows no extension's columns, so it reads a
//! row by the paths it names:
//!
//! - a row is kept only when one of its fields is a note (`….md`, or a
//!   path the reader has once `.md` is added; its `#anchor` aside) the
//!   reader may read; a row that names none (a note outside a share, a
//!   row of text alone) is gone whole, its title and everything else with
//!   it;
//! - in a row kept, a field stays only when it is a note the reader may
//!   read, a file the reader has, or plain text: no `[`, `]`, no file
//!   name (`take.mp3`), no `/` and no `:` but between digits (`3/4`,
//!   `3:05`). Any other field (a link,
//!   a path, a `Private/x: Live.md` no link reads) is emptied, its
//!   spacing kept;
//! - a heading (`#` to `######` and a space: a set's name) stays while a
//!   row under it does; a blank line stays. `#2 | …` is a row.
//!
//! Fences are found as the site's markdown finds them (`pulldown_cmark`,
//! [`Options`]): in a blockquote or a list, never one an HTML block or
//! another fence holds. Text outside listing fences, and any other fence,
//! is kept verbatim.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// The markdown options a site's page is parsed with
/// (`architect_sites::markdown::render`): a fence is found where the page
/// finds it.
#[must_use]
pub fn options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_WIKILINKS);
    options
}

/// `text` (markdown) with each listing fence's rows cut to those naming a
/// note `note` admits, and in each, any other field `file` does not
/// admit emptied (see the module's rules).
#[must_use]
pub fn within(text: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> String {
    // Each body line of a listing fence: its range in `text` (its own
    // text, as the parser reads it) and the start of its source line
    // (before a `> ` or a list's indent).
    let mut fences: Vec<Vec<(std::ops::Range<usize>, usize)>> = Vec::new();
    let mut open: Option<Vec<(std::ops::Range<usize>, usize)>> = None;
    // Whether the parser's text and the source agree everywhere: else
    // every listing fence's body goes (fail closed).
    let mut agree = true;
    for (event, range) in Parser::new_ext(text, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let info = match &kind {
                    CodeBlockKind::Fenced(info) => info.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                if crate::is_listing_fence(info) {
                    open = Some(Vec::new());
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(lines) = open.take() {
                    fences.push(lines);
                }
            }
            Event::Text(t) if open.is_some() => {
                let Some(lines) = open.as_mut() else { continue };
                if text.get(range.clone()) != Some(t.as_ref()) {
                    agree = false;
                    continue;
                }
                let mut at = range.start;
                for line in t.split_inclusive('\n') {
                    let end = at.saturating_add(line.len());
                    // A line split over two events: one line, joined.
                    if let Some((last, _)) = lines.last_mut().filter(|(r, _)| {
                        r.end == at && !text.get(r.clone()).is_some_and(|l| l.ends_with('\n'))
                    }) {
                        last.end = end;
                    } else {
                        let start = text
                            .get(..at)
                            .and_then(|b| b.rfind('\n'))
                            .map_or(0, |i| i.saturating_add(1));
                        lines.push((at..end, start));
                    }
                    at = end;
                }
            }
            _ => {}
        }
    }
    if let Some(lines) = open {
        fences.push(lines);
    }
    // Edits, in order: each body line's range and what replaces it.
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for lines in &fences {
        let bodies: Vec<&str> = lines
            .iter()
            .map(|(r, _)| text.get(r.clone()).unwrap_or_default())
            .collect();
        let cut = if agree {
            cut_lines(&bodies, note, file)
        } else {
            vec![None; bodies.len()]
        };
        for ((range, start), kept) in lines.iter().zip(cut) {
            match kept {
                Some(line) => edits.push((range.clone(), line)),
                // The whole source line, its `> ` or indent too.
                None => edits.push((*start..range.end, String::new())),
            }
        }
    }
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, with) in edits {
        if range.start < at {
            continue;
        }
        out.push_str(text.get(at..range.start).unwrap_or_default());
        out.push_str(&with);
        at = range.end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    out
}

/// A listing fence's body (its lines between the markers) cut as
/// [`within`] cuts it; empty when no row is left.
#[must_use]
pub fn rows_within(body: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    cut_lines(&lines, note, file)
        .into_iter()
        .flatten()
        .collect()
}

/// Each of a fence's body lines (each with its line end) as it is kept,
/// or `None` when it goes. All go when no row is kept.
fn cut_lines(
    lines: &[&str],
    note: &dyn Fn(&str) -> bool,
    file: &dyn Fn(&str) -> bool,
) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; lines.len()];
    // A heading (and the blank lines after it), by index, held until a
    // row under it is kept.
    let mut held: Vec<usize> = Vec::new();
    let mut any = false;
    for (i, line) in lines.iter().enumerate() {
        let bare = line.trim();
        let slot = out.get_mut(i);
        if bare.is_empty() {
            if held.is_empty() {
                if let Some(slot) = slot {
                    *slot = Some((*line).to_owned());
                }
            } else {
                held.push(i);
            }
        } else if is_heading(bare) {
            held.clear();
            held.push(i);
        } else if let Some(row) = row_within(line, note, file) {
            for h in std::mem::take(&mut held) {
                if let (Some(slot), Some(line)) = (out.get_mut(h), lines.get(h)) {
                    *slot = Some((*line).to_owned());
                }
            }
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(row);
            }
            any = true;
        }
    }
    if any { out } else { vec![None; lines.len()] }
}

/// Whether a fence's line is a heading: one to six `#` and a space.
fn is_heading(bare: &str) -> bool {
    let hashes = bare.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes)
        && bare
            .get(hashes..)
            .is_some_and(|r| r.starts_with([' ', '\t']))
}

/// One row, cut: `None` when it names no note `note` admits.
fn row_within(
    line: &str,
    note: &dyn Fn(&str) -> bool,
    file: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let (bare, end) = line
        .strip_suffix("\r\n")
        .map(|b| (b, "\r\n"))
        .or_else(|| line.strip_suffix('\n').map(|b| (b, "\n")))
        .unwrap_or((line, ""));
    let fields = split(bare);
    let readable: Vec<Readable> = fields
        .iter()
        .map(|f| readable(&unescape(f), note, file))
        .collect();
    if !readable.contains(&Readable::Note) {
        return None;
    }
    let kept: Vec<String> = fields
        .iter()
        .zip(&readable)
        .map(|(raw, r)| match r {
            Readable::No => blank(raw),
            _ => (*raw).to_owned(),
        })
        .collect();
    Some(format!("{}{end}", kept.join("|")))
}

/// What a field is to the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readable {
    /// A note it may read.
    Note,
    /// A file it has, or plain text.
    Kept,
    /// Anything else: emptied.
    No,
}

/// What `field` (unescaped, trimmed) is to a reader of notes `note` and
/// files `file`.
fn readable(field: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> Readable {
    if let Some(path) = crate::link::target(field) {
        if is_note(&path) && note(&path) {
            return Readable::Note;
        }
        // `[[Songs/Joy]]`, `Songs/Joy`: a note with `.md` left off. A bare
        // word (`Joy`) is a title, never a note.
        if looks_linked(field) && !has_extension(&path) && note(&format!("{path}.md")) {
            return Readable::Note;
        }
        if file(&path) {
            return Readable::Kept;
        }
    }
    if plain(field) {
        Readable::Kept
    } else {
        Readable::No
    }
}

/// Whether a field is written as a link or a path: brackets, or a `/`.
fn looks_linked(field: &str) -> bool {
    field.contains(['[', ']', '/'])
}

/// Whether a field is plain text: no link syntax, no file name
/// (`take.mp3`), no `/` and no `:` but between digits (a meter, a time),
/// so it names no file.
fn plain(field: &str) -> bool {
    if field.contains(['[', ']', '\\']) || field.contains("](") || has_extension(field) {
        return false;
    }
    let chars: Vec<char> = field.chars().collect();
    chars.iter().enumerate().all(|(i, c)| {
        if *c != '/' && *c != ':' {
            return true;
        }
        let before = i.checked_sub(1).and_then(|b| chars.get(b));
        let after = chars.get(i.saturating_add(1));
        before.is_some_and(char::is_ascii_digit) && after.is_some_and(char::is_ascii_digit)
    })
}

/// Whether vault path `p` is a note (`.md`, any case).
fn is_note(p: &str) -> bool {
    p.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
}

/// Whether `p`'s last segment ends in an extension (`.mp3`, not ` 2.0`).
fn has_extension(p: &str) -> bool {
    let name = p.rsplit('/').next().unwrap_or(p);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && !ext.is_empty()
            && ext.len() <= 5
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
            && !ext.chars().all(|c| c.is_ascii_digit())
    })
}

/// A field emptied, its spacing kept.
fn blank(raw: &str) -> String {
    let lead = raw.len().saturating_sub(raw.trim_start().len());
    raw.get(..lead).unwrap_or_default().to_owned()
}

/// `line` split at each `|` not escaped and not inside brackets (a
/// wikilink's alias, `[[a|b]]`, a link's text), each field raw.
fn split(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    let mut depth = 0usize;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '[' => depth = depth.saturating_add(1),
            ']' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                fields.push(line.get(start..i).unwrap_or_default());
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    fields.push(line.get(start..).unwrap_or_default());
    fields
}

/// A raw field's text: `\x` is `x`, trimmed.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(chars.next().unwrap_or('\\'));
        } else {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Merry & Bright\n\nA set.\n\n```studio-setlist downloads\n## Opening\n1 | Joy | Songs/Joy/Joy.md | E · 99 bpm · 3/4 | Rec/joy.mp3 | Choir | 186.0\n2 | Secret Song | Private/Secret Song.md | E · 99 bpm | Private/secret-take.mp3 | Unreleased Artist\n\n## Encore\n3 | Hidden \\| Two | Private/Two.md#Live | | Private/two.mp3\n```\n\n```kf\n| C | Private/x.md |\n```\n";

    fn listed(p: &str) -> bool {
        p == "Songs/Joy/Joy.md"
    }

    fn none(_: &str) -> bool {
        false
    }

    /// `rows` in a listing fence, cut for a link serving only Joy.
    fn cut(rows: &str) -> String {
        within(&format!("```studio-setlist\n{rows}```\n"), &listed, &none)
    }

    const JOY: &str = "1 | Joy | Songs/Joy/Joy.md | E · 99 bpm · 3/4 | | Choir | 186.0\n";

    #[test]
    fn a_row_of_a_note_outside_is_gone_and_a_file_held_back_is_emptied() {
        let cut = within(NOTE, &listed, &none);
        assert!(cut.contains(&format!("## Opening\n{JOY}")), "{cut}");
        for never in [
            "Secret",
            "Private/Secret",
            "secret-take",
            "Unreleased",
            "## Encore",
            "Hidden",
            "two.mp3",
        ] {
            assert!(!cut.contains(never), "{never:?} in {cut}");
        }
        // Words outside, and another fence, verbatim.
        assert!(
            cut.starts_with("# Merry & Bright\n\nA set.\n\n```studio-setlist downloads\n"),
            "{cut}"
        );
        assert!(
            cut.ends_with("```\n\n```kf\n| C | Private/x.md |\n```\n"),
            "{cut}"
        );
        // A file the reader has stays.
        let served = |p: &str| p == "Rec/joy.mp3";
        assert!(within(NOTE, &listed, &served).contains("| Rec/joy.mp3 |"));
    }

    #[test]
    fn a_fence_with_no_row_left_is_empty_and_an_unclosed_one_is_cut_too() {
        assert_eq!(
            rows_within("## A\n1 | X | Private/X.md\n", &none, &none),
            ""
        );
        let open = "```playlist\n1 | X | Private/X.md\n";
        assert_eq!(within(open, &none, &none), "```playlist\n");
    }

    #[test]
    fn a_link_with_no_extension_in_a_kept_row_is_emptied() {
        let wiki = cut(&format!("{}| [[Private/Secret Song]]\n", JOY.trim_end()));
        assert!(!wiki.contains("Secret"), "{wiki}");
        let md = cut(&format!(
            "{}| [Secret Song](Private/Secret)\n",
            JOY.trim_end()
        ));
        assert!(!md.contains("Secret"), "{md}");
        // A wikilink to the readable note, with no `.md`, keeps its row.
        let joy = cut("1 | Joy | [[Songs/Joy/Joy]]\n");
        assert!(joy.contains("1 | Joy | [[Songs/Joy/Joy]]\n"), "{joy}");
    }

    #[test]
    fn an_alias_is_one_field_and_goes_whole() {
        let alias = cut(&format!(
            "{}| [[Private/Secret.md|Secret Song]] | after\n",
            JOY.trim_end()
        ));
        assert!(!alias.contains("Secret"), "{alias}");
        assert!(alias.contains("| after\n"), "{alias}");
        // A row whose only note is behind an alias of its own is kept.
        let joy = cut("1 | [[Songs/Joy/Joy.md|Joy]] | x\n");
        assert!(joy.contains("1 | [[Songs/Joy/Joy.md|Joy]] | x\n"), "{joy}");
    }

    #[test]
    fn a_path_no_link_reads_is_emptied() {
        let colon = cut(&format!("{}| Private/Secret: Live.md\n", JOY.trim_end()));
        assert!(!colon.contains("Secret"), "{colon}");
        // A time and a meter are text.
        let text = cut("1 | Joy | Songs/Joy/Joy.md | 3:05 | 6/8\n");
        assert!(text.contains("| 3:05 | 6/8\n"), "{text}");
    }

    #[test]
    fn a_row_that_starts_with_a_hash_is_a_row_not_a_heading() {
        let hash = cut(&format!("#2 | Secret Song | Private/Secret.md\n{JOY}"));
        assert!(!hash.contains("Secret"), "{hash}");
        assert!(hash.contains(JOY), "{hash}");
        // A heading is `#` and a space.
        let heading = cut(&format!("## Opening\n{JOY}"));
        assert!(heading.contains("## Opening\n"), "{heading}");
    }

    #[test]
    fn a_fence_in_a_blockquote_or_a_list_is_cut() {
        let quoted = "> ```studio-setlist\n> 2 | Secret Song | Private/Secret.md\n> 1 | Joy | Songs/Joy/Joy.md\n> ```\n";
        let cut = within(quoted, &listed, &none);
        assert_eq!(
            cut,
            "> ```studio-setlist\n> 1 | Joy | Songs/Joy/Joy.md\n> ```\n"
        );
        let listed_in = "- the set\n\n  ```playlist\n  2 | Secret Song | Private/Secret.md\n  1 | Joy | Songs/Joy/Joy.md\n  ```\n";
        let cut = within(listed_in, &listed, &none);
        assert!(!cut.contains("Secret"), "{cut}");
        assert!(cut.contains("  1 | Joy | Songs/Joy/Joy.md\n"), "{cut}");
    }

    #[test]
    fn a_fence_after_an_html_comment_holding_a_marker_is_cut() {
        let note = "<!--\n```\n-->\n```studio-setlist\n2 | Secret Song | Private/Secret.md\n1 | Joy | Songs/Joy/Joy.md\n```\n";
        let cut = within(note, &listed, &none);
        assert!(!cut.contains("Secret"), "{cut}");
        assert!(cut.contains("1 | Joy | Songs/Joy/Joy.md\n"), "{cut}");
        // A marker inside another fence opens nothing.
        let inner = "````md\n```studio-setlist\n2 | Secret | Private/Secret.md\n```\n````\n";
        assert_eq!(within(inner, &listed, &none), inner);
    }

    #[test]
    fn a_field_is_kept_as_the_note_a_served_file_or_plain_text() {
        let served = |p: &str| p == "Rec/take.mp3";
        let r = |f: &str| readable(f, &listed, &served);
        assert_eq!(r("Songs/Joy/Joy.md#Live"), Readable::Note);
        assert_eq!(r("[[Songs/Joy/Joy|Joy]]"), Readable::Note);
        assert_eq!(r("Rec/take.mp3"), Readable::Kept);
        for text in [
            "Mr. Brightside",
            "186.0",
            "3/4",
            "3:05",
            "E · 99 bpm",
            "Choir",
            "Joy",
        ] {
            assert_eq!(r(text), Readable::Kept, "{text}");
        }
        for never in [
            "Private/x.mp3",
            "take.mp3",
            "https://x.y/a.mp3",
            "a/b",
            "Live: x",
            "[x](y)",
            "[[x]]",
        ] {
            assert_eq!(r(never), Readable::No, "{never}");
        }
    }

    /// Studio's island (studio#490) reads its list from the cut fence:
    /// each kept row keeps its number, title, note, summary (its meter),
    /// a served recording, artist, length and key.
    #[test]
    fn a_kept_row_keeps_what_studios_reader_needs() {
        let row = "1 | Joy to the World | Songs/Joy/Joy.md#Choir | D · 120 bpm · 4/4 | Rec/joy.ogg | Isaac Watts | 151.0 | D |\n";
        let served = |p: &str| p == "Rec/joy.ogg";
        let cut = within(&format!("```studio-setlist\n{row}```\n"), &listed, &served);
        assert_eq!(cut, format!("```studio-setlist\n{row}```\n"));
    }
}
