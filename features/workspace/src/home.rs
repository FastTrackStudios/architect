//! The interface as files (feature `home`; `docs/content/architecture/interface-files.md`):
//! under the app's data folder,
//!
//! - `layouts/<name>.json`: a layout — the shell's slots (what fills
//!   each, hidden, a slot's own look), the look over the whole window
//!   (the theme store's workspace layer), and the workspace's arrangement
//!   (`WORKSPACE_APPLY`'s portable tree);
//! - `keys.json`: the person's keymap layer (`architect-commands`'
//!   `PersonKeymap`: rebinds and custom actions);
//! - `layouts/.state/<name>.json`: what the person changed by hand (a
//!   slot hidden, a view put in one), kept apart from the file someone
//!   wrote; a deliberate edit of the file beats it for the slots it names.
//!
//! Each is watched ([`Home::poll`]) and applied live. A file that does
//! not read keeps its last good version and gets `<name>.error` beside it
//! saying why (gone when it reads again); each such error is a
//! [`Problem`]. The app's own layouts ([`Home::open`]'s built-ins) are
//! written on every start, read-only: an edit to one is a problem (clone
//! it), and an edited copy found at start is kept aside as
//! `<name>-edited.json`.
//!
//! A layout's text:
//!
//! ```json
//! { "name": "Board",
//!   "look": { "tokens": { "brand": "#f59e0b" } },
//!   "chrome": {
//!     "status": { "view": "lab.board/status-bar" },
//!     "ribbon": { "hidden": true },
//!     "left":   { "tokens": { "sheet": "#101820" } } },
//!   "workspace": { "windows": [ … ] } }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use facet::Facet;

use crate::control::PortableLayout;

/// The shell's slots a layout names (`architect_shell::Slot`'s names).
pub const SLOTS: [&str; 6] = ["ribbon", "left", "right", "tabs", "status", "title"];

/// What fills one slot, as a layout says.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
#[facet(deny_unknown_fields)]
pub struct SlotSpec {
    /// The view that fills it (`lab.board/status-bar`), or the shell's
    /// own (`shell/<slot>`, the same as none).
    #[facet(default)]
    pub view: Option<String>,
    /// Shown empty.
    #[facet(default)]
    pub hidden: Option<bool>,
    /// Its own look: token overrides over the window's.
    #[facet(default)]
    pub tokens: BTreeMap<String, String>,
}

impl SlotSpec {
    /// `other` over `self`: each field it says wins.
    #[must_use]
    pub fn under(&self, other: &Self) -> Self {
        let mut tokens = self.tokens.clone();
        tokens.extend(other.tokens.clone());
        Self {
            view: other.view.clone().or_else(|| self.view.clone()),
            hidden: other.hidden.or(self.hidden),
            tokens,
        }
    }
}

/// The look over the whole window: the theme store's workspace layer.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
#[facet(deny_unknown_fields)]
pub struct Look {
    /// A theme's id.
    #[facet(default)]
    pub theme: Option<String>,
    #[facet(default)]
    pub tokens: BTreeMap<String, String>,
}

/// A layout file.
#[derive(Debug, Clone, Default, PartialEq, Facet)]
#[facet(deny_unknown_fields)]
pub struct LayoutFile {
    /// What it is called where layouts are listed.
    #[facet(default)]
    pub name: Option<String>,
    /// The window's look.
    #[facet(default)]
    pub look: Option<Look>,
    /// The shell's slots, by name.
    #[facet(default)]
    pub chrome: BTreeMap<String, SlotSpec>,
    /// The workspace's arrangement.
    #[facet(default)]
    pub workspace: Option<PortableLayout>,
}

