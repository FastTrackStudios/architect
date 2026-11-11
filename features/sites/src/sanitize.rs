//! What a block drew, made safe to write into a page anyone reads.
//!
//! A block's renderer is an extension's code: a page, a WASI component, a
//! Rust function. Its HTML (or inline SVG) goes into a page served to
//! people who never installed it, so nothing in it may run: no
//! `<script>`, no `on…` handler, no `javascript:` URL, no `<foreignObject>`
//! carrying HTML into an SVG, no frame or plugin, no `<set>` or
//! `<animate>` that would write a link. What a drawing needs comes
//! through: its elements, `class`, `style`, geometry (`viewBox`, `x`,
//! `width`, `d`, `transform`, …) and its `data-*` attributes, the
//! player's contract (`data-follow-playhead`, `data-at`, `data-media`).
//!
//! [`sanitize`] reads the markup into its own model and writes that model
//! out again: every text escaped, every attribute value quoted and
//! escaped, every element it opened closed. What a browser parses is
//! therefore exactly what was kept, whatever the input tried. The element
//! tree path ([`crate::elements_to_html`]) keeps by the same rules
//! ([`drops_element`], [`keeps_attr`]).

/// Elements dropped with everything in them.
const DROPPED: &[&str] = &[
    "script",
    "foreignobject",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "base",
    "meta",
    "link",
    "noscript",
    "template",
    "portal",
    "xmp",
    "noembed",
    "noframes",
    "plaintext",
];

/// HTML elements with no closing tag.
pub(crate) const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Attributes whose value is a URL.
const URLS: &[&str] = &[
    "href",
    "xlink:href",
    "src",
    "action",
    "formaction",
    "poster",
    "background",
    "cite",
    "data",
    "codebase",
];

/// Whether the element `tag` with `attrs` is dropped, with its children.
pub(crate) fn drops_element<'a>(
    tag: &str,
    mut attrs: impl Iterator<Item = (&'a str, &'a str)>,
) -> bool {
    let tag = tag.to_ascii_lowercase();
    if DROPPED.contains(&tag.as_str()) {
        return true;
    }
    // SMIL that writes a link (`<set attributeName="href"
    // to="javascript:…">`) is a script by another name.
    if matches!(
        tag.as_str(),
        "set" | "animate" | "animatemotion" | "animatetransform"
    ) {
        return attrs.any(|(name, value)| {
            name.eq_ignore_ascii_case("attributeName")
                && value.trim().to_ascii_lowercase().ends_with("href")
        });
    }
    false
}

/// Whether the attribute `name` = `value` (its text, entities already
/// decoded) of a `tag` is kept.
pub(crate) fn keeps_attr(tag: &str, name: &str, value: &str) -> bool {
    if !is_name(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("on") || lower == "srcdoc" || lower == "is" {
        return false;
    }
    if URLS.contains(&lower.as_str()) || lower.ends_with(":href") {
        // An SVG `<use>` only of the drawing's own parts.
        if tag.eq_ignore_ascii_case("use") && !value.trim_start().starts_with('#') {
            return false;
        }
        return safe_url(value);
    }
    true
}

/// Whether `url` names nothing that runs: not `javascript:`/`vbscript:`,
/// and `data:` only for an image. A browser ignores tabs, newlines and
/// leading spaces in a scheme, so they are ignored here too.
fn safe_url(url: &str) -> bool {
    let squeezed: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_ascii_control())
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    if squeezed.starts_with("javascript:") || squeezed.starts_with("vbscript:") {
        return false;
    }
    if squeezed.starts_with("data:") {
        return squeezed.starts_with("data:image/") && !squeezed.starts_with("data:image/svg");
    }
    true
}

/// A tag or attribute name: letters, digits, `-`, `_`, `:` and `.`,
/// starting with a letter.
pub(crate) fn is_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

