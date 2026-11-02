//! The Vault's "Embedded views" screenset (⌘3, or `WORKSPACES_LOAD {name
//! "Embedded views"}`): one pane of each kind of view the lab shows, each
//! naming its kind —
//!
//! - **Dioxus**: a component in the lab's own document;
//! - **Luau `ImGui`**: a script of its own (`imgui_demo.luau`), its window
//!   drawn in the pane;
//! - **Frame surface**: a knob and a meter from a Frame document, laid out
//!   and painted by Frame's `frame-surface` program
//!   (FastTrackStudios/frame, `crates/frame-view`) in a process of its own,
//!   moved by `LAB_FRAME_SET` ([`crate::extensions`]). Without the program
//!   a stand-in says how to get it and follows the same action;
//! - **A2UI**: the agents' surfaces (`A2UI_RENDER`).
//!
//! The lab never links Frame: it declares the `frame.surface` kind itself
//! ([`kinds`]), as `frame_view::spec()` does, and shows the extension's
//! view. Scenario: `apps/lab/scenarios/embedded-views.styx`.

use std::sync::Arc;

use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect};
use architect_canvas::ui::{
    WindowEvent, WindowView, style_from_theme, use_canvases, use_script_canvas,
};
use architect_scripting::luau::Runtime;
use architect_shell::use_shell;
use architect_workspace::{
    Capability, KindSpec, Pane, Placement, Side, Target, View, ViewHost, ViewKinds, Workspace,
    use_pane,
};
use dioxus::prelude::*;
use facet::Facet;

/// The Frame surface's kind (Frame's `frame_view::KIND`).
pub const FRAME_SURFACE: &str = "frame.surface";

/// The kinds the lab draws with: the built-in `dioxus`, and Frame's
/// surface, shown through its extension's process.
#[must_use]
pub fn kinds() -> ViewKinds {
    let mut kinds = ViewKinds::new();
    // As `frame_view::spec()` declares it.
    let _ = kinds.register(
        KindSpec::new(FRAME_SURFACE, "frame-view")
            .host(ViewHost::Blitz)
            .placement(Placement::Pane)
            .placement(Placement::Embed)
            .capability(Capability::Input)
            .capability(Capability::Actions),
    );
    kinds
}

/// The four panes' ids.
pub const DIOXUS_PANE: &str = "embed-dioxus";
pub const LUAU_PANE: &str = "embed-luau";
pub const FRAME_PANE: &str = "embed-frame";
pub const A2UI_PANE: &str = "embed-a2ui";

/// The views of the four.
#[must_use]
pub fn views() -> [View; 4] {
    [
        View::new("embed.dioxus", "Dioxus", |_| rsx! { DioxusPane {} }),
        View::new("embed.luau", "Luau ImGui", |_| rsx! { LuauPane {} }).still(),
        View::new("embed.frame", "Frame surface", |_| rsx! { FramePane {} })
            .drawn_by(FRAME_SURFACE)
            .still(),
        View::new("embed.a2ui", "A2UI", |_| rsx! { A2uiPane {} }),
    ]
}

/// The screenset's layout: two columns, Dioxus over Luau `ImGui` on the
/// left, the Frame surface over A2UI on the right.
#[must_use]
pub fn workspace() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(
        Pane::new(DIOXUS_PANE, "embed.dioxus", "Dioxus"),
        Target::Tab,
    );
    let _ = w.open(
        Pane::new(FRAME_PANE, "embed.frame", "Frame surface"),
        Target::Split(Side::Right),
    );
    let _ = w.open(
        Pane::new(A2UI_PANE, "embed.a2ui", "A2UI"),
        Target::Split(Side::Bottom),
    );
    let _ = w.activate(&architect_workspace::PaneId::new(DIOXUS_PANE));
    let _ = w.open(
        Pane::new(LUAU_PANE, "embed.luau", "Luau ImGui"),
        Target::Split(Side::Bottom),
    );
    let _ = w.activate(&architect_workspace::PaneId::new(DIOXUS_PANE));
    w
}