impl LayoutFile {
    /// Reads and checks a layout's text.
    ///
    /// # Errors
    ///
    /// It is not JSON, has a field it should not, or names a slot there
    /// is not.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = facet_json::from_str(text).map_err(|e| e.to_string())?;
        if let Some(bad) = file.chrome.keys().find(|k| !SLOTS.contains(&k.as_str())) {
            return Err(format!(
                "chrome: {bad:?} is not a slot ({})",
                SLOTS.join(", ")
            ));
        }
        Ok(file)
    }

    /// The file as JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        facet_json::to_string(self).unwrap_or_default()
    }

    /// `state` (what the person changed by hand) over this file.
    #[must_use]
    pub fn with_state(&self, state: &Self) -> Self {
        let mut out = self.clone();
        for (slot, spec) in &state.chrome {
            let merged = out
                .chrome
                .get(slot)
                .map_or_else(|| spec.clone(), |own| own.under(spec));
            out.chrome.insert(slot.clone(), merged);
        }
        out
    }
}

/// Something wrong with one of the files.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Problem {
    /// The file, relative to the home folder (`layouts/main.json`).
    pub file: String,
    pub message: String,
}

/// What a poll found changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// The layouts read anew (or broken: their last good stays).
    pub layouts: Vec<String>,
    /// `keys.json` read anew.
    pub keys: bool,
    /// The problems are not what they were.
    pub problems: bool,
}

impl Changed {
    #[must_use]
    pub fn any(&self) -> bool {
        !self.layouts.is_empty() || self.keys || self.problems
    }
}

/// What a file was when it was last read.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

#[derive(Debug, Default)]
struct State {
    dir: PathBuf,
    builtins: BTreeMap<String, String>,
    /// Each layout's last good version.
    layouts: BTreeMap<String, LayoutFile>,
    /// What the person changed by hand, per layout.
    state: BTreeMap<String, LayoutFile>,
    keys: Option<String>,
    stamps: BTreeMap<PathBuf, Stamp>,
    problems: BTreeMap<String, String>,
    active: String,
    revision: u64,
}

/// The app's interface files. Cheap to clone; `Send`.
#[derive(Debug, Clone, Default)]
pub struct Home(Arc<Mutex<State>>);

