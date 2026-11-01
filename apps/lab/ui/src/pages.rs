//! The lab's pages: a list per tab, an item, and the two sides — each
//! written the way an app writes a page on the shell.

use architect_shell::icon;
use architect_shell::{
    Action, Crumb, Menu, Page, ShellStyle, StatusStyle, Tool, Toolbar, use_shell,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    Building2, Check, CheckCheck, Circle, CircleCheck, FlaskConical, FolderInput, Hash, Inbox,
    Plus, Search, Trash2, Undo2,
};

use crate::items::{Item, PROJECTS};
use crate::screen::{Device, set_device};
use crate::{Lab, Route};

pub fn title(tab: &str) -> &'static str {
    match tab {
        "tasks" => "Tasks",
        "schedule" => "Schedule",
        "vault" => "Vault",
        _ => "Inbox",
    }
}

/// What a new one of `tab`'s is called, and what its field asks.
fn new_one(tab: &str) -> (&'static str, &'static str) {
    match tab {
        "tasks" => ("New task", "What needs doing?"),
        "schedule" => ("New event", "What's happening, and when?"),
        _ => ("Capture", "What's on your mind?"),
    }
}

/// The page's first crumb: the organization it is in.
fn org_crumb() -> Crumb {
    let name = architect_orgs::use_org()
        .and_then(|o| o.current())
        .map_or_else(|| "All organizations".to_owned(), |o| o.name);
    Crumb::new(name).icon(icon!(Building2))
}

/// The lab's own switches, in a menu of their own beside the page's:
/// the right side, the shell's style, the status line and (off a real
/// device) which device the window stands in for. Every choice is a check
/// at the row's start.
// `Lab` is a handful of signals, copied into every closure below.
#[allow(clippy::large_types_passed_by_value)]
fn lab_menu(lab: Lab) -> Menu {
    let style = (lab.style)();
    // The settings are the truth; the lab's signals follow them.
    let pick =
        move |s: ShellStyle| move |()| lab.put("workspace.shell", crate::settings::shell_id(s));
    let menu = Menu::new();
    let mut menu = menu
        .heading("View")
        .check("Details on the right", (lab.right)(), move |()| {
            lab.put_bool("workspace.details", !(lab.right)());
        })
        .heading("Shell")
        .check(
            "Floating pill",
            style == ShellStyle::FloatingPill,
            pick(ShellStyle::FloatingPill),
        )
        .check(
            "Tab bar",
            style == ShellStyle::TabBar,
            pick(ShellStyle::TabBar),
        )
        .check(
            "Rail only",
            style == ShellStyle::RailOnly,
            pick(ShellStyle::RailOnly),
        )
        .heading("Status")
        .check(
            "Bar along the foot",
            (lab.status)() == StatusStyle::Bar,
            move |()| lab.put("workspace.status", "bar"),
        )
        .check(
            "Badge in the corner",
            (lab.status)() == StatusStyle::Badge,
            move |()| lab.put("workspace.status", "badge"),
        );
    if !cfg!(any(target_os = "ios", target_os = "android")) {
        let device = consume_context::<Signal<Device>>()();
        menu = menu.divider().heading("Device");
        for d in Device::ALL {
            menu = menu.check(d.label(), d == device, move |()| set_device(d));
        }
    }
    menu
}

/// The lab's switches as a toolbar button.
#[allow(clippy::large_types_passed_by_value)]
fn lab_tool(lab: Lab) -> Tool {
    Tool::menu("Lab", icon!(FlaskConical), lab_menu(lab))
}

/// A list's order.
#[allow(clippy::large_types_passed_by_value)]
fn sort_menu(lab: Lab) -> Menu {
    let oldest = (lab.oldest)();
    let mut sort = lab.oldest;
    Menu::new()
        .heading("Sort")
        .check("Newest first", !oldest, move |()| sort.set(false))
        .check("Oldest first", oldest, move |()| sort.set(true))
}