/// A pane's head: its kind's name, and the kind's id as the lab knows it.
#[component]
fn KindHead(name: &'static str, kind: &'static str, about: &'static str) -> Element {
    let t = use_shell().theme;
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 4px; padding: 16px 20px 8px;",
            div { style: "display: flex; align-items: baseline; gap: 8px;",
                div { style: "font-size: 15px; font-weight: 650; color: {t.ink};", "{name}" }
                div { style: "font-size: 12px; color: {t.ink_3}; font-family: monospace;", "{kind}" }
            }
            div { style: "max-width: 460px; font-size: 13px; line-height: 1.5; color: {t.ink_2};", "{about}" }
        }
    }
}

/// The swatches the Dioxus pane picks from.
const SWATCHES: [(&str, &str); 3] = [
    ("Amber", "#f59e0b"),
    ("Teal", "#14b8a6"),
    ("Violet", "#8b5cf6"),
];

/// The Dioxus pane: pick a swatch, and the pane keeps it (its state, which
/// `WORKSPACE_PANE_GET` reads).
#[component]
fn DioxusPane() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let picked = pane.state();
    let color = SWATCHES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&picked))
        .map_or(t.pill, |(_, c)| *c);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0; overflow-y: scroll;",
            KindHead {
                name: "Dioxus",
                kind: "dioxus",
                about: "A component in the lab's own document: its buttons are elements, its state the pane's.",
            }
            div { style: "display: flex; flex-direction: column; gap: 12px; padding: 4px 20px 20px;",
                div { style: "width: 160px; height: 56px; border-radius: 10px; border: 1px solid {t.rule}; background: {color};" }
                div { style: "display: flex; gap: 8px;",
                    for (name, _) in SWATCHES {
                        button {
                            key: "{name}",
                            class: "lp",
                            "aria-pressed": if picked.eq_ignore_ascii_case(name) { "true" } else { "false" },
                            style: "min-height: 30px; padding: 0 14px; border-radius: 8px; border: 1px solid {t.rule}; \
                                    background: {t.pill}; color: {t.ink}; font-size: 13px; cursor: pointer;",
                            onclick: {
                                let pane = pane.clone();
                                move |_| pane.set_state(name.to_lowercase())
                            },
                            "{name}"
                        }
                    }
                }
                div { style: "font-size: 12px; color: {t.ink_3};",
                    if picked.is_empty() { "Nothing picked yet." } else { "Picked: {picked}" }
                }
            }
        }
    }
}

/// The demo script's one window (`imgui_demo.luau`'s context and name).
const DEMO_WINDOW: &str = "imgui-demo/Window";

/// The Luau `ImGui` pane: the demo script runs while the pane shows, its
/// window drawn here. Its canvases are the pane's own, so the vault's tool
/// windows never float it.
#[component]
fn LuauPane() -> Element {
    let t = use_shell().theme;
    let _canvases = use_canvases();
    let canvas = use_script_canvas(
        Runtime::new(ActionList::new()),
        ActionCaller::app(),
        include_str!("imgui_demo.luau").to_owned(),
        serde_json::Value::Null,
        "Luau ImGui".to_owned(),
        style_from_theme(&t),
    );
    let window = canvas.output().window(DEMO_WINDOW).cloned();
    let failed = canvas.status().error.map(|e| e.to_string());
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            KindHead {
                name: "Luau ImGui",
                kind: "luau · imgui",
                about: "A Luau script's immediate-mode window: it redraws every frame, the lab paints its shapes.",
            }
            div { style: "flex: 1; position: relative; min-height: 0; margin: 4px 20px 20px; \
                          border: 1px solid {t.rule}; border-radius: 8px; overflow: hidden;",
                match (window, failed) {
                    (Some(window), _) => rsx! {
                        WindowView {
                            window,
                            on_event: move |e: WindowEvent| canvas.window_event(DEMO_WINDOW, e),
                        }
                    },
                    (None, Some(e)) => rsx! {
                        div { style: "padding: 12px; font-size: 12px; color: {t.ink_2};", "The script stopped: {e}" }
                    },
                    (None, None) => rsx! {
                        div { style: "padding: 12px; font-size: 12px; color: {t.ink_3};", "Starting the script…" }
                    },
                }
            }
        }
    }
}

/// The Frame pane: the extension's view, or a stand-in without its
/// program.
#[component]
fn FramePane() -> Element {
    if crate::extensions::frame_surface().is_some() {
        rsx! {
            architect_extensions::remote::RemoteView {
                processes: crate::extensions::processes(),
                extension: crate::extensions::FRAME,
                view: "surface",
            }
        }
    } else {
        rsx! { FrameStandIn {} }
    }
}