impl Home {
    /// The home in `dir` (the app's data folder): its folders made, the
    /// app's `builtins` (each a name and its text) written read-only, an
    /// edited copy of one kept aside, everything read.
    #[must_use]
    pub fn open(dir: impl Into<PathBuf>, builtins: &[(&str, &str)]) -> Self {
        let dir = dir.into();
        let layouts = dir.join("layouts");
        let _ = std::fs::create_dir_all(layouts.join(".state"));
        let mut kept = BTreeMap::new();
        for (name, text) in builtins {
            let path = layouts.join(format!("{name}.json"));
            if let Ok(found) = std::fs::read_to_string(&path)
                && found != *text
            {
                let aside = free_name(&layouts, &format!("{name}-edited"));
                let _ = std::fs::rename(&path, layouts.join(format!("{aside}.json")));
            }
            let _ = set_writable(&path, true);
            let _ = std::fs::write(&path, text);
            let _ = set_writable(&path, false);
            kept.insert((*name).to_owned(), (*text).to_owned());
        }
        let active = std::fs::read_to_string(layouts.join(".state").join("active"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .or_else(|| kept.keys().next().cloned())
            .unwrap_or_else(|| "main".to_owned());
        let home = Self(Arc::new(Mutex::new(State {
            dir,
            builtins: kept,
            active,
            ..State::default()
        })));
        let _ = home.poll();
        home
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The folder it is in.
    #[must_use]
    pub fn dir(&self) -> PathBuf {
        self.lock().dir.clone()
    }

    /// Moves on with every change it applied.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// Reads every file that changed since the last poll: what changed.
    #[must_use = "what changed"]
    pub fn poll(&self) -> Changed {
        let mut s = self.lock();
        let mut changed = Changed::default();
        let before = s.problems.clone();
        let layouts = s.dir.join("layouts");
        let mut seen = Vec::new();
        for entry in std::fs::read_dir(&layouts).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|n| n.to_str()).map(str::to_owned) else {
                continue;
            };
            seen.push(name.clone());
            let now = stamp(&path);
            if s.stamps.get(&path) == Some(&now) {
                continue;
            }
            s.stamps.insert(path.clone(), now);
            let file = format!("layouts/{name}.json");
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let read = match s.builtins.get(&name) {
                Some(shipped) if *shipped != text => Err(format!(
                    "{name} is built in and read-only: clone it (WORKSPACES_CLONE) and edit the clone"
                )),
                Some(shipped) => LayoutFile::parse(shipped),
                None => LayoutFile::parse(&text),
            };
            match read {
                Ok(layout) => {
                    let _ = std::fs::remove_file(layouts.join(format!("{name}.error")));
                    s.problems.remove(&file);
                    // A deliberate edit beats a click for the slots it names.
                    let mut kept = None;
                    if let Some(state) = s.state.get_mut(&name) {
                        let before = state.chrome.len();
                        state
                            .chrome
                            .retain(|slot, _| !layout.chrome.contains_key(slot));
                        if state.chrome.len() != before {
                            kept = Some(state.to_json());
                        }
                    }
                    if let Some(json) = kept {
                        let path = layouts.join(".state").join(format!("{name}.json"));
                        let _ = std::fs::write(&path, json);
                        let now = stamp(&path);
                        s.stamps.insert(path, now);
                    }
                    if s.layouts.get(&name) != Some(&layout) {
                        s.layouts.insert(name.clone(), layout);
                        changed.layouts.push(name.clone());
                    }
                }
                Err(why) => {
                    let _ = std::fs::write(layouts.join(format!("{name}.error")), &why);
                    s.problems.insert(file, why);
                    changed.layouts.push(name.clone());
                }
            }
        }
        // A layout whose file went: gone (a built-in's comes back next start).
        let gone: Vec<String> = s
            .layouts
            .keys()
            .filter(|n| !seen.contains(n) && !s.builtins.contains_key(*n))
            .cloned()
            .collect();
        for name in gone {
            s.layouts.remove(&name);
            s.problems.remove(&format!("layouts/{name}.json"));
            changed.layouts.push(name);
        }
        // What the person changed by hand.
        let state_dir = layouts.join(".state");
        for name in seen {
            let path = state_dir.join(format!("{name}.json"));
            let now = stamp(&path);
            if s.stamps.get(&path) == Some(&now) {
                continue;
            }
            s.stamps.insert(path.clone(), now);
            match std::fs::read_to_string(&path)
                .ok()
                .map(|t| LayoutFile::parse(&t))
            {
                Some(Ok(state)) => {
                    s.state.insert(name.clone(), state);
                }
                _ => {
                    s.state.remove(&name);
                }
            }
            changed.layouts.push(name);
        }
        // keys.json.
        let keys = s.dir.join("keys.json");
        let now = stamp(&keys);
        if s.stamps.get(&keys) != Some(&now) {
            s.stamps.insert(keys.clone(), now);
            match std::fs::read_to_string(&keys) {
                Ok(text) => match architect_commands::PersonKeymap::from_json(&text) {
                    Ok(_) => {
                        let _ = std::fs::remove_file(s.dir.join("keys.error"));
                        s.problems.remove("keys.json");
                        s.keys = Some(text);
                        changed.keys = true;
                    }
                    Err(why) => {
                        let _ = std::fs::write(s.dir.join("keys.error"), &why);
                        s.problems.insert("keys.json".to_owned(), why);
                    }
                },
                Err(_) => {
                    changed.keys = s.keys.take().is_some();
                    s.problems.remove("keys.json");
                }
            }
        }
        changed.layouts.sort();
        changed.layouts.dedup();
        changed.problems = s.problems != before;
        if changed.any() {
            s.revision = s.revision.wrapping_add(1);
        }
        changed
    }

    /// Every layout there is, by name.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.lock().layouts.keys().cloned().collect()
    }

