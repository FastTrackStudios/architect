//! A listing fence ([`crate::is_listing_fence`]) cut to what a reader may
//! see: what a share link's node hands its guest of a collection's
//! words (#668).
//!
//! An extension writes its richer layer of a list as a fence of rows, one
//! line each, its fields split at `|` (`\|` a `|` of a field's own), one
//! of them its note's vault path (Studio's
//! `<n> | <label> | <note>[#<arr>] | <summary> | <recording> | …`). Core
//! knows no extension's columns, so it reads a row by the paths it names:
//!
//! - a row is kept only when one of its fields is a note (`….md`, its
//!   `#anchor` aside) the reader may read; a row that names none (a note
//!   outside a share, a row of text alone) is gone whole, its title and
//!   everything else with it;
//! - in a row kept, a field that names any other file of the vault the
//!   reader may not have is emptied;
//! - a `#` heading (a set's name) stays while a row under it does; a
//!   blank line stays.
//!
//! Text outside listing fences, and any other fence, is kept verbatim.

/// The file extensions a field is taken as a vault file by when it has
/// no folder (`take.mp3`); a field with one (`Rec/take.mp3`) is one by
/// any extension.
const FILE_EXTENSIONS: &[&str] = &[
    "md", "mp3", "wav", "flac", "m4a", "aac", "ogg", "oga", "opus", "aif", "aiff", "mp4", "m4v",
    "mov", "webm", "mkv", "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "lrc", "pdf",
];

/// `text` (markdown) with each listing fence's rows cut to those naming a
/// note `note` admits, and in each, any other file `file` does not
/// admit emptied (see the module's rules).
#[must_use]
pub fn within(text: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    // The fence open: its marker (``` or ~~~ and how many), whether it is
    // a listing, and its body so far.
    let mut open: Option<(char, usize, bool, String)> = None;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        let fenced = fence_marker(bare);
        if let Some((c, n, listing, body)) = open.as_mut() {
            let closes = fenced
                .is_some_and(|(fc, fnum, info)| fc == *c && fnum >= *n && info.trim().is_empty());
            if closes {
                if *listing {
                    out.push_str(&rows_within(body, note, file));
                }
                out.push_str(line);
                open = None;
            } else if *listing {
                body.push_str(line);
            } else {
                out.push_str(line);
            }
        } else {
            if let Some((c, n, info)) = fenced {
                open = Some((c, n, crate::is_listing_fence(info), String::new()));
            }
            out.push_str(line);
        }
    }
    // A fence never closed runs to the end, as markdown reads it.
    if let Some((_, _, true, body)) = open {
        out.push_str(&rows_within(&body, note, file));
    }
    out
}

/// A listing fence's body (its lines between the markers) cut as
/// [`within`] cuts it; empty when no row is left.
#[must_use]
pub fn rows_within(body: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(body.len());
    // A heading (and the blank lines after it) held until a row under it
    // is kept.
    let mut held = String::new();
    let mut any = false;
    for line in body.split_inclusive('\n') {
        let bare = line.trim();
        if bare.is_empty() {
            if held.is_empty() {
                out.push_str(line);
            } else {
                held.push_str(line);
            }
        } else if bare.starts_with('#') {
            held.clear();
            held.push_str(line);
        } else if let Some(row) = row_within(line, note, file) {
            out.push_str(&held);
            held.clear();
            out.push_str(&row);
            any = true;
        }
    }
    if any { out } else { String::new() }
}

/// One row, cut: `None` when it names no note `note` admits.
fn row_within(line: &str, note: &dyn Fn(&str) -> bool, file: &dyn Fn(&str) -> bool) -> Option<String> {
    let (bare, end) = line
        .strip_suffix("\r\n")
        .map(|b| (b, "\r\n"))
        .or_else(|| line.strip_suffix('\n').map(|b| (b, "\n")))
        .unwrap_or((line, ""));
    let fields = split(bare);
    let paths: Vec<Option<String>> = fields.iter().map(|f| vault_file(&unescape(f))).collect();
    let listed = paths
        .iter()
        .flatten()
        .any(|p| is_note(p) && note(p));
    if !listed {
        return None;
    }
    let kept: Vec<String> = fields
        .iter()
        .zip(&paths)
        .map(|(raw, path)| match path {
            Some(p) if !(note(p) || file(p)) => blank(raw),
            _ => (*raw).to_owned(),
        })
        .collect();
    Some(format!("{}{end}", kept.join("|")))
}

