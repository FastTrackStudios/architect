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
//!   its knob bound to the lab's drive and its meter following it
//!   (`LAB_FRAME_SET`, [`crate::extensions`]); a save to the document
//!   reloads it. Without the program a stand-in says how to get it and
//!   follows the same action;
//! - **A2UI**: the agents' surfaces (`A2UI_RENDER`, `A2UI_PATCH`), the
//!   release demo first (`LAB_A2UI_DEMO`, `release.a2ui.json`): a form, a
//!   list of tasks, and a button whose write waits for the person's yes;
//! - **Paint**: the built-in `paint` kind, 10 000 rectangles drawn
//!   in-process by Blitz each frame, under the Frame surface
//!   ([`crate::paint`]).
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
    Capability, KindSpec, Node, PAINT, Pane, Placement, Side, Target, View, ViewHost, ViewKinds,
    Workspace, use_pane,
};
use dioxus::prelude::*;
use facet::Facet;

/// The Frame surface's kind (Frame's `frame_view::KIND`).
pub const FRAME_SURFACE: &str = "frame.surface";

/// The kinds the lab draws with: the built-in `dioxus`, `paint` (in
/// process, on the window's renderer), and Frame's surface, shown through
/// its extension's process.
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
    let _ = kinds.register(KindSpec::paint());
    kinds
}

/// The panes' ids.
pub const DIOXUS_PANE: &str = "embed-dioxus";
pub const LUAU_PANE: &str = "embed-luau";
pub const FRAME_PANE: &str = "embed-frame";
pub const A2UI_PANE: &str = "embed-a2ui";
pub const PAINT_PANE: &str = "embed-paint";

/// The views of the five.
#[must_use]
pub fn views() -> [View; 5] {
    [
        View::new("embed.dioxus", "Dioxus", |_| rsx! { DioxusPane {} }),
        View::new("embed.luau", "Luau ImGui", |_| rsx! { LuauPane {} }).still(),
        View::new("embed.frame", "Frame surface", |_| rsx! { FramePane {} })
            .drawn_by(FRAME_SURFACE)
            .still(),
        View::new("embed.a2ui", "A2UI", |_| rsx! { A2uiPane {} }),
        View::new(
            "embed.paint",
            "Paint",
            |_| rsx! { crate::paint::PaintLabPane {} },
        )
        .drawn_by(PAINT),
    ]
}