    /// Whether `name` is one of the app's own.
    #[must_use]
    pub fn is_builtin(&self, name: &str) -> bool {
        self.lock().builtins.contains_key(name)
    }

    /// The layout shown now.
    #[must_use]
    pub fn active(&self) -> String {
        self.lock().active.clone()
    }

    /// Shows layout `name` from now on (kept for the next start).
    ///
    /// # Errors
    ///
    /// There is no such layout.
    pub fn use_layout(&self, name: &str) -> Result<(), String> {
        let mut s = self.lock();
        if !s.layouts.contains_key(name) {
            return Err(format!(
                "there is no layout {name:?} ({})",
                s.layouts.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        s.active = name.to_owned();
        let _ = std::fs::write(s.dir.join("layouts/.state/active"), name);
        s.revision = s.revision.wrapping_add(1);
        Ok(())
    }

    /// Layout `name` as it shows: its last good file, with what the person
    /// changed by hand over it.
    #[must_use]
    pub fn layout(&self, name: &str) -> Option<LayoutFile> {
        let s = self.lock();
        let file = s.layouts.get(name)?;
        Some(match s.state.get(name) {
            Some(state) => file.with_state(state),
            None => file.clone(),
        })
    }

    /// The layout shown now, as it shows.
    #[must_use]
    pub fn current(&self) -> Option<LayoutFile> {
        self.layout(&self.active())
    }

    /// Slot `slot` of the layout shown, changed by hand (a person's click,
    /// an agent's `HOME_SLOT_SET`): kept in `layouts/.state/`, not in the
    /// layout's file. `None` takes the change back.
    ///
    /// # Errors
    ///
    /// No such slot.
    pub fn set_slot(&self, slot: &str, spec: Option<SlotSpec>) -> Result<(), String> {
        if !SLOTS.contains(&slot) {
            return Err(format!("{slot:?} is not a slot ({})", SLOTS.join(", ")));
        }
        let mut s = self.lock();
        let name = s.active.clone();
        let state = s.state.entry(name.clone()).or_default();
        match spec {
            Some(spec) => {
                state.chrome.insert(slot.to_owned(), spec);
            }
            None => {
                state.chrome.remove(slot);
            }
        }
        let json = state.to_json();
        let path = s.dir.join("layouts/.state").join(format!("{name}.json"));
        let _ = std::fs::write(&path, json);
        let now = stamp(&path);
        s.stamps.insert(path, now);
        s.revision = s.revision.wrapping_add(1);
        Ok(())
    }

    /// `keys.json`'s text, when it reads.
    #[must_use]
    pub fn keys(&self) -> Option<String> {
        self.lock().keys.clone()
    }

    /// What is wrong with the files now.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        self.lock()
            .problems
            .iter()
            .map(|(file, message)| Problem {
                file: file.clone(),
                message: message.clone(),
            })
            .collect()
    }
}

/// The interface files on the Action List.
///
/// | action | what it does |
/// |---|---|
/// | `HOME_LAYOUTS` | every layout (name, built in, the one shown), the folder, and the files' problems |
/// | `HOME_LAYOUT_USE` | shows layout `{name}` |
/// | `HOME_SLOT_SET` | a slot of the layout shown, changed by hand: `{slot, view?, hidden?, tokens?}`, or `{slot, clear: true}` |
/// | `HOME_PROBLEMS` | what is wrong with the files: each one's path and why (its `.error`'s text) |
pub mod actions {
    use std::collections::BTreeMap;

    use architect::action::{ActionCaller, ActionError, ActionList, ActionMeta, Effect, RawJson};
    use facet::Facet;

    use super::{Home, SlotSpec};

    const fn meta(id: &'static str, name: &'static str, description: &'static str) -> ActionMeta {
        ActionMeta {
            id,
            trait_name: "",
            method_name: id,
            display_name: name,
            description,
            category: "Interface",
            group: "Layouts",
            shortcut: "",
            toggleable: false,
            undo: false,
        }
    }

    static LAYOUTS: ActionMeta = meta(
        "HOME_LAYOUTS",
        "Interface layouts",
        "Every layout file (layouts/<name>.json under the app's folder): its name, whether it is \
         built in (read-only), which shows; the folder; and the files' problems.",
    );
    static USE: ActionMeta = meta(
        "HOME_LAYOUT_USE",
        "Show a layout",
        "Shows layout `name` (its slots, its look, its arrangement), kept for the next start.",
    );
    static SLOT: ActionMeta = meta(
        "HOME_SLOT_SET",
        "Change a slot",
        "Changes slot `slot` (ribbon, left, right, tabs, status, title) of the layout shown, by \
         hand: the view that fills it, hidden, its tokens; `clear` takes the change back. Kept in \
         layouts/.state/, apart from the layout's file; an edit of the file that names the slot \
         beats it.",
    );
    static PROBLEMS: ActionMeta = meta(
        "HOME_PROBLEMS",
        "Interface file problems",
        "What is wrong with the interface files (layouts, keys.json): each file and why, the text \
         of the .error beside it. A broken file keeps its last good version.",
    );

    /// `HOME_LAYOUT_USE`'s arguments.
    #[derive(Debug, Facet)]
    pub struct UseArgs {
        pub name: String,
    }

    /// `HOME_SLOT_SET`'s arguments.
    #[derive(Debug, Facet)]
    pub struct SlotArgs {
        pub slot: String,
        pub view: Option<String>,
        pub hidden: Option<bool>,
        pub tokens: Option<BTreeMap<String, String>>,
        pub clear: Option<bool>,
    }

    fn json(v: &serde_json::Value) -> RawJson {
        RawJson::from(v.to_string())
    }

    /// Puts the `HOME_*` actions on `list`.
    ///
    /// # Errors
    ///
    /// An id is taken.
    pub fn register(list: &ActionList, home: &Home) -> Result<(), ActionError> {
        let h = home.clone();
        list.register_with(
            &LAYOUTS,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                let active = h.active();
                let layouts: Vec<serde_json::Value> = h
                    .names()
                    .into_iter()
                    .map(|n| {
                        serde_json::json!({
                            "name": n,
                            "title": h.layout(&n).and_then(|l| l.name),
                            "builtin": h.is_builtin(&n),
                            "shown": n == active,
                        })
                    })
                    .collect();
                Ok::<_, String>(json(&serde_json::json!({
                    "dir": h.dir().display().to_string(),
                    "active": active,
                    "layouts": layouts,
                    "problems": problems(&h),
                })))
            },
        )?;
        let h = home.clone();
        list.register_with(
            &USE,
            Effect::Mutating,
            move |_: &ActionCaller, a: UseArgs| {
                h.use_layout(&a.name)?;
                Ok::<_, String>(json(&serde_json::json!({ "shown": a.name })))
            },
        )?;
        let h = home.clone();
        list.register_with(&SLOT, Effect::Mutating, move |_: &ActionCaller, a: SlotArgs| {
            let spec = (a.clear != Some(true)).then(|| SlotSpec {
                view: a.view,
                hidden: a.hidden,
                tokens: a.tokens.unwrap_or_default(),
            });
            h.set_slot(&a.slot, spec)?;
            Ok::<_, String>(json(&serde_json::json!({
                "layout": h.active(),
                "slot": a.slot,
                "now": h.current().and_then(|l| l.chrome.get(&a.slot).map(|s| serde_json::json!({
                    "view": s.view, "hidden": s.hidden, "tokens": s.tokens,
                }))),
            })))
        })?;
        let h = home.clone();
        list.register_with(
            &PROBLEMS,
            Effect::ReadOnly,
            move |_: &ActionCaller, (): ()| {
                Ok::<_, String>(json(&serde_json::json!({ "problems": problems(&h) })))
            },
        )?;
        Ok(())
    }

    fn problems(h: &Home) -> serde_json::Value {
        serde_json::Value::Array(
            h.problems()
                .into_iter()
                .map(|p| serde_json::json!({ "file": p.file, "message": p.message }))
                .collect(),
        )
    }
}

/// The interface files in the window (features `home` and `ui`).
#[cfg(feature = "ui")]
pub mod ui {
    use std::sync::Arc;
    use std::time::Duration;

