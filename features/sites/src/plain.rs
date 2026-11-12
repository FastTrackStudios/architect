//! A page's prose as plain blocks (#629).
//!
//! What a share link's guest reads in the app
//! ([`architect_spaces::GuestNote`]), from the same [`crate::Page::html`]
//! the page is drawn from, so it says what the page says and nothing
//! more.
//!
//! Only text and the URLs of its media come out: no markup, style or
//! script. A media URL is kept only when it is a path on the site
//! (`/…`, no scheme, no `//host`); an image or a medium elsewhere is
//! left out. A block an extension drew (`site-block`) is a
//! [`BlockKind::Drawn`] with its words and its claim, for the app to draw
//! natively when it can.

use architect_spaces::{BlockKind, GuestNote, PlainBlock};

use crate::sanitize::decode;
use crate::site::{Page, Site};

/// The file beside a page's `index.html` its plain blocks are written to
/// ([`crate::Options::page_data`]); a vault file of this name is never
/// published.
pub const NOTE_FILE: &str = "_note.json";

/// The note `page` of `site` as its guest reads it.
#[must_use]
pub fn note(site: &Site, page: &Page) -> GuestNote {
    GuestNote {
        path: page.rel_path.clone().unwrap_or_default(),
        title: page.title.clone(),
        kind: None,
        page: page.url.clone(),
        tags: page.tags.clone(),
        blocks: blocks(&page.html, &site.base),
        source: None,
    }
}

/// One piece of markup.
enum Token<'a> {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Close(String),
    Text(&'a str),
}

/// Read `html` into tags and text. Comments, doctype and processing
/// instructions are dropped; an attribute is `name="value"`,
/// `name='value'`, `name=value` or bare.
fn tokens(html: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            out.push(Token::Text(rest));
            break;
        };
        if lt > 0 {
            out.push(Token::Text(rest.get(..lt).unwrap_or_default()));
        }
        rest = rest.get(lt..).unwrap_or_default();
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after
                .find("-->")
                .map_or("", |e| after.get(e.saturating_add(3)..).unwrap_or_default());
            continue;
        }
        let Some(gt) = tag_end(rest) else {
            // A `<` that opens nothing: text.
            out.push(Token::Text(rest.get(..1).unwrap_or_default()));
            rest = rest.get(1..).unwrap_or_default();
            continue;
        };
        let inner = rest.get(1..gt).unwrap_or_default();
        rest = rest.get(gt.saturating_add(1)..).unwrap_or_default();
        if inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Token::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        let inner = inner.trim_end_matches('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner
            .get(..name_end)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            continue;
        }
        let attrs = attributes(inner.get(name_end..).unwrap_or_default());
        // The text of a script or a style is never prose.
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            rest = find_ci(rest, &close)
                .and_then(|at| rest.get(at..))
                .and_then(|r| r.find('>').and_then(|g| r.get(g.saturating_add(1)..)))
                .unwrap_or("");
            continue;
        }
        out.push(Token::Open { name, attrs });
    }
    out
}

/// Where the tag at the start of `s` ends (its `>`), quotes respected.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase().find(needle)
}

