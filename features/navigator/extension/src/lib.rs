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
    CalendarGrid, Grouping, HeaderPeriod, ListMode, ListRowKind, MemorySource, NavTarget,
    NavigatorSource, NavigatorState, NoteFile, OpenIn, Orientation, Outcome, Pane, PeriodKind,
    PropertyValue, SortOption,
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
        "NAVIGATOR_COLOUR" => {
            let target =
                text(args, "target").ok_or("`target`: tag:<tag> or property:<key>[=<value>]")?;
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
            periodic(nav, on_open, out, args)
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
            periodic(nav, on_open, out, args)
        }
        other => Err(format!("Navigator has no action {other}")),
    }
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
    nav: Navigator,
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
