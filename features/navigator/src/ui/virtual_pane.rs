//! A virtualized pane: its own scroll (wheel, a finger's drag, a drawn
//! scrollbar), its view measured while mounted. Only the rows in the
//! state's window are children; each is placed at its offset less the
//! scroll, so a list of 100,000 draws a screenful.
//!
//! The scroll is the state's, not the box's (`overflow: hidden`): Blitz
//! does not report a box's scroll offset to the page, and a box that keeps
//! its offset when its rows change would show empty space
//! (`skills/architect/references/blitz-quirks.md`).

use std::rc::Rc;
use std::time::Duration;

use dioxus::html::geometry::WheelDelta;
use dioxus::prelude::*;

use super::Navigator;
use crate::layout::Pane;

/// A wheel's notch, in points.
const LINE: f64 = 40.0;
/// How far a finger moves before a press is a drag, not a tap.
const SLOP: f64 = 6.0;
/// The scrollbar's thumb is never shorter.
const THUMB_MIN: f64 = 24.0;

#[component]
pub(super) fn VirtualPane(
    nav: Navigator,
    pane: Pane,
    label: &'static str,
    children: Element,
) -> Element {
    let mut mounted: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    // Measured again while mounted: Blitz fires no resize.
    use_future(move || async move {
        loop {
            if let Some(m) = mounted.peek().clone()
                && let Ok(r) = m.get_client_rect().await
            {
                let h = r.size.height;
                if pane == Pane::Navigation {
                    let at = Some((r.origin.x, r.origin.y, r.size.width, h));
                    if *nav.nav_box.peek() != at {
                        let mut nav_box = nav.nav_box;
                        nav_box.set(at);
                    }
                }
                // In the layout's lengths, the zoom taken off.
                let h = h / nav.zoom();
                if h > 0.0 && (nav.state.peek().scroll(pane).view - h).abs() > 0.5 {
                    let mut state = nav.state;
                    state.write().set_view(pane, h);
                }
            }
            futures_timer::Delay::new(Duration::from_millis(250)).await;
        }
    });
    let mut press: Signal<Option<(f64, f64)>> = use_signal(|| None);
    let mut thumb_drag: Signal<Option<(f64, f64)>> = use_signal(|| None);
    let (scroll, total) = {
        let s = nav.state.read();
        let total = match pane {
            Pane::Navigation => s.nav_offsets().total(),
            Pane::Files => s.list_offsets().total(),
        };
        (s.scroll(pane), total)
    };
    let view = scroll.view;
    let scrollable = view > 0.0 && total > view + 0.5;
    let thumb_h = if scrollable {
        (view * view / total).max(THUMB_MIN)
    } else {
        0.0
    };
    let track = (view - thumb_h).max(1.0);
    let most = (total - view).max(1.0);
    let thumb_top = scroll.offset / most * track;
    let name = match pane {
        Pane::Navigation => "navigation",
        Pane::Files => "files",
    };
    rsx! {
        div {
            class: "nv-scroller",
            "data-scroller": "{name}",
            "aria-label": "{label}",
            tabindex: "0",
            style: "position:absolute; inset:0; overflow:hidden; outline:none; touch-action:none;",
            onmounted: move |e| mounted.set(Some(e.data())),
            onwheel: move |e| {
                // Signed as Blitz scrolls a box: a wheel turned down gives
                // a negative delta, and should bring later rows up.
                let dy = -match e.data().delta() {
                    WheelDelta::Pixels(d) => d.y,
                    WheelDelta::Lines(d) => d.y * LINE,
                    WheelDelta::Pages(d) => d.y * view,
                };
                if dy != 0.0 {
                    e.prevent_default();
                    let mut state = nav.state;
                    state.write().scroll_by(pane, dy / nav.zoom());
                }
            },
            // The pane pressed has the keyboard; a finger drags it.
            onpointerdown: move |e: PointerEvent| {
                let mut state = nav.state;
                if state.peek().focus != pane {
                    state.write().focus = pane;
                }
                nav.dragged.set(false);
                if e.data().pointer_type() != "mouse" {
                    press.set(Some((e.data().client_coordinates().y, nav.state.peek().scroll(pane).offset)));
                }
            },
            onpointermove: move |e: PointerEvent| {
                let Some((y0, from)) = *press.peek() else { return };
                let dy = (e.data().client_coordinates().y - y0) / nav.zoom();
                if nav.dragged.cloned() || dy.abs() > SLOP {
                    nav.dragged.set(true);
                    let mut state = nav.state;
                    state.write().scroll_to(pane, from - dy);
                }
            },
            onpointerup: move |_| press.set(None),
            {children}
            if scrollable {
                div {
                    class: "nv-thumb",
                    style: "position:absolute; right:2px; width:6px; border-radius:3px; top:{thumb_top}px; height:{thumb_h}px; z-index:2; touch-action:none;",
                    onpointerdown: move |e: PointerEvent| {
                        e.stop_propagation();
                        thumb_drag.set(Some((e.data().client_coordinates().y, nav.state.peek().scroll(pane).offset)));
                    },
                    onpointermove: move |e: PointerEvent| {
                        let Some((y0, from)) = *thumb_drag.peek() else { return };
                        let dy = (e.data().client_coordinates().y - y0) / nav.zoom();
                        let mut state = nav.state;
                        state.write().scroll_to(pane, from + dy * most / track);
                    },
                    onpointerup: move |_| thumb_drag.set(None),
                }
            }
        }
    }
}