fn attributes(mut s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    loop {
        s = s.trim_start();
        if s.is_empty() {
            return out;
        }
        let end = s
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(s.len());
        let name = s.get(..end).unwrap_or_default().to_ascii_lowercase();
        s = s.get(end..).unwrap_or_default().trim_start();
        let rest = s;
        let (value, past) = rest.strip_prefix('=').map_or_else(
            || (String::new(), rest),
            |after| {
                let after = after.trim_start();
                let (raw, past) = if let Some(q @ ('"' | '\'')) = after.chars().next() {
                    let body = after.get(1..).unwrap_or_default();
                    let close = body.find(q).unwrap_or(body.len());
                    (
                        body.get(..close).unwrap_or_default(),
                        body.get(close.saturating_add(1)..).unwrap_or_default(),
                    )
                } else {
                    let close = after.find(char::is_whitespace).unwrap_or(after.len());
                    (
                        after.get(..close).unwrap_or_default(),
                        after.get(close..).unwrap_or_default(),
                    )
                };
                (decode(raw), past)
            },
        );
        s = past;
        if !name.is_empty() {
            out.push((name, value));
        }
    }
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// A URL a guest may load: a path on the site's own node, below `base`.
fn on_site(url: &str, base: &str) -> Option<String> {
    let url = url.trim();
    let ok = url.starts_with('/')
        && !url.starts_with("//")
        && !url.contains('\\')
        && url.starts_with(base)
        && !url.split(['/', '?', '#']).any(|s| s == "..");
    ok.then(|| url.to_owned())
}

/// What is being read.
enum Open {
    Heading(u8),
    Paragraph,
    List,
    Quote,
    Code(Option<String>),
    /// A block an extension drew, and how deep its divs are.
    Drawn(Option<String>, usize),
    Table,
}

/// A page's prose (`html`, a site's at `base`) as plain blocks.
#[must_use]
pub fn blocks(html: &str, base: &str) -> Vec<PlainBlock> {
    let mut out: Vec<PlainBlock> = Vec::new();
    let mut open: Option<Open> = None;
    let mut text = String::new();
    let mut items: Vec<String> = Vec::new();
    let mut item = String::new();
    // Inside an anchor of a heading (`#`), skipped.
    let mut skipping = 0_usize;
    let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let flush = |out: &mut Vec<PlainBlock>, kind: BlockKind, text: &mut String| {
        let said = words(text);
        text.clear();
        if !said.is_empty() {
            out.push(PlainBlock::new(kind, said));
        }
    };
    for token in tokens(html) {
        match token {
            Token::Text(t) => {
                if skipping > 0 {
                    continue;
                }
                let t = decode(t);
                match &open {
                    Some(Open::List | Open::Table) => item.push_str(&t),
                    Some(_) => text.push_str(&t),
                    None if !t.trim().is_empty() => {
                        text.push_str(&t);
                        open = Some(Open::Paragraph);
                    }
                    None => {}
                }
            }
            Token::Open { name, attrs } => {
                let class = attr(&attrs, "class").unwrap_or_default();
                if let Some(Open::Drawn(_, depth)) = &mut open {
                    if name == "div" {
                        *depth = depth.saturating_add(1);
                    }
                    // A drawing's words (an SVG's `<text>`s) apart.
                    text.push(' ');
                    continue;
                }
                if name == "a" && class.split_whitespace().any(|c| c == "site-anchor") {
                    skipping = skipping.saturating_add(1);
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        let level = name.get(1..).and_then(|l| l.parse().ok()).unwrap_or(2);
                        open = Some(Open::Heading(level));
                    }
                    "p" if !matches!(open, Some(Open::List | Open::Quote | Open::Table)) => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        open = Some(Open::Paragraph);
                    }
                    "ul" | "ol" if !matches!(open, Some(Open::List)) => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        items.clear();
                        open = Some(Open::List);
                        out.push(PlainBlock {
                            level: u8::from(name == "ol"),
                            ..PlainBlock::new(BlockKind::List, "")
                        });
                    }
                    "li" | "tr" => {
                        let said = words(&item);
                        if !said.is_empty() {
                            items.push(said);
                        }
                        item.clear();
                    }
                    "td" | "th" if !item.trim().is_empty() => item.push_str(" · "),
                    "blockquote" => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        open = Some(Open::Quote);
                    }
                    "pre" => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        open = Some(Open::Code(attr(&attrs, "data-lang").map(str::to_owned)));
                    }
                    "table" => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        items.clear();
                        open = Some(Open::Table);
                        out.push(PlainBlock::new(BlockKind::List, ""));
                    }
                    "div" if class.split_whitespace().any(|c| c == "site-block") => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        open = Some(Open::Drawn(
                            attr(&attrs, "data-claim").map(str::to_owned),
                            0,
                        ));
                    }
                    "br" => text.push('\n'),
                    "hr" => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        out.push(PlainBlock::new(BlockKind::Rule, ""));
                    }
                    "img" | "audio" | "video" => {
                        let Some(src) = attr(&attrs, "src").and_then(|s| on_site(s, base)) else {
                            continue;
                        };
                        let kind = match name.as_str() {
                            "img" => BlockKind::Image,
                            "audio" => BlockKind::Audio,
                            _ => BlockKind::Video,
                        };
                        let label = attr(&attrs, "alt")
                            .filter(|a| !a.trim().is_empty())
                            .map_or_else(|| file_name(&src), str::to_owned);
                        // An image in a paragraph stands on its own.
                        if matches!(open, Some(Open::Paragraph) | None) {
                            flush(&mut out, BlockKind::Paragraph, &mut text);
                        }
                        out.push(PlainBlock {
                            src: Some(src),
                            ..PlainBlock::new(kind, label)
                        });
                    }
                    _ => {}
                }
            }
            Token::Close(name) => {
                if skipping > 0 && name == "a" {
                    skipping = skipping.saturating_sub(1);
                    continue;
                }
                match (&mut open, name.as_str()) {
                    (Some(Open::Drawn(claim, depth)), "div") => {
                        if *depth > 0 {
                            *depth = depth.saturating_sub(1);
                            continue;
                        }
                        let said = words(&text);
                        text.clear();
                        out.push(PlainBlock {
                            info: claim.take(),
                            ..PlainBlock::new(BlockKind::Drawn, said)
                        });
                        open = None;
                    }
                    (Some(Open::Drawn(..)), _) => text.push(' '),
                    (Some(Open::Heading(level)), h) if h.starts_with('h') => {
                        let said = words(&text);
                        text.clear();
                        if !said.is_empty() {
                            out.push(PlainBlock {
                                level: *level,
                                ..PlainBlock::new(BlockKind::Heading, said)
                            });
                        }
                        open = None;
                    }
                    (Some(Open::Paragraph), "p") => {
                        flush(&mut out, BlockKind::Paragraph, &mut text);
                        open = None;
                    }
                    (Some(Open::Quote), "blockquote") => {
                        flush(&mut out, BlockKind::Quote, &mut text);
                        open = None;
                    }
                    (Some(Open::Code(info)), "pre") => {
                        let body = std::mem::take(&mut text);
                        let body = body.trim_end_matches('\n').to_owned();
                        out.push(PlainBlock {
                            info: info.take(),
                            ..PlainBlock::new(BlockKind::Code, body)
                        });
                        open = None;
                    }
                    (Some(Open::List), "ul" | "ol") | (Some(Open::Table), "table") => {
                        let said = words(&item);
                        item.clear();
                        if !said.is_empty() {
                            items.push(said);
                        }
                        if let Some(list) = out.last_mut() {
                            list.items = std::mem::take(&mut items);
                        }
                        // An empty list says nothing.
                        if out.last().is_some_and(|l| l.items.is_empty()) {
                            out.pop();
                        }
                        open = None;
                    }
                    _ => {}
                }
            }
        }
    }
    if matches!(open, Some(Open::Paragraph)) || open.is_none() {
        flush(&mut out, BlockKind::Paragraph, &mut text);
    }
    out
}

