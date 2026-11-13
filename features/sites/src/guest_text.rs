//! A note's text as a share link's guest's view is handed it (#648, #669):
//! the markdown after the frontmatter, holding nothing the link's page
//! does not show the same visitor.
//!
//! Written by the share's build beside the page (`GuestNote::source` in
//! `_note.json`, [`crate::plain`]), from the same link resolution the page
//! is drawn with ([`crate::markdown::Target`]), so the two never disagree:
//!
//! - `%%comments%%` are gone (`architect_spaces::view::without_comments`,
//!   the rule a member's view is handed the body by);
//! - raw HTML is gone, comments and all (the page draws it only
//!   sanitised; a view draws text);
//! - a link to anything outside the link (another note of the vault, one
//!   that names nothing) is its words alone, as the page draws it, never
//!   its target;
//! - an embed or link of an attachment the link holds back, and an embed
//!   of anything outside the link, is gone whole;
//! - a reference definition (`[x]: path`) that points outside the link is
//!   gone;
//! - links into the link, files it serves and the web's own links stay as
//!   written. Code, inline and fenced, is kept verbatim.

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::markdown::Target;

/// `body` (the markdown after the frontmatter) as a share link's guest's
/// view is handed it; `resolve` is the page's own link resolution (a
/// destination as written, `true` for a wikilink).
#[must_use]
pub fn guest_text(body: &str, resolve: &dyn Fn(&str, bool) -> Target) -> String {
    let body = architect_spaces::view::without_comments(body);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_WIKILINKS);
    let parser = Parser::new_ext(&body, options).into_offset_iter();
    let mut cuts: Vec<(Range<usize>, String)> = Vec::new();
    for (_, def) in parser.reference_definitions().iter() {
        if !kept(resolve, &def.dest, false) {
            cuts.push((def.span.clone(), String::new()));
        }
    }
    // A link being reduced to its words: where it is and the words so far.
    let mut words: Option<(Range<usize>, String)> = None;
    // Everything before this is already cut.
    let mut cut_to = 0_usize;
    for (event, range) in parser {
        if range.start < cut_to {
            continue;
        }
        if let Some((_, text)) = words.as_mut() {
            match event {
                Event::Text(t) | Event::Code(t) => text.push_str(&t),
                Event::SoftBreak | Event::HardBreak => text.push(' '),
                Event::End(TagEnd::Link) => {
                    if let Some((at, text)) = words.take() {
                        cut_to = at.end;
                        cuts.push((at, text));
                    }
                }
                // An image inside the words: never its text.
                Event::Start(Tag::Image { .. }) => cut_to = range.end,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let wiki = matches!(link_type, LinkType::WikiLink { .. });
                match target(resolve, &dest_url, wiki) {
                    None | Some(Target::Page(_) | Target::File(_) | Target::Keep) => {}
                    Some(Target::Held) => {
                        cut_to = range.end;
                        cuts.push((range, String::new()));
                    }
                    Some(Target::Outside | Target::Broken) => words = Some((range, String::new())),
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                let wiki = matches!(link_type, LinkType::WikiLink { .. });
                if !kept(resolve, &dest_url, wiki) {
                    cut_to = range.end;
                    cuts.push((range, String::new()));
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                cut_to = range.end;
                cuts.push((range, String::new()));
            }
            _ => {}
        }
    }
    splice(&body, cuts)
}

/// How `dest` resolves; `None` for a link within the page (`#below`).
fn target(resolve: &dyn Fn(&str, bool) -> Target, dest: &str, wiki: bool) -> Option<Target> {
    let path = dest.split('#').next().unwrap_or_default();
    (!path.is_empty()).then(|| resolve(dest, wiki))
}

/// Whether a link or embed of `dest` stays as written.
fn kept(resolve: &dyn Fn(&str, bool) -> Target, dest: &str, wiki: bool) -> bool {
    matches!(
        target(resolve, dest, wiki),
        None | Some(Target::Page(_) | Target::File(_) | Target::Keep)
    )
}

/// `body` with each cut's range replaced by its text; a cut that overlaps
/// one before it is dropped (the earlier one already took it).
fn splice(body: &str, mut cuts: Vec<(Range<usize>, String)>) -> String {
    cuts.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(body.len());
    let mut at = 0_usize;
    for (range, text) in cuts {
        if range.start < at {
            continue;
        }
        out.push_str(body.get(at..range.start).unwrap_or_default());
        out.push_str(&text);
        at = range.end;
    }
    out.push_str(body.get(at..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(dest: &str, _wiki: bool) -> Target {
        match dest.split('#').next().unwrap_or_default() {
            "Joy" | "Songs/Joy.md" => Target::Page("/joy/".into()),
            "Art/joy.png" | "joy.png" => Target::File("/Art/joy.png".into()),
            "secret.mp3" => Target::Held,
            "Private/Plans" | "Private/Plans.md" | "plans.png" => Target::Outside,
            d if d.contains("://") => Target::Keep,
            _ => Target::Broken,
        }
    }

    #[test]
    fn a_guest_gets_the_words_the_page_shows_and_nothing_past_the_link() {
        let body = "# Set\n\
            1. [[Joy]] then [[Private/Plans|the plan]] and [[Private/Plans]].\n\
            2. [notes](Private/Plans.md) and [web](https://x.dev) and [here](#below)\n\
            ![[Art/joy.png]] ![[plans.png]] ![alt](plans.png) ![[secret.mp3]] [take](secret.mp3)\n\
            %%a private plan%% <!-- html note --> <span title=\"Private/Plans\">x</span>\n\n\
            [ref]: Private/Plans.md\n\n\
            [ok]: Songs/Joy.md\n\n\
            `[[Private/Plans]]` stays code.\n\n\
            ```\n%% kept in code %%\n[[Private/Plans]]\n```\n";
        let got = guest_text(body, &resolve);
        assert!(
            got.contains("1. [[Joy]] then the plan and Private/Plans."),
            "{got}"
        );
        assert!(
            got.contains("2. notes and [web](https://x.dev) and [here](#below)"),
            "{got}"
        );
        assert!(got.contains("![[Art/joy.png]]"), "{got}");
        assert!(got.contains("[ok]: Songs/Joy.md"), "{got}");
        assert!(got.contains("`[[Private/Plans]]` stays code."), "{got}");
        assert!(
            got.contains("%% kept in code %%\n[[Private/Plans]]\n```"),
            "{got}"
        );
        for gone in [
            "plans.png",
            "secret.mp3",
            "take",
            "a private plan",
            "html note",
            "<span",
            "[ref]",
            "[[Private/Plans|",
            "(Private/Plans.md)",
        ] {
            assert!(!got.contains(gone), "{gone:?} in {got}");
        }
    }

    #[test]
    fn plain_text_is_kept_byte_for_byte() {
        let body = "Verse one\n\nChorus: *joy* to the world\n";
        assert_eq!(guest_text(body, &resolve), body);
    }
}