    use architect::action::ActionList;
    use architect_shell::{Chrome, Filler, Slot};
    use dioxus::prelude::*;

    use super::{Home, LayoutFile};

    /// How often the files are looked at.
    const POLL: Duration = Duration::from_millis(400);

    /// Watches `home` while mounted: its revision as a signal, moving with
    /// every change it applied (a file saved, a layout shown, a slot
    /// changed by hand).
    pub fn use_home(home: Home) -> Signal<u64> {
        let mut revision = use_signal(|| home.revision());
        use_future(move || {
            let home = home.clone();
            async move {
                loop {
                    futures_timer::Delay::new(POLL).await;
                    let _ = home.poll();
                    let now = home.revision();
                    if *revision.peek() != now {
                        revision.set(now);
                    }
                }
            }
        });
        revision
    }

    /// The shell's chrome a layout asks for: each slot's view (made by
    /// `fill`; `shell/<slot>` is the shell's own over whatever an
    /// extension put there), hidden, its look.
    #[must_use]
    pub fn chrome_of(layout: &LayoutFile, fill: impl Fn(&str) -> Option<Filler>) -> Chrome {
        let mut chrome = Chrome::default();
        for (name, spec) in &layout.chrome {
            let Some(slot) = Slot::named(name) else {
                continue;
            };
            if spec.hidden == Some(true) {
                chrome.hidden.insert(slot);
            }
            match spec.view.as_deref() {
                Some(view) if view.starts_with("shell/") => {
                    chrome.own.insert(slot);
                }
                Some(view) => {
                    if let Some(filler) = fill(view) {
                        chrome.fillers.insert(slot, filler);
                    }
                }
                None => {}
            }
            if !spec.tokens.is_empty() {
                chrome.looks.insert(slot, spec.tokens.clone());
            }
        }
        chrome
    }

