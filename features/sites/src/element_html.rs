//! An element tree, as a page draws one (`architect_pages::Element`),
//! written out as HTML.
//!
//! The tree is what an extension's page drew; the HTML goes into a page
//! anyone reads. So it is written defensively: text and attribute values
//! escaped, handlers (`on`, and any `on…` attribute) dropped, a
//! `javascript:` URL dropped, and an element whose tag is not a plain
//! name skipped with everything in it.

use architect_pages::Element;

/// Elements that have no closing tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// `elements`, as HTML.
#[must_use]
pub fn elements_to_html(elements: &[Element]) -> String {
    let mut out = String::new();
    for element in elements {
        write_element(&mut out, element);
    }
    out
}

fn write_element(out: &mut String, element: &Element) {
    if element.tag == "#text" {
        out.push_str(&escape(element.text.as_deref().unwrap_or_default()));
        return;
    }
    if !is_name(&element.tag) || element.tag.eq_ignore_ascii_case("script") {
        return;
    }
    out.push('<');
    out.push_str(&element.tag);
    for attr in &element.attrs {
        let name = attr.name.to_ascii_lowercase();
        if !is_name(&attr.name) || name.starts_with("on") {
            continue;
        }
        if matches!(name.as_str(), "href" | "src" | "action")
            && attr
                .value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("javascript:")
        {
            continue;
        }
        out.push(' ');
        out.push_str(&attr.name);
        out.push_str("=\"");
        out.push_str(&escape(&attr.value));
        out.push('"');
    }
    out.push('>');
    if VOID.contains(&element.tag.to_ascii_lowercase().as_str()) {
        return;
    }
    if let Some(text) = &element.text {
        out.push_str(&escape(text));
    }
    for child in &element.children {
        write_element(out, child);
    }
    out.push_str("</");
    out.push_str(&element.tag);
    out.push('>');
}

/// A tag or attribute name: letters, digits, `-`, `_`, `:` and `.`,
/// starting with a letter.
fn is_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

/// `text` safe in HTML text and in a double-quoted attribute.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use architect_pages::{Attr, Listener};

    fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
        Element {
            tag: tag.into(),
            attrs: attrs
                .iter()
                .map(|(n, v)| Attr {
                    name: (*n).into(),
                    value: (*v).into(),
                })
                .collect(),
            children,
            ..Element::default()
        }
    }

    fn text(t: &str) -> Element {
        Element {
            tag: "#text".into(),
            text: Some(t.into()),
            ..Element::default()
        }
    }

    #[test]
    fn writes_a_tree_with_its_styles_and_escapes_it() {
        let mut bar = el(
            "div",
            &[
                ("class", "kf-bar"),
                ("style", "border: 1px solid var(--border)"),
            ],
            vec![text("C <maj>")],
        );
        bar.on.push(Listener {
            event: "click".into(),
            handler: 1,
        });
        let tree = el(
            "div",
            &[("id", "kf-chart"), ("onclick", "evil()")],
            vec![bar, el("br", &[], vec![])],
        );
        assert_eq!(
            elements_to_html(&[tree]),
            "<div id=\"kf-chart\"><div class=\"kf-bar\" style=\"border: 1px solid var(--border)\">C &lt;maj&gt;</div><br></div>"
        );
    }

    #[test]
    fn drops_scripts_and_script_urls() {
        let tree = vec![
            el("script", &[], vec![text("alert(1)")]),
            el("a", &[("href", " JavaScript:alert(1)")], vec![text("x")]),
            el("bad tag", &[], vec![]),
        ];
        assert_eq!(elements_to_html(&tree), "<a>x</a>");
    }
}
