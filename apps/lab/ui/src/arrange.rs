//! The Vault tab's arrangement: a stand-in for Studio's (four lanes, time
//! across, scrolled and zoomed by its buttons) that publishes its
//! coordinate spaces, and a script — the marker lane
//! (`apps/lab/ui/src/marker_lane.luau`) — drawing over it with `Canvas`
//! and opening a tool window with `ImGui`, docked like any pane.

use architect::action::{ActionCaller, ActionList};
use architect_canvas::ui::{CanvasLayers, Canvases, style_from_theme, use_script_canvas};
use architect_canvas::{Lane, Plane, Spaces, TimeAxis};
use architect_scripting::luau::Runtime;
use architect_shell::use_shell;
use dioxus::prelude::*;

/// The arrangement's kind: what the marker lane draws over.
pub const ARRANGE: &str = "lab.arrange";

const RULER: f32 = 24.0;
const LANE: f32 = 56.0;
const LANES: [(&str, &str); 4] = [
    ("drums", "Drums"),
    ("bass", "Bass"),
    ("keys", "Keys"),
    ("vocals", "Vocals"),
];

/// Starts the marker lane in the [`Canvases`] in context.
pub fn use_marker_lane() {
    let theme = use_shell().theme;
    use_script_canvas(
        Runtime::new(ActionList::new()),
        ActionCaller::app(),
        include_str!("marker_lane.luau").to_owned(),
        serde_json::Value::Null,
        "Marker lane".to_owned(),
        style_from_theme(&theme),
    );
}

/// The arrangement view.
#[component]
pub fn ArrangeView() -> Element {
    let t = use_shell().theme;
    let mut start = use_signal(|| 0.0_f64);
    let mut zoom = use_signal(|| 16.0_f64);
    let mut size = use_signal(|| (0.0_f32, 0.0_f32));
    let canvases = try_use_context::<Canvases>();
    // Its spaces, each time they change: the layers redraw then.
    use_effect(move || {
        let (w, h) = size();
        let spaces = (w > 0.0).then(|| {
            let mut s = Spaces::sized(w, h).with_time(TimeAxis {
                start: start(),
                pixels_per_second: zoom(),
            });
            for (i, (id, name)) in LANES.iter().enumerate() {
                s = s.with_lane(Lane::new(*id, *name, lane_top(i), LANE));
            }
            s
        });
        if let Some(c) = canvases {
            c.publish(ARRANGE, spaces.as_ref());
        }
    });
    use_drop(move || {
        if let Some(c) = canvases {
            c.publish(ARRANGE, None);
        }
    });
    let mut mounted = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let measure = move |m: std::rc::Rc<MountedData>| async move {
        if let Ok(r) = m.get_client_rect().await {
            size.set((narrow(r.size.width), narrow(r.size.height)));
        }
    };
    let (from, scale) = (start(), zoom());
    let seconds = (0..16)
        .map(|n| f64::from(n) * 4.0)
        .map(move |s| (s, narrow((s - from) * scale)))
        .filter(|(_, x)| *x >= 0.0);
    let button = format!(
        "min-width: 28px; height: 24px; border-radius: 6px; border: 1px solid {}; background: {}; color: {}; cursor: pointer;",
        t.rule, t.pill, t.ink
    );
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            div { style: "display: flex; gap: 6px; padding: 6px 8px; border-bottom: 1px solid {t.rule}; align-items: center;",
                button { style: "{button}", onclick: move |_| start.set((start() - 4.0).max(0.0)), "‹" }
                button { style: "{button}", onclick: move |_| start.set(start() + 4.0), "›" }
                button { style: "{button}", onclick: move |_| zoom.set((zoom() / 1.5).max(4.0)), "−" }
                button { style: "{button}", onclick: move |_| zoom.set((zoom() * 1.5).min(120.0)), "+" }
                span { style: "font-size: 12px; color: {t.ink_3};", "from {start():.0} s, {zoom():.0} pt/s" }
            }
            div {
                style: "flex: 1; position: relative; overflow: hidden; background: {t.desk};",
                onmounted: move |e| {
                    mounted.set(Some(e.data()));
                    spawn(measure(e.data()));
                },
                // A pane resized since: measured again when the pointer
                // comes back.
                onpointerenter: move |_| {
                    if let Some(m) = mounted() {
                        spawn(measure(m));
                    }
                },
                for (s, x) in seconds {
                    div {
                        key: "tick-{s}",
                        style: "position: absolute; left: {x}px; top: 0; bottom: 0; width: 1px; background: {t.rule};",
                        span { style: "position: absolute; left: 4px; top: 4px; font-size: 11px; color: {t.ink_3};",
                            "{s:.0}"
                        }
                    }
                }
                for (i, (id, name)) in LANES.iter().enumerate() {
                    div {
                        key: "{id}",
                        style: "position: absolute; left: 0; right: 0; top: {lane_top(i)}px; height: {LANE - 1.0}px; \
                                border-bottom: 1px solid {t.rule}; background: rgba(255,255,255,0.02);",
                        span { style: "position: absolute; left: 8px; bottom: 6px; font-size: 12px; color: {t.ink_2};", "{name}" }
                    }
                }
                // Each script's layers over the arrangement's items.
                CanvasLayers { view: ARRANGE, plane: Plane::Over }
            }
        }
    }
}

/// Lane `i`'s top: under the ruler, one lane height each.
fn lane_top(i: usize) -> f32 {
    let i = u16::try_from(i).unwrap_or(u16::MAX);
    f32::from(i).mul_add(LANE, RULER)
}

/// A screen length as points (window sizes fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}