/// Whether vault path `p` is a note (`.md`, any case).
fn is_note(p: &str) -> bool {
    p.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
}

/// A field emptied, its spacing kept.
fn blank(raw: &str) -> String {
    let lead = raw.len().saturating_sub(raw.trim_start().len());
    raw.get(..lead).unwrap_or_default().to_owned()
}

/// `line` split at each `|` not escaped, each field raw.
fn split(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '|' => {
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

/// The vault file a field names, its `#anchor` aside: a path
/// ([`crate::link::target`]) whose last segment has an extension, in a
/// folder or one of [`FILE_EXTENSIONS`]; `None` for anything else (a
/// title, a length, `3/4`).
fn vault_file(field: &str) -> Option<String> {
    let path = crate::link::target(field)?;
    let name = path.rsplit('/').next().unwrap_or(&path);
    let (stem, ext) = name.rsplit_once('.')?;
    let ext_ok = !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.len() <= 5;
    if stem.is_empty() || !ext_ok || ext.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let known = FILE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext));
    (known || path.contains('/')).then_some(path)
}

/// A fence's marker line: its character, how many, and its info string.
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len().saturating_sub(line.trim_start_matches(' ').len());
    if indent > 3 {
        return None;
    }
    let rest = line.get(indent..)?;
    let c = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = rest.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    let info = rest.get(n..).unwrap_or_default();
    // A backtick fence's info has no backtick.
    if c == '`' && info.contains('`') {
        return None;
    }
    Some((c, n, info.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Merry & Bright\n\nA set.\n\n```studio-setlist downloads\n## Opening\n1 | Joy | Songs/Joy/Joy.md | E · 99 bpm · 3/4 | Rec/joy.mp3 | Choir | 186.0\n2 | Secret Song | Private/Secret Song.md | E · 99 bpm | Private/secret-take.mp3 | Unreleased Artist\n\n## Encore\n3 | Hidden \\| Two | Private/Two.md#Live | | Private/two.mp3\n```\n\n```kf\n| C | Private/x.md |\n```\n";

    fn listed(p: &str) -> bool {
        p == "Songs/Joy/Joy.md"
    }

    #[test]
    fn a_row_of_a_note_outside_is_gone_and_a_file_held_back_is_emptied() {
        let none = |_: &str| false;
        let cut = within(NOTE, &listed, &none);
        assert!(cut.contains("## Opening\n1 | Joy | Songs/Joy/Joy.md | E · 99 bpm · 3/4 | | Choir | 186.0\n"), "{cut}");
        for never in ["Secret", "Private/Secret", "secret-take", "Unreleased", "## Encore", "Hidden", "two.mp3"] {
            assert!(!cut.contains(never), "{never:?} in {cut}");
        }
        // Words outside, and another fence, verbatim.
        assert!(cut.starts_with("# Merry & Bright\n\nA set.\n\n```studio-setlist downloads\n"), "{cut}");
        assert!(cut.ends_with("```\n\n```kf\n| C | Private/x.md |\n```\n"), "{cut}");
        // A file the reader has stays.
        let served = |p: &str| p == "Rec/joy.mp3";
        assert!(within(NOTE, &listed, &served).contains("| Rec/joy.mp3 |"));
    }

    #[test]
    fn a_fence_with_no_row_left_is_empty_and_an_unclosed_one_is_cut_too() {
        let none = |_: &str| false;
        assert_eq!(rows_within("## A\n1 | X | Private/X.md\n", &none, &none), "");
        let open = "```playlist\n1 | X | Private/X.md\n";
        assert_eq!(within(open, &none, &none), "```playlist\n");
    }

    #[test]
    fn a_field_is_a_file_by_its_path_never_a_title_or_a_number() {
        assert_eq!(vault_file("Songs/Joy/Joy.md#Live").as_deref(), Some("Songs/Joy/Joy.md"));
        assert_eq!(vault_file("take.mp3").as_deref(), Some("take.mp3"));
        assert_eq!(vault_file("Rec/take.xyz").as_deref(), Some("Rec/take.xyz"));
        for not in ["Mr. Brightside", "186.0", "3/4", "E · 99 bpm", "Choir", "https://x.y/a.mp3", "notes.txt2x"] {
            assert_eq!(vault_file(not), None, "{not}");
        }
    }
}
