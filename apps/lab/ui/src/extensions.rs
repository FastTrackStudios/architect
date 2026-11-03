//! The lab's extensions: the metronome, a real one
//! (`apps/lab/extensions/metronome`), installed from the lab's registry
//! (`apps/lab/extensions`, found above the working directory, or at
//! `ARCHITECT_LAB_EXTENSIONS`) when the Workspace page's switch turns it
//! on, and turned off when it goes off. Loaded, its section is on the
//! settings page and its action in ⌘K; a save to its files reloads it.
//! With no registry to be found (a phone), the copy linked into the build
//! runs.
//!
//! The clicker (`apps/lab/clicker`) is a process extension: when its
//! program was built beside the lab's, it runs as a development copy and
//! its view is the Vault's Clicker pane ([`processes`]). It is the lab's
//! themed extension: its theme (Clicker Neon) and its `clicker.*` tokens
//! go into the lab's theme store while it is loaded (`Host::with_themes`),
//! its process is sent the theme as data (`ProcessRunner::with_theme`),
//! and its ImGui window ([`ClickerWindow`], `window.luau` from its
//! package) reads `theme`.
//!
//! So is Frame's `frame-surface` (FastTrackStudios/frame,
//! `crates/frame-view`), when it is found ([`frame_surface`]): its view,
//! of the `frame.surface` kind, is the "Embedded views" Frame pane
//! (`crate::embedded`). It shows the lab's Frame document
//! ([`frame_document`]), reloaded when it is saved, its knob bound to the
//! lab's drive and its meter following: `LAB_FRAME_SET` / `LAB_FRAME_STATE`
//! on the lab's list move and read the drive, and a knob turned on the
//! surface moves it too (a stand-in without the program).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::Host;
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::process::{ProcessRunner, Processes};
use architect_extensions::ui::use_extension_sections;
use architect_extensions::{Package, VersionReq};
use dioxus::prelude::*;
use facet::Facet;

const METRONOME: &str = "fts.metronome";

/// The clicker's id, as the Vault's pane names it.
pub const CLICKER: &str = "lab.clicker";

static PROCESSES: LazyLock<Processes> = LazyLock::new(Processes::default);

/// The process extensions running, for the panes that show their views.
pub fn processes() -> Processes {
    PROCESSES.clone()
}

/// The clicker's package, past its program: the manifest, its theme and
/// its ImGui window.
const CLICKER_FILES: [(&str, &str); 3] = [
    (
        "extension.toml",
        include_str!("../../clicker/extension.toml"),
    ),
    (
        "themes/clicker-neon.json",
        include_str!("../../clicker/themes/clicker-neon.json"),
    ),
    ("window.luau", include_str!("../../clicker/window.luau")),
];

/// The clicker's ImGui window's script: from its development copy (a
/// save there shows when the pane opens again), else the one built in.
fn clicker_window() -> String {
    architect::settings::app_dir("architect-lab")
        .ok()
        .and_then(|root| {
            std::fs::read_to_string(root.join("dev").join(CLICKER).join("window.luau")).ok()
        })
        .unwrap_or_else(|| include_str!("../../clicker/window.luau").to_owned())
}

/// The clicker's window (`window.luau`'s context and name).
const CLICKER_WINDOW: &str = "clicker/Window";

