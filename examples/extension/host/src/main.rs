//! The host, with a window (Blitz, as the apps are painted).
//!
//! ```text
//! cargo build -p example-extension-counter   # the process extension's program
//! cargo run -p example-extension-host
//! ```
//!
//! It finds the example's registry (`examples/extension/registry`, above
//! the working directory, or at `EXAMPLE_EXTENSION_REGISTRY`), and runs
//! the counter as a development copy when its program sits beside this
//! one. The header installs and removes Hello from the registry: watch
//! its settings section and its ruler come and go. Edit
//! `registry/hello/ruler.luau` while it runs and the ruler redraws as
//! saved; `kill -9` the counter's process and its pane says so, then
//! shows it again.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::path::PathBuf;
use std::time::Duration;

use architect::action::{ActionCaller, ActionList};
use architect_canvas::imgui::Style;
use architect_canvas::ui::{
    CanvasLayers, Canvases, WindowView, measure, use_canvases, use_script_canvas,
};
use architect_canvas::{Lane, Plane, Spaces, TimeAxis};
use architect_extensions::VersionReq;
use architect_extensions::extensions::Extensions;
use architect_extensions::install::FolderRegistry;
use architect_extensions::process::Processes;
use architect_extensions::remote::RemoteView;
use architect_extensions::ui::use_extension_sections;
use architect_settings::{Registry, ScopedValues, SettingsPage, use_settings_provider};
use dioxus::prelude::*;
use example_extension_host::{
    COUNTER, COUNTER_PROGRAM, HELLO, Overlay, TIMELINE, counter_package, extensions, find_registry,
    overlays,
};

/// How often loaded extensions' files are checked for a save.
const POLL: Duration = Duration::from_millis(500);

/// The timeline's scale, and its lanes: name, top, height.
const PIXELS_PER_SECOND: f64 = 40.0;
const LANES: [(&str, f32, f32); 2] = [("Lane A", 24.0, 48.0), ("Lane B", 72.0, 48.0)];

/// The app's state, in context: its extensions, and a count that moves
/// each time one loads, reloads or goes (what reads it draws again).
#[derive(Clone)]
struct App {
    extensions: Extensions,
    processes: Processes,
    changed: Signal<u64>,
}

impl App {
    fn bump(&self) {
        let mut changed = self.changed;
        let next = changed.peek().wrapping_add(1);
        changed.set(next);
    }
}

/// Where the host keeps installs and development copies.
fn data_dir() -> PathBuf {
    std::env::temp_dir().join("architect-extension-example")
}

fn registry() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("EXAMPLE_EXTENSION_REGISTRY") {
        return Some(dir.into());
    }
    find_registry(&std::env::current_dir().ok()?)
}

/// The counter as a development copy, when its program was built beside
/// this one.
fn counter() -> Option<PathBuf> {
    let program = std::env::current_exe()
        .ok()?
        .with_file_name(COUNTER_PROGRAM);
    if !program.is_file() {
        return None;
    }
    counter_package(&program, &data_dir().join("dev").join(COUNTER)).ok()
}

