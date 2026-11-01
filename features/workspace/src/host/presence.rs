//! The others, drawn over a pane: a layer a view mounts as its last child.
//!
//! ```ignore
//! let overlay = use_presence_overlay();
//! overlay.set(marks); // from the roster, placed in this view's pixels
//! rsx! { div { /* the view */ } PresenceLayer { overlay } }
//! ```
//!
//! The pane body is positioned, so the layer covers exactly the pane, and
//! it takes no pointer events: the view under it keeps every press.

use dioxus::prelude::*;

use crate::presence::{PresenceMarks, css_color};

/// Over the view's own content, under the shell's edges and floats.
const PRESENCE_Z: u8 = 4;

/// The marks a pane's [`PresenceLayer`] draws, written by its view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PresenceOverlay {
    marks: Signal<PresenceMarks>,
}

impl PresenceOverlay {
    /// Draw `marks` (the same marks again draws nothing new).
    pub fn set(&self, marks: PresenceMarks) {
        let mut current = self.marks;
        if *current.peek() != marks {
            current.set(marks);
        }
    }

    /// What is drawn. Reactive.
    #[must_use]
    pub fn marks(&self) -> PresenceMarks {
        self.marks.read().clone()
    }
}

/// A presence layer for the calling view, owned by its scope.
#[must_use]
pub fn use_presence_overlay() -> PresenceOverlay {
    PresenceOverlay {
        marks: use_signal(PresenceMarks::default),
    }
}

/// Draws `overlay`'s marks over the pane: washes, then playheads, then
/// pointers with their names.
#[component]
pub fn PresenceLayer(overlay: PresenceOverlay) -> Element {
    let marks = overlay.marks();
    if marks.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            style: "position:absolute; inset:0; pointer-events:none; overflow:hidden; z-index:{PRESENCE_Z};",
            for wash in marks.washes {
                div {
                    key: "wash-{wash.peer}-{wash.x}-{wash.y}",
                    style: "position:absolute; left:{wash.x}px; top:{wash.y}px; \
                            width:{wash.w}px; height:{wash.h}px; background:{css_color(wash.color, 0.18)}; \
                            outline:1px solid {css_color(wash.color, 0.5)};",
                }
            }
            for line in marks.playheads {
                div {
                    key: "play-{line.peer}",
                    style: "position:absolute; top:0; bottom:0; left:{line.x}px; width:2px; \
                            background:{css_color(line.color, 0.6)};",
                }
            }
            for pointer in marks.pointers {
                div {
                    key: "pointer-{pointer.peer}",
                    style: "position:absolute; left:{pointer.x}px; top:{pointer.y}px; display:flex; \
                            align-items:flex-start; gap:4px;",
                    div { style: "width:10px; height:10px; margin:-5px 0 0 -5px; border-radius:5px; \
                                  background:{css_color(pointer.color, 1.0)}; border:2px solid #fff;" }
                    div { style: "padding:1px 6px; border-radius:4px; font-size:11px; line-height:16px; \
                                  white-space:nowrap; color:#fff; background:{css_color(pointer.color, 1.0)};",
                        "{pointer.name}"
                    }
                }
            }
        }
    }
}