/// Where an item can be filed: a project's tasks, the Inbox; all but
/// where it is (the Schedule wants a time, which a move cannot ask).
#[allow(clippy::large_types_passed_by_value)]
fn move_menu(lab: Lab, item: &Item) -> Menu {
    let id = item.id;
    let mut menu = Menu::new().heading("Tasks in");
    for p in PROJECTS {
        if !(item.tab == "tasks" && item.project == Some(p)) {
            menu = menu.item(p, icon!(Hash), move |()| {
                lab.move_item(id, "tasks", Some(p));
            });
        }
    }
    if item.tab != "inbox" {
        menu = menu.divider().item("Inbox", icon!(Inbox), move |()| {
            lab.move_item(id, "inbox", None);
        });
    }
    menu
}

/// A tab's page: its list (on Tasks, the picked project's). ⌃N and ⌃P
/// pick the next and previous row, ⌘↓ opens it and ⌘↵ marks it done.
#[component]
pub fn ListPage(tab: &'static str) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let project = if tab == "tasks" {
        (lab.project)()
    } else {
        None
    };
    let (new_label, _) = new_one(tab);
    // The page's action named, its order in ⋯, the lab's switches apart
    // (the search is the left side's, ⌘O).
    let mut toolbar = Toolbar::new();
    // The Inbox's add is the shell's own Capture button; another would be
    // a second way to say the same.
    if tab != "inbox" {
        toolbar = toolbar
            .separate(Action::new(new_label, icon!(Plus), move |()| lab.capture(tab)).with_label());
    }
    let toolbar = toolbar
        .joined([Tool::more(sort_menu(lab))])
        .separate(lab_tool(lab));
    let lists = lab.lists.read();
    let rows: Vec<Item> = lists
        .of(tab, project, (lab.oldest)())
        .into_iter()
        .cloned()
        .collect();
    drop(lists);
    let fresh = (lab.fresh)();
    let undone = (lab.undone)()
        .last()
        .filter(|u| (lab.notice)() && u.item.tab == tab)
        .cloned();
    let selected = (lab.selected)();
    let completing = (lab.completing)();
    let can_finish = tab != "schedule";
    let mut crumbs = vec![org_crumb()];
    if project.is_some() {
        crumbs.push(Crumb::new(title(tab)).on_press(move |()| lab.jump(Route::Tab(tab))));
    }
    rsx! {
        Page {
            title: project.unwrap_or_else(|| title(tab)),
            crumbs,
            toolbar,
            // The count, and the list's keys where they are needed.
            div { style: "display: flex; align-items: baseline; gap: 12px; padding-bottom: 10px; \
                          font-family: {t.font}; font-size: 13px; color: {t.ink_3};",
                span { style: "flex: 1;",
                    if rows.is_empty() {
                    } else if tab == "inbox" {
                        "{rows.len().saturating_sub(completing.len())} to sort"
                    } else {
                        {crate::count(rows.len().saturating_sub(completing.len()), "item")}
                    }
                }
                if !rows.is_empty() {
                    span { style: "white-space: nowrap;", "{keys_hint(can_finish)}" }
                }
            }
            // Done and Delete answer where they were asked, with the way back.
            if let Some(u) = undone {
                div { style: "display: flex; align-items: center; gap: 10px; margin-bottom: 10px; padding: 8px 8px 8px 14px; \
                              border-radius: 10px; background: {t.accent_soft}; font-family: {t.font}; font-size: 14px; color: {t.ink};",
                    span { style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                        "{u.said}"
                    }
                    button {
                        class: "lp",
                        style: "flex-shrink: 0; height: 30px; display: flex; align-items: center; gap: 6px; padding: 0 12px; \
                                border: none; border-radius: 8px; background: {t.pill}; color: {t.ink}; \
                                font-family: {t.font}; font-size: 13px; font-weight: 600;",
                        onclick: move |_| lab.undo(),
                        Undo2 { size: 14, color: t.ink }
                        "Undo"
                        span { style: "font-size: 12px; font-weight: 500; color: {t.ink_3};", "⌘Z" }
                    }
                }
            }
            if rows.is_empty() {
                Empty { tab, project }
            }
            // One sheet, its rows divided by hairlines.
            if !rows.is_empty() {
            div { style: "display: flex; flex-direction: column; border: 1px solid {t.rule}; border-radius: 12px; \
                          background: {t.sheet}; overflow: hidden;",
                for (n, item) in rows.into_iter().enumerate() {
                    Row {
                        key: "{item.id}",
                        first: n == 0,
                        fresh: fresh == Some(item.id),
                        name: item.name.clone(),
                        detail: if project.is_some() { item.detail.clone() } else { item.meta() },
                        selected: selected == Some(item.id),
                        done: completing.contains(&item.id),
                        // A click selects (the details follow), a double
                        // click or Return opens: Finder's and Mail's way.
                        on_press: move |()| {
                            let mut selected = lab.selected;
                            selected.set(Some(item.id));
                        },
                        on_open: move |()| {
                            let mut selected = lab.selected;
                            selected.set(Some(item.id));
                            lab.go(Route::Item(tab, item.id));
                        },
                        on_done: can_finish.then(|| EventHandler::new(move |()| lab.complete(item.id))),
                    }
                }
            }
            }
        }
    }
}

