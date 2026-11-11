//! An element tree, as a page draws one (`architect_pages::Element`),
//! written out as HTML.
//!
//! The tree is what an extension's page drew; the HTML goes into a page
//! anyone reads. So it is written defensively: text and attribute values
//! escaped, handlers (`on`, and any `on…` attribute) dropped, a
//! `javascript:` URL dropped, and an element whose tag is not a plain
//! name skipped with everything in it. What is kept and dropped is the
//! same as for a renderer's finished markup (`crate::sanitize`): an SVG
//! keeps its geometry, classes and `data-*`, never a `<script>` or a
//! `<foreignObject>`.

use architect_pages::Element;

use crate::sanitize::{VOID, drops_element, is_name, keeps_attr};

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
    let attrs = element
        .attrs
        .iter()
        .map(|a| (a.name.as_str(), a.value.as_str()));
    if !is_name(&element.tag) || drops_element(&element.tag, attrs) {
        return;
    }
    out.push('<');
    out.push_str(&element.tag);
    for attr in &element.attrs {
        if !keeps_attr(&element.tag, &attr.name, &attr.value) {
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

/// `text` safe in HTML text and in a double-quoted attribute.
#[must_use]
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    crate::sanitize::escape(text, &mut out);
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

    /// The playhead contract (#511): an extension's block follows the
    /// page's player with attributes alone, since its scripts are never
    /// written. They come through as drawn.
    #[test]
    fn a_blocks_playhead_attributes_come_through() {
        let bar = |at: &str| {
            el(
                "div",
                &[("class", "kf-bar"), ("data-at", at)],
                vec![text("C")],
            )
        };
        let tree = el(
            "div",
            &[("data-follow-playhead", ""), ("data-media", "/s/x/joy.mp3")],
            vec![bar("0"), bar("2.5"), el("script", &[], vec![text("x()")])],
        );
        assert_eq!(
            elements_to_html(&[tree]),
            "<div data-follow-playhead=\"\" data-media=\"/s/x/joy.mp3\"><div class=\"kf-bar\" data-at=\"0\">C</div><div class=\"kf-bar\" data-at=\"2.5\">C</div></div>"
        );
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

    /// An SVG drawn as a tree (keyflow#59's chart, from a page): its
    /// geometry, classes and `data-*` come through; a `<foreignObject>`,
    /// a script inside the SVG and a `<set>` writing a link do not.
    #[test]
    fn an_svg_tree_keeps_its_drawing_and_loses_what_runs() {
        let rect = |at: &str, x: &str| {
            el(
                "rect",
                &[
                    ("class", "kf-bar"),
                    ("data-at", at),
                    ("x", x),
                    ("width", "100"),
                    ("height", "40"),
                ],
                vec![],
            )
        };
        let tree = el(
            "svg",
            &[
                ("viewBox", "0 0 200 40"),
                ("data-follow-playhead", ""),
                ("data-media", "/s/x/joy.mp3"),
                ("onload", "x()"),
            ],
            vec![
                rect("0", "0"),
                rect("2.5", "100"),
                el(
                    "foreignObject",
                    &[],
                    vec![el("div", &[], vec![text("html")])],
                ),
                el("script", &[], vec![text("x()")]),
                el(
                    "set",
                    &[("attributeName", "href"), ("to", "javascript:x()")],
                    vec![],
                ),
                el("a", &[("xlink:href", "javascript:x()")], vec![text("a")]),
            ],
        );
        assert_eq!(
            elements_to_html(&[tree]),
            "<svg viewBox=\"0 0 200 40\" data-follow-playhead=\"\" data-media=\"/s/x/joy.mp3\"><rect class=\"kf-bar\" data-at=\"0\" x=\"0\" width=\"100\" height=\"40\"></rect><rect class=\"kf-bar\" data-at=\"2.5\" x=\"100\" width=\"100\" height=\"40\"></rect><a>a</a></svg>"
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
