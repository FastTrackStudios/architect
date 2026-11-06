//! Every Lucide icon by its name (`folder`, `layout-dashboard`), as data.
//!
//! For icons named in data a person chose (a folder's icon in the
//! navigator, a manifest's), where [`crate::named_icon`]'s few will not
//! do. The table is made from lucide-dioxus, the workspace's pinned tag
//! (`features/shell/scripts/lucide.py`); with feature `ui`, [`LucideIcon`]
//! draws one.

#[allow(clippy::large_stack_arrays)]
mod lucide_data;

/// One shape of an icon, on Lucide's 24-unit grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// `d`.
    Path(&'static str),
    /// `cx`, `cy`, `r`.
    Circle(&'static str, &'static str, &'static str),
    /// `points`.
    Polyline(&'static str),
    /// `points`.
    Polygon(&'static str),
}

/// The shapes of icon `name` (`folder`, or `lucide-folder`); `None` for a
/// name Lucide has not.
#[must_use]
pub fn glyphs(name: &str) -> Option<&'static [Glyph]> {
    let name = name.trim();
    let name = name.strip_prefix("lucide-").unwrap_or(name);
    lucide_data::ICONS
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .and_then(|i| lucide_data::ICONS.get(i))
        .map(|(_, g)| *g)
}

/// Every icon's name, A to Z (an icon picker's list).
pub fn names() -> impl Iterator<Item = &'static str> {
    lucide_data::ICONS.iter().map(|(n, _)| *n)
}

#[cfg(feature = "ui")]
use dioxus::prelude::*;

/// Icon `name` drawn `size` points square in `color` (a CSS colour);
/// nothing for a name Lucide has not.
#[cfg(feature = "ui")]
#[component]
pub fn LucideIcon(name: String, size: Option<u32>, color: Option<String>) -> Element {
    let Some(shapes) = glyphs(&name) else {
        return rsx! {};
    };
    let size = size.unwrap_or(16);
    let color = color.unwrap_or_else(|| "currentColor".to_owned());
    rsx! {
        svg {
            "xmlns": "http://www.w3.org/2000/svg",
            "width": "{size}",
            "height": "{size}",
            "viewBox": "0 0 24 24",
            "fill": "none",
            "stroke": "{color}",
            "stroke-width": "2",
            "stroke-linecap": "round",
            "stroke-linejoin": "round",
            "data-icon": "{name}",
            for (i, g) in shapes.iter().enumerate() {
                match *g {
                    Glyph::Path(d) => rsx! { path { key: "{i}", "d": d } },
                    Glyph::Circle(cx, cy, r) => rsx! { circle { key: "{i}", "cx": cx, "cy": cy, "r": r } },
                    Glyph::Polyline(p) => rsx! { polyline { key: "{i}", "points": p } },
                    Glyph::Polygon(p) => rsx! { polygon { key: "{i}", "points": p } },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_found_by_name_with_or_without_the_prefix() {
        assert!(glyphs("folder").is_some());
        assert_eq!(glyphs("lucide-folder"), glyphs("folder"));
        assert!(glyphs("circle").is_some_and(|g| matches!(g.first(), Some(Glyph::Circle(..)))));
        assert!(glyphs("not-an-icon").is_none());
        let all: Vec<&str> = names().collect();
        assert!(all.len() > 1900);
        assert!(
            all.windows(2).all(|w| w.first() < w.get(1)),
            "sorted, for the search"
        );
    }
}
