//! What an extension's view on a note may be handed of it.
//!
//! The rules the node (for a share link's guest) and the web app (for
//! everyone) both keep, so neither has its own copy (#648, #669).
//!
//! - [`without_comments`]: a note's text as a view gets it, its
//!   `%%comments%%` gone. A member's view gets the note's body through it;
//!   a guest's gets the share build's text through it too.
//! - [`ViewFileKind`]: the files a view may read beside the note (an
//!   extension that declares `reads = ["files"]`): art and chart images,
//!   and synced lyrics. Audio and video never: they stream, through the
//!   app's own player.
//! - [`VIEW_FILE_MAX`], [`VIEW_FILES_TOTAL_MAX`], [`VIEW_FILES_MAX`]: how
//!   much of them, per file and per note ([`within_caps`]).

/// The largest file a view is handed: 2 MiB. Art and a chart page are
/// well under it; anything larger is left out, never cut short.
pub const VIEW_FILE_MAX: u64 = 2 * 1024 * 1024;

/// The most a view is handed in files for one note: 8 MiB. The files
/// travel into its frame as `data:` URLs (a third larger again), once
/// per note shown.
pub const VIEW_FILES_TOTAL_MAX: u64 = 8 * 1024 * 1024;

/// The most files a view is handed for one note.
pub const VIEW_FILES_MAX: usize = 32;

/// What a file a view may read is, by its name's extension (matched
/// without regard to case).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFileKind {
    /// An image, with its media type: handed in as a `data:` URL, which
    /// the app page and the view's frame already allow (`img-src data:`).
    Image(&'static str),
    /// Synced lyrics (`.lrc`): handed in as text.
    Lyrics,
}

impl ViewFileKind {
    /// The kind of the file at `path`, when a view may be handed it.
    #[must_use]
    pub fn of(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = name.rsplit_once('.')?;
        if stem.is_empty() {
            return None;
        }
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Image("image/png")),
            "jpg" | "jpeg" => Some(Self::Image("image/jpeg")),
            "webp" => Some(Self::Image("image/webp")),
            "gif" => Some(Self::Image("image/gif")),
            "lrc" => Some(Self::Lyrics),
            _ => None,
        }
    }

    /// Its word on the wire: `image` or `lyrics`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Image(_) => "image",
            Self::Lyrics => "lyrics",
        }
    }
}

/// The files of `files` a view is handed, in their order.
///
/// `size` is each one's size, bytes. Only a kind it may read ([`ViewFileKind::of`] on `path`),
/// none over [`VIEW_FILE_MAX`], at most [`VIEW_FILES_MAX`] and, together,
/// at most [`VIEW_FILES_TOTAL_MAX`]. One that would go past the total is
/// left out and the rest are still tried; each path once.
pub fn within_caps<T>(
    files: impl IntoIterator<Item = T>,
    path: impl Fn(&T) -> &str,
    size: impl Fn(&T) -> u64,
) -> Vec<T> {
    let mut kept: Vec<T> = Vec::new();
    let mut total: u64 = 0;
    for file in files {
        if kept.len() >= VIEW_FILES_MAX {
            break;
        }
        let bytes = size(&file);
        if ViewFileKind::of(path(&file)).is_none()
            || bytes > VIEW_FILE_MAX
            || kept.iter().any(|k| path(k) == path(&file))
        {
            continue;
        }
        let Some(after) = total
            .checked_add(bytes)
            .filter(|t| *t <= VIEW_FILES_TOTAL_MAX)
        else {
            continue;
        };
        total = after;
        kept.push(file);
    }
    kept
}

/// `body` without its `%%comments%%`: what a view on a note is handed of
/// its text, a member's or a share link's guest's alike.
///
/// A comment may span lines, and a `%%` never closed hides the rest. A
/// `%%` inside code is text, not a comment's edge: a fenced block (three
/// or more backticks or tildes, closed by as many) is kept whole, and so
/// is an inline code span on one line.
#[must_use]
pub fn without_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<(char, usize)> = None;
    let mut hidden = false;
    for line in body.split_inclusive('\n') {
        if !hidden {
            if let Some((mark, len)) = fence {
                out.push_str(line);
                if fence_of(line).is_some_and(|(c, n, bare)| c == mark && n >= len && bare) {
                    fence = None;
                }
                continue;
            }
            if let Some((mark, len, _)) = fence_of(line) {
                fence = Some((mark, len));
                out.push_str(line);
                continue;
            }
        }
        hidden = uncommented(line, hidden, &mut out);
    }
    out
}