/// The Vault's "Clicker window" pane: the clicker's `ImGui` window, a Luau
/// script the lab runs from its package, in the app's theme.
#[component]
pub fn ClickerWindow() -> Element {
    use architect::action::ActionCaller;
    use architect_canvas::ui::{
        WindowEvent, WindowView, style_from_theme, use_canvases, use_script_canvas,
    };
    let t = architect_shell::use_shell().theme;
    let _canvases = use_canvases();
    let canvas = use_script_canvas(
        architect_scripting::luau::Runtime::new(ActionList::new()),
        ActionCaller::app(),
        use_hook(clicker_window),
        serde_json::Value::Null,
        "Clicker window".to_owned(),
        style_from_theme(&t),
    );
    let window = canvas.output().window(CLICKER_WINDOW).cloned();
    let failed = canvas.status().error.map(|e| e.to_string());
    rsx! {
        div { style: "flex: 1; position: relative; min-height: 0;",
            match (window, failed) {
                (Some(window), _) => rsx! {
                    WindowView {
                        window,
                        on_event: move |e: WindowEvent| canvas.window_event(CLICKER_WINDOW, e),
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

/// The clicker as a development copy: its package and a link to its
/// program, when the program sits beside the lab's.
fn clicker(root: &std::path::Path) -> Option<PathBuf> {
    let program = std::env::current_exe()
        .ok()?
        .with_file_name("architect-lab-clicker");
    if !program.is_file() {
        return None;
    }
    let dir = root.join("dev").join(CLICKER);
    std::fs::create_dir_all(dir.join("themes")).ok()?;
    for (path, text) in CLICKER_FILES {
        std::fs::write(dir.join(path), text).ok()?;
    }
    let link = dir.join("architect-lab-clicker");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&program, &link).ok()?;
    Some(dir)
}

/// Frame's surface extension's id (its `extension.toml`).
pub const FRAME: &str = "frame.surface";

/// Its program's name.
const FRAME_PROGRAM: &str = "frame-surface";

/// Frame's `frame-surface` program: at `ARCHITECT_LAB_FRAME_SURFACE`,
/// beside the lab's own, or on the `PATH`.
pub fn frame_surface() -> Option<&'static Path> {
    static FOUND: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
        if let Some(p) = std::env::var_os("ARCHITECT_LAB_FRAME_SURFACE") {
            let p = PathBuf::from(p);
            return p.is_file().then_some(p);
        }
        let beside = std::env::current_exe()
            .ok()
            .map(|e| e.with_file_name(FRAME_PROGRAM));
        beside.filter(|p| p.is_file()).or_else(|| {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .map(|d| d.join(FRAME_PROGRAM))
                .find(|p| p.is_file())
        })
    });
    FOUND.as_deref()
}

/// Frame's surface as a development copy: the manifest its program
/// prints (`--manifest`) and a link to the program.
fn frame_dev(root: &Path) -> Option<PathBuf> {
    let program = frame_surface()?;
    let out = std::process::Command::new(program)
        .arg("--manifest")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let dir = root.join("dev").join(FRAME);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(dir.join("extension.toml"), &out.stdout).ok()?;
    let link = dir.join(FRAME_PROGRAM);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(program, &link).ok()?;
    Some(dir)
}

/// The lab's Frame document: `ARCHITECT_LAB_FRAME_DOCUMENT`, or
/// `apps/lab/surfaces/drive.frame` above the working directory. A save to
/// it reloads the surface.
pub fn frame_document() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ARCHITECT_LAB_FRAME_DOCUMENT") {
        return Some(p.into());
    }
    let here = std::env::current_dir().ok()?;
    here.ancestors()
        .map(|a| a.join("apps/lab/surfaces/drive.frame"))
        .find(|p| p.is_file())
}

/// The extension's own actions (`frame-view`'s `extension.toml`).
const SURFACE_LOAD: &str = "FRAME_SURFACE_LOAD";
const SURFACE_PARAM_SET: &str = "FRAME_SURFACE_PARAM_SET";
const SURFACE_FEED: &str = "FRAME_SURFACE_FEED";
const SURFACE_STATE: &str = "FRAME_SURFACE_STATE";

/// The document's knob and meter: the param the lab's drive is bound to,
/// and the feed that follows it.
const DRIVE_PARAM: &str = "surface/drive";
const LEVEL_FEED: &str = "level";

/// The level the meter reads at `drive` (0..1): -60 dB at nothing, +6 dB
/// at full, to a tenth of a dB.
#[must_use]
pub fn level_db(drive: f64) -> f64 {
    (drive.mul_add(66.0, -60.0) * 10.0).round() / 10.0
}

/// The lab's drive, and what the surface last said of itself.
struct Bound {
    drive: f64,
    by: &'static str,
    /// Bumped by each `LAB_FRAME_SET`: a knob read from the surface while
    /// one was under way is stale.
    generation: u64,
    /// Whether the extension running now was given the document.
    loaded: bool,
    document: Option<String>,
    loads: u64,
    error: Option<String>,
}

static BOUND: Mutex<Bound> = Mutex::new(Bound {
    drive: 0.5,
    by: STAND_IN,
    generation: 0,
    loaded: false,
    document: None,
    loads: 0,
    error: None,
});

fn bound() -> std::sync::MutexGuard<'static, Bound> {
    BOUND.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The stand-in's name in [`FrameState::by`].
const STAND_IN: &str = "stand-in";

/// The stand-in's drive (0..1) and level (dB).
pub fn frame_stand_in() -> (f64, f64) {
    let drive = bound().drive;
    (drive, level_db(drive))
}

/// What `LAB_FRAME_SET` takes: the drive, 0..1.
#[derive(Debug, Clone, Copy, Default, Facet)]
struct FrameSet {
    drive: f64,
}

/// The lab's drive and the level the meter follows it to, who drew them
/// (`frame-surface`, or the lab's `stand-in`), and the document the
/// surface shows: its path, how many times it was read in, why the last
/// save did not take.
#[derive(Debug, Clone, PartialEq, Facet)]
struct FrameState {
    drive: f64,
    level: f64,
    by: String,
    document: Option<String>,
    loads: u64,
    error: Option<String>,
}