/// The screenset's layout: three columns, Dioxus over Luau `ImGui` on the
/// left, the Frame surface over Paint in the middle, A2UI on the right.
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
        Target::Split(Side::Right),
    );
    // The surface asks for the most room, the Frame surface the least.
    let columns = w.windows.first().and_then(|main| match &main.center {
        Some(Node::Split(s)) if s.children.len() == 3 => Some(s.id),
        _ => None,
    });
    if let Some(columns) = columns {
        let _ = w.set_shares(columns, &[0.34, 0.29, 0.37]);
    }
    let _ = w.activate(&architect_workspace::PaneId::new(FRAME_PANE));
    let _ = w.open(
        Pane::new(PAINT_PANE, "embed.paint", "Paint"),
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
pub(crate) fn KindHead(name: &'static str, kind: &'static str, about: &'static str) -> Element {
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
/// the lab's drive and the level its meter follows it to, drawn plainly.
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
    let (drive, db) = now();
    let drive_pc = drive * 100.0;
    let level_pc = (db + 60.0) / 66.0 * 100.0;
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
                        div { style: "width: {drive_pc}%; height: 8px; border-radius: 4px; background: #ff7a1a;" }
                    }
                    div { style: "font-size: 13px; color: {t.ink};", {format!("{drive_pc:.0}%")} }
                }
                div { style: "display: flex; flex-direction: column; gap: 6px;",
                    div { style: "font-size: 12px; color: {t.ink_3};", "Level" }
                    div { style: "position: relative; width: 10px; height: 80px; border-radius: 3px; background: {t.pill}; overflow: hidden;",
                        div { style: "position: absolute; left: 0; right: 0; bottom: 0; height: {level_pc}%; background: #3dd67a;" }
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

/// What `LAB_RELEASE_SHIP` takes: the release surface's form and tasks.
#[derive(Debug, Clone, Facet)]
struct ShipArgs {
    version: String,
    /// A `ChoicePicker`'s value: a list of one.
    channel: Vec<String>,
    tasks: Vec<ShipTask>,
}

/// One of a release's tasks.
#[derive(Debug, Clone, Facet)]
struct ShipTask {
    title: String,
    done: bool,
}

/// The demo surface's id, and who renders it: an agent, so that its
/// button's write waits for the person's yes.
pub const RELEASE: &str = "release";
const DEMO_AGENT: &str = "demo-agent";

static A2UI_DEMO: ActionMeta = ActionMeta {
    id: "LAB_A2UI_DEMO",
    trait_name: "",
    method_name: "",
    display_name: "Show the release surface",
    description: "Renders the A2UI demo surface “release” in the Embedded views' A2UI pane, as the agent demo-agent: a form (version, channel), a list of tasks to tick, and a Ship it button whose LAB_RELEASE_SHIP waits for the person's yes. Patch it live with A2UI_PATCH (surfaceId release).",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static RELEASE_SHIP: ActionMeta = ActionMeta {
    id: "LAB_RELEASE_SHIP",
    trait_name: "",
    method_name: "",
    display_name: "Ship the release",
    description: "Ships a release (the demo's: nothing leaves the lab): version, channel (beta or stable, or a list of one), tasks ({title, done}). Answers {summary, version, channel, open}, the tasks not done. The release surface's Ship it button runs it.",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `LAB_RELEASE_SHIP` answers `args` with: a line that says what
/// shipped, and how many of its tasks were still open.
fn shipped(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let version = args
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            format!(
                "`version` is the release's version, not {}",
                args.get("version").unwrap_or(&serde_json::Value::Null)
            )
        })?;
    // A ChoicePicker's value is a list.
    let channel = match args.get("channel") {
        Some(serde_json::Value::Array(vs)) => vs.first().and_then(serde_json::Value::as_str),
        Some(v) => v.as_str(),
        None => None,
    }
    .ok_or("`channel` is beta or stable")?;
    if !matches!(channel, "beta" | "stable") {
        return Err(format!("`channel` is beta or stable, not {channel}"));
    }
    let open: Vec<&str> = args
        .get("tasks")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t["done"] != true)
        .filter_map(|t| t["title"].as_str())
        .collect();
    let summary = match open.len() {
        0 => format!("v{version} shipped to {channel}."),
        1 => format!("v{version} shipped to {channel}; 1 task still open."),
        n => format!("v{version} shipped to {channel}; {n} tasks still open."),
    };
    Ok(
        serde_json::json!({ "summary": summary, "version": version, "channel": channel, "open": open }),
    )
}

/// Puts on `list` `LAB_A2UI_DATA`, a surface's data model, which its
/// components write as they are used; `LAB_A2UI_DEMO`, which renders the
/// release surface; `LAB_RELEASE_SHIP`, its button's action; and
/// `LAB_PAINT_STATE`, the paint pane's.
pub fn register(list: &ActionList, surfaces: &architect_a2ui::Surfaces) {
    crate::paint::register(list);
    let failed = |id: &str, message: String| ActionError::Failed {
        id: id.to_owned(),
        message,
    };
    let demo = surfaces.clone();
    let _ = list.register_dynamic(
        &A2UI_DEMO,
        Effect::Mutating,
        None,
        None,
        Arc::new(move |_: &[u8]| {
            let fail = |e: architect_a2ui::store::SurfaceError| failed(A2UI_DEMO.id, e.to_string());
            let mut messages = Vec::new();
            if demo.get(RELEASE).is_some() {
                messages.push(
                    architect_a2ui::store::message(
                        serde_json::json!({ "deleteSurface": { "surfaceId": RELEASE } }),
                    )
                    .map_err(fail)?,
                );
            }
            messages.extend(
                architect_a2ui::store::parse(include_str!("release.a2ui.json")).map_err(fail)?,
            );
            let agent = architect::action::Principal::Service {
                name: DEMO_AGENT.to_owned(),
            };
            demo.apply(&messages, Some(&agent)).map_err(fail)?;
            Ok(
                serde_json::to_vec(&serde_json::json!({ "surfaceId": RELEASE }))
                    .unwrap_or_default(),
            )
        }),
    );
    let _ = list.register_dynamic(
        &RELEASE_SHIP,
        Effect::Mutating,
        Some(ShipArgs::SHAPE),
        None,
        Arc::new(move |args: &[u8]| {
            let args: serde_json::Value =
                serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                    id: RELEASE_SHIP.id.to_owned(),
                    message: e.to_string(),
                    expected: None,
                })?;
            let out = shipped(&args).map_err(|e| failed(RELEASE_SHIP.id, e))?;
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        }),
    );
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

/// The A2UI pane: every surface an agent rendered (`A2UI_RENDER`); the
/// release surface (`LAB_A2UI_DEMO`) when it first shows.
#[component]
fn A2uiPane() -> Element {
    let list = use_context::<Arc<ActionList>>();
    use_hook(move || {
        let shown = format!(r#"{{"surfaceId":"{RELEASE}"}}"#);
        if list.run_json(A2UI_DATA.id, shown.as_bytes()).is_err() {
            let _ = list.run_json(A2UI_DEMO.id, b"{}");
        }
    });
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            KindHead {
                name: "A2UI",
                kind: "a2ui",
                about: "An agent's UI from the catalog: rendered with A2UI_RENDER, patched live with A2UI_PATCH. Its writes wait for your yes.",
            }
            architect_a2ui::ui::SurfacesPane {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ship_button_ships_what_the_form_says() {
        let surfaces =
            architect_a2ui::Surfaces::new(crdt::CrdtDoc::ephemeral(), architect_a2ui::store::ROOT);
        let list = ActionList::new();
        register(&list, &surfaces);
        // Shown when the pane opens, and again by the guide: the second
        // replaces the first.
        for _ in 0..2 {
            assert!(list.run_json(A2UI_DEMO.id, b"{}").is_ok());
        }
        let release = surfaces.get(RELEASE);
        let pressed = release
            .as_ref()
            .map(|s| architect_a2ui::events::resolve(s, "ship", ""));
        let Some(Ok((id, args, respond))) = pressed else {
            panic!("the Ship it button has no action: {pressed:?}");
        };
        assert_eq!(id, RELEASE_SHIP.id);
        assert_eq!(respond.as_deref(), Some("/shipped"));
        let out = list.run_json(&id, args.to_string().as_bytes());
        let out: Option<serde_json::Value> = out.ok().and_then(|o| serde_json::from_slice(&o).ok());
        assert_eq!(
            out.map(|o| o["summary"].clone()),
            Some(serde_json::json!(
                "v0.11.0 shipped to beta; 1 task still open."
            )),
            "{args}"
        );
    }
}