/// A line's code fence: its mark, how many, and whether nothing follows
/// them (what a closing fence is).
fn fence_of(line: &str) -> Option<(char, usize, bool)> {
    let text = line.trim_start_matches(' ');
    if line.len().saturating_sub(text.len()) > 3 {
        return None;
    }
    let mark = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = text.chars().take_while(|c| *c == mark).count();
    (len >= 3).then(|| (mark, len, text.trim_start_matches(mark).trim().is_empty()))
}

/// One line outside a fence onto `out` without its comments, given
/// whether it starts inside one; whether it ends inside one.
fn uncommented(line: &str, mut hidden: bool, out: &mut String) -> bool {
    let mut rest = line;
    loop {
        if hidden {
            let Some((_, tail)) = rest.split_once("%%") else {
                return true;
            };
            hidden = false;
            rest = tail;
            continue;
        }
        let Some(comment) = rest.find("%%") else {
            out.push_str(rest);
            return false;
        };
        if let Some(tick) = rest.find('`').filter(|tick| *tick < comment) {
            let (before, from) = rest.split_at_checked(tick).unwrap_or((rest, ""));
            out.push_str(before);
            let len = from.chars().take_while(|c| *c == '`').count();
            let (run, after) = from.split_at_checked(len).unwrap_or((from, ""));
            out.push_str(run);
            // A span is kept whole; a run never closed is only text.
            let (span, tail) = code_span_end(after, len)
                .and_then(|end| after.split_at_checked(end))
                .unwrap_or(("", after));
            out.push_str(span);
            rest = tail;
        } else {
            let (before, from) = rest.split_at_checked(comment).unwrap_or((rest, ""));
            out.push_str(before);
            rest = from.strip_prefix("%%").unwrap_or(from);
            hidden = true;
        }
    }
}

/// Where the code span opened by `len` backticks ends in `after`: just
/// past the next run of exactly `len`.
fn code_span_end(after: &str, len: usize) -> Option<usize> {
    let mut seen = 0_usize;
    let mut rest = after;
    loop {
        let tick = rest.find('`')?;
        let from = rest.get(tick..)?;
        let run = from.chars().take_while(|c| *c == '`').count();
        let next = tick.checked_add(run)?;
        seen = seen.checked_add(next)?;
        if run == len {
            return Some(seen);
        }
        rest = rest.get(next..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_reads_images_and_lyrics_only() {
        assert_eq!(
            ViewFileKind::of("Art/Joy.PNG"),
            Some(ViewFileKind::Image("image/png"))
        );
        assert_eq!(
            ViewFileKind::of("a.jpeg"),
            Some(ViewFileKind::Image("image/jpeg"))
        );
        assert_eq!(
            ViewFileKind::of("Songs/Joy.lrc"),
            Some(ViewFileKind::Lyrics)
        );
        for no in [
            "Joy.mp3",
            "Joy.svg",
            "Joy.html",
            "Joy.md",
            ".png",
            "png",
            "Joy.png/x",
        ] {
            assert_eq!(ViewFileKind::of(no), None, "{no}");
        }
    }

    #[test]
    fn caps_hold_per_file_per_note_and_in_count() {
        let f = |p: &str, s: u64| (p.to_owned(), s);
        let kept = within_caps(
            [
                f("big.png", VIEW_FILE_MAX + 1),
                f("a.png", VIEW_FILE_MAX),
                f("a.png", 1),
                f("x.mp3", 1),
                f("b.png", VIEW_FILE_MAX),
                f("c.png", VIEW_FILE_MAX),
                f("d.png", VIEW_FILE_MAX),
                f("e.png", 1),
                f("f.lrc", 0),
            ],
            |(p, _)| p.as_str(),
            |(_, s)| *s,
        );
        let names: Vec<&str> = kept.iter().map(|(p, _)| p.as_str()).collect();
        // Four at the per-file cap fill the total; then only the empty one.
        assert_eq!(names, ["a.png", "b.png", "c.png", "d.png", "f.lrc"]);
        let many = within_caps((0..100).map(|i| format!("{i}.png")), String::as_str, |_| 1);
        assert_eq!(many.len(), VIEW_FILES_MAX);
    }
}