static FRAME_SET: ActionMeta = ActionMeta {
    id: "LAB_FRAME_SET",
    trait_name: "",
    method_name: "",
    display_name: "Set the drive",
    description: "Sets the lab's drive (0 to 1): the Embedded views' Frame surface's knob is bound to it and its meter follows it (-60 dB at 0, +6 dB at 1). Frame's frame-surface draws it when the lab found it, a stand-in otherwise. Turning the knob sets it too.",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static FRAME_STATE: ActionMeta = ActionMeta {
    id: "LAB_FRAME_STATE",
    trait_name: "",
    method_name: "",
    display_name: "The drive and its surface",
    description: "The lab's drive (0 to 1) and the level its meter reads (dB); whether frame-surface or the lab's stand-in draws them; the Frame document shown, how many times it was read in (a save reloads it), and why the last save did not take.",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

fn state() -> FrameState {
    let b = bound();
    FrameState {
        drive: b.drive,
        level: level_db(b.drive),
        by: b.by.to_owned(),
        document: b.document.clone(),
        loads: b.loads,
        error: b.error.clone(),
    }
}

/// Runs the extension's `id` with `args`: its answer, the surface's state.
async fn run(
    on: &ActionList,
    id: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let out = on
        .run_json_async(id, args.to_string().as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    serde_json::from_slice(&out).map_err(|e| format!("{FRAME_PROGRAM} answered {id} oddly: {e}"))
}

/// Takes the document's part of the surface's `state`; whether it was
/// read in again since.
fn heard(state: &serde_json::Value) -> bool {
    let mut b = bound();
    let loads = state["loads"].as_u64().unwrap_or_default();
    let reloaded = loads != b.loads;
    b.document = state["document"].as_str().map(str::to_owned);
    b.loads = loads;
    b.error = state["error"].as_str().map(str::to_owned);
    reloaded
}

/// Puts `drive` on the surface: its knob, and the meter that follows.
async fn push(on: &ActionList, drive: f64) -> Result<serde_json::Value, String> {
    run(
        on,
        SURFACE_PARAM_SET,
        &serde_json::json!({ "addr": DRIVE_PARAM, "value": drive }),
    )
    .await?;
    run(
        on,
        SURFACE_FEED,
        &serde_json::json!({ "id": LEVEL_FEED, "values": [level_db(drive)] }),
    )
    .await
}

/// Puts `LAB_FRAME_SET` and `LAB_FRAME_STATE` on the lab's list (`lab`):
/// a set moves the lab's drive and, while the extension is loaded on
/// `extensions`, the surface's knob and meter, and is a `frame` change;
/// the state is read at once (a check's read).
fn frame_actions(lab: &Arc<ActionList>, extensions: &Arc<ActionList>) {
    let (to, told) = (Arc::clone(extensions), Arc::clone(lab));
    let _ = lab.register_async(&FRAME_SET, Effect::Mutating, move |args: FrameSet| {
        let (to, told) = (Arc::clone(&to), Arc::clone(&told));
        async move {
            let drive = args.drive;
            if !(0.0..=1.0).contains(&drive) {
                return Err(format!("`drive` is a number from 0 to 1, not {drive}"));
            }
            {
                let mut b = bound();
                b.generation = b.generation.wrapping_add(1);
            }
            let by = if to.info(SURFACE_PARAM_SET).is_some() {
                heard(&push(&to, drive).await?);
                FRAME_PROGRAM
            } else {
                STAND_IN
            };
            {
                let mut b = bound();
                b.drive = drive;
                b.by = by;
            }
            told.changed("frame", by);
            Ok(state())
        }
    });
    let _ = lab.register(&FRAME_STATE, Effect::ReadOnly, |(): ()| {
        Ok::<_, String>(state())
    });
}

/// How often the binding looks at the surface.
const BINDING_POLL: Duration = Duration::from_millis(150);

/// The binding between the lab's drive and the surface, for as long as
/// the lab runs: an extension that loads is given the lab's document and
/// its drive; a knob turned on the surface (dragged, or
/// `FRAME_SURFACE_PARAM_SET` from anywhere) moves the drive and the meter
/// with it; a save to the document is a `frame` change.
async fn frame_binding(lab: Arc<ActionList>, extensions: Arc<ActionList>) {
    loop {
        futures_timer::Delay::new(BINDING_POLL).await;
        if extensions.info(SURFACE_STATE).is_none() {
            bound().loaded = false;
            continue;
        }
        if !bound().loaded {
            if let Some(path) = frame_document() {
                let path = path.canonicalize().unwrap_or(path);
                let _ = run(
                    &extensions,
                    SURFACE_LOAD,
                    &serde_json::json!({ "path": path.display().to_string() }),
                )
                .await;
            }
            let drive = bound().drive;
            if push(&extensions, drive).await.is_ok() {
                let mut b = bound();
                b.loaded = true;
                b.by = FRAME_PROGRAM;
            }
            lab.changed("frame", FRAME_PROGRAM);
            continue;
        }
        let generation = bound().generation;
        let Ok(now) = run(&extensions, SURFACE_STATE, &serde_json::json!({})).await else {
            continue;
        };
        let knob = now
            .get("params")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|p| p.get("addr").and_then(serde_json::Value::as_str) == Some(DRIVE_PARAM))
            .and_then(|p| p.get("value").and_then(serde_json::Value::as_f64));
        let moved = knob.is_some_and(|knob| {
            let mut b = bound();
            let turned = b.generation == generation && (knob - b.drive).abs() > 1e-9;
            if turned {
                b.drive = knob;
            }
            turned
        });
        if moved {
            let _ = run(
                &extensions,
                SURFACE_FEED,
                &serde_json::json!({ "id": LEVEL_FEED, "values": [level_db(bound().drive)] }),
            )
            .await;
        }
        if heard(&now) || moved {
            lab.changed("frame", FRAME_PROGRAM);
        }
    }
}

/// How often a loaded extension's files are checked for a save.
const POLL: Duration = Duration::from_millis(500);

/// The lab's registry folder.
fn registry() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ARCHITECT_LAB_EXTENSIONS") {
        return Some(dir.into());
    }
    let here = std::env::current_dir().ok()?;
    here.ancestors()
        .map(|a| a.join("apps/lab/extensions"))
        .find(|d| d.join("index.toml").is_file())
}