/// What a list's keys are, quietly over it.
const fn keys_hint(can_finish: bool) -> &'static str {
    if can_finish {
        "↑↓ select · ↵ open · ⌘↵ done · ⇧⌘M move"
    } else {
        "↑↓ select · ↵ open · ⇧⌘M move"
    }
}

/// An empty list: what goes in it, and the button that adds one.
#[component]
fn Empty(tab: &'static str, project: Option<&'static str>) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let (new_label, _) = new_one(tab);
    let what = project.map_or_else(
        || {
            if tab == "inbox" {
                "Inbox clear".to_owned()
            } else {
                format!("Nothing in {} yet", title(tab))
            }
        },
        |p| format!("No tasks in {p} yet"),
    );
    let then = if tab == "inbox" {
        "Nothing left to sort. What you capture next lands here."
    } else {
        "What you add here shows up at the top."
    };
    let clear = tab == "inbox" && project.is_none();
    rsx! {
        div { style: "display: flex; flex-direction: column; align-items: flex-start; gap: 10px; padding: 24px 0; \
                      font-family: {t.font};",
            if clear {
                CheckCheck { size: 28, color: t.accent }
            }
            span { style: "font-size: 16px; font-weight: 650; color: {t.ink};", "{what}" }
            span { style: "font-size: 14px; color: {t.ink_3};", "{then}" }
            button {
                class: "lp",
                style: "min-height: 34px; padding: 0 14px; border-radius: 9px; border: 1px solid {t.rule}; \
                        background: {t.pill}; color: {t.ink}; font-family: {t.font}; font-size: 14px;",
                onclick: move |_| lab.capture(tab),
                "{new_label}"
            }
        }
    }
}

/// One row: the circle that marks it done (where a list has done), then
/// its name over its detail, which a click selects and a double click
/// opens.
#[component]
fn Row(
    /// The sheet's first row (no hairline over it).
    first: bool,
    name: String,
    detail: String,
    fresh: bool,
    selected: bool,
    /// Being completed: the circle checked, the row fading out.
    done: bool,
    on_press: EventHandler<()>,
    on_open: EventHandler<()>,
    on_done: Option<EventHandler<()>>,
) -> Element {
    let t = use_shell().theme;
    let (name_ink, fade) = if done { (t.ink_3, 0.4) } else { (t.ink, 1.0) };
    // The row the keys act on: the accent's wash and its bar, the one mark
    // for where the keys are (a row added or put back is selected too).
    // A row just added that the keys have left: a quieter fill.
    let fill = if selected {
        t.accent_soft
    } else if fresh {
        t.pill
    } else {
        "transparent"
    };
    // Where the keys are: a bar along the row's start, not colour alone.
    let bar = if selected { t.accent } else { "transparent" };
    let rule = if first { "transparent" } else { t.rule };
    let lead = if on_done.is_some() { 0 } else { 16 };
    rsx! {
        div {
            style: "position: relative; display: flex; align-items: stretch; border-top: 1px solid {rule}; \
                    background: {fill}; opacity: {fade}; transition: opacity 300ms ease-out 120ms;",
            span { style: "position: absolute; top: 0; bottom: 0; left: 0; width: 3px; background: {bar};" }
            if let Some(on_done) = on_done {
                button {
                    class: "lp",
                    "aria-label": "Mark “{name}” done",
                    style: "flex-shrink: 0; width: 48px; display: flex; align-items: center; justify-content: center; \
                            border: none; background: transparent; padding: 0;",
                    onclick: move |_| on_done.call(()),
                    if done {
                        CircleCheck { size: 20, color: t.accent }
                    } else {
                        Circle { size: 20, color: t.ink_3 }
                    }
                }
            }
            button {
                class: "lp",
                style: "flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 4px; \
                        padding: 12px 16px 12px {lead}px; border: none; \
                        background: transparent; color: {t.ink}; font-family: {t.font}; text-align: left;",
                onclick: move |_| on_press.call(()),
                ondoubleclick: move |_| on_open.call(()),
                span { style: "font-size: 16px; font-weight: 650; color: {name_ink};", "{name}" }
                span { style: "font-size: 13px; color: {t.ink_3};", "{detail}" }
            }
        }
    }
}

