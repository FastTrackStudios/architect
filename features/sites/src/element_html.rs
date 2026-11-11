//! An element tree, as a page draws one (`architect_pages::Element`),
//! written out as HTML.
//!
//! The tree is what an extension drew; the HTML goes into a page anyone
//! reads. It is written as plain markup (names checked, text and values
//! escaped) and then re-written by [`crate::sanitize()`], so a tree keeps
//! and loses exactly what a renderer's finished markup does: an SVG keeps
//! its geometry, classes and `data-*`, never a `<script>`, a handler or a
//! `<foreignObject>`.
//!
//! Writing is bounded: at most [`MAX_DEPTH`] levels, [`MAX_ELEMENTS`]
//! elements and [`MAX_HTML_BYTES`] bytes, whatever the tree. A tree that
//! shares subtrees cannot be written as more than that.

use architect_pages::Element;

use crate::sanitize::{VOID, is_name, sanitize};

/// How deep an element tree is written: what is below is left out, so a
/// tree an extension drew cannot overflow the stack writing it.
pub const MAX_DEPTH: usize = 256;

/// How many elements (text included) one tree is written with, at most.
pub const MAX_ELEMENTS: usize = 100_000;

/// How many bytes of markup one tree is written as, at most.
pub const MAX_HTML_BYTES: usize = 16 << 20;

/// `elements`, as HTML.
///
/// # Errors
///
/// The tree is more than [`MAX_ELEMENTS`] elements or [`MAX_HTML_BYTES`]
/// bytes written out.
pub fn elements_to_html(elements: &[Element]) -> Result<String, String> {
    Ok(sanitize(&elements_to_markup(elements)?))
}

/// `elements` written as markup, bounded as [`elements_to_html`] is, not
/// yet sanitised: for [`crate::Drawn::into_block_html`], which sanitises
/// it as a block.
pub fn elements_to_markup(elements: &[Element]) -> Result<String, String> {
    let mut w = TreeWriter::default();
    for element in elements {
        w.element(element, 0)?;
    }
    Ok(w.out)
}

#[derive(Default)]
struct TreeWriter {
    out: String,
    written: usize,
}

impl TreeWriter {
    /// Counts one more element, and checks what is written so far.
    fn count(&mut self) -> Result<(), String> {
        self.written = self.written.saturating_add(1);
        if self.written > MAX_ELEMENTS {
            return Err(format!("a drawing of over {MAX_ELEMENTS} elements"));
        }
        self.check_bytes()
    }

    fn check_bytes(&self) -> Result<(), String> {
        if self.out.len() > MAX_HTML_BYTES {
            return Err(format!("a drawing of over {MAX_HTML_BYTES} bytes"));
        }
        Ok(())
    }

    fn element(&mut self, element: &Element, depth: usize) -> Result<(), String> {
        if depth >= MAX_DEPTH {
            return Ok(());
        }
        self.count()?;
        if element.tag == "#text" {
            crate::sanitize::escape(element.text.as_deref().unwrap_or_default(), &mut self.out);
            return self.check_bytes();
        }
        if !is_name(&element.tag) {
            return Ok(());
        }
        self.out.push('<');
        self.out.push_str(&element.tag);
        for attr in element.attrs.iter().filter(|a| is_name(&a.name)) {
            self.out.push(' ');
            self.out.push_str(&attr.name);
            self.out.push_str("=\"");
            crate::sanitize::escape(&attr.value, &mut self.out);
            self.out.push('"');
        }
        self.out.push('>');
        if VOID.contains(&element.tag.to_ascii_lowercase().as_str()) {
            return self.check_bytes();
        }
        // A style sheet is CSS, not text: written as it is, for the
        // sanitiser to take or refuse whole.
        let sheet = element.tag.eq_ignore_ascii_case("style");
        if let Some(text) = &element.text {
            if sheet {
                self.out.push_str(text);
            } else {
                crate::sanitize::escape(text, &mut self.out);
            }
        }
        self.check_bytes()?;
        for child in &element.children {
            if sheet && child.tag == "#text" {
                self.count()?;
                self.out.push_str(child.text.as_deref().unwrap_or_default());
                self.check_bytes()?;
            } else {
                self.element(child, depth.saturating_add(1))?;
            }
        }
        self.out.push_str("</");
        self.out.push_str(&element.tag);
        self.out.push('>');
        self.check_bytes()
    }
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
            elements_to_html(&[tree]).unwrap(),
            "<div data-follow-playhead=\"\" data-media=\"/s/x/joy.mp3\"><div class=\"kf-bar\" data-at=\"0\">C</div><div class=\"kf-bar\" data-at=\"2.5\">C</div></div>"
        );
    }

    #[test]
    fn writes_a_tree_with_its_styles_and_escapes_it() {
        let mut bar = el(
            "div",
            &[
                ("class", "kf-bar"),
                ("style", "border: 1px solid currentColor"),
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
            elements_to_html(&[tree]).unwrap(),
            "<div id=\"kf-chart\"><div class=\"kf-bar\" style=\"border: 1px solid currentColor\">C &lt;maj&gt;</div><br></div>"
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
            elements_to_html(&[tree]).unwrap(),
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
        assert_eq!(elements_to_html(&tree).unwrap(), "<a>x</a>");
    }

    /// A tree that shares one subtree at every level (as a component's
    /// drawing could before #520's review) is refused at
    /// [`MAX_ELEMENTS`], not written 2^levels times.
    #[test]
    fn a_tree_too_big_to_write_is_refused() {
        let mut tree = el("g", &[], vec![]);
        for _ in 0..30 {
            tree = el("g", &[], vec![tree.clone(), tree]);
            // Each level clones; stop growing the owned tree past the
            // limit so the test itself stays small.
            if count(&tree) > MAX_ELEMENTS {
                break;
            }
        }
        let err = elements_to_html(&[el("svg", &[], vec![tree])]).unwrap_err();
        assert!(err.contains("elements"), "{err}");

        let big = el("p", &[], vec![text(&"x".repeat(MAX_HTML_BYTES + 1))]);
        let err = elements_to_html(&[big]).unwrap_err();
        assert!(err.contains("bytes"), "{err}");
    }

    fn count(e: &Element) -> usize {
        e.children.iter().map(count).fold(1, usize::saturating_add)
    }
}
