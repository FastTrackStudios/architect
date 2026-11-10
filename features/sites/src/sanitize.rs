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
#[must_use]
pub fn sanitize(markup: &str) -> String {
    let mut w = Writer::default();
    let mut rest = markup;
    while !rest.is_empty() {
        if w.in_style() {
            // A style sheet is raw text up to its end tag.
            let end = find_ci(rest, "</style").unwrap_or(rest.len());
            w.text(&rest[..end]);
            rest = &rest[end..];
            if !rest.is_empty() {
                rest = w.markup(rest);
            }
            continue;
        }
        let Some(lt) = rest.find('<') else {
            w.text(rest);
            break;
        };
        w.text(&rest[..lt]);
        rest = &rest[lt..];
        rest = w.markup(rest);
    }
    w.finish()
}

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

    /// What starts at `<` in `rest`; the rest after it.
    fn markup<'a>(&mut self, rest: &'a str) -> &'a str {
        let after = &rest[1..];
        if let Some(body) = after.strip_prefix("!--") {
            return body.find("-->").map_or("", |end| &body[end + 3..]);
        }
        if let Some(body) = after.strip_prefix("![CDATA[") {
            let end = body.find("]]>").unwrap_or(body.len());
            let mut text = String::new();
            escape(&body[..end], &mut text);
            self.out.push_str(&text);
            return body.get(end + 3..).unwrap_or("");
        }
        if after.starts_with('!') || after.starts_with('?') {
            return after.find('>').map_or("", |end| &after[end + 1..]);
        }
        if let Some(body) = after.strip_prefix('/') {
            let (name, tail) = take_name(body);
            let rest = tail.find('>').map_or("", |end| &tail[end + 1..]);
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
        if !is_name(name) {
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
        let lower = name.to_ascii_lowercase();
        if VOID.contains(&lower.as_str()) {
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
        let Some(at) = self
            .open
            .iter()
            .rposition(|t| t.eq_ignore_ascii_case(name))
        else {
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
    (&s[..end], &s[end..])
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
        if let Some(rest) = s.strip_prefix('/') {
            s = rest;
            continue;
        }
        let (name, tail) = take_name(s);
        if name.is_empty() {
            // A stray `=`: skip it.
            s = &s[1..];
            continue;
        }
        let tail = tail.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let Some(value_at) = tail.strip_prefix('=') else {
            attrs.push((name.to_owned(), String::new()));
            s = tail;
            continue;
        };
        let value_at = value_at.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let (value, rest) = match value_at.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = &value_at[1..];
                let end = body.find(q).unwrap_or(body.len());
                (&body[..end], body.get(end + 1..).unwrap_or(""))
            }
            _ => {
                let end = value_at
                    .find(|c: char| c.is_ascii_whitespace() || c == '>')
                    .unwrap_or(value_at.len());
                (&value_at[..end], &value_at[end..])
            }
        };
        attrs.push((name.to_owned(), value.to_owned()));
        s = rest;
    }
}

/// Past the end of a dropped `name` whose start tag was just read: its
/// matching end tag, nested ones of the same name counted. Unclosed, the
/// rest is dropped.
fn skip_element<'a>(name: &str, mut rest: &'a str) -> &'a str {
    let lower = name.to_ascii_lowercase();
    let mut depth = 1usize;
    loop {
        let Some(lt) = rest.find('<') else { return "" };
        rest = &rest[lt + 1..];
        let closing = rest.starts_with('/');
        let (found, tail) = take_name(rest.trim_start_matches('/'));
        if found.eq_ignore_ascii_case(&lower) {
            let end = tail.find('>').map_or(tail.len(), |e| e + 1);
            let self_closing = tail[..end].trim_end_matches('>').ends_with('/');
            if closing {
                depth -= 1;
                if depth == 0 {
                    return &tail[end..];
                }
            } else if !self_closing {
                depth += 1;
            }
        }
    }
}

/// `text` escaped for HTML text and a double-quoted attribute, a well
/// formed character reference (`&amp;`, `&#38;`, `&#x26;`) kept as it is.
fn escape_keeping_entities(text: &str, out: &mut String) {
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        escape(&rest[..amp], out);
        let after = &rest[amp..];
        match entity_len(after) {
            Some(len) => out.push_str(&after[..len]),
            None => out.push_str("&amp;"),
        }
        rest = &after[entity_len(after).unwrap_or(1)..];
    }
    escape(rest, out);
}

/// The length of the character reference `s` starts with (`&name;`,
/// `&#9;`, `&#x9;`), if it is one.
fn entity_len(s: &str) -> Option<usize> {
    let body = s.strip_prefix('&')?;
    let end = body.find(';')?;
    let name = &body[..end];
    let ok = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        !hex.is_empty() && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit())
    } else if let Some(dec) = name.strip_prefix('#') {
        !dec.is_empty() && dec.len() <= 7 && dec.chars().all(|c| c.is_ascii_digit())
    } else {
        !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric())
    };
    ok.then_some(end + 2)
}

/// `value` with its character references decoded, for checking a URL.
/// A named one it does not know becomes U+FFFD, which no scheme has.
fn decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let Some(len) = entity_len(after) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let name = &after[1..len - 1];
        let ch = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = name.strip_prefix('#') {
            dec.parse().ok().and_then(char::from_u32)
        } else {
            match name {
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
            }
        };
        out.push(ch.unwrap_or('\u{fffd}'));
        rest = &after[len..];
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
        assert!(!drops_element("animate", [("attributeName", "x")].into_iter()));
        assert!(!drops_element("rect", std::iter::empty()));
        assert!(keeps_attr("rect", "data-at", "1.5"));
        assert!(keeps_attr("svg", "viewBox", "0 0 1 1"));
        assert!(!keeps_attr("rect", "onclick", "x()"));
        assert!(!keeps_attr("a", "href", "\tjavascript:x"));
        assert!(keeps_attr("img", "src", "data:image/png;base64,AA"));
        assert!(!keeps_attr("use", "href", "other.svg#x"));
    }
}