/// The metronome, linked into the build.
fn builtin() -> Option<Package> {
    Package::embedded(
        include_str!("../../extensions/metronome/extension.toml"),
        &[(
            "count_in.luau",
            include_str!("../../extensions/metronome/count_in.luau"),
        )],
    )
    .ok()
}

/// Runs the lab's extensions while mounted, the metronome while
/// `metronome` is on.
#[component]
pub fn LabExtensions(metronome: bool) -> Element {
    let sections = use_extension_sections();
    let list = use_hook(|| Arc::new(ActionList::new()));
    architect_commands::use_action_list(Arc::clone(&list));
    let lab = use_context::<Arc<ActionList>>();
    use_hook(|| frame_actions(&lab, &list));
    {
        let (lab, list) = (Arc::clone(&lab), Arc::clone(&list));
        use_future(move || frame_binding(Arc::clone(&lab), Arc::clone(&list)));
    }
    let themes = architect_theme::try_use_theme().map(|t| t.store());
    let (extensions, from_registry) = use_hook(|| {
        // The kinds its views may be: Frame's surface besides Dioxus.
        let mut host =
            Host::new((*list).clone(), sections.clone()).with_view_kinds(crate::embedded::kinds());
        // Extensions' themes and tokens into the lab's store; the theme,
        // as data, to its processes.
        if let Some(store) = &themes {
            host = host.with_themes(store.clone());
        }
        if let Ok(mut runner) = ProcessRunner::new() {
            if let Some(store) = &themes {
                runner = runner.with_theme(store.clone());
            }
            host = host.with_runner(runner.with_processes(processes()));
        }
        let root = architect::settings::app_dir("architect-lab")
            .unwrap_or_else(|_| std::env::temp_dir().join("architect-lab"));
        let clicker = clicker(&root);
        let frame = frame_dev(&root);
        let mut extensions = Extensions::new(host, Installs::new(root.join("extensions")));
        let found = registry();
        match &found {
            Some(dir) => extensions = extensions.with_source(FolderRegistry::new(dir)),
            None => {
                if let Some(p) = builtin() {
                    extensions = extensions.with_builtin(p);
                }
            }
        }
        let _ = extensions.install_actions();
        // Off until the switch says otherwise.
        let _ = extensions.disable(METRONOME);
        for dir in clicker.into_iter().chain(frame) {
            let _ = extensions.dev(dir);
        }
        (extensions, found.is_some())
    });
    let follow = extensions.clone();
    use_effect(use_reactive!(|metronome| {
        if metronome {
            if from_registry && follow.installs().get(METRONOME).is_none() {
                let _ = follow.install(METRONOME, &VersionReq::STAR);
            }
            let _ = follow.enable(METRONOME);
        } else {
            let _ = follow.disable(METRONOME);
        }
    }));
    use_future(move || {
        let extensions = extensions.clone();
        async move {
            loop {
                futures_timer::Delay::new(POLL).await;
                let _ = extensions.reload_changed();
            }
        }
    });
    rsx! {}
}
