//! What every runner draws around a page, natively (`crate::runner`) and
//! in a browser (`crate::web`): the pane's own element's style, the
//! banner over a broken page, and a pane's event as the page's.

use architect_extensions::wire::{Edit, EventData, Value as WireValue, ViewEvent};

use crate::api::{Attr, Element, PageEvent};
use crate::diff::{Mirror, TEXT};

/// A render that threw: its error says so (and goes when one works).
pub const RENDER_FAILED: &str = "its render failed: ";

/// The pane's own element: filling it, in the theme's colours.
#[must_use]
pub fn root_style() -> Vec<Edit> {
    vec![
        Edit::PushId { id: 0 },
        Edit::SetAttribute {
            name: "style".to_owned(),
            namespace: None,
            value: WireValue::Text(
                "display: block; position: relative; min-height: 100%; box-sizing: border-box; \
                 color: var(--foreground); font-family: -apple-system, system-ui, sans-serif; font-size: 13px;"
                    .to_owned(),
            ),
        },
        Edit::Pop,
    ]
}

/// The banner over a broken page `name` (its file's name): the first line
/// of `why`, then what is showing (`then`).
#[must_use]
pub fn banner(name: &str, why: &str, then: &str) -> Element {
    let first = why.lines().next().unwrap_or(why);
    let text = |t: String| Element {
        tag: TEXT.to_owned(),
        text: Some(t),
        ..Element::default()
    };
    Element {
        tag: "div".to_owned(),
        key: Some("architect-page-error".to_owned()),
        attrs: vec![
            Attr {
                name: "class".to_owned(),
                value: "page-error".to_owned(),
            },
            Attr {
                name: "style".to_owned(),
                value: "position: absolute; left: 8px; right: 8px; bottom: 8px; padding: 8px 12px; \
                        border-radius: 8px; font-size: 12px; white-space: pre-wrap; \
                        background: var(--destructive, #b91c1c); color: var(--destructive-foreground, #fff);"
                    .to_owned(),
            },
        ],
        children: vec![text(format!("⚠ {name}: {first}\n{then}"))],
        ..Element::default()
    }
}

/// What a broken page shows: its last good page, or nothing yet.
#[must_use]
pub const fn showing(kept: bool) -> &'static str {
    if kept {
        "the last good page is showing"
    } else {
        "nothing to show until it loads"
    }
}

/// The page's view of a pane event: the handler it is for, and what it
/// carries. `None` when nothing listens for it there.
#[must_use]
pub fn page_event(mirror: &Mirror, e: &ViewEvent) -> Option<PageEvent> {
    let handler = mirror.handler(e.element, &e.name)?;
    let mut out = PageEvent {
        handler,
        event: e.name.clone(),
        ..PageEvent::default()
    };
    match &e.data {
        EventData::Mouse(p) => at(&mut out, p),
        EventData::Pointer(p) => {
            at(&mut out, &p.at);
            out.pointer_id = p.pointer_id;
            out.pointer_type = Some(p.pointer_type.clone());
        }
        EventData::Wheel(w) => {
            at(&mut out, &w.at);
            (out.dx, out.dy) = (w.delta_x, w.delta_y);
        }
        EventData::Touch(t) => {
            if let Some(first) = t.changed.first() {
                (out.x, out.y) = (first.client_x, first.client_y);
                out.pointer_id = first.id;
            }
            out.pointer_type = Some("touch".to_owned());
            (out.alt, out.ctrl, out.meta, out.shift) =
                (t.mods.alt, t.mods.ctrl, t.mods.meta, t.mods.shift);
        }
        EventData::Scroll(s) => (out.scroll_top, out.scroll_left) = (s.top, s.left),
        EventData::Composition(text) => out.value = Some(text.clone()),
        EventData::Key(k) => {
            out.key = Some(k.key.clone());
            (out.alt, out.ctrl, out.meta, out.shift) =
                (k.mods.alt, k.mods.ctrl, k.mods.meta, k.mods.shift);
        }
        EventData::Form(value) => out.value = Some(value.clone()),
        EventData::Focus => {}
    }
    Some(out)
}

/// A pointer's place, buttons and modifiers, into `out`.
const fn at(out: &mut PageEvent, p: &architect_extensions::wire::Pointer) {
    out.x = p.element_x;
    out.y = p.element_y;
    out.buttons = p.buttons;
    (out.alt, out.ctrl, out.meta, out.shift) = (p.mods.alt, p.mods.ctrl, p.mods.meta, p.mods.shift);
}
