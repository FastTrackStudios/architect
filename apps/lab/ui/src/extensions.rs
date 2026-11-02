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
//! its view is the Vault's Clicker pane ([`processes`]).
//!
//! So is Frame's `frame-surface` (FastTrackStudios/frame,
//! `crates/frame-view`), when it is found ([`frame_surface`]): its view,
//! of the `frame.surface` kind, is the "Embedded views" Frame pane
//! (`crate::embedded`), and `LAB_FRAME_SET` / `LAB_FRAME_STATE` on the
//! lab's list move and read it (a stand-in without it).

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

/// The clicker as a development copy: its manifest and a link to its
/// program, when the program sits beside the lab's.
fn clicker(root: &std::path::Path) -> Option<PathBuf> {
    let program = std::env::current_exe()
        .ok()?
        .with_file_name("architect-lab-clicker");
    if !program.is_file() {
        return None;
    }
    let dir = root.join("dev").join(CLICKER);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("extension.toml"),
        include_str!("../../clicker/extension.toml"),
    )
    .ok()?;
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

/// The extension's own action that moves it (`frame-view`'s
/// `extension.toml`).
const SURFACE_SET: &str = "FRAME_SURFACE_SET";

/// Where the last `LAB_FRAME_SET` left the knob and the meter, and who
/// drew them; the stand-in draws from it.
static LAST: Mutex<(f64, f64, &str)> = Mutex::new((0.5, 0.6, STAND_IN));

/// The stand-in's name in [`FrameState::by`].
const STAND_IN: &str = "stand-in";

/// The stand-in's knob and meter (each 0..1).
pub fn frame_stand_in() -> (f64, f64) {
    let (knob, meter, _) = *LAST.lock().unwrap_or_else(PoisonError::into_inner);
    (knob, meter)
}

/// What `LAB_FRAME_SET` takes: the knob, the meter, or both (each 0..1).
#[derive(Debug, Clone, Copy, Default, Facet)]
struct FrameSet {
    #[facet(default)]
    knob: Option<f64>,
    #[facet(default)]
    meter: Option<f64>,
}

/// Where the Frame surface is, and who drew it: `frame-surface`, or the
/// lab's `stand-in`.
#[derive(Debug, Clone, PartialEq, Facet)]
struct FrameState {
    knob: f64,
    meter: f64,
    by: String,
}

static FRAME_SET: ActionMeta = ActionMeta {
    id: "LAB_FRAME_SET",
    trait_name: "",
    method_name: "",
    display_name: "Move the Frame surface",
    description: "Turns the Embedded views' Frame surface's knob and sets its meter (each 0 to 1; either may be left out). Frame's frame-surface draws it when the lab found it, a stand-in otherwise.",
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
    display_name: "Where the Frame surface is",
    description: "Where the last LAB_FRAME_SET left the Embedded views' Frame surface's knob and meter (each 0 to 1), and whether frame-surface or the lab's stand-in drew them.",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// `frame-surface`'s answer (`{knob, meter}`).
fn answered(bytes: &[u8]) -> Result<(f64, f64), String> {
    let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let read = |key: &str| {
        v.get(key)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| format!("{FRAME_PROGRAM} answered without `{key}`: {v}"))
    };
    Ok((read("knob")?, read("meter")?))
}

fn last() -> FrameState {
    let (knob, meter, by) = *LAST.lock().unwrap_or_else(PoisonError::into_inner);
    FrameState {
        knob,
        meter,
        by: by.to_owned(),
    }
}

/// Puts `LAB_FRAME_SET` and `LAB_FRAME_STATE` on the lab's list (`lab`):
/// a move runs the extension's own action on `extensions` while it is
/// loaded and moves the stand-in otherwise, and is a `frame` change; the
/// state is where the last move left it, read at once (a check's read).
fn frame_actions(lab: &Arc<ActionList>, extensions: &Arc<ActionList>) {
    let (to, told) = (Arc::clone(extensions), Arc::clone(lab));
    let _ = lab.register_async(&FRAME_SET, Effect::Mutating, move |args: FrameSet| {
        let (to, told) = (Arc::clone(&to), Arc::clone(&told));
        async move {
            let mut asked = serde_json::Map::new();
            for (key, value) in [("knob", args.knob), ("meter", args.meter)] {
                if let Some(v) = value {
                    if !(0.0..=1.0).contains(&v) {
                        return Err(format!("`{key}` is a number from 0 to 1, not {v}"));
                    }
                    asked.insert(key.to_owned(), v.into());
                }
            }
            let (knob, meter, by) = if to.info(SURFACE_SET).is_some() {
                let args = serde_json::Value::Object(asked).to_string();
                let out = to
                    .run_json_async(SURFACE_SET, args.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                let (knob, meter) = answered(&out)?;
                (knob, meter, FRAME_PROGRAM)
            } else {
                let (knob, meter, _) = *LAST.lock().unwrap_or_else(PoisonError::into_inner);
                (
                    args.knob.unwrap_or(knob),
                    args.meter.unwrap_or(meter),
                    STAND_IN,
                )
            };
            *LAST.lock().unwrap_or_else(PoisonError::into_inner) = (knob, meter, by);
            told.changed("frame", by);
            Ok(last())
        }
    });
    let _ = lab.register(&FRAME_STATE, Effect::ReadOnly, |(): ()| {
        Ok::<_, String>(last())
    });
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
    let (extensions, from_registry) = use_hook(|| {
        // The kinds its views may be: Frame's surface besides Dioxus.
        let mut host =
            Host::new((*list).clone(), sections.clone()).with_view_kinds(crate::embedded::kinds());
        if let Ok(runner) = ProcessRunner::new() {
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