/// One item: the bar has the way back and its actions, the page what it
/// is. An item since completed or deleted says so, with the way back.
#[component]
pub fn ItemPage(tab: &'static str, id: usize) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    // The note being written: empty each time (Blitz leaves a filled
    // field's caret at its start).
    let mut note = use_signal(String::new);
    let item = lab.lists.read().get(id).cloned();
    let crumbs = vec![
        org_crumb(),
        Crumb::new(title(tab)).on_press(move |()| lab.jump(Route::Tab(tab))),
    ];
    let on_back = if lab.can_go_back() {
        Some(EventHandler::new(move |()| lab.go_back()))
    } else {
        None
    };
    let Some(item) = item else {
        return rsx! {
            Page { title: "Not here any more", crumbs, on_back,
                div { style: "display: flex; flex-direction: column; align-items: flex-start; gap: 10px; \
                              font-family: {t.font};",
                    span { style: "font-size: 15px; color: {t.ink_2};",
                        "This was completed or deleted. Undo on the status line puts it back."
                    }
                    button {
                        class: "lp",
                        style: "min-height: 34px; padding: 0 14px; border-radius: 9px; border: 1px solid {t.rule}; \
                                background: {t.pill}; color: {t.ink}; font-family: {t.font}; font-size: 14px;",
                        onclick: move |_| lab.jump(Route::Tab(tab)),
                        "Back to {title(tab)}"
                    }
                }
            }
        };
    };
    let moves = move_menu(lab, &item);
    let menu = Menu::new()
        .item("Delete", icon!(Trash2), move |()| lab.remove(id, "Deleted"))
        .danger();
    // Done names itself: the page's one verb (⌘↵).
    let done =
        Action::new("Done", icon!(Check), move |()| lab.remove(id, "Completed")).with_label();
    rsx! {
        Page {
            title: item.name.clone(),
            crumbs,
            on_back,
            // Triage, the verb, the rest; the lab's switches apart.
            toolbar: Toolbar::new()
                .separate(Tool::menu("Move to", icon!(FolderInput), moves))
                .joined([Tool::action(done), Tool::more(menu)])
                .separate(lab_tool(lab)),
            div { style: "display: flex; flex-direction: column; gap: 14px; font-family: {t.font}; color: {t.ink};",
                span { style: "font-size: 22px; font-weight: 700; letter-spacing: -0.01em;", "{item.name}" }
                span { style: "font-size: 14px; color: {t.ink_3};", "{item.meta()}" }
                div { style: "height: 1px; background: {t.rule};" }
                span { style: "font-size: 13px; font-weight: 650; color: {t.ink_2};", "Notes" }
                if item.notes.is_empty() {
                    span { style: "font-size: 15px; line-height: 1.5; color: {t.ink_3};",
                        "None yet. What you write below stays with it."
                    }
                }
                for (n, line) in item.notes.iter().enumerate() {
                    span { key: "{n}", style: "font-size: 15px; line-height: 1.5;", "{line}" }
                }
                // Blitz draws no `placeholder`: the field says what it is by
                // its label.
                label { r#for: "lab-note", style: "font-size: 13px; color: {t.ink_3};", "Add a note (Return keeps it)" }
                input {
                    id: "lab-note",
                    r#type: "text",
                    value: "{note}",
                    style: "height: 40px; box-sizing: border-box; padding: 0 12px; border-radius: 10px; \
                            border: 1px solid {t.rule}; background: {t.pill}; color: {t.ink}; \
                            font-family: {t.font}; font-size: 15px;",
                    oninput: move |e: FormEvent| note.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            let text = note.peek().clone();
                            let mut lists = lab.lists;
                            if lists.write().note(id, &text) {
                                note.set(String::new());
                            }
                        }
                    },
                }
            }
        }
    }
}

