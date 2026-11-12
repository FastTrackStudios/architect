//! A note's markdown in, finished HTML out: one parse.
//!
//! - **Links** (`[[wikilinks]]`, markdown links, Zola `@/x.md` links) are
//!   resolved by the caller (the vault's link index) and become the page's
//!   URL, marked as a link into the site. One that names nothing becomes
//!   marked text and is reported. `[[…]]` inside code is never a link:
//!   the parser never sees it as one.
//! - **Headings** get an `id` (an explicit `{#id}` wins), unique in the
//!   page, and are kept as data for the page's contents and search.
//! - **Fences** a [`Blocks`] renderer claims are drawn by it; the rest
//!   are code blocks.
//! - A leading `# heading` that repeats the page's title is dropped: the
//!   layout prints the title.

use std::collections::BTreeMap;

use architect_extensions::contrib::Claim;
use pulldown_cmark::{
    CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd, html,
};

use crate::blocks::{Block, Blocks};
use crate::element_html::escape;
use crate::lyrics::{lyrics_html, parse_lrc};
use crate::media::{MediaKind, is_lyrics, media_kind};
use crate::url::fragment_id;

/// One heading in a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// `1` for `#`, `2` for `##`…
    pub level: u8,
    /// Its text, markup flattened.
    pub text: String,
    /// Its `id`: what `#fragment` addresses.
    pub id: String,
}

/// A stretch of a page's text under one heading: what search reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    /// The heading's `id`; `None` for the text before the first one.
    pub id: Option<String>,
    /// The heading's text.
    pub heading: String,
    /// The prose under it, as plain text.
    pub text: String,
}

/// Where a link goes, as the caller resolved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A page of the site, at this URL.
    Page(String),
    /// A file the site copies (an attachment), at this URL.
    File(String),
    /// Into the vault, but to nothing: reported, drawn as marked text.
    Broken,
    /// Not the vault's (another site, a path the site does not manage).
    Keep,
    /// A note of the vault outside a scoped site ([`crate::Scope`]):
    /// drawn as plain text, with no URL, no title and no report.
    Outside,
    /// An attachment the site holds back (`Options::hold_back`): nothing
    /// of it, not its name nor a link's text.
    Held,
}

/// What [`render`] needs.
pub struct Input<'a> {
    /// The markdown, frontmatter already gone.
    pub body: &'a str,
    /// The note's vault path.
    pub page: &'a str,
    /// The page's title, to drop a first `# heading` that repeats it.
    pub title: &'a str,
    /// Resolves a link's destination as written; `true` for a wikilink.
    pub resolve: &'a dyn Fn(&str, bool) -> Target,
    /// Who draws claimed fences.
    pub blocks: &'a Blocks,
    /// The text of the attachment a link's destination names (as
    /// written; `true` for a wikilink), for one the page draws from its
    /// text: a synced-lyrics file (`![[Joy.lrc]]`). `None` draws it as
    /// a link.
    pub text: &'a dyn Fn(&str, bool) -> Option<String>,
}

/// What [`render`] made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendered {
    pub html: String,
    pub headings: Vec<Heading>,
    pub sections: Vec<Section>,
    /// Words of prose.
    pub words: u32,
    /// Links into the vault that named nothing, as written.
    pub broken: Vec<String>,
    /// Blocks a renderer failed on: `(claim, why)`.
    pub block_errors: Vec<(String, String)>,
    /// Blocks an extension drew: `(claim, by)`.
    pub drawn: Vec<(String, String)>,
    /// URLs of the site's pages it links to, once each, in order.
    pub links: Vec<String>,
}

/// A heading being read: its events and text arrive in pieces.
struct OpenHeading<'a> {
    level: u8,
    id: Option<String>,
    events: Vec<Event<'a>>,
    text: String,
}

