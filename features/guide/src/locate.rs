//! Where a [`Target`] is on screen (feature `ui`): one path for the
//! headless runner and for a live window.
//!
//! A target is a query (`@tab"files"`, `@text"View"`, …) turned into a
//! CSS selector by [`Resolver`]s, looked up in a Blitz document, and
//! measured. The runner holds its documents ([`find_in`]); a component in
//! a running window reaches its own through any element it mounted
//! ([`find_live`]): Blitz's `MountedData` is a handle on the node and its
//! document.

use architect_workspace::Rect;
use blitz_dom::BaseDocument;
use dioxus::prelude::MountedData;

use crate::Target;

/// Turns a target's kind and name into a CSS selector, or says it is not
/// its kind (`None`): `("tab", "files")` → `[data-pane-tab="files"]`.
pub type Resolver = fn(kind: &str, name: &str) -> Option<String>;

/// The kinds every document has: `data-testid` and CSS. (An element `id`
/// and `@text` are looked up directly.)
#[must_use]
pub fn builtin(kind: &str, name: &str) -> Option<String> {
    match kind {
        "testid" => Some(format!("[data-testid=\"{name}\"]")),
        "css" => Some(name.to_owned()),
        _ => None,
    }
}

/// Where `target` is in `doc`, in the document's own points.
///
/// The element it resolves to, if that has a size. `outside` leaves an
/// element and everything in it out (a tour's own layer, whose text is
/// not the app's).
#[must_use]
pub fn find_in(
    doc: &BaseDocument,
    target: &Target,
    resolvers: &[Resolver],
    outside: Option<usize>,
) -> Option<Rect> {
    if let Target::At { x, y } = target {
        return Some(Rect::new(*x, *y, 0.0, 0.0));
    }
    let (kind, name) = target.kind_and_name()?;
    let node = match kind {
        "text" => by_text(doc, name, outside),
        "id" => doc.get_element_by_id(name),
        _ => {
            let selector = resolvers
                .iter()
                .find_map(|r| r(kind, name))
                .or_else(|| builtin(kind, name))?;
            doc.query_selector(&selector).ok().flatten()
        }
    }?;
    let r = doc.get_client_bounding_rect(node)?;
    (r.width > 0.0 && r.height > 0.0)
        .then(|| Rect::new(narrow(r.x), narrow(r.y), narrow(r.width), narrow(r.height)))
}

/// [`find_in`] from a live component.
///
/// `mounted` is any element it mounted in the window (a tour's layer).
/// The rectangle is from that
/// element's top left, which is where the component draws; the element
/// and what is in it are left out of the search. `None` when the target
/// is not on screen, the element is not a Blitz one (a browser), or the
/// document is busy this moment (ask again next frame).
#[must_use]
pub fn find_live(mounted: &MountedData, target: &Target, resolvers: &[Resolver]) -> Option<Rect> {
    let node = mounted.downcast::<dioxus_native_dom::NodeHandle>()?;
    let doc = node.try_doc()?;
    let origin = doc.get_client_bounding_rect(node.node_id())?;
    let r = find_in(&doc, target, resolvers, Some(node.node_id()))?;
    Some(Rect::new(
        r.x - narrow(origin.x),
        r.y - narrow(origin.y),
        r.w,
        r.h,
    ))
}

/// The size of the element `mounted` is, in points; `None` as
/// [`find_live`].
#[must_use]
pub fn size_live(mounted: &MountedData) -> Option<(f32, f32)> {
    let node = mounted.downcast::<dioxus_native_dom::NodeHandle>()?;
    let doc = node.try_doc()?;
    let r = doc.get_client_bounding_rect(node.node_id())?;
    Some((narrow(r.width), narrow(r.height)))
}

/// The element whose whole text, trimmed, is `text` and that has a size:
/// the last in the page, which is the innermost of a chain of them nested
/// (a button, its label) and, of the rest, the one drawn over the others
/// (a menu or a popover comes after the page it opens over).
fn by_text(doc: &BaseDocument, text: &str, outside: Option<usize>) -> Option<usize> {
    doc.query_selector_all("*")
        .ok()?
        .into_iter()
        .rev()
        .find(|id| {
            doc.get_node(*id)
                .is_some_and(|n| n.text_content().trim() == text)
                && !outside.is_some_and(|o| within(doc, *id, o))
                && doc
                    .get_client_bounding_rect(*id)
                    .is_some_and(|r| r.width > 0.0 && r.height > 0.0)
        })
}

/// Whether node `id` is `ancestor` or inside it.
fn within(doc: &BaseDocument, id: usize, ancestor: usize) -> bool {
    let mut at = Some(id);
    while let Some(n) = at {
        if n == ancestor {
            return true;
        }
        at = doc.get_node(n).and_then(|node| node.parent);
    }
    false
}

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}