    /// A layout's look as the theme store's workspace layer.
    #[must_use]
    pub fn look_of(layout: &LayoutFile) -> architect_theme::Layer {
        let mut layer = architect_theme::Layer::default();
        if let Some(look) = &layout.look {
            if let Some(theme) = &look.theme {
                layer = layer.choice(architect_theme::ThemeChoice::One(theme.clone()));
            }
            for (k, v) in &look.tokens {
                layer = layer.token(k.clone(), v.clone());
            }
        }
        layer
    }

    /// Applies what the layout shown asks of the window but its chrome:
    /// its look (into `store`'s workspace layer), `keys.json` (the
    /// person's keymap layer, live), and its arrangement
    /// (`WORKSPACE_APPLY` on `list`, when the workspace is there).
    pub fn use_home_apply(
        home: Home,
        revision: Signal<u64>,
        store: Option<architect_theme::ThemeStore>,
        list: Arc<ActionList>,
    ) {
        let commands = architect_commands::try_use_commands();
        let mut applied = use_signal(|| None::<(String, Option<String>)>);
        let mut keys_applied = use_signal(|| None::<String>);
        use_effect(move || {
            let _ = revision();
            let layout = home.current().unwrap_or_default();
            if let Some(store) = &store {
                store.set(architect_theme::LayerKind::Workspace, look_of(&layout));
            }
            if let (Some(keys), Some(c)) = (home.keys(), commands)
                && keys_applied.peek().as_deref() != Some(keys.as_str())
                && let Ok(person) = architect_commands::PersonKeymap::from_json(&keys)
            {
                c.set_person_keymap(person);
                keys_applied.set(Some(keys));
            }
            let tree = layout
                .workspace
                .as_ref()
                .and_then(|w| facet_json::to_string(w).ok());
            let now = (home.active(), tree.clone());
            if applied.peek().as_ref() != Some(&now) {
                applied.set(Some(now));
                if let Some(tree) = tree {
                    let list = Arc::clone(&list);
                    spawn(async move {
                        let args = format!("{{\"layout\":{tree}}}");
                        let _ = list
                            .run_json_async("WORKSPACE_APPLY", args.as_bytes())
                            .await;
                    });
                }
            }
        });
    }
}

/// `base`, or `base-2`, `base-3`… : the first that is not a layout yet.
fn free_name(dir: &Path, base: &str) -> String {
    let mut name = base.to_owned();
    let mut n = 1_u32;
    while dir.join(format!("{name}.json")).exists() {
        n = n.saturating_add(1);
        name = format!("{base}-{n}");
    }
    name
}

fn set_writable(path: &Path, writable: bool) -> std::io::Result<()> {
    let mut perms = std::fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        perms.set_mode(if writable { 0o644 } else { 0o444 });
    }
    #[cfg(not(unix))]
    perms.set_readonly(!writable);
    std::fs::set_permissions(path, perms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = r#"{ "name": "Main" }"#;

    fn write(home: &Home, file: &str, text: &str) {
        let path = home.dir().join(file);
        let _ = set_writable(&path, true);
        std::fs::write(&path, text).unwrap();
        // A second write within one mtime tick: make the length differ
        // or bump the time so the poll sees it.
        let later = SystemTime::now() + std::time::Duration::from_secs(1);
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(later).unwrap();
    }

    #[test]
    fn a_layout_reads_applies_and_a_broken_one_keeps_its_last_good() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        assert_eq!(home.active(), "main");
        assert_eq!(home.current().unwrap().name.as_deref(), Some("Main"));

        write(
            &home,
            "layouts/board.json",
            r##"{ "chrome": { "status": { "view": "lab.board/status-bar" }, "ribbon": { "hidden": true } },
                 "look": { "tokens": { "brand": "#f59e0b" } } }"##,
        );
        let changed = home.poll();
        assert_eq!(changed.layouts, ["board"]);
        home.use_layout("board").unwrap();
        let board = home.current().unwrap();
        assert_eq!(
            board.chrome["status"].view.as_deref(),
            Some("lab.board/status-bar")
        );
        assert_eq!(board.chrome["ribbon"].hidden, Some(true));

