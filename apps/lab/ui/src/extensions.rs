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
use architect_extensions::browse::Browse;
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::Host;
use architect_extensions::hostapi::{AppContext, SettingValues};
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::interface::{ExtensionUi, use_extension_ui};
use architect_extensions::manager::extension::ManagerExtension;
use architect_extensions::manager::{Manager, kinds};
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
        &[
            (
                "count_in.luau",
                include_str!("../../extensions/metronome/count_in.luau"),
            ),
            (
                "status.luau",
                include_str!("../../extensions/metronome/status.luau"),
            ),
        ],
    )
    .ok()
}

/// The extension actions an agent reaches on the lab's own list (its
/// control socket, MCP): forwarded to the extensions' list, each run there
/// as whoever ran it on the lab's (the lab's authorizer and confirmation
/// asked first).
const FORWARDED: [&str; 28] = [
    "EXT_LIST",
    "EXT_SEARCH",
    "EXT_INSTALL",
    "EXT_UNINSTALL",
    "EXT_RELOAD",
    "EXT_CHECK",
    "EXT_CLONE",
    "EXT_PREVIEW",
    "EXT_INFO",
    "EXT_UPDATE",
    "EXT_PIN",
    "EXT_GRANT",
    "EXT_REVOKE",
    "EXT_GRANTS",
    "EXT_LOGS",
    "EXT_HOST_CALL",
    "MANAGER_KINDS",
    "MANAGER_LIST",
    "MANAGER_GET",
    "MANAGER_RUN",
    "EXT_CONTRIBUTIONS",
    "EXT_OPEN",
    "EXT_VIEW_OPEN",
    "EXT_STATE_SET",
    "EXT_STATE_CLEAR",
    "EXT_EMIT",
    "EXT_ENABLE",
    "EXT_DISABLE",
];

/// Puts `ids` of `from` on `to`, run on `from` as whoever ran them.
fn forward(to: &ActionList, from: &ActionList, ids: &[&str]) {
    for id in ids {
        let Some(info) = from.info(id) else { continue };
        let (list, id) = (from.clone(), (*id).to_owned());
        let _ = to.register_forwarded_with(
            &info,
            Arc::new(
                move |caller: &architect::action::ActionCaller, args: &[u8]| {
                    let (list, caller, id, args) =
                        (list.clone(), caller.clone(), id.clone(), args.to_vec());
                    let run: architect::action::ActionFuture =
                        Box::pin(
                            async move { list.run_json_async_with(&caller, &id, &args).await },
                        );
                    Ok(run)
                },
            ),
        );
    }
}

/// Runs the lab's extensions — the metronome while `metronome` is on, the
/// clicker and Frame's surface when their programs are found, and the
/// Board (`apps/lab/board`), linked in (`ARCHITECT_LAB_BOARD_PROCESS=1`
/// runs its program as a development copy instead) — and puts what they
/// add into the window: call it in the lab's root, above the shell.
pub fn use_lab_extensions(metronome: bool) -> ExtensionUi {
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
    use_context_provider(processes);
    // The settings page's values, for the host service (its own thread):
    // a copy kept here, and changes sent back to the page's store.
    let values = use_hook(LabValues::default);
    use_settings_bridge(&values);
    // The extensions manager: its kinds are added below, once what they
    // manage exists (the layouts' in the lab's root).
    let manager = use_hook(Manager::new);
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
        // Hooks hear the lab's own events (workspace.changed, …).
        host.watch(&lab);
        let root = architect::settings::app_dir("architect-lab")
            .unwrap_or_else(|_| std::env::temp_dir().join("architect-lab"));
        // The host service: consent kept on this device (an installed
        // extension runs with what the person granted), each one's data
        // and files, the selection, the settings page's values.
        host = host
            .with_grants(Grants::open(root.join("extension-grants.json")))
            .with_data_root(root.join("extension-data"))
            .with_app_context(LabContext {
                list: Arc::clone(&lab),
            })
            .with_setting_values(values.clone());
        let clicker = clicker(&root);
        let frame = frame_dev(&root);
        // `EXT_CLONE` puts its copies beside the development ones.
        let mut extensions = Extensions::new(host, Installs::new(root.join("extensions")))
            .with_dev_root(root.join("dev"));
        // The Board: one Rust value, linked in as a built-in. The
        // extensions manager is one too.
        if let Ok(with) = extensions.clone().with_linked(&architect_lab_board::Board) {
            extensions = with;
        }
        if let Ok(with) = extensions
            .clone()
            .with_linked(&ManagerExtension::new(manager.clone()))
        {
            extensions = with;
        }
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
        let _ = extensions.install_contribution_actions();
        // What is wrong with them is in the lab's `APP_STATUS`, and their
        // `EXT_*` actions on the lab's list (the CLI's and MCP's).
        extensions.report_problems(lab.problems());
        // An agent's grant waits for the person's yes, on the lab's list
        // too (the sheet).
        Extensions::confirm_on(&lab);
        // Off until the switch says otherwise.
        let _ = extensions.disable(METRONOME);
        for dir in clicker.into_iter().chain(frame).chain(board_dev(&root)) {
            let _ = extensions.dev(dir);
        }
        let _ = extensions.sync();
        // After the first load: the linked manager's MANAGER_* exist now.
        forward(&lab, &list, &FORWARDED);
        (extensions, found.is_some())
    });
    let follow = extensions.clone();
    use_effect(use_reactive!(|metronome| {
        if metronome {
            // Installed only with the person's yes: one that asks for
            // more waits on the extensions page.
            if from_registry && follow.installs().get(METRONOME).is_none() {
                let _ = follow.install_with(METRONOME, &VersionReq::STAR, None, "person", false);
            }
            let _ = follow.enable(METRONOME);
        } else {
            let _ = follow.disable(METRONOME);
        }
    }));
    let polled = extensions.clone();
    use_future(move || {
        let extensions = polled.clone();
        async move {
            loop {
                futures_timer::Delay::new(POLL).await;
                let _ = extensions.reload_changed();
            }
        }
    });
    // The Board's root context, provided here for every view below.
    use_hook(|| {
        for (_, built) in extensions.host().linked() {
            built.provide_context();
        }
    });
    // The extensions page: a settings section, ⌘O's `ext:`, the Vault's
    // "Extensions" view.
    let browse = use_hook(|| Browse::new(extensions.clone()));
    use_hook(|| {
        manager.register(kinds::ExtensionKind::new(browse.clone()));
        manager.register(kinds::ScriptKind::new(extensions.clone()));
        manager.register(kinds::WorkerKind::new(processes()));
        if let Some(store) = &themes {
            manager.register(kinds::ThemeKind::new(store.clone(), (*lab).clone()));
        }
        manager.confirm_on(&lab);
    });
    architect_extensions::page::use_manager_page(manager.clone(), browse);
    use_extension_ui(extensions.host().clone(), Arc::clone(&lab))
}