/// A new item for `tab`: one line, Return keeps it, Esc lets it go.
#[component]
pub fn CapturePage(tab: &'static str) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let mut text = use_signal(String::new);
    let (heading, ask) = new_one(tab);
    let into = if tab == "inbox" {
        "the Inbox".to_owned()
    } else {
        (lab.project)().map_or_else(|| title(tab).to_owned(), str::to_owned)
    };
    let save = move || {
        let name = text.peek().clone();
        lab.add(tab, &name);
    };
    let empty = text.read().trim().is_empty();
    let on_back = if lab.can_go_back() {
        Some(EventHandler::new(move |()| lab.go_back()))
    } else {
        None
    };
    rsx! {
        Page {
            title: heading,
            crumbs: vec![org_crumb(), Crumb::new(title(tab)).on_press(move |()| lab.jump(Route::Tab(tab)))],
            on_back,
            toolbar: Toolbar::new().separate(Action::new("Add", icon!(Plus), move |()| save()).with_label()),
            div { style: "display: flex; flex-direction: column; gap: 10px; font-family: {t.font}; color: {t.ink};",
                // Blitz draws no `placeholder`: the question is a label.
                label { r#for: "lab-capture", style: "font-size: 14px; font-weight: 600; color: {t.ink_2};", "{ask}" }
                input {
                    id: "lab-capture",
                    r#type: "text",
                    autofocus: true,
                    value: "{text}",
                    style: "height: 44px; box-sizing: border-box; padding: 0 14px; border-radius: 10px; \
                            border: 1px solid {t.rule}; background: {t.pill}; color: {t.ink}; \
                            font-family: {t.font}; font-size: 16px;",
                    oninput: move |e: FormEvent| text.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            save();
                        } else if e.key() == Key::Escape {
                            // Esc lets it go, as everywhere on a Mac; Undo
                            // brings it back, so a stray Esc loses nothing.
                            let typed = text.peek().clone();
                            if typed.trim().is_empty() {
                                lab.go_back();
                            } else {
                                lab.drop_capture(tab, &typed);
                            }
                        }
                    },
                }
                span { style: "font-size: 13px; color: {t.ink_3};",
                    if empty { "Return adds it to {into}; Esc goes back." } else { "Return adds it to {into}; Esc drops it (⌘Z brings it back)." }
                }
            }
        }
    }
}

/// The left side: a search, the Inbox and the projects (the shell puts
/// the account at its foot). What is on show is marked.
#[component]
pub fn LeftSide() -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let commands = use_context::<architect_commands::Commands>();
    let route = (lab.route)();
    let project = (lab.project)();
    let lists = lab.lists.read();
    let inbox = lists.count("inbox");
    let counts = PROJECTS.map(|p| (p, lists.in_project(p)));
    drop(lists);
    let in_inbox = matches!(
        route,
        Route::Tab("inbox") | Route::Item("inbox", _) | Route::Capture("inbox")
    );
    rsx! {
        // Opens ⌘O's quick open: a field that is a button, as Linear's.
        button {
            class: "lp",
            style: "margin: 0 12px 10px; height: 38px; border-radius: 10px; background: {t.pill}; border: 1px solid {t.rule}; \
                    display: flex; align-items: center; justify-content: flex-start; gap: 8px; padding: 0 12px; \
                    font-family: {t.font}; text-align: left;",
            onclick: move |_| commands.show(architect_commands::Overlay::QuickOpen),
            Search { size: 16, color: t.ink_3 }
            span { style: "flex: 1; font-size: 14px; color: {t.ink_3};", "Search" }
            span { style: "font-size: 12px; color: {t.ink_3};", "⌘O" }
        }
        div { style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 8px; font-family: {t.font};",
            SideRow { label: "Inbox", count: inbox, on: in_inbox,
                icon: rsx! { Inbox { size: 18, color: if in_inbox { t.ink } else { t.ink_3 } } },
                on_press: move |()| lab.jump(Route::Tab("inbox")) }
            span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3}; padding: 14px 12px 6px;", "Projects" }
            for (p, n) in counts {
                SideRow { key: "{p}", label: p, count: n, on: project == Some(p),
                    icon: rsx! { Hash { size: 18, color: if project == Some(p) { t.ink } else { t.ink_3 } } },
                    on_press: move |()| lab.show_project(p) }
            }
        }
    }
}

