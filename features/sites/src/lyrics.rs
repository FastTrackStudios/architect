//! Synced lyrics (LRC) drawn as lines that follow the playhead (#511).
//!
//! A note's ```` ```lrc ```` fence, or an embed of its `.lrc` attachment
//! (`![[Joy.lrc]]`), is parsed here, at build time, into timed lines and
//! drawn as markup the page's player follows ([`lyrics_html`]): a list
//! marked `data-follow-playhead`, each line a `data-at` in seconds. The
//! player (`assets/player.js`) marks the line at the playhead
//! `aria-current="time"`, scrolls it into view and seeks on a click. The
//! script never reads LRC: timing is decided here.
//!
//! What is read: `[mm:ss]`, `[mm:ss.xx]`, `[mm:ss.xxx]` and `[mm:ss:xx]`
//! stamps, several on one line (a chorus sung twice), the `[offset:±ms]`
//! tag (positive shows lines sooner), `[ti:]` and `[ar:]`; word stamps
//! (`<mm:ss.xx>`, "enhanced" LRC) are dropped from the text. Any other
//! tag, and a line without a stamp, is skipped.

use std::fmt::Write as _;
use std::sync::Arc;

use crate::blocks::{Block, BlockRenderer, Drawn};
use crate::element_html::escape;
use crate::{Blocks, Claim};

/// One timed line.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// When it starts, in seconds from the top of the track (never below
    /// zero, the offset applied).
    pub at: f64,
    /// What is sung; empty for a gap.
    pub text: String,
}

/// A parsed LRC file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lyrics {
    /// `[ti:]`.
    pub title: Option<String>,
    /// `[ar:]`.
    pub artist: Option<String>,
    /// Its lines in time order (lines with the same time keep the file's
    /// order).
    pub lines: Vec<LyricLine>,
}

/// A `[mm:ss(.xx)]` stamp in milliseconds, or `None` when `tag` is not
/// one.
fn stamp(tag: &str) -> Option<i64> {
    let (minutes, rest) = tag.split_once(':')?;
    let minutes: i64 = minutes.trim().parse().ok()?;
    // `ss.xx`, or `ss:xx` as some writers have it.
    let (seconds, fraction) = rest
        .split_once(['.', ':'])
        .map_or((rest, ""), |(s, f)| (s, f));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(seconds) || !(fraction.is_empty() || digits(fraction)) {
        return None;
    }
    let seconds: i64 = seconds.parse().ok()?;
    // The first three digits of the fraction, as milliseconds.
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    minutes
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(millis)
}

/// `text` without its word stamps (`<00:12.30>`).
fn without_word_stamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        let inner = after.get(1..).unwrap_or_default();
        match inner.split_once('>') {
            Some((tag, beyond)) if stamp(tag).is_some() => rest = beyond,
            _ => {
                out.push('<');
                rest = inner;
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse LRC `source`.
#[must_use]
pub fn parse_lrc(source: &str) -> Lyrics {
    let mut lyrics = Lyrics::default();
    let mut offset_ms: i64 = 0;
    let mut timed: Vec<(i64, String)> = Vec::new();
    for line in source.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some((tag, beyond)) = inner.split_once(']') else {
                break;
            };
            rest = beyond;
            if let Some(at) = stamp(tag) {
                stamps.push(at);
                continue;
            }
            let (key, value) = tag.split_once(':').unwrap_or((tag, ""));
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "offset" => offset_ms = value.trim_start_matches('+').parse().unwrap_or(0),
                "ti" if !value.is_empty() => lyrics.title = Some(value.to_owned()),
                "ar" if !value.is_empty() => lyrics.artist = Some(value.to_owned()),
                _ => {}
            }
        }
        if stamps.is_empty() {
            continue;
        }
        let text = without_word_stamps(rest);
        timed.extend(stamps.into_iter().map(|at| (at, text.clone())));
    }
    // A stable sort: lines at one time keep the file's order.
    timed.sort_by_key(|(at, _)| *at);
    lyrics.lines = timed
        .into_iter()
        .map(|(at, text)| LyricLine {
            // Past i32's milliseconds (24 days) is no song: clamped.
            at: f64::from(i32::try_from(at.saturating_sub(offset_ms).max(0)).unwrap_or(i32::MAX))
                / 1000.0,
            text,
        })
        .collect();
    lyrics
}