/// `markup` (HTML or inline SVG), with only what [`drops_element`] and
/// [`keeps_attr`] keep, re-written well formed.
///
/// It takes time and space linear in `markup` on any input: block markup
/// comes from extensions and notes, so a sanitiser that could be made to
/// loop or to allocate without bound would be a way to take a node down.
#[must_use]
pub fn sanitize(markup: &str) -> String {
    sanitize_counting(markup).0
}

/// [`sanitize`], and how many passes its loop made (never more than the
/// bytes in `markup`).
fn sanitize_counting(markup: &str) -> (String, usize) {
    let mut w = Writer::default();
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
        passes += 1;
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
struct Writer {
    out: String,
    /// The elements open, by their name as written.
    open: Vec<String>,
}

impl Writer {
    fn in_style(&self) -> bool {
        self.open
            .last()
            .is_some_and(|t| t.eq_ignore_ascii_case("style"))
    }

    /// One pass of [`sanitize`]: text up to the next `<` and the markup
    /// after it. Returns the rest, always shorter than `rest`.
    fn step<'a>(&mut self, rest: &'a str) -> &'a str {
        if self.in_style() {
            // A style sheet is raw text up to its end tag.
            let (text, tail) = cut(rest, find_ci(rest, "</style").unwrap_or(rest.len()));
            self.text(text);
            // `tail` is empty, or starts with the `<` of `</style`, which
            // this pass consumes: `markup` reads what follows a `<`.
            return tail.get(1..).map_or("", |after| self.markup(after));
        }
        match rest.split_once('<') {
            None => {
                self.text(rest);
                ""
            }
            Some((text, after)) => {
                self.text(text);
                self.markup(after)
            }
        }
    }

    /// Text, its entities kept when well formed.
    fn text(&mut self, text: &str) {
        if self.in_style() {
            // A style sheet is raw text in HTML: written as is, without
            // anything that could end it.
            self.out.extend(text.chars().filter(|&c| c != '<'));
        } else {
            escape_keeping_entities(text, &mut self.out);
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
        if drops_element(name, attrs.iter().map(|(n, v)| (n.as_str(), v.as_str()))) {
            return if self_closing {
                rest
            } else {
                skip_element(name, rest)
            };
        }
        let lower = name.to_ascii_lowercase();
        let void = VOID.contains(&lower.as_str());
        if !is_name(name) || (!void && !self_closing && self.open.len() >= MAX_OPEN) {
            return rest;
        }
        self.out.push('<');
        self.out.push_str(name);
        for (attr, value) in &attrs {
            if keeps_attr(name, attr, &decode(value)) {
                self.out.push(' ');
                self.out.push_str(attr);
                self.out.push_str("=\"");
                escape_keeping_entities(value, &mut self.out);
                self.out.push('"');
            }
        }
        self.out.push('>');
        if void {
            return rest;
        }
        if self_closing {
            self.out.push_str("</");
            self.out.push_str(name);
            self.out.push('>');
        } else {
            self.open.push(name.to_owned());
        }
        rest
    }

    /// Close `name` and what is open inside it; nothing when it is not
    /// open.
    fn close(&mut self, name: &str) {
        let Some(at) = self.open.iter().rposition(|t| t.eq_ignore_ascii_case(name)) else {
            return;
        };
        for tag in self.open.split_off(at).iter().rev() {
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        }
    }

    fn finish(mut self) -> String {
        for tag in self.open.drain(..).rev() {
            self.out.push_str("</");
            self.out.push_str(&tag);
            self.out.push('>');
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
        let tag = &tail[..end];
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
    let (name, rest) = (s.get(..semi)?, s.get(semi + 1..)?);
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

/// `value` with its character references decoded, for checking a URL.
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

    /// Keyflow's engraving (keyflow#59): an SVG whose root follows the
    /// playhead, a transparent rect per played bar. All of it comes
    /// through: geometry, classes, `data-*`, a style sheet, gradients
    /// referenced by `#id`, text.
    const CHART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 400 120" width="400" height="120" class="kf-chart" data-follow-playhead="" data-media="/s/abc/joy.mp3"><style>.kf-bar { fill: transparent } .kf-bar[aria-current] { fill: var(--accent) }</style><defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/></linearGradient></defs><rect class="kf-bar" data-at="0" x="0" y="0" width="100" height="120"/><rect class="kf-bar" data-at="2.5" x="100" y="0" width="100" height="120"/><path d="M0 60 L400 60" stroke="currentColor" stroke-width="1.5" transform="translate(0,1)"/><use href="#g"/><text x="4" y="20" font-size="12">C &amp; G</text><a href="#bar-2"><circle cx="5" cy="5" r="2"/></a></svg>"##;

    #[test]
    fn an_svg_drawing_comes_through_whole() {
        let out = sanitize(CHART);
        for kept in [
            r#"viewBox="0 0 400 120""#,
            r#"class="kf-chart""#,
            r#"data-follow-playhead="""#,
            r#"data-media="/s/abc/joy.mp3""#,
            r#"<rect class="kf-bar" data-at="0" x="0" y="0" width="100" height="120"></rect>"#,
            r#"data-at="2.5""#,
            r#"<path d="M0 60 L400 60" stroke="currentColor" stroke-width="1.5" transform="translate(0,1)"></path>"#,
            r##"<linearGradient id="g"><stop offset="0" stop-color="#fff"></stop></linearGradient>"##,
            r##"<use href="#g"></use>"##,
            "C &amp; G",
            ".kf-bar[aria-current] { fill: var(--accent) }",
            r##"<a href="#bar-2"><circle cx="5" cy="5" r="2"></circle></a>"##,
            r#"xmlns:xlink="http://www.w3.org/1999/xlink""#,
        ] {
            assert!(out.contains(kept), "lost {kept} in {out}");
        }
        assert!(out.ends_with("</svg>"), "{out}");
        // Written again, it is the same: nothing left to take out.
        assert_eq!(sanitize(&out), out);
    }

    #[test]
    fn nothing_scriptable_comes_through() {
        let bad = [
            "<svg><script>alert(1)</script><rect/></svg>",
            "<svg><script href=\"x.js\"/><rect/></svg>",
            "<svg onload=\"alert(1)\"><rect onclick='x()' ONMOUSEOVER=x/></svg>",
            "<svg><a href=\"javascript:alert(1)\">x</a></svg>",
            "<svg><a xlink:href=\" JaVa&#x09;Script:alert(1)\">x</a></svg>",
            "<svg><a href=\"&#106;avascript:alert(1)\">x</a></svg>",
            "<svg><a href=\"java&Tab;script:alert(1)\">x</a></svg>",
            "<svg><foreignObject><div onclick=\"x()\"><script>alert(1)</script></div><iframe src=\"/\"></iframe></foreignObject><rect/></svg>",
            "<svg><foreignobject><foreignObject></foreignObject><img src=x onerror=alert(1)></foreignobject></svg>",
            "<svg><set attributeName=\"href\" to=\"javascript:alert(1)\"/><animate attributeName=\"xlink:href\" values=\"javascript:alert(1)\"></animate></svg>",
            "<svg><use href=\"data:image/svg+xml;base64,PHN2Zz4=#x\"/><use href=\"https://evil.example/x.svg#y\"/></svg>",
            "<svg><image href=\"data:text/html,<script>alert(1)</script>\"/></svg>",
            "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe><object data=\"x\"></object><embed src=\"x\">",
            "<div><script>document.write('</div><img src=x onerror=alert(1)>')</script></div>",
            "<svg><!--<script>alert(1)</script>--><rect/></svg>",
            "<svg><![CDATA[<script>alert(1)</script>]]></svg>",
            "<scr<script>ipt>alert(1)</script>",
            "<a href='javascript:alert(1)'>x</a><base href=\"javascript:/\"><meta http-equiv=refresh content=\"0;url=javascript:alert(1)\">",
        ];
        for markup in bad {
            let out = sanitize(markup).to_ascii_lowercase();
            for never in [
                "<script",
                "<foreignobject",
                "<iframe",
                "<object",
                "<embed",
                "<img",
                "<base",
                "<meta",
                "<set",
                "<animate",
                " on",
                "javascript",
                "srcdoc",
                "data:text",
                "data:image/svg",
                "evil.example",
            ] {
                assert!(!out.contains(never), "{never} in {out}\n  from {markup}");
            }
        }
    }

    /// A style sheet is raw text: markup in it stays text, without a `<`
    /// that could open a tag in an SVG, where a `<style>` is not raw.
    #[test]
    fn a_style_sheet_holds_no_markup() {
        assert_eq!(
            sanitize("<svg><style><img src=x onerror=alert(1)></style><rect/></svg>"),
            "<svg><style>img src=x onerror=alert(1)></style><rect></rect></svg>"
        );
        assert_eq!(
            sanitize("<style>a{}</STYLE ><script>x()</script>"),
            "<style>a{}</style>"
        );
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
    fn element_rules_drop_the_same_things() {
        assert!(drops_element("foreignObject", std::iter::empty()));
        assert!(drops_element("SCRIPT", std::iter::empty()));
        assert!(drops_element(
            "set",
            [("attributeName", "xlink:href")].into_iter()
        ));
        assert!(!drops_element(
            "animate",
            [("attributeName", "x")].into_iter()
        ));
        assert!(!drops_element("rect", std::iter::empty()));
        assert!(keeps_attr("rect", "data-at", "1.5"));
        assert!(keeps_attr("svg", "viewBox", "0 0 1 1"));
        assert!(!keeps_attr("rect", "onclick", "x()"));
        assert!(!keeps_attr("a", "href", "\tjavascript:x"));
        assert!(keeps_attr("img", "src", "data:image/png;base64,AA"));
        assert!(!keeps_attr("use", "href", "other.svg#x"));
    }

    /// `markup` sanitised, checked for the linear bound: at most one loop
    /// pass per input byte and six output bytes per input byte (plus the
    /// end tags of what was left open).
    fn bounded(markup: &str) -> String {
        let (out, passes) = sanitize_counting(markup);
        assert!(
            passes <= markup.len(),
            "{passes} passes over {} bytes",
            markup.len()
        );
        assert!(
            out.len() <= 6 * markup.len() + MAX_OPEN * 3,
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
        assert_eq!(bounded("<style>a{}</style>b"), "<style>a{}</style>b");
        assert_eq!(
            bounded("<svg><style>.x{}</style><rect/></svg>"),
            "<svg><style>.x{}</style><rect></rect></svg>"
        );
        // An end tag that only starts like one, and one never ended.
        assert_eq!(bounded("<style>a</styles>b</style"), "<style>ab</style>");
        bounded(&"<style></style>".repeat(10_000));
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
            "<p title=\"&amp\">&lt",
        ] {
            let out = bounded(markup);
            assert!(!out.to_ascii_lowercase().contains("<script"), "{out}");
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
            format!("<script>{}", "<script".repeat(n / 7)),
            format!("<script>{}>", "</script<b".repeat(n / 10)),
            "<g>".repeat(n / 3),
            format!("{}{}", "<g>".repeat(n / 6), "</x>".repeat(n / 8)),
            "<svg><foreignObject>".repeat(n / 20),
            "<!--".repeat(n / 4),
        ] {
            let out = bounded(&markup);
            assert!(!out.to_ascii_lowercase().contains("<script"));
        }
        // Nesting past `MAX_OPEN` writes no more start tags.
        let deep = bounded(&"<g>".repeat(n / 3));
        assert_eq!(deep.matches("<g>").count(), MAX_OPEN);
        assert_eq!(deep.matches("</g>").count(), MAX_OPEN);
    }
}