/// How often the stand-in looks at what `LAB_FRAME_SET` left.
const STAND_IN_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// What the Frame pane shows without `frame-surface`: where to get it, and
/// the knob and the meter `LAB_FRAME_SET` moves, drawn plainly.
#[component]
fn FrameStandIn() -> Element {
    let t = use_shell().theme;
    let mut now = use_signal(crate::extensions::frame_stand_in);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(STAND_IN_POLL).await;
            let s = crate::extensions::frame_stand_in();
            if *now.peek() != s {
                now.set(s);
            }
        }
    });
    let (knob, meter) = now();
    let db = meter.mul_add(66.0, -60.0);
    let (knob_pc, meter_pc) = (knob * 100.0, meter * 100.0);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0; overflow-y: scroll;",
            KindHead {
                name: "Frame surface",
                kind: "frame.surface",
                about: "Drawn by Frame's frame-surface program, which is not here: build it in FastTrackStudios/frame \
                        (cargo build --bin frame-surface) and set ARCHITECT_LAB_FRAME_SURFACE to it. Until then, a stand-in.",
            }
            div { style: "display: flex; gap: 20px; align-items: flex-end; padding: 4px 20px 20px;",
                div { style: "display: flex; flex-direction: column; gap: 6px; width: 120px;",
                    div { style: "font-size: 12px; color: {t.ink_3};", "Drive" }
                    div { style: "height: 8px; border-radius: 4px; background: {t.pill};",
                        div { style: "width: {knob_pc}%; height: 8px; border-radius: 4px; background: #ff7a1a;" }
                    }
                    div { style: "font-size: 13px; color: {t.ink};", {format!("{knob_pc:.0}%")} }
                }
                div { style: "display: flex; flex-direction: column; gap: 6px;",
                    div { style: "position: relative; width: 10px; height: 80px; border-radius: 3px; background: {t.pill}; overflow: hidden;",
                        div { style: "position: absolute; left: 0; right: 0; bottom: 0; height: {meter_pc}%; background: #3dd67a;" }
                    }
                    div { style: "font-size: 13px; color: {t.ink};", {format!("{db:.1} dB")} }
                }
            }
        }
    }
}

static A2UI_DATA: ActionMeta = ActionMeta {
    id: "LAB_A2UI_DATA",
    trait_name: "",
    method_name: "",
    display_name: "An agent surface's data",
    description: "One A2UI surface's data model (surfaceId), read at once: what a scenario's check reads, as A2UI_SURFACE answers in time.",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `LAB_A2UI_DATA` takes: the surface.
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
struct SurfaceArgs {
    surface_id: String,
}

/// Puts `LAB_A2UI_DATA` on `list`: a surface's data model, which its
/// components write as they are used.
pub fn register(list: &ActionList, surfaces: &architect_a2ui::Surfaces) {
    let surfaces = surfaces.clone();
    let bad = |message: String| ActionError::BadArgs {
        id: A2UI_DATA.id.to_owned(),
        message,
        expected: None,
    };
    let _ = list.register_dynamic(
        &A2UI_DATA,
        Effect::ReadOnly,
        Some(SurfaceArgs::SHAPE),
        None,
        Arc::new(move |args: &[u8]| {
            let args: serde_json::Value =
                serde_json::from_slice(args).map_err(|e| bad(e.to_string()))?;
            let id = args
                .get("surfaceId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| bad("surfaceId is required".to_owned()))?;
            let surface = surfaces.get(id).ok_or_else(|| ActionError::Failed {
                id: A2UI_DATA.id.to_owned(),
                message: format!("no surface “{id}”"),
            })?;
            let data = surface
                .to_json()
                .get("dataModel")
                .cloned()
                .unwrap_or_default();
            Ok(serde_json::to_vec(&data).unwrap_or_default())
        }),
    );
}

/// The A2UI pane: every surface an agent rendered (`A2UI_RENDER`).
#[component]
fn A2uiPane() -> Element {
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            KindHead {
                name: "A2UI",
                kind: "a2ui",
                about: "An agent's generated UI: components from the catalog, rendered with A2UI_RENDER, drawn by architect-ui.",
            }
            architect_a2ui::ui::SurfacesPane {}
        }
    }
}