/// `lyrics` as the markup the page's player follows.
///
/// An `<ol>` marked `data-follow-playhead`, a line per `<li data-at="…">`
/// (seconds), each a button that seeks there. `media` ties it to one
/// media file's URL
/// (`data-media`); without it, it follows whatever plays from its page.
#[must_use]
pub fn lyrics_html(lyrics: &Lyrics, media: Option<&str>) -> String {
    let mut html = String::from("<ol class=\"site-lyrics\" data-follow-playhead");
    if let Some(media) = media {
        let _ = write!(html, " data-media=\"{}\"", escape(media));
    }
    html.push('>');
    for line in &lyrics.lines {
        let text = if line.text.is_empty() {
            "♪".to_owned()
        } else {
            escape(&line.text)
        };
        let _ = write!(
            html,
            "<li data-at=\"{:.3}\"><button type=\"button\" class=\"site-lyric\">{text}</button></li>",
            line.at
        );
    }
    html.push_str("</ol>\n");
    html
}

/// The built-in ```` ```lrc ```` renderer. It is added after the
/// caller's renderers, so an extension claiming `fence:lrc` wins.
pub struct LyricsBlocks;

impl LyricsBlocks {
    /// `blocks` with this renderer added last.
    #[must_use]
    pub fn added_to(blocks: &Blocks) -> Blocks {
        let mut blocks = blocks.clone();
        blocks.add(Arc::new(Self));
        blocks
    }
}

impl BlockRenderer for LyricsBlocks {
    fn name(&self) -> &'static str {
        "architect.lyrics"
    }

    fn claims(&self, claim: &Claim) -> bool {
        *claim == Claim::Fence("lrc".to_owned())
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        let lyrics = parse_lrc(block.source);
        if lyrics.lines.is_empty() {
            return Err("it has no timed lines (`[mm:ss.xx] words`)".to_owned());
        }
        Ok(Drawn::Html(lyrics_html(&lyrics, None)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_offsets_and_order() {
        let lyrics = parse_lrc(
            "[ti: Joy]\n[ar:The Band]\n[offset:+500]\n\
             [00:12.30]First line\n\
             [00:05.00][01:02.5]Chorus  <00:05.40>sung <00:05.90>twice\n\
             no stamp here\n\
             [00:20]\n\
             [00:30:25]Colon fraction\n\
             [xx:yy]not a stamp\n",
        );
        assert_eq!(lyrics.title.as_deref(), Some("Joy"));
        assert_eq!(lyrics.artist.as_deref(), Some("The Band"));
        let got: Vec<(f64, &str)> = lyrics
            .lines
            .iter()
            .map(|l| (l.at, l.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (4.5, "Chorus sung twice"),
                (11.8, "First line"),
                (19.5, ""),
                (29.75, "Colon fraction"),
                (62.0, "Chorus sung twice"),
            ]
        );
    }

    #[test]
    fn negative_offset_and_floor_at_zero() {
        let late = parse_lrc("[offset:-1000]\n[00:01.00]a");
        assert_eq!(late.lines.first().map(|l| l.at), Some(2.0));
        let early = parse_lrc("[offset:3000]\n[00:01.00]a");
        assert_eq!(early.lines.first().map(|l| l.at), Some(0.0));
    }

    #[test]
    fn a_lone_angle_bracket_is_text() {
        let lyrics = parse_lrc("[00:01]a < b <i>");
        assert_eq!(
            lyrics.lines.first().map(|l| l.text.as_str()),
            Some("a < b <i>")
        );
    }

    #[test]
    fn markup_carries_seconds_and_escapes() {
        let html = lyrics_html(
            &parse_lrc("[00:01.5]<Me> & you\n[00:03]"),
            Some("/s/x/a b.mp3"),
        );
        assert!(
            html.starts_with(
                "<ol class=\"site-lyrics\" data-follow-playhead data-media=\"/s/x/a b.mp3\">"
            ),
            "{html}"
        );
        assert!(html.contains("<li data-at=\"1.500\"><button type=\"button\" class=\"site-lyric\">&lt;Me&gt; &amp; you</button></li>"), "{html}");
        assert!(
            html.contains(
                "<li data-at=\"3.000\"><button type=\"button\" class=\"site-lyric\">♪</button></li>"
            ),
            "{html}"
        );
    }

    #[test]
    fn the_fence_renderer_refuses_untimed_text() {
        let claim = Claim::Fence("lrc".to_owned());
        assert!(LyricsBlocks.claims(&claim));
        let block = |source| Block {
            claim: &claim,
            info: "lrc",
            source,
            page: "a.md",
        };
        assert!(LyricsBlocks.render(&block("just words")).is_err());
        assert!(matches!(
            LyricsBlocks.render(&block("[00:01]x")),
            Ok(Drawn::Html(h)) if h.contains("data-at=\"1.000\"")
        ));
    }
}