        // Broken: the last good stays, an .error says why, a problem too.
        write(
            &home,
            "layouts/board.json",
            r#"{ "chrome": { "footer": {} } }"#,
        );
        let changed = home.poll();
        assert!(changed.problems);
        assert_eq!(home.current().unwrap(), board, "the last good one");
        let error = std::fs::read_to_string(tmp.path().join("layouts/board.error")).unwrap();
        assert!(error.contains("\"footer\" is not a slot"), "{error}");
        assert_eq!(home.problems()[0].file, "layouts/board.json");
        write(&home, "layouts/board.json", r#"{ "chrome": { "nope": 1 "#);
        let _ = home.poll();
        assert_eq!(home.current().unwrap(), board);

        // Fixed: the .error goes.
        write(
            &home,
            "layouts/board.json",
            r#"{ "chrome": { "status": { "hidden": true } } }"#,
        );
        let _ = home.poll();
        assert!(!tmp.path().join("layouts/board.error").exists());
        assert!(home.problems().is_empty());
        assert_eq!(home.current().unwrap().chrome["status"].hidden, Some(true));
        assert!(home.use_layout("nowhere").is_err());
    }

    #[test]
    fn a_change_by_hand_is_kept_apart_and_a_file_edit_beats_it() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        write(&home, "layouts/mine.json", r#"{ "chrome": {} }"#);
        let _ = home.poll();
        home.use_layout("mine").unwrap();
        home.set_slot(
            "ribbon",
            Some(SlotSpec {
                hidden: Some(true),
                ..SlotSpec::default()
            }),
        )
        .unwrap();
        assert_eq!(home.current().unwrap().chrome["ribbon"].hidden, Some(true));
        let file = std::fs::read_to_string(tmp.path().join("layouts/mine.json")).unwrap();
        assert!(!file.contains("ribbon"), "the authored file is untouched");
        assert!(tmp.path().join("layouts/.state/mine.json").exists());
        assert!(home.set_slot("footer", None).is_err());

        // The file now names the ribbon: its word wins.
        write(
            &home,
            "layouts/mine.json",
            r#"{ "chrome": { "ribbon": { "hidden": false } } }"#,
        );
        let _ = home.poll();
        assert_eq!(home.current().unwrap().chrome["ribbon"].hidden, Some(false));
    }