#[component]
fn Root() -> Element {
    let settings = use_settings_provider(Registry::new, ScopedValues::default, None);
    let _canvases = use_canvases();
    // The settings page as the host sees it: extensions' sections go here.
    let sections = use_extension_sections();
    let changed = use_signal(|| 0_u64);
    let Some(registry) = use_hook(registry) else {
        return rsx! {
            p { "No registry: run from the repository, or set EXAMPLE_EXTENSION_REGISTRY." }
        };
    };
    let app = use_context_provider(|| {
        let processes = Processes::default();
        let extensions = extensions(
            ActionList::new(),
            sections.clone(),
            processes.clone(),
            data_dir().join("installed"),
            FolderRegistry::new(registry),
        );
        // Whatever was installed last time loads now.
        let _ = extensions.sync();
        if let Some(dir) = counter() {
            let _ = extensions.dev(dir);
        }
        App {
            extensions,
            processes,
            changed,
        }
    });
    // Hot reload: a save to a loaded extension's files reloads it.
    let polled = app.clone();
    use_future(move || {
        let app = polled.clone();
        async move {
            loop {
                futures_timer::Delay::new(POLL).await;
                if !app.extensions.reload_changed().is_empty() {
                    app.bump();
                }
            }
        }
    });
    let installed = {
        let _ = changed();
        app.extensions.host().get(HELLO).is_some()
    };
    let toggle = {
        let app = app.clone();
        move |_| {
            if installed {
                let _ = app.extensions.uninstall(HELLO);
            } else {
                let _ = app.extensions.install(HELLO, &VersionReq::STAR);
            }
            app.bump();
        }
    };
    let mut greeting = use_signal(String::new);
    let list = app.extensions.host().list().clone();
    let greet = move |_| {
        let list = list.clone();
        spawn(async move {
            let out = list
                .run_json_async("EXAMPLE_HELLO_GREET", br#"{"name":"host"}"#)
                .await;
            greeting.set(match out {
                Ok(json) => String::from_utf8_lossy(&json).into_owned(),
                Err(e) => e.to_string(),
            });
        });
    };
    let scripts = {
        let _ = changed();
        overlays(app.extensions.host(), TIMELINE)
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; height: 100vh; font-family: system-ui, sans-serif; background: #18181b; color: #e4e4e7;",
            div { style: "display: flex; gap: 8px; align-items: center; padding: 8px; border-bottom: 1px solid #3f3f46;",
                button { onclick: toggle, if installed { "Uninstall Hello" } else { "Install Hello" } }
                button { disabled: !installed, onclick: greet, "Run EXAMPLE_HELLO_GREET" }
                button { onclick: move |_| settings.show(None), "Settings" }
                span { "{greeting}" }
            }
            div { style: "flex: 1; display: flex; min-height: 0;",
                Timeline {}
                div { style: "width: 320px; display: flex; flex-direction: column; border-left: 1px solid #3f3f46;",
                    div { style: "flex: 1; position: relative;",
                        RemoteView { processes: app.processes, extension: COUNTER, view: "counter" }
                    }
                    ToolWindows {}
                }
            }
        }
        // Each overlay runs while its extension is loaded; a reload that
        // changes its script changes its key, which starts it again.
        for overlay in scripts {
            OverlayScript { key: "{overlay.key()}", overlay }
        }
        SettingsPage {}
    }
}

/// One overlay's script, running for as long as this is mounted.
#[component]
fn OverlayScript(overlay: Overlay) -> Element {
    let app = use_context::<App>();
    let runtime = overlay.runtime(app.extensions.host().list().clone());
    let _ = use_script_canvas(
        runtime,
        ActionCaller::app(),
        overlay.source,
        serde_json::Value::Null,
        overlay.manifest.name,
        Style::default(),
    );
    rsx! {}
}

/// The host's one view: time across, two lanes down. It publishes those
/// spaces to every script, and mounts what they draw over it.
#[component]
fn Timeline() -> Element {
    let canvases = use_context::<Canvases>();
    let mut size = use_signal(|| (0.0_f32, 0.0_f32));
    use_effect(move || {
        let (w, h) = size();
        let spaces = (w > 0.0).then(|| {
            LANES.iter().fold(
                Spaces::sized(w, h).with_time(TimeAxis {
                    start: 0.0,
                    pixels_per_second: PIXELS_PER_SECOND,
                }),
                |s, (name, top, height)| s.with_lane(Lane::new(*name, *name, *top, *height)),
            )
        });
        canvases.publish(TIMELINE, spaces.as_ref());
    });
    rsx! {
        div {
            style: "flex: 1; position: relative; overflow: hidden;",
            onmounted: move |e| async move {
                if let Some(s) = measure(&e.data()).await {
                    size.set((s.w, s.h));
                }
            },
            for (name, top, height) in LANES {
                div {
                    key: "{name}",
                    style: "position: absolute; left: 0; right: 0; top: {top}px; height: {height - 1.0}px; border-bottom: 1px solid #3f3f46;",
                    span { style: "position: absolute; left: 8px; bottom: 4px; font-size: 12px;", "{name}" }
                }
            }
            CanvasLayers { view: TIMELINE, plane: Plane::Over }
        }
    }
}

/// Every window the scripts draw, stacked (a real app docks them as
/// workspace panes: `architect_canvas::ui::use_tool_windows`).
#[component]
fn ToolWindows() -> Element {
    let canvases = use_context::<Canvases>();
    let windows: Vec<_> = canvases
        .all()
        .into_iter()
        .flat_map(|c| c.output().windows.into_iter().map(move |w| (c, w)))
        .collect();
    rsx! {
        for (canvas, window) in windows {
            div { key: "{window.id}", style: "height: 180px; position: relative; border-top: 1px solid #3f3f46;",
                WindowView {
                    window: window.clone(),
                    on_event: move |e| canvas.window_event(&window.id, e),
                }
            }
        }
    }
}

fn main() {
    dioxus_native::launch(Root);
}
