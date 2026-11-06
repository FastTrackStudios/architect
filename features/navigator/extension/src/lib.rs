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
use architect_navigator::state::Drag;
use architect_navigator::state::{Made, MergeSeparator, NewKind, NewNote, Paint};
use architect_navigator::template::{CommandLocation, TemplateCommand};
use architect_navigator::ui::Renaming;
use architect_navigator::ui::{Navigator, NavigatorView, Open, use_navigator};
use architect_navigator::{
    CalendarGrid, Grouping, HeaderPeriod, Join, ListMode, ListRowKind, MemorySource, Mods,
    NavRowKind, NavTarget, NavigatorSource, NavigatorState, NoteFile, OpenIn, Orientation, Outcome,
    Pane, PeriodKind, PropertyValue, SortOption,
};
use architect_navigator::{FileVisibility, HideTarget, Shortcut, ShortcutKind, VaultProfile};
use architect_navigator::{Hotkey, KeyAction, KeyScope};
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

/// What the app hands Navigator's view, in context above it.
///
/// A `Signal<NavigatorMount>`: its navigator (made with `use_navigator` over
/// its vault, where it lives on), what opens a note, its context menu,
/// and whether it is a phone's.
#[derive(Clone, Copy, PartialEq)]
pub struct NavigatorMount {
    pub navigator: Navigator,
    pub on_open: EventHandler<Open>,
    pub on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    pub on_focus_editor: Option<EventHandler<()>>,
    pub phone: bool,
    /// The shell sidebar's columns, when the app's shell sets it
    /// (`NavigatorView`'s `columns`: one pane, or folders beside notes).
    pub columns: Option<u8>,
    /// New note in the list's header, given the folder on show
    /// (`NavigatorView`'s `on_new_note`).
    pub on_new_note: Option<EventHandler<String>>,
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
    "NAVIGATOR_TOGGLE_CALENDAR",
    "NAVIGATOR_CALENDAR",
    "NAVIGATOR_OPEN_DAILY_NOTE",
    "NAVIGATOR_OPEN_WEEKLY_NOTE",
    "NAVIGATOR_OPEN_MONTHLY_NOTE",
    "NAVIGATOR_OPEN_QUARTERLY_NOTE",
    "NAVIGATOR_OPEN_YEARLY_NOTE",
    "NAVIGATOR_NEXT_PERIOD",
    "NAVIGATOR_PREVIOUS_PERIOD",
    "NAVIGATOR_PARENT_PERIOD",
    "NAVIGATOR_TAGS",
    "NAVIGATOR_PROPERTIES",
    "NAVIGATOR_TOGGLE",
    "NAVIGATOR_TAG_ADD",
    "NAVIGATOR_TAG_REMOVE",
    "NAVIGATOR_TAG_CLEAR",
    "NAVIGATOR_TAG_RENAME",
    "NAVIGATOR_TAG_DELETE",
    "NAVIGATOR_TAG_NEW_NOTE",
    "NAVIGATOR_PROPERTY_SET",
    "NAVIGATOR_DROP",
    "NAVIGATOR_SEARCH",
    "NAVIGATOR_TOGGLE_SEARCH",
    "NAVIGATOR_SEARCH_VAULT",
    "NAVIGATOR_TOGGLE_HIDDEN",
    "NAVIGATOR_HIDE",
    "NAVIGATOR_SHORTCUTS",
    "NAVIGATOR_ADD_SHORTCUT",
    "NAVIGATOR_OPEN_SHORTCUT_1",
    "NAVIGATOR_OPEN_SHORTCUT_2",
    "NAVIGATOR_OPEN_SHORTCUT_3",
    "NAVIGATOR_OPEN_SHORTCUT_4",
    "NAVIGATOR_OPEN_SHORTCUT_5",
    "NAVIGATOR_OPEN_SHORTCUT_6",
    "NAVIGATOR_OPEN_SHORTCUT_7",
    "NAVIGATOR_OPEN_SHORTCUT_8",
    "NAVIGATOR_OPEN_SHORTCUT_9",
    "NAVIGATOR_RECENT",
    "NAVIGATOR_PROFILE",
    "NAVIGATOR_SELECT_PROFILE_1",
    "NAVIGATOR_SELECT_PROFILE_2",
    "NAVIGATOR_SELECT_PROFILE_3",
    "NAVIGATOR_NEW_NOTE",
    "NAVIGATOR_NEW_FOLDER",
    "NAVIGATOR_RENAME",
    "NAVIGATOR_DUPLICATE",
    "NAVIGATOR_MOVE_TO",
    "NAVIGATOR_COPY_TO",
    "NAVIGATOR_DELETE",
    "NAVIGATOR_MERGE",
    "NAVIGATOR_OPEN_ALL",
    "NAVIGATOR_OPEN_SELECTED",
    "NAVIGATOR_FOLDER_TEMPLATE",
    "NAVIGATOR_TEMPLATE_COMMANDS",
    "NAVIGATOR_TEMPLATE_COMMAND",
    "NAVIGATOR_ICON",
    "NAVIGATOR_KEYS",
    "NAVIGATOR_KEY",
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
    try_use_context::<Signal<NavigatorMount>>().map_or_else(
        || rsx! { Standalone {} },
        |mount| rsx! { Drive { mount: mount() } },
    )
}