/// The last segment of a URL path, decoded: what a medium is called.
fn file_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or(path);
    let mut out = Vec::with_capacity(last.len());
    let bytes = last.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes.get(i).copied().unwrap_or(0);
        if b == b'%' {
            if let Some(v) = last
                .get(i.saturating_add(1)..i.saturating_add(3))
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i = i.saturating_add(3);
                continue;
            }
        }
        out.push(b);
        i = i.saturating_add(1);
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(b: &[PlainBlock]) -> Vec<BlockKind> {
        b.iter().map(|b| b.kind).collect()
    }

    #[test]
    fn prose_reads_as_its_blocks() {
        let html = "<h2 id=\"intro\">Intro <a class=\"site-anchor\" href=\"#intro\" aria-label=\"x\">#</a></h2>\n\
            <p>Hello <strong>there</strong> &amp; welcome</p>\n\
            <ul><li>One</li><li>Two <em>too</em></li></ul>\n\
            <blockquote><p>Said</p></blockquote>\n\
            <pre class=\"site-code\" data-lang=\"rust\"><code class=\"language-rust\">fn x() {}\n  y &lt; z\n</code></pre>\n\
            <hr />\n\
            <p><img src=\"/s/c/Set/art.png\" alt=\"Art\"> after</p>";
        let b = blocks(html, "/s/c/");
        assert_eq!(
            kinds(&b),
            [
                BlockKind::Heading,
                BlockKind::Paragraph,
                BlockKind::List,
                BlockKind::Quote,
                BlockKind::Code,
                BlockKind::Rule,
                BlockKind::Image,
                BlockKind::Paragraph
            ]
        );
        assert_eq!(b[0].text, "Intro");
        assert_eq!(b[0].level, 2);
        assert_eq!(b[1].text, "Hello there & welcome");
        assert_eq!(b[2].items, ["One", "Two too"]);
        assert_eq!(b[3].text, "Said");
        assert_eq!(b[4].text, "fn x() {}\n  y < z");
        assert_eq!(b[4].info.as_deref(), Some("rust"));
        assert_eq!(b[6].src.as_deref(), Some("/s/c/Set/art.png"));
        assert_eq!(b[6].text, "Art");
    }

    #[test]
    fn media_off_the_link_is_left_out() {
        let html = "<p><img src=\"https://tracker.example/p.gif\"><img src=\"//x.example/a.png\">\
            <img src=\"/s/other/a.png\"><img src=\"/s/c/../x.png\"><img src=\"data:image/png;base64,AA\"></p>\
            <audio class=\"site-media\" controls src=\"/s/c/Set/take%20one.mp3\"></audio>";
        let b = blocks(html, "/s/c/");
        assert_eq!(kinds(&b), [BlockKind::Audio]);
        assert_eq!(b[0].text, "take one.mp3");
    }

    #[test]
    fn an_extensions_block_is_its_words_and_claim() {
        let html = "<p>Before</p><div class=\"site-block site-block-abc\" style=\"x\" data-claim=\"fence:kf\" data-by=\"keyflow\">\
            <svg><style>.a { fill: red }</style><text>C</text><g><text>G</text></g></svg><div><p>Bar 1</p></div></div><p>After</p>";
        let b = blocks(html, "/");
        assert_eq!(
            kinds(&b),
            [BlockKind::Paragraph, BlockKind::Drawn, BlockKind::Paragraph]
        );
        assert_eq!(b[1].info.as_deref(), Some("fence:kf"));
        assert_eq!(b[1].text, "C G Bar 1");
        assert_eq!(b[2].text, "After");
    }

    #[test]
    fn a_table_is_its_rows() {
        let html = "<div class=\"site-table\"><table><thead><tr><th>Key</th><th>Song</th></tr></thead>\
            <tbody><tr><td>D</td><td>Joy</td></tr></tbody></table></div>";
        let b = blocks(html, "/");
        assert_eq!(kinds(&b), [BlockKind::List]);
        assert_eq!(b[0].items, ["Key · Song", "D · Joy"]);
    }

    #[test]
    fn nothing_that_runs_comes_out() {
        let html = "<p>a<script>alert(1)</script>b<style>p{}</style>c<!-- hidden --></p>";
        let b = blocks(html, "/");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].text, "abc");
    }
}
