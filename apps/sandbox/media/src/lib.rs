//! The lab as a guide scenario's app (`architect-guide`): its root,
//! sized to the scenario's window, a fresh Action List, and its fonts.
//!
//! Each run has a lab folder of its own, fresh and empty
//! (`ARCHITECT_SANDBOX_CONFIG_DIR`, unless the caller set it), so nothing of
//! the person's is read and one scenario's files never reach the next; the
//! lab reads and watches it as a window's lab does ([`LabFiles`]).
//!
//! The film's own layer sits over the lab: a caption and a terminal a
//! scenario fills with [`CAPTION`] and [`TERMINAL`], and [`WRITE`] saves a
//! file in the run's folder as a person's editor would (only here, never
//! in the lab itself): so a GIF can say what it shows, which commands an
//! agent ran, and what was saved.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_app::{Device, LabApp, LabFiles};
use architect_guide::ViewSize;
use architect_guide::run::App;
use architect_shell::{Insets, Screen};
use dioxus::prelude::*;
use facet::Facet;

/// Where the lab's scenarios are, from the workspace's root.
pub const SCENARIOS: &str = "apps/sandbox/scenarios";

/// Shows `{text}` as the film's caption (empty: none).
pub const CAPTION: &str = "FILM_CAPTION";
/// Adds `{line}` to the film's terminal (`clear true` empties it first;
/// an empty terminal is not drawn).
pub const TERMINAL: &str = "FILM_TERMINAL";
/// Writes `{path, text}` under the run's lab folder (`remove true`
/// deletes it instead), as a person saving in their editor.
pub const WRITE: &str = "FILM_WRITE";

/// The lab's folder variable (`architect::settings::app_dir`).
const FOLDER_VAR: &str = "ARCHITECT_SANDBOX_CONFIG_DIR";

/// The lab, for one run, on a fresh folder of its own.
#[must_use]
pub fn app() -> App {
    fresh_folder();
    App::new(Root, Arc::new(ActionList::new())).fonts(architect_app::font_context())
}

/// The Architect app (not the sandbox) for one run.
///
/// What its own tours play over (Projects, Tasks, capture:
/// `architect_app::builtin_tours`). The first call makes this process the Architect app
/// (`architect_app::Mode`, for good: run it in a process of its own, as
/// `tests/architect_tours.rs` is) on a folder of its own and a copy of the
/// demo vault (`$ARCHITECT_CONFIG_DIR`, `$ARCHITECT_VAULT`, unless the
/// caller set them), both made fresh and never the person's. `sandbox-media`
/// films with it when `SANDBOX_MEDIA_APP=architect`.
#[must_use]
pub fn architect_app() -> App {
    static SET: OnceLock<()> = OnceLock::new();
    SET.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("architect-film-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (var, dir) in [
            ("ARCHITECT_CONFIG_DIR", "app"),
            ("ARCHITECT_VAULT", "vault"),
        ] {
            if std::env::var_os(var).is_none_or(|v| v.is_empty()) {
                // SAFETY: once, before the app (and its threads) start.
                unsafe { std::env::set_var(var, root.join(dir)) };
            }
        }
        if let Some(vault) = std::env::var_os("ARCHITECT_VAULT") {
            let _ = architect_app::vaults::write_demo(std::path::Path::new(&vault));
        }
        architect_app::Mode::Architect.set();
    });
    App::new(Root, Arc::new(ActionList::new())).fonts(architect_app::font_context())
}

/// Points the lab's folder at a new empty one, unless the process was
/// started with one (a test that sets its own).
fn fresh_folder() {
    static GIVEN: OnceLock<bool> = OnceLock::new();
    static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if *GIVEN.get_or_init(|| std::env::var_os(FOLDER_VAR).is_some_and(|v| !v.is_empty())) {
        return;
    }
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let at = |n: u32| {
        std::env::temp_dir().join(format!("architect-sandbox-film-{}-{n}", std::process::id()))
    };
    if let Some(last) = n.checked_sub(1) {
        let _ = std::fs::remove_dir_all(at(last));
    }
    let dir = at(n);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    // SAFETY: set between runs, before this run's lab (and its threads)
    // start; the runner plays one scenario at a time.
    unsafe { std::env::set_var(FOLDER_VAR, &dir) };
}

/// The run's lab folder.
fn folder() -> Option<PathBuf> {
    architect_app::app_dir()
}

/// What [`WRITE`] takes.
#[derive(Debug, Clone, Facet)]
pub struct WriteArgs {
    /// Under the lab's folder: `themes/mine.json`, `pages/lab.board/board.tsx`.
    pub path: String,
    #[facet(default)]
    pub text: String,
    #[facet(default)]
    pub remove: bool,
}

/// What [`CAPTION`] takes.
#[derive(Debug, Clone, Facet)]
pub struct CaptionArgs {
    #[facet(default)]
    pub text: String,
}

/// What [`TERMINAL`] takes.
#[derive(Debug, Clone, Facet)]
pub struct TerminalArgs {
    #[facet(default)]
    pub line: String,
    #[facet(default)]
    pub clear: bool,
    /// The terminal's title from now on (unset: as it was; at first,
    /// what an agent's commands go through).
    #[facet(default)]
    pub title: Option<String>,
}