/// Its own navigator, when no app gave one: its process copy's.
#[component]
fn Standalone() -> Element {
    let nav = use_navigator(|| {
        let source: Arc<dyn NavigatorSource> = Arc::new(sample());
        source
    });
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
                columns: None,
                on_new_note: None,
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
                let out = run(&nav, mount.on_open, &ask.action, &ask.args);
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
            columns: mount.columns,
            on_new_note: mount.on_new_note,
            commands: false,
        }
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A whole number, as JSON or as text (the CLI's `--arg by=-2`).
fn int(v: &Value, key: &str) -> Option<i64> {
    let at = v.get(key)?;
    at.as_i64().or_else(|| at.as_str()?.trim().parse().ok())
}

/// A yes or no, as JSON or as text (`--arg on=false`).
fn flag(v: &Value, key: &str) -> Option<bool> {
    let at = v.get(key)?;
    at.as_bool().or_else(|| at.as_str()?.trim().parse().ok())
}

/// The notes an action is about: `paths`, `path`, else the selected ones.
fn paths_of(args: &Value, s: &NavigatorState) -> Vec<String> {
    if let Some(list) = args.get("paths").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
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
fn run(
    nav: &Navigator,
    on_open: EventHandler<Open>,
    id: &str,
    args: &Value,
) -> Result<Value, String> {
    let nav = *nav;
    let ok = |nav: Navigator| Ok(state(&nav.state.peek()));
    match id {
        "NAVIGATOR_STATE" => ok(nav),
        "NAVIGATOR_SELECT" => {
            if let Some(tag) = text(args, "tag") {
                nav.update(|s| s.click_tag(tag, Mods::NONE));
            } else if let Some(key) = text(args, "property") {
                nav.update(|s| s.click_property(key, text(args, "value"), Mods::NONE));
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
                nav.settings
                    .update(move |s| s.group_property.clone_from(&p));
            }
            nav.edit_list(move |_, s| s.grouping = g);
            ok(nav)
        }
        "NAVIGATOR_TOGGLE_GROUP" => {
            let group = if let Some(g) = text(args, "group") {
                g.to_owned()
            } else {
                // By its heading's label.
                {
                    let label = text(args, "label").ok_or("which group? (`group` or `label`)")?;
                    nav.state
                        .peek()
                        .list_rows()
                        .iter()
                        .find_map(|r| match &r.kind {
                            ListRowKind::Header {
                                label: l, group, ..
                            } if l == label => Some(group.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| format!("no group {label:?} in the list"))?
                }
            };
            nav.update(|s| s.toggle_group(&group));
            ok(nav)
        }
        "NAVIGATOR_OWN_LIST" => {
            let on = flag(args, "on").unwrap_or(true);
            let mut own = nav.own;
            own.set(on);
            if !on {
                let target = nav.state.peek().target();
                nav.update(|s| {
                    s.set_appearance(&target, architect_navigator::Appearance::default());
                });
            }
            ok(nav)
        }
        "NAVIGATOR_COLOUR" | "NAVIGATOR_ICON" => {
            let target = paint_target(args)?;
            let (what, value) = if id == "NAVIGATOR_ICON" {
                (Paint::Icon, args.get("icon"))
            } else if flag(args, "background") == Some(true) {
                (Paint::Background, args.get("colour"))
            } else {
                (Paint::Colour, args.get("colour"))
            };
            let Some(value) = value.map(|v| v.as_str().unwrap_or("").to_owned()) else {
                // None given: asked for, as the menus do.
                let mut prompt = nav.prompt;
                prompt.set(Some(architect_navigator::ui::Prompt::paint(what, target)));
                return ok(nav);
            };
            let out = nav.update(|s| s.paint(what, &target, Some(value.as_str())))?;
            if let Some(out) = out {
                nav.write(out, on_open, None)?;
            }
            Ok(looks_state(&nav.state.peek(), &target))
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
            if let Some(p) = text(args, "path") {
                let p = p.to_owned();
                nav.update(|s| s.click_file(&p, architect_navigator::Mods::NONE));
            }
            let by = isize::try_from(int(args, "by").unwrap_or(1)).unwrap_or(1);
            if nav.state.peek().effective().sort != SortOption::Manual {
                return Err(
                    "notes move only in a manual sort (NAVIGATOR_SORT by=manual)".to_owned(),
                );
            }
            // None: nothing selected, or nowhere to go.
            let out = nav.state.peek().move_in_order(by);
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
        "NAVIGATOR_TOGGLE_CALENDAR" => {
            if !nav.state.peek().settings().calendar {
                return Err("The calendar is off in Navigator's settings".to_owned());
            }
            nav.update(NavigatorState::toggle_calendar);
            Ok(calendar(&nav.state.peek()))
        }
        "NAVIGATOR_CALENDAR" => {
            if let Some(n) = int(args, "weeks") {
                let n = u32::try_from(n)
                    .ok()
                    .filter(|n| (1..=6).contains(n))
                    .ok_or("`weeks` is 1 to 6 (6, the whole month)")?;
                nav.settings.update(move |s| s.calendar_weeks = n);
            }
            if flag(args, "today") == Some(true) {
                nav.update(|s| {
                    let today = s.today();
                    s.calendar_go(today);
                });
            }
            if let Some(d) = text(args, "date") {
                let d = date(d)?;
                nav.update(|s| s.calendar_go(d));
            }
            if let Some(by) = int(args, "move") {
                let by = i32::try_from(by).map_err(|_| "`move` is too far")?;
                nav.update(|s| s.calendar_move(by));
            }
            Ok(calendar(&nav.state.peek()))
        }
        "NAVIGATOR_OPEN_DAILY_NOTE"
        | "NAVIGATOR_OPEN_WEEKLY_NOTE"
        | "NAVIGATOR_OPEN_MONTHLY_NOTE"
        | "NAVIGATOR_OPEN_QUARTERLY_NOTE"
        | "NAVIGATOR_OPEN_YEARLY_NOTE" => {
            let kind = id
                .strip_prefix("NAVIGATOR_OPEN_")
                .and_then(|k| k.strip_suffix("_NOTE"))
                .and_then(PeriodKind::from_id)
                .ok_or("no such periodic note")?;
            let day = match text(args, "date") {
                Some(d) => date(d)?,
                None => nav.state.peek().today(),
            };
            let out = nav.state.peek().open_periodic(kind, day, how(args));
            periodic(&nav, on_open, out, args)
        }
        "NAVIGATOR_NEXT_PERIOD" | "NAVIGATOR_PREVIOUS_PERIOD" | "NAVIGATOR_PARENT_PERIOD" => {
            let out = {
                let s = nav.state.peek();
                match id {
                    "NAVIGATOR_NEXT_PERIOD" => s.open_adjacent_period(1, how(args)),
                    "NAVIGATOR_PREVIOUS_PERIOD" => s.open_adjacent_period(-1, how(args)),
                    _ => s.open_parent_period(how(args)),
                }
            };
            periodic(&nav, on_open, out, args)
        }
        "NAVIGATOR_TAGS" | "NAVIGATOR_PROPERTIES" => Ok(trees(&nav.state.peek())),
        "NAVIGATOR_TOGGLE" => {
            let target = target_of(args)?;
            nav.update(|s| s.toggle_nav(&target));
            Ok(trees(&nav.state.peek()))
        }
        "NAVIGATOR_TAG_ADD" | "NAVIGATOR_TAG_REMOVE" | "NAVIGATOR_TAG_CLEAR" => {
            let paths = paths_of(args, &nav.state.peek());
            let tag = text(args, "tag").unwrap_or_default();
            let out = {
                let s = nav.state.peek();
                match id {
                    "NAVIGATOR_TAG_ADD" => s.add_tag(&paths, tag),
                    "NAVIGATOR_TAG_REMOVE" => s.remove_tag(&paths, tag),
                    _ => s.clear_tags(&paths),
                }
            }?;
            write(&nav, on_open, out)
        }
        "NAVIGATOR_TAG_RENAME" => {
            let from = text(args, "from").ok_or("`from`: the tag to rename")?;
            let to = text(args, "to").ok_or("`to`: its new name")?;
            let out = nav.state.peek().rename_tag(from, to)?;
            write(&nav, on_open, out)
        }
        "NAVIGATOR_TAG_DELETE" => {
            let tag = text(args, "tag").ok_or("`tag`: the tag to delete")?;
            let out = nav.state.peek().delete_tag(tag)?;
            write(&nav, on_open, out)
        }
        "NAVIGATOR_TAG_NEW_NOTE" => {
            let tag = text(args, "tag").ok_or("`tag`: the tag for the new note")?;
            let out = nav.state.peek().new_note_in_tag(tag)?;
            let path = match &out {
                Outcome::Write {
                    open: Some((p, _)), ..
                } => p.clone(),
                _ => String::new(),
            };
            nav.act(Some(out), on_open);
            Ok(json!({ "path": path }))
        }
        "NAVIGATOR_PROPERTY_SET" => {
            let key = text(args, "key").ok_or("`key`: the property to set")?;
            let paths = paths_of(args, &nav.state.peek());
            let out = nav
                .state
                .peek()
                .set_property(&paths, key, text(args, "value"))?;
            write(&nav, on_open, out)
        }
        "NAVIGATOR_DROP" => {
            let drag = text(args, "drag_folder").map_or_else(
                || {
                    text(args, "drag_tag").map_or_else(
                        || Drag::Notes(paths_of(args, &nav.state.peek())),
                        |t| Drag::Tag(t.to_owned()),
                    )
                },
                |f| Drag::Folder(f.trim_matches('/').to_owned()),
            );
            let on = match args.get("to").and_then(Value::as_str) {
                Some(f) => NavTarget::Folder(f.trim_matches('/').to_owned()),
                None => target_of(args)?,
            };
            let copy = flag(args, "copy") == Some(true);
            let out = nav.state.peek().drop_on(&drag, &on, copy)?;
            write(&nav, on_open, out)
        }
        "NAVIGATOR_SEARCH" => {
            if flag(args, "clear") == Some(true) {
                nav.set_search("");
            } else if let Some(q) = args.get("query").and_then(Value::as_str) {
                nav.set_search(q);
                nav.update(|s| s.search_open = true);
            } else if text(args, "tag").is_some() || text(args, "property").is_some() {
                // As a ⌘/Ctrl-click (⇧ too with `or`).
                let mods = Mods {
                    command: true,
                    shift: flag(args, "or") == Some(true),
                };
                match target_of(args)? {
                    NavTarget::Tag(t) => nav.update(|s| s.click_tag(&t, mods)),
                    NavTarget::Property { key, value } => {
                        nav.update(|s| s.click_property(&key, value.as_deref(), mods));
                    }
                    _ => {}
                }
            }
            if let Some(open) = flag(args, "open") {
                nav.update(|s| {
                    if s.search_open != open {
                        s.toggle_search();
                    }
                });
            }
            Ok(search_state(&nav.state.peek()))
        }
        "NAVIGATOR_TOGGLE_SEARCH" => {
            nav.update(NavigatorState::toggle_search);
            Ok(search_state(&nav.state.peek()))
        }
        "NAVIGATOR_SEARCH_VAULT" => {
            nav.search_vault();
            if let Some(q) = args.get("query").and_then(Value::as_str) {
                nav.set_search(q);
            }
            Ok(search_state(&nav.state.peek()))
        }
        "NAVIGATOR_TOGGLE_HIDDEN" => {
            nav.update(NavigatorState::toggle_hidden);
            Ok(profile_state(&nav.state.peek()))
        }
        "NAVIGATOR_HIDE" => {
            let unhide = flag(args, "unhide") == Some(true);
            let what = if let Some(f) = text(args, "folder") {
                HideTarget::Folder(f.trim_matches('/').to_owned())
            } else if let Some(t) = text(args, "tag") {
                HideTarget::Tag(t.to_owned())
            } else if let Some(p) = text(args, "path") {
                HideTarget::Note(p.to_owned())
            } else {
                return Err("hide what? (`folder`, `tag` or `path`)".to_owned());
            };
            nav.update(|s| s.hide(&what, unhide));
            Ok(profile_state(&nav.state.peek()))
        }
        "NAVIGATOR_SHORTCUTS" | "NAVIGATOR_ADD_SHORTCUT" => {
            let s = nav.state.peek();
            let shortcut = shortcut_of(args, &s);
            drop(s);
            if id == "NAVIGATOR_ADD_SHORTCUT" || text(args, "add").is_some() {
                let shortcut = shortcut?;
                let at = int(args, "at").and_then(|n| usize::try_from(n).ok());
                if !nav.update(|s| s.add_shortcut(shortcut, at)) {
                    return Err("It is in the shortcuts".to_owned());
                }
            } else if let Some(n) = int(args, "remove").and_then(|n| usize::try_from(n).ok()) {
                let sc = nav
                    .state
                    .peek()
                    .profile()
                    .shortcuts
                    .get(n.saturating_sub(1))
                    .cloned()
                    .ok_or_else(|| format!("No shortcut {n}"))?;
                nav.update(|s| s.remove_shortcut(&sc));
            } else if let (Some(from), Some(to)) = (int(args, "move"), int(args, "to")) {
                let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
                    return Err("`move` and `to` are numbers from 1".to_owned());
                };
                nav.update(|s| s.move_shortcut(from.saturating_sub(1), to.saturating_sub(1)));
            } else if let Some(n) = int(args, "rename").and_then(|n| usize::try_from(n).ok()) {
                let name = text(args, "name").unwrap_or_default();
                if !nav.update(|s| s.rename_shortcut(n.saturating_sub(1), name)) {
                    return Err(format!("No shortcut {n}"));
                }
            } else if let Some(n) = int(args, "open").and_then(|n| usize::try_from(n).ok()) {
                let out = nav.update(|s| s.open_shortcut(n))?;
                nav.act(out, on_open);
            }
            Ok(profile_state(&nav.state.peek()))
        }
        _ if id.starts_with("NAVIGATOR_OPEN_SHORTCUT_") => {
            let n: usize = id
                .trim_start_matches("NAVIGATOR_OPEN_SHORTCUT_")
                .parse()
                .map_err(|_| format!("Navigator has no action {id}"))?;
            let out = nav.update(|s| s.open_shortcut(n))?;
            nav.act(out, on_open);
            ok(nav)
        }
        "NAVIGATOR_RECENT" => {
            if flag(args, "clear") == Some(true) {
                nav.update(|s| s.edit_profile(|p| p.recent.clear()));
            } else if let Some(n) = int(args, "open").and_then(|n| usize::try_from(n).ok()) {
                let s = nav.state.peek();
                let path = s
                    .profile()
                    .recent
                    .iter()
                    .filter(|p| s.index().file(p).is_some())
                    .nth(n.saturating_sub(1))
                    .cloned()
                    .ok_or_else(|| format!("No recent note {n}"))?;
                let out = s.open_recent(&path);
                drop(s);
                nav.act(out, on_open);
            }
            Ok(profile_state(&nav.state.peek()))
        }
        "NAVIGATOR_PROFILE" => {
            if let Some(which) = text(args, "select") {
                nav.update(|s| s.select_profile(which))?;
            } else if let Some(name) = text(args, "add") {
                let copy = flag(args, "copy").unwrap_or(true);
                nav.update(|s| s.add_profile(name, copy))?;
            } else if let Some(which) = text(args, "remove") {
                nav.update(|s| s.remove_profile(which))?;
            } else if let Some(which) = text(args, "rename") {
                let name = text(args, "name").ok_or("the new `name`?")?;
                nav.update(|s| s.rename_profile(which, name))?;
            }
            if let Some(set) = args.get("set").filter(|v| v.is_object()) {
                let mut edit: Result<(), String> = Ok(());
                nav.update(|s| s.edit_profile(|p| edit = set_profile(p, set)));
                edit?;
            }
            Ok(profile_state(&nav.state.peek()))
        }
        _ if id.starts_with("NAVIGATOR_SELECT_PROFILE_") => {
            let n = id.trim_start_matches("NAVIGATOR_SELECT_PROFILE_");
            nav.update(|s| s.select_profile(n))?;
            Ok(profile_state(&nav.state.peek()))
        }
        _ => files(&nav, on_open, id, args),
    }
}

/// The paths an action is about, from `paths`, `path` or `folder`; else
/// the notes selected.
fn targets(args: &Value, s: &NavigatorState) -> Vec<String> {
    if let Some(f) =
        text(args, "folder").filter(|_| args.get("paths").is_none() && text(args, "path").is_none())
    {
        return vec![f.trim_matches('/').to_owned()];
    }
    paths_of(args, s)
}

/// A request for a new note from an action's args.
fn new_note_of(args: &Value, s: &NavigatorState) -> Result<NewNote, String> {
    let kind = match text(args, "kind").unwrap_or("note") {
        "note" => NewKind::Note,
        "canvas" => NewKind::Canvas,
        "base" => NewKind::Base,
        other => return Err(format!("no kind {other:?} (note, canvas, base)")),
    };
    let prompts = args
        .get("prompts")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    Ok(NewNote {
        folder: text(args, "folder")
            .map_or_else(|| s.current_folder(), |f| f.trim_matches('/').to_owned()),
        kind,
        template: text(args, "template").map(str::to_owned),
        no_template: flag(args, "no_template") == Some(true),
        name_format: text(args, "name_format").map(str::to_owned),
        name: text(args, "name").map(str::to_owned),
        prompts,
        how: if flag(args, "tab") == Some(true) {
            OpenIn::Tab
        } else {
            OpenIn::Here
        },
        date: None,
    })
}

/// A note made, as an action answers it: its path, the cursor, what to
/// say; or the prompts it needs answered.
fn made(nav: &Navigator, on_open: EventHandler<Open>, m: Made) -> Result<Value, String> {
    match m {
        Made::Ask(labels) => Err(format!(
            "answer its prompts first: `prompts` {{{}}}",
            labels
                .iter()
                .map(|l| format!("\"{l}\": \"…\""))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Made::Done {
            out,
            cursor,
            warning,
        } => {
            let path = match &out {
                Outcome::Write {
                    open: Some((p, _)), ..
                } => p.clone(),
                _ => String::new(),
            };
            nav.write(out, on_open, cursor)?;
            Ok(json!({ "path": path, "cursor": cursor, "warning": warning }))
        }
    }
}

/// The file operations and templates (#339).
fn files(
    nav: &Navigator,
    on_open: EventHandler<Open>,
    id: &str,
    args: &Value,
) -> Result<Value, String> {
    let nav = *nav;
    let ok = |nav: Navigator| Ok(state(&nav.state.peek()));
    let done = |out: Result<Outcome, String>| -> Result<Value, String> {
        nav.write(out?, on_open, None)?;
        Ok(state(&nav.state.peek()))
    };
    match id {
        "NAVIGATOR_NEW_NOTE" => {
            let req = new_note_of(args, &nav.state.peek())?;
            let m = nav.state.peek().new_note(&req)?;
            made(&nav, on_open, m)
        }
        "NAVIGATOR_NEW_FOLDER" => {
            let name = text(args, "name").ok_or("the folder's `name`?")?;
            let parent = text(args, "parent").map_or_else(
                || nav.state.peek().current_folder(),
                |p| p.trim_matches('/').to_owned(),
            );
            done(nav.state.peek().new_folder(&parent, name))
        }
        "NAVIGATOR_RENAME" => {
            let folder = text(args, "folder").filter(|_| text(args, "path").is_none());
            match (text(args, "name"), folder) {
                (Some(name), Some(f)) => done(nav.state.peek().rename_folder(f, name)),
                (Some(name), None) => {
                    let p = one_path(args, &nav.state.peek())?;
                    done(nav.state.peek().rename_file(&p, name))
                }
                // No name: its name is edited in place.
                (None, f) => {
                    let what = match f {
                        Some(f) => Renaming::Folder(f.trim_matches('/').to_owned()),
                        None => Renaming::File(one_path(args, &nav.state.peek())?),
                    };
                    let mut renaming = nav.renaming;
                    renaming.set(Some(what));
                    ok(nav)
                }
            }
        }
        "NAVIGATOR_DUPLICATE" => {
            if let Some(f) = text(args, "folder").filter(|_| text(args, "path").is_none()) {
                done(nav.state.peek().duplicate_folder(f))
            } else {
                let p = one_path(args, &nav.state.peek())?;
                done(nav.state.peek().duplicate(&p))
            }
        }
        "NAVIGATOR_MOVE_TO" | "NAVIGATOR_COPY_TO" => {
            let Some(to) = args.get("to").and_then(Value::as_str) else {
                // No folder named: asked for.
                let paths = paths_of(args, &nav.state.peek());
                let mut prompt = nav.prompt;
                prompt.set(Some(architect_navigator::ui::Prompt::move_to(paths)));
                return ok(nav);
            };
            let to = to.trim_matches('/');
            let s = nav.state.peek();
            let out = match (text(args, "folder"), id) {
                (Some(f), "NAVIGATOR_MOVE_TO")
                    if text(args, "path").is_none() && args.get("paths").is_none() =>
                {
                    s.move_folder(f, to)
                }
                (_, "NAVIGATOR_MOVE_TO") => s.move_files(&paths_of(args, &s), to),
                _ => s.copy_files(&paths_of(args, &s), to),
            };
            drop(s);
            done(out)
        }
        "NAVIGATOR_DELETE" => {
            let paths = targets(args, &nav.state.peek());
            if flag(args, "confirm") == Some(false) {
                done(nav.state.peek().trash(&paths))
            } else {
                nav.trash(&paths, on_open);
                ok(nav)
            }
        }
        "NAVIGATOR_MERGE" => {
            let paths = if args.get("paths").is_some() {
                paths_of(args, &nav.state.peek())
            } else {
                nav.state.peek().selected_in_order()
            };
            let sep = match text(args, "separator") {
                Some(t) => MergeSeparator::parse(t).ok_or_else(|| {
                    format!("no separator {t:?} (none, blank-line, horizontal-rule, heading)")
                })?,
                None => MergeSeparator::BlankLine,
            };
            let name = text(args, "name").unwrap_or("Merged");
            done(nav.state.peek().merge(
                &paths,
                name,
                text(args, "into"),
                sep,
                flag(args, "trash") == Some(true),
            ))
        }
        "NAVIGATOR_OPEN_ALL" | "NAVIGATOR_OPEN_SELECTED" => {
            if let Some(f) = text(args, "folder") {
                let out = nav.update(|s| s.click_folder(f.trim_matches('/')));
                nav.act(out, on_open);
            }
            let paths = if id == "NAVIGATOR_OPEN_ALL" {
                nav.state.peek().open_all()?
            } else {
                nav.state.peek().selected_in_order()
            };
            let n = paths.len();
            if flag(args, "confirm") == Some(false) {
                nav.then(architect_navigator::ui::AskThen::OpenAll(paths), on_open);
            } else {
                nav.open_many(paths, on_open);
            }
            Ok(json!({ "notes": n }))
        }
        "NAVIGATOR_FOLDER_TEMPLATE" => {
            let folder = text(args, "folder")
                .unwrap_or("")
                .trim_matches('/')
                .to_owned();
            let template = args
                .get("template")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let below = flag(args, "subfolders").unwrap_or(true);
            nav.update(|s| s.edit_templates(|t| t.set_folder(&folder, &template, below)));
            Ok(templates_state(&nav.state.peek()))
        }
        "NAVIGATOR_TEMPLATE_COMMANDS" => {
            if let Some(name) = text(args, "add") {
                let location = if text(args, "folder").is_some() {
                    CommandLocation::Folder
                } else {
                    CommandLocation::Current
                };
                let n = nav.state.peek().templates().commands.len();
                let c = TemplateCommand {
                    id: text(args, "id")
                        .map_or_else(|| format!("c{}", n.saturating_add(1)), str::to_owned),
                    name: name.to_owned(),
                    template: text(args, "template").unwrap_or("").to_owned(),
                    name_format: text(args, "name_format").unwrap_or("").to_owned(),
                    location,
                    folder: text(args, "folder").unwrap_or("").to_owned(),
                };
                if nav
                    .state
                    .peek()
                    .templates()
                    .command(&c.id)
                    .is_some_and(|x| x.id == c.id)
                {
                    return Err(format!("there is a command {:?} already", c.id));
                }
                nav.update(|s| s.edit_templates(|t| t.commands.push(c)));
            } else if let Some(which) = text(args, "remove") {
                let id = nav
                    .state
                    .peek()
                    .templates()
                    .command(which)
                    .map(|c| c.id.clone())
                    .ok_or_else(|| format!("no create-note command {which:?}"))?;
                nav.update(|s| s.edit_templates(|t| t.commands.retain(|c| c.id != id)));
            }
            Ok(templates_state(&nav.state.peek()))
        }
        "NAVIGATOR_TEMPLATE_COMMAND" => {
            let which = text(args, "id").ok_or("which command? (`id`, its name or number)")?;
            let mut req = nav
                .state
                .peek()
                .template_command_request(which, OpenIn::Here)?;
            let asked = new_note_of(args, &nav.state.peek())?;
            req.prompts = asked.prompts;
            let m = nav.state.peek().new_note(&req)?;
            made(&nav, on_open, m)
        }
        "NAVIGATOR_KEYS" => {
            if let Some(a) = text(args, "action") {
                let action = KeyAction::from_id(a).ok_or_else(|| format!("no key action {a:?}"))?;
                let keys = if flag(args, "reset") == Some(true) {
                    None
                } else {
                    let keys: Vec<String> = args
                        .get("keys")
                        .and_then(Value::as_array)
                        .map(|l| {
                            l.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    if let Some(bad) = keys.iter().find(|k| Hotkey::parse(k).is_none()) {
                        return Err(format!("{bad:?} is no hotkey (Mod+Shift+P, ArrowUp, k)"));
                    }
                    Some(keys)
                };
                nav.update(|s| s.bind_keys(action, keys));
            }
            Ok(keys_state(&nav.state.peek()))
        }
        "NAVIGATOR_KEY" => {
            let a =
                text(args, "action").ok_or("which? (`action`: pane:move-down, list:open, …)")?;
            let action = KeyAction::from_id(a).ok_or_else(|| format!("no key action {a:?}"))?;
            let scope = if nav.state.peek().focus == Pane::Navigation {
                KeyScope::Navigation
            } else {
                KeyScope::List
            };
            match action {
                KeyAction::PaneRename => nav.rename_selected(scope),
                KeyAction::DeleteSelected => nav.delete_selected(scope, on_open),
                _ => {
                    let out = nav.update(|s| s.run_key(action));
                    nav.act(out, on_open);
                }
            }
            ok(nav)
        }
        other => Err(format!("Navigator has no action {other}")),
    }
}

/// The target an icon or a colour is for: `target` as written
/// (`folder:<path>`, `tag:<tag>`, `property:<key>[=<value>]`,
/// `file:<path>`), else `path`, `folder`, `tag`, or `property` (and
/// `value`).
fn paint_target(args: &Value) -> Result<String, String> {
    if let Some(t) = text(args, "target") {
        return Ok(t.to_owned());
    }
    if let Some(p) = text(args, "path") {
        return Ok(format!("file:{p}"));
    }
    if let Some(f) = args.get("folder").and_then(Value::as_str) {
        return Ok(format!("folder:{}", f.trim_matches('/')));
    }
    if let Some(t) = text(args, "tag") {
        return Ok(format!("tag:{t}"));
    }
    if let Some(k) = text(args, "property") {
        return Ok(text(args, "value")
            .map_or_else(|| format!("property:{k}"), |v| format!("property:{k}={v}")));
    }
    Err("which? (`target`, or `path`, `folder`, `tag`, `property`)".to_owned())
}

/// A target's icon, colour and background, as `NAVIGATOR_ICON` and
/// `NAVIGATOR_COLOUR` answer.
fn looks_state(s: &NavigatorState, target: &str) -> Value {
    let icon = target
        .strip_prefix("file:")
        .map_or_else(|| s.own_icon(target), |p| s.file_icon(p));
    json!({
        "target": target,
        "icon": icon.map(|i| i.id()),
        "colour": s.colour(target),
        "background": s.background(target),
    })
}

/// The keyboard map as `NAVIGATOR_KEYS` answers it: each action's id and
/// hotkeys, and which a person set.
fn keys_state(s: &NavigatorState) -> Value {
    let own: Vec<&str> = s.key_bindings().iter().map(|b| b.action.as_str()).collect();
    json!({
        "vim": s.settings().vim_keys,
        "actions": KeyAction::ALL.iter().map(|a| json!({
            "action": a.id(),
            "keys": s.keymap().keys(*a).iter().map(Hotkey::text).collect::<Vec<_>>(),
            "own": own.contains(&a.id()),
        })).collect::<Vec<_>>(),
    })
}

/// The templates as `NAVIGATOR_FOLDER_TEMPLATE` and
/// `NAVIGATOR_TEMPLATE_COMMANDS` answer them.
fn templates_state(s: &NavigatorState) -> Value {
    let t = s.templates();
    json!({
        "folders": t.folders.iter().map(|f| json!({
            "folder": f.folder, "template": f.template, "subfolders": f.subfolders,
        })).collect::<Vec<_>>(),
        "commands": t.commands.iter().map(|c| json!({
            "id": c.id, "name": c.name, "template": c.template, "name_format": c.name_format,
            "location": match c.location { CommandLocation::Current => "current", CommandLocation::Folder => "folder" },
            "folder": c.folder,
        })).collect::<Vec<_>>(),
    })
}

/// The search as `NAVIGATOR_SEARCH` answers it: its text, whether it is
/// an expression of tags and properties (and its joins), whether the box
/// is open, and the notes it lists.
fn search_state(s: &NavigatorState) -> Value {
    let q = s.query();
    json!({
        "query": s.search().text(),
        "expression": s.search().is_expression(),
        "join": q.terms.iter().map(|(j, _)| match j { Join::And => "and", Join::Or => "or" }).collect::<Vec<_>>(),
        "open": s.search_open,
        "files": s.list_order(),
    })
}

/// A shortcut as the actions answer it.
fn shortcut_json(at: usize, sc: &Shortcut) -> Value {
    json!({
        "n": at.saturating_add(1),
        "kind": sc.kind.id(),
        "target": sc.target,
        "value": sc.value,
        "name": sc.label(),
    })
}

/// The profiles, the one in use, what it hides, its shortcuts and its
/// recent notes.
fn profile_state(s: &NavigatorState) -> Value {
    let p = s.profile();
    json!({
        "profile": p.id,
        "profiles": s.profiles().all().map(|p| json!({ "id": p.id, "name": p.name })).collect::<Vec<_>>(),
        "show_hidden": s.show_hidden(),
        "file_visibility": p.file_visibility.id(),
        "hidden_folders": p.hidden_folders,
        "descendant_excluded": p.descendant_excluded,
        "hidden_tags": p.hidden_tags,
        "hidden_file_tags": p.hidden_file_tags,
        "hidden_file_names": p.hidden_file_names,
        "hidden_file_properties": p.hidden_file_properties,
        "banner": p.banner,
        "shortcuts": p.shortcuts.iter().enumerate().map(|(i, sc)| shortcut_json(i, sc)).collect::<Vec<_>>(),
        "recent": p.recent.iter().filter(|r| s.index().file(r).is_some()).collect::<Vec<_>>(),
    })
}

/// The shortcut an action names: `add` its kind (`note`, `folder`, `tag`,
/// `property`, `search`) with `path`, `folder`, `tag`, `property` (and
/// `value`), or `query` (and `name`); else the note selected, else the
/// folder, tag or property selected.
fn shortcut_of(args: &Value, s: &NavigatorState) -> Result<Shortcut, String> {
    let kind = text(args, "add").or_else(|| text(args, "kind"));
    let mut sc = match kind
        .map(|k| ShortcutKind::from_id(k).ok_or_else(|| format!("no shortcut kind {k}")))
        .transpose()?
    {
        Some(ShortcutKind::Note) => Shortcut::note(one_path(args, s)?),
        Some(ShortcutKind::Folder) => Shortcut::to(&NavTarget::Folder(
            text(args, "folder")
                .or_else(|| text(args, "path"))
                .unwrap_or("")
                .trim_matches('/')
                .to_owned(),
        )),
        Some(ShortcutKind::Tag | ShortcutKind::Property) => Shortcut::to(&target_of(args)?),
        Some(ShortcutKind::Search) => {
            let q = text(args, "query").ok_or("which search? (`query`)")?;
            Shortcut::search(text(args, "name").unwrap_or(q), q)
        }
        None => text(args, "path")
            .or_else(|| s.selection().cursor())
            .map_or_else(|| Shortcut::to(&s.target()), Shortcut::note),
    };
    if let (Some(name), false) = (text(args, "name"), sc.kind == ShortcutKind::Search) {
        name.clone_into(&mut sc.name);
    }
    Ok(sc)
}

/// A profile's hiding, banner and file visibility, from `set`.
fn set_profile(p: &mut VaultProfile, set: &Value) -> Result<(), String> {
    let list = |v: &Value| -> Vec<String> {
        v.as_array().map_or_else(
            || {
                v.as_str()
                    .map(|s| {
                        s.split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            },
            |a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            },
        )
    };
    for (key, v) in set.as_object().into_iter().flatten() {
        match key.as_str() {
            "hidden_folders" => p.hidden_folders = list(v),
            "descendant_excluded" => p.descendant_excluded = list(v),
            "hidden_tags" => p.hidden_tags = list(v),
            "hidden_file_tags" => p.hidden_file_tags = list(v),
            "hidden_file_names" => p.hidden_file_names = list(v),
            "hidden_file_properties" => p.hidden_file_properties = list(v),
            "banner" => {
                p.banner = v
                    .as_str()
                    .filter(|b| !b.trim().is_empty())
                    .map(str::to_owned);
            }
            "file_visibility" => {
                let id = v.as_str().unwrap_or_default();
                p.file_visibility = FileVisibility::from_id(id).ok_or_else(|| {
                    format!("file_visibility is documents, supported or all, not {id}")
                })?;
            }
            other => return Err(format!("a profile has no {other}")),
        }
    }
    Ok(())
}

/// A tag (`tag`) or a property (`property`, and `value`) an action is
/// about.
fn target_of(args: &Value) -> Result<NavTarget, String> {
    if let Some(t) = text(args, "tag") {
        return Ok(NavTarget::Tag(t.to_owned()));
    }
    if let Some(k) = text(args, "property") {
        return Ok(NavTarget::Property {
            key: architect_navigator::properties::fold(k),
            value: text(args, "value").map(architect_navigator::properties::value_path),
        });
    }
    Err("which? (`tag`, or `property` and `value`)".to_owned())
}

/// Does a write and says what changed: the outcome's ops, and the list.
fn write(nav: &Navigator, on_open: EventHandler<Open>, out: Outcome) -> Result<Value, String> {
    let ops = match &out {
        Outcome::Write { ops, .. } => format!("{ops:?}"),
        _ => String::new(),
    };
    let before = nav.notice.peek().clone();
    nav.act(Some(out), on_open);
    let after = nav.notice.peek().clone();
    if after != before
        && let Some(e) = after
    {
        return Err(e);
    }
    Ok(json!({ "done": ops, "state": state(&nav.state.peek()) }))
}

/// What `NAVIGATOR_TAGS` and `NAVIGATOR_PROPERTIES` say: the tag rows and
/// the property rows on show, with their counts.
#[must_use]
pub fn trees(s: &NavigatorState) -> Value {
    let mut tags = Vec::new();
    let mut properties = Vec::new();
    for r in s.nav_rows() {
        match &r.kind {
            NavRowKind::Tag {
                tag,
                name,
                expanded,
                has_children,
                count,
                ..
            } => tags.push(json!({
                "tag": tag, "name": name, "level": r.level, "open": expanded,
                "children": has_children, "notes": count.current, "below": count.descendants,
            })),
            NavRowKind::Property {
                key,
                value,
                name,
                expanded,
                has_children,
                count,
            } => properties.push(json!({
                "key": key, "value": value, "name": name, "level": r.level, "open": expanded,
                "children": has_children, "notes": count.current, "below": count.descendants,
            })),
            _ => {}
        }
    }
    json!({
        "tags": tags,
        "tagged": s.tag_tree().tagged,
        "untagged": s.tag_tree().untagged,
        "properties": properties,
        "search": s.search().text(),
    })
}

/// A day as `YYYY-MM-DD`.
fn date(text: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("`date` is a day as YYYY-MM-DD, not {text:?}"))
}

/// Where a note opens: `tab` true, a new tab.
fn how(args: &Value) -> OpenIn {
    if flag(args, "tab") == Some(true) {
        OpenIn::Tab
    } else {
        OpenIn::Here
    }
}

/// Opens or makes a periodic note, and says which: its `path`, whether it
/// is `new`, and whether the person is `asked` first (`confirm` false
/// does not ask).
fn periodic(
    nav: &Navigator,
    on_open: EventHandler<Open>,
    out: Result<Outcome, String>,
    args: &Value,
) -> Result<Value, String> {
    let out = out?;
    let (path, new) = match &out {
        Outcome::Open { path, .. } => (path.clone(), false),
        Outcome::Write {
            open: Some((path, _)),
            ..
        } => (path.clone(), true),
        _ => (String::new(), false),
    };
    let asked = new
        && flag(args, "confirm") != Some(false)
        && nav.state.peek().settings().calendar_confirm_create;
    if asked {
        nav.open_or_create(Ok(out), on_open);
    } else {
        nav.act(Some(out), on_open);
    }
    Ok(json!({ "path": path, "new": new, "asked": asked }))
}

fn header(h: &HeaderPeriod) -> Value {
    json!({
        "label": h.label, "note": h.note, "active": h.active, "enabled": h.enabled,
        "token": h.period.filter_token(),
    })
}

/// What `NAVIGATOR_CALENDAR` says: the month on show, week by week.
#[must_use]
pub fn calendar(s: &NavigatorState) -> Value {
    let g: CalendarGrid = s.calendar();
    let weeks: Vec<Value> = g
        .weeks
        .iter()
        .map(|w| {
            let days: Vec<Value> = w
                .days
                .iter()
                .filter(|d| !d.hidden)
                .map(|d| {
                    json!({
                        "date": d.date.to_string(), "in_month": d.in_month, "today": d.today,
                        "note": d.note, "image": d.image.is_some(), "unfinished": d.unfinished,
                        "active": d.active,
                    })
                })
                .collect();
            json!({
                "start": w.start.to_string(), "number": w.number, "note": w.note,
                "active": w.active, "days": days,
            })
        })
        .collect();
    json!({
        "shown": s.calendar_visible(),
        "cursor": g.cursor.to_string(),
        "today": s.today().to_string(),
        "month": header(&g.month),
        "quarter": header(&g.quarter),
        "year": header(&g.year),
        "weekdays": g.weekdays,
        "weeks": weeks,
        "active_period": s.active_period().map(|p| json!({ "kind": p.kind.id(), "start": p.start.to_string() })),
    })
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
        NavTarget::Property { key, value } => json!({ "property": key, "value": value }),
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
            ListRowKind::File {
                path,
                pinned,
                thumb,
                ..
            } => Some(json!({
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
        "search": s.search().text(),
        "profile": s.profile().id,
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