#[component]
fn SideRow(
    label: &'static str,
    count: usize,
    on: bool,
    icon: Element,
    on_press: EventHandler<()>,
) -> Element {
    let t = use_shell().theme;
    let (bg, colour, weight) = if on {
        (t.active, t.ink, 650)
    } else {
        ("transparent", t.ink_2, 500)
    };
    rsx! {
        button {
            class: "lp",
            style: "height: 40px; display: flex; align-items: center; justify-content: flex-start; gap: 12px; \
                    padding: 0 12px; border: none; border-radius: 10px; background: {bg}; color: {colour}; \
                    font-family: {t.font}; font-size: 15px; font-weight: {weight}; text-align: left;",
            onclick: move |_| on_press.call(()),
            {icon}
            span { style: "flex: 1; text-align: left;", "{label}" }
            span { style: "font-size: 12px; color: {t.ink_3};", "{count}" }
        }
    }
}

/// The right side: the selection's details, and what can be done with it
/// (each a command, its key beside it).
#[component]
pub fn RightSide() -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let commands = use_context::<architect_commands::Commands>();
    let on_list = matches!((lab.route)(), Route::Tab(_));
    // The item on show, or the list's selected row.
    let item = match (lab.route)() {
        Route::Item(_, id) => lab.lists.read().get(id).cloned(),
        Route::Tab(_) => (lab.selected)().and_then(|id| lab.lists.read().get(id).cloned()),
        Route::Capture(_) | Route::Help(_) => None,
    };
    let Some(item) = item else {
        return rsx! {
            div { style: "display: flex; flex-direction: column; gap: 8px; padding: 12px 20px; font-family: {t.font};",
                span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3};", "Details" }
                span { style: "font-size: 15px; font-weight: 650; color: {t.ink_2};", "Nothing selected" }
                span { style: "font-size: 13px; line-height: 1.5; color: {t.ink_3};",
                    "Click a row, or press ↓, to see where it is, when, and whose it is."
                }
                span { style: "font-size: 13px; line-height: 1.5; color: {t.ink_3};",
                    "Every key: ⌘/ (Help)"
                }
            }
        };
    };
    // On a list, what the row can do (an item's page has them in its
    // bar).
    let mut actions: Vec<(&'static str, &'static str, &'static str)> = Vec::new();
    if on_list {
        actions.push(("Open", "↵", "LAB_OPEN_SELECTED"));
        if item.tab != "schedule" {
            actions.push(("Done", "⌘↵", "LAB_DONE"));
        }
        actions.push(("Move to…", "⇧⌘M", "LAB_MOVE"));
    }
    // Where it is, as one line: "Inbox", "Tasks · Signal".
    let place = item.project.map_or_else(
        || title(item.tab).to_owned(),
        |p| format!("{} · {p}", title(item.tab)),
    );
    let rows = [
        ("In", place),
        ("When", item.detail.clone()),
        ("Owner", "You".to_owned()),
    ];
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; padding: 12px 20px; font-family: {t.font}; color: {t.ink};",
            span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3};", "Details" }
            span { style: "font-size: 18px; font-weight: 700;", "{item.name}" }
            for (k, v) in rows {
                div { key: "{k}", style: "display: flex; justify-content: space-between; gap: 12px; font-size: 14px; \
                                          padding: 10px 0; border-bottom: 1px solid {t.rule};",
                    span { style: "color: {t.ink_3};", "{k}" }
                    span { style: "text-align: right;", "{v}" }
                }
            }
            div { style: "display: flex; flex-direction: column; gap: 6px; padding-top: 6px;",
                for (label, key, id) in actions {
                    button {
                        key: "{id}",
                        class: "lp",
                        style: "height: 36px; display: flex; align-items: center; gap: 10px; padding: 0 12px; \
                                border-radius: 9px; border: 1px solid {t.rule}; background: {t.pill}; color: {t.ink}; \
                                font-family: {t.font}; font-size: 14px; text-align: left;",
                        onclick: move |_| {
                            let _ = commands.run(id);
                        },
                        span { style: "flex: 1; text-align: left;", "{label}" }
                        span { style: "font-size: 12px; color: {t.ink_3};", "{key}" }
                    }
                }
            }
        }
    }
}

