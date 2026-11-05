//! Navigator, a built-in extension (#272): the navigator
//! (`architect-navigator`, a port of Notebook Navigator) as a feature of
//! its own. Its manifest (`extension.toml`) declares its sidebar view
//! (`navigator/files`, docked left), its `NAVIGATOR_*` actions, its rows on
//! a note's context menu, and its settings section ("Navigator", the keys
//! of `NavigatorSettings`).
//!
//! One [`NavigatorExtension`] value runs it linked into an app
//! (`Extensions::with_linked`) or as its own process (`src/main.rs`, a
//! development copy made by `EXT_CLONE {"id": "navigator"}`).
//!
//! - **Its view** draws the navigator the app put in context
//!   ([`NavigatorMount`]: the app's vault, what opens a note, its context
//!   menu), or, with none (its process copy), its own over a sample vault.
//! - **Its actions** run wherever the host runs them; each asks the view
//!   on show to do it (`bridge`) and answers what it did. No view on show,
//!   no navigator: an action says so.
//!
//! **Licence.** It links `architect-navigator`, so it is GPL-3.0-or-later
//! too; an app that links it ships under the GPL.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::linked::{Builder, Extension, Handle};
use architect_navigator::ui::{Navigator, NavigatorView, Open, use_navigator};
use architect_navigator::{
    Grouping, ListMode, ListRowKind, MemorySource, NavTarget, NavigatorSource, NavigatorState,
    NoteFile, Orientation, Pane, PropertyValue, SortOption,
};
use dioxus::prelude::*;
use serde_json::{Value, json};

pub mod bridge;

/// Its id.
pub const ID: &str = "navigator";

/// Its manifest.
pub const MANIFEST: &str = include_str!("../extension.toml");

/// Navigator, as a `linked::Extension`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NavigatorExtension;

static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// What the app hands Navigator's view, in context above it: its
/// navigator (made with `use_navigator` over its vault, where it lives
/// on), what opens a note, its context menu, and the room it has.
#[derive(Clone, Copy, PartialEq)]
pub struct NavigatorMount {
    pub navigator: Navigator,
    pub on_open: EventHandler<Open>,
    pub on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    pub on_focus_editor: Option<EventHandler<()>>,
    pub phone: bool,
}

/// Every action it declares (`extension.toml`), in its order.
pub const ACTIONS: &[&str] = &[
    "NAVIGATOR_STATE",
    "NAVIGATOR_SELECT",
    "NAVIGATOR_REVEAL",
    "NAVIGATOR_COLLAPSE_OR_EXPAND",
    "NAVIGATOR_BACK",
    "NAVIGATOR_FORWARD",
    "NAVIGATOR_FOCUS_FOLDERS",
    "NAVIGATOR_FOCUS_NOTES",
    "NAVIGATOR_SELECT_ALL",
    "NAVIGATOR_TOGGLE_DUAL_PANE",
    "NAVIGATOR_TOGGLE_ORIENTATION",
    "NAVIGATOR_TOGGLE_DESCENDANTS",
    "NAVIGATOR_TOGGLE_COMPACT",
    "NAVIGATOR_REORDER_ROOTS",
    "NAVIGATOR_SORT",
    "NAVIGATOR_GROUP",
    "NAVIGATOR_TOGGLE_GROUP",
    "NAVIGATOR_OWN_LIST",
    "NAVIGATOR_COLOUR",
    "NAVIGATOR_PIN",
    "NAVIGATOR_PIN_FOLDER_NOTES",
    "NAVIGATOR_MOVE",
    "NAVIGATOR_CONVERT_TO_FOLDER_NOTE",
    "NAVIGATOR_SET_AS_FOLDER_NOTE",
    "NAVIGATOR_DETACH_FOLDER_NOTE",
    "NAVIGATOR_OPEN_FOLDER_NOTE_TAB",
];

impl Extension for NavigatorExtension {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext.handle());
        ext.view("files", FilesView);
        for id in ACTIONS {
            ext.action_in(*id, move |args, cx| async move {
                // A context-menu row: the file it was opened on.
                let args = match (args, cx.get("id").and_then(Value::as_str)) {
                    (Value::Object(mut m), Some(id)) if !m.contains_key("path") => {
                        m.insert("path".to_owned(), json!(id));
                        Value::Object(m)
                    }
                    (Value::Null, Some(id)) => json!({ "path": id }),
                    (args, _) => args,
                };
                bridge::ask(id, args).await
            });
        }
    }
}