static CAPTION_META: ActionMeta = film_meta(
    CAPTION,
    "Film caption",
    "Shows `text` over the film as its caption; empty hides it. Only in a filmed run.",
);
static WRITE_META: ActionMeta = film_meta(
    WRITE,
    "Film: save a file",
    "Writes `text` to `path` under the run's lab folder (`remove` deletes it), as an editor saving. Only in a filmed run.",
);
static TERMINAL_META: ActionMeta = film_meta(
    TERMINAL,
    "Film terminal",
    "Adds `line` to the film's terminal (`clear` empties it first). Only in a filmed run.",
);

const fn film_meta(
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: "",
        display_name,
        description,
        category: "Film",
        group: "Film",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

/// What the film's layer is told.
enum Say {
    Caption(String),
    Line(String, bool, Option<String>),
}

/// What the lab's desk window gives it (`apps/desktop`), headless: the
/// screen it is on, the scenario's window, a desk. Over it, the film's
/// caption and terminal.
#[component]
fn Root() -> Element {
    let view = use_context::<ViewSize>();
    let list = use_context::<Arc<ActionList>>();
    use_context_provider(|| Signal::new(Device::Native));
    use_context_provider(|| LabFiles);
    use_context_provider(|| {
        Signal::new(Screen {
            width: f64::from(view.width),
            height: f64::from(view.height),
            touch: false,
            insets: Insets::default(),
        })
    });
    let mut caption = use_signal(String::new);
    let mut lines = use_signal(Vec::<String>::new);
    let mut title = use_signal(|| "agent — the lab's control socket".to_owned());
    let said = use_coroutine(move |mut rx: UnboundedReceiver<Say>| async move {
        use futures_util::StreamExt as _;
        while let Some(say) = rx.next().await {
            match say {
                Say::Caption(text) => caption.set(text),
                Say::Line(line, clear, named) => lines.with_mut(|l| {
                    if let Some(named) = named {
                        title.set(named);
                    }
                    if clear {
                        l.clear();
                    }
                    if !line.is_empty() {
                        l.push(line);
                    }
                    let extra = l.len().saturating_sub(TERMINAL_LINES);
                    l.drain(..extra);
                }),
            }
        }
    });
    use_hook(move || {
        let tx = said.tx();
        let _ = list.register(&CAPTION_META, Effect::ReadOnly, move |a: CaptionArgs| {
            tx.unbounded_send(Say::Caption(a.text))
                .map_err(|_| "the film is gone".to_owned())
        });
        let _ = list.register(&WRITE_META, Effect::Mutating, |a: WriteArgs| {
            let root = folder().ok_or_else(|| "no lab folder".to_owned())?;
            if a.path.split('/').any(|p| p == "..") {
                return Err(format!("{}: outside the lab's folder", a.path));
            }
            let file = root.join(&a.path);
            if a.remove {
                return std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", a.path));
            }
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", a.path))?;
            }
            std::fs::write(&file, a.text).map_err(|e| format!("{}: {e}", a.path))
        });
        let tx = said.tx();
        let _ = list.register(&TERMINAL_META, Effect::ReadOnly, move |a: TerminalArgs| {
            tx.unbounded_send(Say::Line(a.line, a.clear, a.title))
                .map_err(|_| "the film is gone".to_owned())
        });
    });
    // Drawn only when filled, so a run that never asks leaves the lab's
    // document as it is (its goldens, `tests/builtins.rs`).
    let caption_on = !caption().is_empty();
    let terminal_on = !lines().is_empty();
    rsx! {
        LabApp {}
        if caption_on { div {
            "data-testid": "film-caption",
            style: "position: absolute; left: 50%; bottom: 22px; transform: translateX(-50%); \
                    display: block; z-index: 300; pointer-events: none; \
                    padding: 9px 18px; border-radius: 10px; background: rgba(9,9,11,0.86); \
                    color: #fafafa; font-family: Inter, sans-serif; font-size: 19px; font-weight: 600; \
                    white-space: nowrap; box-shadow: 0 8px 28px rgba(0,0,0,0.45);",
            "{caption}"
        } }
        if terminal_on { div {
            "data-testid": "film-terminal",
            style: "position: absolute; left: 18px; bottom: 76px; width: 660px; z-index: 300; pointer-events: none; \
                    display: flex; flex-direction: column; gap: 3px; box-sizing: border-box; \
                    padding: 12px 14px; border-radius: 10px; background: rgba(12,12,14,0.94); \
                    border: 1px solid #3f3f46; box-shadow: 0 12px 36px rgba(0,0,0,0.5); \
                    font-family: 'DejaVu Sans Mono', Menlo, monospace; font-size: 14px; line-height: 1.4;",
            div { style: "display: flex; gap: 6px; margin-bottom: 6px;",
                span { style: "width: 10px; height: 10px; border-radius: 5px; background: #f87171;" }
                span { style: "width: 10px; height: 10px; border-radius: 5px; background: #fbbf24;" }
                span { style: "width: 10px; height: 10px; border-radius: 5px; background: #4ade80;" }
                span { style: "margin-left: 8px; color: #a1a1aa; font-size: 12px; font-family: Inter, sans-serif;", "{title}" }
            }
            for (n, line) in lines().into_iter().enumerate() {
                div {
                    key: "{n}",
                    style: if line.starts_with('$') { "color: #86efac; white-space: pre;" } else { "color: #d4d4d8; white-space: pre;" },
                    "{line}"
                }
            }
        } }
    }
}

/// The terminal's lines kept on show.
const TERMINAL_LINES: usize = 9;