/// The keys, by where they work.
const KEYS: [(&str, &[(&str, &str)]); 3] = [
    (
        "In a list",
        &[
            ("↑ ↓", "Select a row (⌃N, ⌃P too)"),
            ("↵", "Open it (or double-click)"),
            ("⌘↵", "Mark it done"),
            ("⇧⌘M", "Move it to a project or the Inbox"),
        ],
    ),
    (
        "Anywhere",
        &[
            ("⇧⌘N", "Capture into the Inbox"),
            ("⌘Z", "Undo the last Done, Move, Delete or Capture"),
            ("⇧⌘Z", "Redo it"),
            ("⌘K", "Every command, by name"),
            ("⌘O", "Open anything"),
            ("⌃⌘1–4", "Inbox, Tasks, Schedule, Vault"),
            ("⌘/", "This page"),
        ],
    ),
    (
        "In a field or the palette",
        &[
            ("↵", "Add it, or run the row picked"),
            (
                "Esc",
                "Close; a capture dropped with text comes back with ⌘Z",
            ),
        ],
    ),
];

/// Where things are, a line each.
const PLACES: [(&str, &str); 5] = [
    (
        "Inbox",
        "Whatever you capture lands here, to sort: file it in a project (⇧⌘M) or mark it done.",
    ),
    (
        "Projects",
        "On the left. A project's tasks are the Tasks tab, narrowed to it.",
    ),
    (
        "Organization",
        "The switcher at the left side's foot (Personal) picks whose work is on show.",
    ),
    (
        "The flask",
        "The lab's own switches: the shell's style, the status line and the device the window stands in for.",
    ),
    (
        "Vault",
        "A workspace of panes, Obsidian's way: drag a tab to a pane's edge to split it.",
    ),
];

/// The help: the keys, where things are, and what does not work yet.
#[component]
pub fn HelpPage() -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let on_back = if lab.can_go_back() {
        Some(EventHandler::new(move |()| lab.go_back()))
    } else {
        None
    };
    rsx! {
        Page { title: "Keys and help", crumbs: vec![org_crumb()], on_back,
            // Clear of the floating Capture button at the foot.
            div { style: "display: flex; flex-direction: column; gap: 24px; max-width: 640px; padding-bottom: 88px; \
                          font-family: {t.font}; color: {t.ink};",
                for (heading, keys) in KEYS {
                    div { key: "{heading}", style: "display: flex; flex-direction: column;",
                        span { style: "font-size: 13px; font-weight: 650; color: {t.ink_2}; padding-bottom: 6px;", "{heading}" }
                        for (key, what) in keys.iter().copied() {
                            div { key: "{key}{what}",
                                style: "display: flex; align-items: flex-start; gap: 16px; padding: 8px 0; border-top: 1px solid {t.rule}; font-size: 14px;",
                                span { style: "flex-shrink: 0; width: 64px; font-weight: 650;", "{key}" }
                                span { style: "color: {t.ink_2};", "{what}" }
                            }
                        }
                    }
                }
                div { style: "display: flex; flex-direction: column;",
                    span { style: "font-size: 13px; font-weight: 650; color: {t.ink_2}; padding-bottom: 6px;", "Where things are" }
                    for (place, what) in PLACES {
                        div { key: "{place}",
                            style: "display: flex; align-items: flex-start; gap: 16px; padding: 8px 0; border-top: 1px solid {t.rule}; font-size: 14px;",
                            span { style: "flex-shrink: 0; width: 104px; font-weight: 650;", "{place}" }
                            span { style: "color: {t.ink_2}; line-height: 1.45;", "{what}" }
                        }
                    }
                }
                div { style: "display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; border-radius: 10px; \
                              border: 1px solid {t.rule}; font-size: 14px; line-height: 1.45;",
                    span { style: "font-weight: 650;", "Not yet: screen readers" }
                    span { style: "color: {t.ink_2};",
                        "VoiceOver cannot read this window yet: the renderer gives it no names. Everything here works from the keys above."
                    }
                }
            }
        }
    }
}