/// What the lab tells an extension that may know it: the workspace's
/// focus (`APP_STATUS`'s `workspace`), no organization.
struct LabContext {
    list: Arc<ActionList>,
}

impl AppContext for LabContext {
    fn selection(&self) -> Option<serde_json::Value> {
        let (_, json) = self
            .list
            .status()
            .read()
            .into_iter()
            .find(|(name, _)| name == "workspace")?;
        let ws: serde_json::Value = serde_json::from_str(&json).ok()?;
        Some(serde_json::json!({
            "focused": ws.get("focused"),
            "focused_title": ws.get("focused_title"),
            "screenset": ws.get("screenset"),
        }))
    }
}

/// The settings page's values as the host service reads and writes them:
/// a copy of each extension's settings, and the writes waiting for the
/// page's store.
#[derive(Clone, Default)]
struct LabValues {
    now: Arc<Mutex<std::collections::BTreeMap<String, serde_json::Value>>>,
    writes: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl SettingValues for LabValues {
    fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.now
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn set(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.now
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.clone());
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value));
        Ok(())
    }
}

/// A JSON value as a setting's.
fn setting_value(v: &serde_json::Value) -> Option<architect_settings::Value> {
    match v {
        serde_json::Value::Bool(b) => Some(architect_settings::Value::Bool(*b)),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(architect_settings::Value::Int)
            .or_else(|| n.as_f64().map(architect_settings::Value::Float)),
        serde_json::Value::String(t) => Some(architect_settings::Value::Text(t.clone())),
        _ => None,
    }
}

fn setting_json(v: &architect_settings::Value) -> serde_json::Value {
    match v {
        architect_settings::Value::Bool(b) => serde_json::json!(b),
        architect_settings::Value::Int(i) => serde_json::json!(i),
        architect_settings::Value::Float(f) => serde_json::json!(f),
        architect_settings::Value::Text(t) => serde_json::json!(t),
    }
}

/// Keeps `values` in step with the settings page: every extension
/// setting's value copied in as it changes, every host-service write put
/// in the page's store.
fn use_settings_bridge(values: &LabValues) {
    let Some(h) = architect_settings::try_use_settings() else {
        return;
    };
    let copy = values.clone();
    use_effect(move || {
        let registry = h.registry.read();
        let now = h.values.read();
        let mut out = std::collections::BTreeMap::new();
        for section in registry.sections() {
            if !matches!(section.owner, architect_settings::Owner::Extension(_)) {
                continue;
            }
            for setting in section.settings() {
                if let Some(v) = now.get(&registry, &setting.key) {
                    out.insert(setting.key.clone(), setting_json(&v));
                }
            }
        }
        *copy.now.lock().unwrap_or_else(PoisonError::into_inner) = out;
    });
    let writes = values.clone();
    use_future(move || {
        let writes = writes.clone();
        async move {
            loop {
                futures_timer::Delay::new(Duration::from_millis(150)).await;
                let queued: Vec<_> = std::mem::take(
                    &mut *writes.writes.lock().unwrap_or_else(PoisonError::into_inner),
                );
                for (key, value) in queued {
                    if let Some(v) = setting_value(&value) {
                        h.set(&key, v);
                    }
                }
            }
        }
    });
}

/// The Board as a development copy running its program, when
/// `ARCHITECT_LAB_BOARD_PROCESS=1` and the program sits beside the lab's.
fn board_dev(root: &Path) -> Option<PathBuf> {
    if std::env::var_os("ARCHITECT_LAB_BOARD_PROCESS").is_none_or(|v| v != "1") {
        return None;
    }
    let program = std::env::current_exe()
        .ok()?
        .with_file_name("architect-lab-board");
    if !program.is_file() {
        return None;
    }
    let dir = root.join("dev").join(architect_lab_board::ID);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("extension.toml"),
        include_str!("../../board/extension.toml"),
    )
    .ok()?;
    let link = dir.join("architect-lab-board");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&program, &link).ok()?;
    Some(dir)
}

/// A tab an extension added (`lab.board/board`): its view, under a page
/// bar with its title.
#[component]
pub fn ExtensionTab(ui: ExtensionUi, id: &'static str) -> Element {
    let Some((extension, tab)) = ui.tab(id) else {
        return rsx! {};
    };
    rsx! {
        architect_shell::Page { title: tab.title.clone(), scroll: false,
            div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column;",
                architect_extensions::interface::ExtensionView { ui, extension, view: tab.view }
            }
        }
    }
}