    #[test]
    fn built_ins_are_read_only_and_an_edited_one_is_kept_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        let main = tmp.path().join("layouts/main.json");
        assert!(std::fs::metadata(&main).unwrap().permissions().readonly());
        // Edited anyway: a problem, the shipped one still shows.
        write(&home, "layouts/main.json", r#"{ "name": "Mine" }"#);
        let _ = home.poll();
        assert_eq!(home.current().unwrap().name.as_deref(), Some("Main"));
        assert!(home.problems()[0].message.contains("read-only"));
        drop(home);
        // The next start keeps the edit aside and puts the shipped one back.
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        let aside = std::fs::read_to_string(tmp.path().join("layouts/main-edited.json")).unwrap();
        assert!(aside.contains("Mine"));
        assert!(home.names().contains(&"main-edited".to_owned()));
        assert!(home.problems().is_empty());
        assert!(home.is_builtin("main") && !home.is_builtin("main-edited"));
    }

    #[test]
    fn the_files_are_on_the_action_list() {
        use architect::action::ActionList;
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        let list = ActionList::new();
        actions::register(&list, &home).unwrap();
        let run = |id: &str, args: &str| -> Result<serde_json::Value, String> {
            list.run_json(id, args.as_bytes())
                .map(|out| serde_json::from_slice(&out).unwrap())
                .map_err(|e| e.to_string())
        };
        let layouts = run("HOME_LAYOUTS", "").unwrap();
        assert_eq!(layouts["active"], "main");
        assert_eq!(layouts["layouts"][0]["builtin"], true);
        assert!(run("HOME_LAYOUT_USE", r#"{"name":"nope"}"#).is_err());
        let set = run("HOME_SLOT_SET", r#"{"slot":"status","hidden":true}"#).unwrap();
        assert_eq!(set["now"]["hidden"], true);
        assert!(run("HOME_SLOT_SET", r#"{"slot":"footer"}"#).is_err());
        write(&home, "layouts/bad.json", "{");
        let _ = home.poll();
        let problems = run("HOME_PROBLEMS", "").unwrap();
        assert_eq!(problems["problems"][0]["file"], "layouts/bad.json");
        list.validate().unwrap();
    }

    #[test]
    fn keys_json_is_read_and_a_broken_one_keeps_its_last_good() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Home::open(tmp.path(), &[("main", MAIN)]);
        assert!(home.keys().is_none());
        let good = r#"{"rebinds":[{"context":"","keys":"Ctrl+Shift+b","action":"LAB_BOARD_PIN"}]}"#;
        write(&home, "keys.json", good);
        assert!(home.poll().keys);
        write(&home, "keys.json", "{ not json");
        let _ = home.poll();
        assert_eq!(home.keys().as_deref(), Some(good));
        assert!(tmp.path().join("keys.error").exists());
        assert_eq!(home.problems()[0].file, "keys.json");
    }
}