/// Render one note.
#[must_use]
pub fn render(input: &Input<'_>) -> Rendered {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);
    options.insert(Options::ENABLE_WIKILINKS);

    let mut out = Rendered::default();
    let mut events: Vec<Event<'_>> = Vec::new();
    let mut heading: Option<OpenHeading<'_>> = None;
    let mut fence: Option<(String, String)> = None;
    // What closes each open link: our own markup, or the parser's.
    let mut links: Vec<Option<&'static str>> = Vec::new();
    // Per image: whether it was dropped (it names a note outside the
    // scope), with all it holds.
    let mut images: Vec<bool> = Vec::new();
    // How deep inside a link to a held-back file: nothing of it is drawn.
    let mut held_link: usize = 0;
    // Per kept image: what closes it when it was drawn as our own markup
    // (an `<audio>` or `<video>`), else `None` (the parser's `<img>`).
    let mut image_close: Vec<Option<&'static str>> = Vec::new();
    let mut used_ids: BTreeMap<String, u32> = BTreeMap::new();
    let mut section = Section::default();
    let mut seen_content = false;

    for event in Parser::new_ext(input.body, options) {
        if held_link > 0 {
            match event {
                Event::Start(Tag::Link { .. }) => held_link = held_link.saturating_add(1),
                Event::End(TagEnd::Link) => held_link = held_link.saturating_sub(1),
                _ => {}
            }
            continue;
        }
        // Inside an embed of something outside the scope: nothing of it,
        // not even its alt text.
        if images.contains(&true) && !matches!(event, Event::End(TagEnd::Image)) {
            if matches!(event, Event::Start(Tag::Image { .. })) {
                images.push(true);
            }
            continue;
        }
        if let Some(open) = heading.as_mut() {
            match event {
                Event::End(TagEnd::Heading(_)) => {}
                Event::Text(ref t) | Event::Code(ref t) => {
                    open.text.push_str(t);
                    open.events.push(event);
                    continue;
                }
                other => {
                    open.events.push(other);
                    continue;
                }
            }
        }
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                heading = Some(OpenHeading {
                    level: level_number(level),
                    id: id.map(CowStr::into_string),
                    events: Vec::new(),
                    text: String::new(),
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                let Some(open) = heading.take() else {
                    continue;
                };
                let repeats_title = open.level == 1
                    && !seen_content
                    && open.text.trim().eq_ignore_ascii_case(input.title.trim());
                seen_content = true;
                if repeats_title {
                    continue;
                }
                let wanted = open.id.unwrap_or_else(|| fragment_id(&open.text));
                let id = unique_id(&wanted, &mut used_ids);
                let level = open.level;
                events.push(Event::Html(
                    format!("<h{level} id=\"{}\">", escape(&id)).into(),
                ));
                events.extend(open.events);
                events.push(Event::Html(
                    format!("<a class=\"site-anchor\" href=\"#{}\" aria-label=\"Link to this section\">#</a></h{level}>\n", escape(&id)).into(),
                ));
                out.sections.push(std::mem::replace(
                    &mut section,
                    Section {
                        id: Some(id.clone()),
                        heading: open.text.clone(),
                        text: String::new(),
                    },
                ));
                out.headings.push(Heading {
                    level,
                    text: open.text,
                    id,
                });
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                seen_content = true;
                fence = Some((info.into_string(), String::new()));
            }
            Event::Text(text) if fence.is_some() => {
                if let Some((_, body)) = fence.as_mut() {
                    body.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) if fence.is_some() => {
                let Some((info, body)) = fence.take() else {
                    continue;
                };
                events.push(Event::Html(
                    draw_fence(input, &info, &body, &mut out).into(),
                ));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let wiki = matches!(link_type, LinkType::WikiLink { .. });
                if matches!((input.resolve)(&dest_url, wiki), Target::Held) {
                    held_link = 1;
                    continue;
                }
                let (open, close) = link_markup(input, &dest_url, wiki, &mut out);
                events.push(Event::Html(open.into()));
                links.push(Some(close));
            }
            Event::End(TagEnd::Image) => {
                if images.pop() != Some(true) {
                    match image_close.pop().flatten() {
                        Some(close) => events.push(Event::Html(close.into())),
                        None => events.push(Event::End(TagEnd::Image)),
                    }
                }
            }
            Event::End(TagEnd::Link) => {
                if let Some(Some(close)) = links.pop() {
                    events.push(Event::Html(close.into()));
                } else {
                    events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                seen_content = true;
                let wiki = matches!(link_type, LinkType::WikiLink { .. });
                let original = dest_url.to_string();
                let resolved = (input.resolve)(&dest_url, wiki);
                let is_file = matches!(resolved, Target::File(_));
                let dest_url = match resolved {
                    Target::Page(url) | Target::File(url) => CowStr::from(url),
                    Target::Broken => {
                        out.broken.push(dest_url.to_string());
                        dest_url
                    }
                    Target::Keep => dest_url,
                    Target::Outside | Target::Held => {
                        images.push(true);
                        continue;
                    }
                };
                // An embedded lyrics file: its lines, drawn from its
                // text; nothing of the embed itself (#511).
                if is_file && is_lyrics(&dest_url) {
                    if let Some(lyrics) = (input.text)(&original, wiki) {
                        events.push(Event::Html(lyrics_html(&parse_lrc(&lyrics), None).into()));
                        images.push(true);
                        continue;
                    }
                }
                images.push(false);
                // Audio or video: a native element the page's player
                // follows (`data-site-media`); the alt text is its
                // fallback.
                if let Some(kind) = media_kind(&dest_url).filter(|_| is_file) {
                    let tag = kind.tag();
                    let extra = if kind == MediaKind::Video {
                        " playsinline"
                    } else {
                        ""
                    };
                    events.push(Event::Html(
                        format!(
                            "<{tag} class=\"site-media\" controls preload=\"metadata\"{extra} data-site-media=\"{tag}\" src=\"{}\">",
                            escape(&dest_url)
                        )
                        .into(),
                    ));
                    image_close.push(Some(if kind == MediaKind::Video {
                        "</video>"
                    } else {
                        "</audio>"
                    }));
                    continue;
                }
                image_close.push(None);
                events.push(Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }));
            }
            Event::Start(Tag::Table(alignments)) => {
                seen_content = true;
                events.push(Event::Html("<div class=\"site-table\">".into()));
                events.push(Event::Start(Tag::Table(alignments)));
            }
            Event::End(TagEnd::Table) => {
                events.push(Event::End(TagEnd::Table));
                events.push(Event::Html("</div>".into()));
            }
            Event::Text(ref text) | Event::Code(ref text) => {
                count_words(text, &mut out.words);
                push_text(&mut section.text, text);
                events.push(event);
            }
            Event::SoftBreak | Event::HardBreak => {
                section.text.push(' ');
                events.push(event);
            }
            Event::End(ref end) => {
                if !matches!(
                    end,
                    TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough
                ) {
                    section.text.push(' ');
                }
                events.push(event);
            }
            Event::Start(_) | Event::Html(_) => {
                seen_content = true;
                events.push(event);
            }
            other => events.push(other),
        }
    }
    out.sections.push(section);
    for s in &mut out.sections {
        s.text = s.text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    out.sections
        .retain(|s| s.id.is_some() || !s.text.trim().is_empty());

    html::push_html(&mut out.html, events.into_iter());
    out
}

/// A link's opening and closing markup.
fn link_markup(
    input: &Input<'_>,
    dest: &str,
    wiki: bool,
    out: &mut Rendered,
) -> (String, &'static str) {
    let (path, fragment) = dest
        .split_once('#')
        .map_or((dest, None), |(p, f)| (p, Some(f)));
    let fragment = fragment
        .filter(|f| !f.is_empty())
        .map(|f| if wiki { fragment_id(f) } else { f.to_owned() });
    let with_fragment = |url: String| match &fragment {
        Some(f) => format!("{url}#{f}"),
        None => url,
    };
    if path.is_empty() {
        // `[x](#below)`: within this page.
        let href = fragment.map(|f| format!("#{f}")).unwrap_or_default();
        return (format!("<a href=\"{}\">", escape(&href)), "</a>");
    }
    match (input.resolve)(dest, wiki) {
        Target::Page(url) => {
            if !out.links.contains(&url) {
                out.links.push(url.clone());
            }
            (
                format!(
                    "<a class=\"site-link\" href=\"{}\">",
                    escape(&with_fragment(url))
                ),
                "</a>",
            )
        }
        Target::File(url) => (format!("<a href=\"{}\">", escape(&url)), "</a>"),
        Target::Broken => {
            out.broken.push(dest.to_owned());
            (
                format!(
                    "<span class=\"site-broken-link\" title=\"Not in this site: {}\">",
                    escape(dest)
                ),
                "</span>",
            )
        }
        Target::Outside | Target::Held => ("<span class=\"site-outside\">".to_owned(), "</span>"),
        Target::Keep => {
            let external = dest.contains("://") || dest.starts_with("mailto:");
            let rel = if external { " rel=\"noopener\"" } else { "" };
            (format!("<a href=\"{}\"{rel}>", escape(dest)), "</a>")
        }
    }
}

/// A fenced block: drawn by whoever claims it, else a code block.
fn draw_fence(input: &Input<'_>, info: &str, body: &str, out: &mut Rendered) -> String {
    let code = |lang: Option<&str>| {
        let lang = lang.filter(|l| {
            l.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+'))
        });
        lang.map_or_else(
            || format!("<pre class=\"site-code\"><code>{}</code></pre>\n", escape(body)),
            |lang| {
                format!(
                    "<pre class=\"site-code\" data-lang=\"{lang}\"><code class=\"language-{lang}\">{}</code></pre>\n",
                    escape(body)
                )
            },
        )
    };
    let Some(claim) = Claim::fence(info) else {
        return code(None);
    };
    let block = Block {
        claim: &claim,
        info,
        source: body,
        page: input.page,
    };
    let lang = claim.name().to_owned();
    match input.blocks.draw(&block) {
        None => code(Some(&lang)),
        Some((by, Ok(html))) => {
            out.drawn.push((claim.to_string(), by.clone()));
            // The site's box, not the block's: its inline style (which no
            // sheet of the block's can outrank, and which the block's own
            // markup cannot reach) keeps what the block draws inside it.
            format!(
                "<div class=\"site-block {}\" style=\"{}\" data-claim=\"{}\" data-by=\"{}\">{html}</div>\n",
                block.scope(),
                crate::sanitize::BLOCK_BOX_STYLE,
                escape(&claim.to_string()),
                escape(&by)
            )
        }
        Some((by, Err(why))) => {
            out.block_errors
                .push((claim.to_string(), format!("{by}: {why}")));
            format!(
                "<div class=\"site-block site-block-failed\" data-claim=\"{}\"><p class=\"site-block-why\">{} could not draw this block: {}</p>{}</div>\n",
                escape(&claim.to_string()),
                escape(&by),
                escape(&why),
                code(Some(&lang))
            )
        }
    }
}

const fn level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// `wanted`, or `wanted-2`, `wanted-3`… when the page already has it.
fn unique_id(wanted: &str, used: &mut BTreeMap<String, u32>) -> String {
    let seen = used.entry(wanted.to_owned()).or_insert(0);
    *seen = seen.saturating_add(1);
    if *seen == 1 {
        wanted.to_owned()
    } else {
        let id = format!("{wanted}-{seen}");
        used.insert(id.clone(), 1);
        id
    }
}

fn count_words(text: &str, words: &mut u32) {
    let n = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
    *words = words.saturating_add(n);
}

/// Prose into a section's text: as written, one space between words.
fn push_text(into: &mut String, text: &str) {
    if text.starts_with(char::is_whitespace) && !into.ends_with(' ') && !into.is_empty() {
        into.push(' ');
    }
    into.push_str(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    if text.ends_with(char::is_whitespace) {
        into.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{Drawn, FnBlock};
    use crate::url::is_md;

    fn resolve(dest: &str, wiki: bool) -> Target {
        let path = dest.split('#').next().unwrap_or_default();
        match path {
            "Shell" | "@/features/shell.md" => Target::Page("/features/shell/".into()),
            "pic.png" => Target::File("/pic.png".into()),
            "take.mp3" => Target::File("/Set/take.mp3".into()),
            "clip.mp4" => Target::File("/clip.mp4".into()),
            "Joy.lrc" | "Gone.lrc" => Target::File(format!("/{path}")),
            _ if wiki || is_md(path) => Target::Broken,
            _ => Target::Keep,
        }
    }

    fn go(body: &str, blocks: &Blocks) -> Rendered {
        render(&Input {
            body,
            page: "guide/ui.md",
            title: "The UI",
            resolve: &resolve,
            blocks,
            text: &|_, _| None,
        })
    }

    #[test]
    fn media_embeds_are_native_players_and_lyrics_are_lines() {
        let text = |dest: &str, _wiki: bool| {
            (dest == "Joy.lrc").then(|| "[00:01.25]Hello\n[00:03]World".to_owned())
        };
        let r = render(&Input {
            body: "![[take.mp3|The take]]\n\n![[clip.mp4]]\n\n![[Joy.lrc]]\n\n![[Gone.lrc]]\n\n![[missing.mp3]]",
            page: "guide/ui.md",
            title: "The UI",
            resolve: &resolve,
            blocks: &Blocks::new(),
            text: &text,
        });
        assert!(
            r.html.contains("<audio class=\"site-media\" controls preload=\"metadata\" data-site-media=\"audio\" src=\"/Set/take.mp3\">"),
            "{}",
            r.html
        );
        assert!(r.html.contains("</audio>"), "{}", r.html);
        assert!(
            r.html.contains("<video class=\"site-media\" controls preload=\"metadata\" playsinline data-site-media=\"video\" src=\"/clip.mp4\">"),
            "{}",
            r.html
        );
        assert!(
            r.html.contains("<li data-at=\"1.250\"><button type=\"button\" class=\"site-lyric\">Hello</button></li>"),
            "{}",
            r.html
        );
        // A lyrics file whose text is not there stays an image link.
        assert!(r.html.contains("src=\"/Gone.lrc\""), "{}", r.html);
        // A media name that resolves to nothing is no player.
        assert!(!r.html.contains("missing.mp3\" data-site"), "{}", r.html);
        assert_eq!(r.html.matches("data-site-media").count(), 2, "{}", r.html);
    }

    #[test]
    fn links_resolve_or_are_reported_and_code_is_verbatim() {
        let r = go(
            "See [[Shell#The bar]], [it](@/features/shell.md#tabs), [[Nope]], [web](https://x.dev) and `[[Shell]]`.",
            &Blocks::new(),
        );
        assert!(
            r.html.contains(
                "<a class=\"site-link\" href=\"/features/shell/#the-bar\">Shell#The bar</a>"
            ),
            "{}",
            r.html
        );
        assert!(
            r.html.contains("href=\"/features/shell/#tabs\""),
            "{}",
            r.html
        );
        assert!(
            r.html.contains("<span class=\"site-broken-link\""),
            "{}",
            r.html
        );
        assert!(r.html.contains("<code>[[Shell]]</code>"), "{}", r.html);
        assert!(r.html.contains("rel=\"noopener\""));
        assert_eq!(r.broken, vec!["Nope"]);
    }

    #[test]
    fn headings_get_unique_ids_and_a_repeated_title_goes() {
        let r = go(
            "# The UI\n\nIntro.\n\n## Tabs\n\nA.\n\n## Tabs\n\n### Custom {#mine}\n",
            &Blocks::new(),
        );
        assert!(!r.html.contains("<h1"));
        let ids: Vec<_> = r.headings.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["tabs", "tabs-2", "mine"]);
        assert_eq!(r.sections.first().map(|s| s.text.as_str()), Some("Intro."));
    }

    #[test]
    fn a_claimed_fence_is_drawn_and_the_rest_are_code() {
        let blocks = Blocks::new().with(FnBlock::new("demo", &["kf"], |b| {
            Ok(Drawn::Html(format!("<b>{}</b>", b.source.trim())))
        }));
        let r = go("```kf\n| C |\n```\n\n```rust\nfn x() {}\n```\n", &blocks);
        let drawn = format!(
            "\" style=\"{}\" data-claim=\"fence:kf\" data-by=\"demo\"><b>| C |</b></div>",
            crate::BLOCK_BOX_STYLE
        );
        assert!(
            r.html.contains("<div class=\"site-block site-block-"),
            "{}",
            r.html
        );
        assert!(r.html.contains(&drawn), "{}", r.html);
        assert!(
            r.html
                .contains("<pre class=\"site-code\" data-lang=\"rust\">")
        );
        assert_eq!(r.drawn, vec![("fence:kf".to_owned(), "demo".to_owned())]);
    }

    /// #520's second review, finding 2: the site wraps every block in a
    /// box of its own class and style, and a block's sheet is put under
    /// that class, so it styles the block and nothing else on the page.
    #[test]
    fn a_block_is_boxed_and_its_sheet_scoped_to_its_box() {
        let blocks = Blocks::new().with(FnBlock::new("demo", &["kf", "x"], |b| {
            Ok(Drawn::Html(format!(
                "<svg><style>.kf-bar{{fill:red}} rect, g > .a {{ opacity: 0.5 }}</style><rect class=\"kf-bar\"/><text>{}</text></svg>",
                b.source.trim()
            )))
        }));
        let r = go("```kf\n| C |\n```\n\n```x\nbody\n```\n", &blocks);
        let scopes: Vec<&str> = r
            .html
            .match_indices("<div class=\"site-block ")
            .filter_map(|(at, found)| {
                let rest = r.html.get(at + found.len()..)?;
                rest.split_once('"').map(|(scope, _)| scope)
            })
            .collect();
        assert_eq!(scopes.len(), 2, "{}", r.html);
        assert_ne!(scopes[0], scopes[1], "each block has its own scope");
        for scope in &scopes {
            assert!(
                scope.starts_with("site-block-") && scope.len() == 27,
                "{scope}"
            );
            assert!(
                r.html.contains(&format!(
                    "<div class=\"site-block {scope}\" style=\"{}\"",
                    crate::BLOCK_BOX_STYLE
                )),
                "{}",
                r.html
            );
            assert!(
                r.html.contains(&format!(
                    "<style>.{scope} .kf-bar {{ fill:red }} .{scope} rect, .{scope} g > .a {{ opacity: 0.5 }}</style>"
                )),
                "{}",
                r.html
            );
        }
        // Built again, the page is the same.
        assert_eq!(
            go("```kf\n| C |\n```\n\n```x\nbody\n```\n", &blocks).html,
            r.html
        );
    }

    #[test]
    fn a_failed_block_is_reported_and_shows_its_source() {
        let blocks = Blocks::new().with(FnBlock::new("demo", &["kf"], |_| Err("no".into())));
        let r = go("```kf\n| C |\n```\n", &blocks);
        assert_eq!(
            r.block_errors,
            vec![("fence:kf".to_owned(), "demo: no".to_owned())]
        );
        assert!(r.html.contains("site-block-failed") && r.html.contains("| C |"));
    }

    #[test]
    fn images_resolve_to_copied_files() {
        let r = go("![[pic.png]] and ![alt](pic.png)", &Blocks::new());
        assert_eq!(r.html.matches("src=\"/pic.png\"").count(), 2, "{}", r.html);
    }
}