/// Its sidebar view: the app's navigator, or its own over a sample vault.
#[component]
pub fn FilesView() -> Element {
    match try_use_context::<NavigatorMount>() {
        Some(mount) => rsx! { Drive { mount } },
        None => rsx! { Standalone {} },
    }
}

/// Its own navigator, when no app gave one: its process copy's.
#[component]
fn Standalone() -> Element {
    let nav = use_navigator(|| Arc::new(sample()) as Arc<dyn NavigatorSource>);
    let on_open = EventHandler::new(|o: Open| {
        if let Some(h) = handle() {
            h.status("navigator", format!("Opened {}", o.path));
        }
    });
    rsx! {
        Drive {
            mount: NavigatorMount {
                navigator: nav,
                on_open,
                on_context_menu: None,
                on_focus_editor: None,
                phone: false,
            }
        }
    }
}

/// The navigator drawn, and the actions asked of it done.
#[component]
fn Drive(mount: NavigatorMount) -> Element {
    let nav = mount.navigator;
    let _mounted = use_hook(|| std::rc::Rc::new(bridge::Mounted::new()));
    use_future(move || async move {
        loop {
            for ask in bridge::take() {
                let out = run(nav, mount.on_open, &ask.action, &ask.args);
                let _ = ask.answer.send(out);
            }
            futures_timer::Delay::new(Duration::from_millis(40)).await;
        }
    });
    rsx! {
        NavigatorView {
            navigator: nav,
            on_open: mount.on_open,
            on_context_menu: mount.on_context_menu,
            on_focus_editor: mount.on_focus_editor,
            phone: mount.phone,
            commands: false,
        }
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The notes an action is about: `paths`, `path`, else the selected ones.
fn paths_of(args: &Value, s: &NavigatorState) -> Vec<String> {
    if let Some(list) = args.get("paths").and_then(Value::as_array) {
        return list.iter().filter_map(Value::as_str).map(str::to_owned).collect();
    }
    if let Some(p) = text(args, "path") {
        return vec![p.to_owned()];
    }
    s.selection().files().iter().cloned().collect()
}

fn one_path(args: &Value, s: &NavigatorState) -> Result<String, String> {
    text(args, "path")
        .map(str::to_owned)
        .or_else(|| s.selection().cursor().map(str::to_owned))
        .ok_or_else(|| "which note? (`path`, or select one)".to_owned())
}

/// Does action `id` to the navigator.
fn run(nav: Navigator, on_open: EventHandler<Open>, id: &str, args: &Value) -> Result<Value, String> {
    let ok = |nav: Navigator| Ok(state(&nav.state.peek()));
    match id {
        "NAVIGATOR_STATE" => ok(nav),
        "NAVIGATOR_SELECT" => {
            if let Some(tag) = text(args, "tag") {
                nav.update(|s| s.select_tag(tag));
            } else {
                let folder = text(args, "folder").unwrap_or("");
                if !nav.state.peek().index().has_folder(folder) {
                    return Err(format!("no folder {folder:?}"));
                }
                let out = nav.update(|s| s.click_folder(folder));
                nav.act(out, on_open);
            }
            ok(nav)
        }
        "NAVIGATOR_REVEAL" => {
            nav.update(NavigatorState::reveal_active);
            ok(nav)
        }
        "NAVIGATOR_COLLAPSE_OR_EXPAND" => {
            nav.update(NavigatorState::collapse_or_expand_all);
            ok(nav)
        }
        "NAVIGATOR_BACK" => {
            nav.update(NavigatorState::back);
            ok(nav)
        }
        "NAVIGATOR_FORWARD" => {
            nav.update(NavigatorState::forward);
            ok(nav)
        }
        "NAVIGATOR_FOCUS_FOLDERS" | "NAVIGATOR_FOCUS_NOTES" => {
            let pane = if id == "NAVIGATOR_FOCUS_FOLDERS" {
                Pane::Navigation
            } else {
                Pane::Files
            };
            nav.update(|s| s.focus_pane(pane));
            ok(nav)
        }
        "NAVIGATOR_SELECT_ALL" => {
            nav.update(NavigatorState::select_all);
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_DUAL_PANE" => {
            nav.settings.update(|s| s.dual_pane = !s.dual_pane);
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_ORIENTATION" => {
            nav.settings.update(|s| {
                s.orientation = match s.orientation {
                    Orientation::Horizontal => Orientation::Vertical,
                    Orientation::Vertical => Orientation::Horizontal,
                };
            });
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_DESCENDANTS" => {
            nav.edit_list(|_, s| s.include_descendants = !s.include_descendants);
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_COMPACT" => {
            nav.edit_list(|_, s| {
                s.mode = match s.mode {
                    ListMode::Standard => ListMode::Compact,
                    ListMode::Compact => ListMode::Standard,
                };
            });
            ok(nav)
        }
        "NAVIGATOR_REORDER_ROOTS" => {
            nav.update(|s| s.reordering = !s.reordering);
            ok(nav)
        }
        "NAVIGATOR_SORT" => {
            let by = text(args, "by").unwrap_or_default();
            let sort = SortOption::from_id(by).ok_or_else(|| {
                let ids: Vec<_> = SortOption::ALL.iter().map(|s| s.id()).collect();
                format!("sort `by` is one of {}", ids.join(", "))
            })?;
            nav.edit_list(move |_, s| s.sort = sort);
            ok(nav)
        }
        "NAVIGATOR_GROUP" => {
            let by = text(args, "by").unwrap_or_default();
            let g = Grouping::from_id(by)
                .ok_or_else(|| "group `by` is none, date, folder or property".to_owned())?;
            if let Some(p) = text(args, "property") {
                let p = p.to_owned();
                nav.settings.update(move |s| s.group_property.clone_from(&p));
            }
            nav.edit_list(move |_, s| s.grouping = g);
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_GROUP" => {
            let group = match text(args, "group") {
                Some(g) => g.to_owned(),
                // By its heading's label.
                None => {
                    let label = text(args, "label").ok_or("which group? (`group` or `label`)")?;
                    nav.state
                        .peek()
                        .list_rows()
                        .iter()
                        .find_map(|r| match &r.kind {
                            ListRowKind::Header { label: l, group, .. } if l == label => {
                                Some(group.clone())
                            }
                            _ => None,
                        })
                        .ok_or_else(|| format!("no group {label:?} in the list"))?
                }
            };
            nav.update(|s| s.toggle_group(&group));
            ok(nav)
        }
        "NAVIGATOR_OWN_LIST" => {
            let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
            let mut own = nav.own;
            own.set(on);
            if !on {
                let target = nav.state.peek().target();
                nav.update(|s| s.set_appearance(&target, architect_navigator::Appearance::default()));
            }
            ok(nav)
        }
        "NAVIGATOR_COLOUR" => {
            let target = text(args, "target").ok_or("`target`: tag:<tag> or property:<key>[=<value>]")?;
            nav.update(|s| s.set_colour(target, text(args, "colour")));
            ok(nav)
        }
        "NAVIGATOR_PIN" => {
            let paths = paths_of(args, &nav.state.peek());
            if paths.is_empty() {
                return Err("which notes? (`path`, `paths`, or select some)".to_owned());
            }
            nav.update(|s| s.toggle_pin(&paths));
            ok(nav)
        }
        "NAVIGATOR_PIN_FOLDER_NOTES" => {
            let n = nav.update(NavigatorState::pin_folder_notes);
            nav.say(format!("Pinned {n} folder notes"));
            Ok(json!({ "pinned": n }))
        }
        "NAVIGATOR_MOVE" => {
            let by = args.get("by").and_then(Value::as_i64).unwrap_or(1);
            let by = isize::try_from(by).unwrap_or(1);
            let out = nav.state.peek().move_in_order(by);
            if out.is_none() {
                return Err("notes move only in a manual sort (NAVIGATOR_SORT by=manual)".to_owned());
            }
            nav.act(out, on_open);
            ok(nav)
        }
        "NAVIGATOR_CONVERT_TO_FOLDER_NOTE" | "NAVIGATOR_SET_AS_FOLDER_NOTE" => {
            let path = one_path(args, &nav.state.peek())?;
            let out = nav.update(|s| {
                if id == "NAVIGATOR_CONVERT_TO_FOLDER_NOTE" {
                    s.convert_to_folder_note(&path)
                } else {
                    s.set_as_folder_note(&path)
                }
            })?;
            nav.act(Some(out), on_open);
            ok(nav)
        }
        "NAVIGATOR_DETACH_FOLDER_NOTE" => {
            let folder = folder_of(args, &nav.state.peek())?;
            let out = nav.update(|s| s.detach_folder_note(&folder, text(args, "name")))?;
            nav.act(Some(out), on_open);
            ok(nav)
        }
        "NAVIGATOR_OPEN_FOLDER_NOTE_TAB" => {
            let folder = folder_of(args, &nav.state.peek())?;
            let out = nav
                .state
                .peek()
                .open_folder_note(&folder, true)
                .ok_or_else(|| format!("{folder} has no folder note"))?;
            nav.act(Some(out), on_open);
            ok(nav)
        }
        other => Err(format!("Navigator has no action {other}")),
    }
}

/// The folder an action is about: `folder`, the folder of `path` (a
/// folder note's), else the one selected.
fn folder_of(args: &Value, s: &NavigatorState) -> Result<String, String> {
    if let Some(f) = text(args, "folder") {
        return Ok(f.to_owned());
    }
    if let Some(p) = text(args, "path") {
        return Ok(architect_navigator::source::parent(p).to_owned());
    }
    s.selection()
        .folder()
        .map(str::to_owned)
        .ok_or_else(|| "which folder? (`folder`, or select one)".to_owned())
}

/// What `NAVIGATOR_STATE` says: the place selected, how its list is
/// drawn, and its rows (`#heading` and paths, folded groups marked).
#[must_use]
pub fn state(s: &NavigatorState) -> Value {
    let target = match s.target() {
        NavTarget::Folder(f) => json!({ "folder": f }),
        NavTarget::Tag(t) => json!({ "tag": t }),
        _ => Value::Null,
    };
    let e = s.effective();
    let rows: Vec<Value> = s
        .list_rows()
        .iter()
        .filter_map(|r| match &r.kind {
            ListRowKind::Header {
                label,
                group,
                collapsed,
                count,
                pinned,
            } => Some(json!({
                "heading": label, "group": group, "collapsed": collapsed,
                "count": count, "pinned": pinned,
            })),
            ListRowKind::File { path, pinned, thumb, .. } => Some(json!({
                "path": path, "pinned": pinned, "thumb": thumb.as_ref().map(|t| format!("{t:?}")),
            })),
            _ => None,
        })
        .collect();
    json!({
        "target": target,
        "own_list": s.appearance(&s.target()).is_some(),
        "sort": e.sort.id(),
        "grouping": e.grouping.under(e.sort).id(),
        "compact": e.mode == ListMode::Compact,
        "descendants": e.include_descendants,
        "selected": s.selection().files(),
        "active": s.active(),
        "rows": rows,
    })
}

/// The sample vault its process copy shows: a few folders, tags, a
/// pinned-worthy plan, an image, a PDF and a drawing.
#[must_use]
pub fn sample() -> MemorySource {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    let h: i64 = 3_600_000;
    let at = |hours: i64| now.saturating_sub(h.saturating_mul(hours));
    let text = |t: &str| PropertyValue::Text(t.to_owned());
    MemorySource::new("Sample")
        .with_file(
            NoteFile::new("Projects/Plan.md", at(200), at(1))
                .preview("What we ship this month, and why.")
                .tag("work")
                .property("status", text("doing"))
                .tasks(5, 2),
        )
        .with_file(
            NoteFile::new("Projects/Launch.md", at(300), at(30))
                .preview("The day's order of things.")
                .property("status", text("planned")),
        )
        .with_file(NoteFile::new("Projects/Brief.pdf", at(400), at(90)))
        .with_file(NoteFile::new("Ideas/Sketch.excalidraw.md", at(50), at(50)))
        .with_file(
            NoteFile::new("Ideas/Garden.md", at(800), at(700))
                .preview("Seeds, beds, and the order to plant them in.")
                .tag("home"),
        )
        .with_file(NoteFile::new("Welcome.md", at(1000), at(26)).preview("Start here."))
}
