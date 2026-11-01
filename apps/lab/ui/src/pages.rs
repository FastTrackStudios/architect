//! The lab's pages: a list per tab, an item, and the two sides — each
//! written the way an app writes a page on the shell.

use architect_shell::icon;
use architect_shell::{
    Action, Crumb, Menu, Page, ShellStyle, StatusStyle, Tool, Toolbar, use_shell,
};
use dioxus::prelude::*;
use lucide_dioxus::{Building2, Check, Copy, Funnel, Hash, Inbox, Plus, Search, Share, Trash2};

use crate::items::{Item, PROJECTS};
use crate::screen::{Device, set_device};
use crate::{Lab, Route};

fn title(tab: &str) -> &'static str {
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

/// What every lab page adds to its ⋯: the order of the list, the right
/// side, the shell's style and (off a real device) which device the window
/// stands in for. Every choice is a check at the row's start.
// `Lab` is a handful of signals, copied into every closure below.
#[allow(clippy::large_types_passed_by_value)]
fn lab_menu(menu: Menu, lab: Lab, sorts: bool) -> Menu {
    let style = (lab.style)();
    let oldest = (lab.oldest)();
    let mut sort = lab.oldest;
    // The settings are the truth; the lab's signals follow them.
    let pick =
        move |s: ShellStyle| move |()| lab.put("workspace.shell", crate::settings::shell_id(s));
    let mut menu = menu;
    if sorts {
        menu = menu
            .heading("Sort")
            .check("Newest first", !oldest, move |()| sort.set(false))
            .check("Oldest first", oldest, move |()| sort.set(true))
            .divider();
    }
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

/// A tab's page: its list (on Tasks, the picked project's).
#[component]
pub fn ListPage(tab: &'static str) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let project = if tab == "tasks" {
        (lab.project)()
    } else {
        None
    };
    // T3 Code's: the scope and the search each on its own, the page's
    // actions joined in one pill.
    let mut scope = use_signal(|| "All");
    let pick = move |s: &'static str| move |()| scope.set(s);
    let scope_menu = Menu::new()
        .heading("Show")
        .check("All", scope() == "All", pick("All"))
        .check("Mine", scope() == "Mine", pick("Mine"))
        .check("Flagged", scope() == "Flagged", pick("Flagged"));
    let (new_label, _) = new_one(tab);
    let toolbar = Toolbar::new()
        .separate(
            Tool::menu("Scope", icon!(Funnel), scope_menu).badge(usize::from(scope() != "All")),
        )
        .separate(Action::new("Search", icon!(Search), |()| {}))
        .joined([
            Tool::action(Action::new(new_label, icon!(Plus), move |()| {
                lab.capture(tab);
            })),
            Tool::more(lab_menu(Menu::new(), lab, true)),
        ]);
    let lists = lab.lists.read();
    let rows: Vec<Item> = lists
        .of(tab, project, (lab.oldest)())
        .into_iter()
        .cloned()
        .collect();
    drop(lists);
    let fresh = (lab.fresh)();
    let mut crumbs = vec![org_crumb()];
    if project.is_some() {
        crumbs.push(Crumb::new(title(tab)).on_press(move |()| lab.jump(Route::Tab(tab))));
    }
    rsx! {
        Page {
            title: project.unwrap_or_else(|| title(tab)),
            crumbs,
            toolbar,
            span { style: "font-size: 13px; color: {t.ink_3}; font-family: {t.font}; padding-bottom: 10px;",
                {crate::count(rows.len(), "item")}
            }
            if rows.is_empty() {
                Empty { tab, project }
            }
            div { style: "display: flex; flex-direction: column; gap: 10px;",
                for item in rows {
                    Row {
                        key: "{item.id}",
                        fresh: fresh == Some(item.id),
                        name: item.name.clone(),
                        detail: if project.is_some() { item.detail.clone() } else { item.meta() },
                        on_press: move |()| lab.go(Route::Item(tab, item.id)),
                    }
                }
            }
        }
    }
}

/// An empty list: what goes in it, and the button that adds one.
#[component]
fn Empty(tab: &'static str, project: Option<&'static str>) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let (new_label, _) = new_one(tab);
    let what = project.map_or_else(
        || format!("Nothing in {} yet", title(tab)),
        |p| format!("No tasks in {p} yet"),
    );
    rsx! {
        div { style: "display: flex; flex-direction: column; align-items: flex-start; gap: 10px; padding: 24px 0; \
                      font-family: {t.font};",
            span { style: "font-size: 16px; font-weight: 650; color: {t.ink};", "{what}" }
            span { style: "font-size: 14px; color: {t.ink_3};", "What you add here shows up at the top." }
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

#[component]
fn Row(name: String, detail: String, fresh: bool, on_press: EventHandler<()>) -> Element {
    let t = use_shell().theme;
    // The row just added: the accent's edge until the lab moves on.
    let edge = if fresh { t.accent } else { t.rule };
    rsx! {
        button {
            class: "lp",
            style: "display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 14px 16px; \
                    border-radius: 14px; border: 1px solid {edge}; background: {t.sheet}; color: {t.ink}; \
                    font-family: {t.font}; text-align: left;",
            onclick: move |_| on_press.call(()),
            span { style: "font-size: 16px; font-weight: 650;", "{name}" }
            span { style: "font-size: 13px; color: {t.ink_3};", "{detail}" }
        }
    }
}

/// One item: the bar has the way back and its actions, the page what it
/// is. An item since completed or deleted says so, with the way back.
#[component]
pub fn ItemPage(tab: &'static str, id: usize) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
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
    let menu = Menu::new()
        .item("Duplicate", icon!(Copy), |()| {})
        .divider()
        .item("Delete", icon!(Trash2), move |()| lab.remove(id, "Deleted"))
        .danger()
        .divider();
    rsx! {
        Page {
            title: item.name.clone(),
            crumbs,
            on_back,
            // T3 Code's thread: three buttons in one pill.
            toolbar: Toolbar::new().joined([
                Tool::action(Action::new("Share", icon!(Share), |()| {})),
                Tool::action(Action::new("Done", icon!(Check), move |()| lab.remove(id, "Completed"))),
                Tool::more(lab_menu(menu, lab, false)),
            ]),
            div { style: "display: flex; flex-direction: column; gap: 14px; font-family: {t.font}; color: {t.ink};",
                span { style: "font-size: 14px; color: {t.ink_3};", "{item.meta()}" }
                div { style: "height: 1px; background: {t.rule};" }
                span { style: "font-size: 15px; line-height: 1.5; color: {t.ink_3};", "No notes yet." }
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
            toolbar: Toolbar::new().joined([Tool::action(Action::new("Save", icon!(Check), move |()| save()))]),
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
                            lab.go_back();
                        }
                    },
                }
                span { style: "font-size: 13px; color: {t.ink_3};",
                    if empty { "Return adds it to {into}; Esc lets it go." } else { "Return adds it to {into}." }
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
        div { style: "margin: 0 12px 10px; height: 38px; border-radius: 10px; background: {t.pill}; border: 1px solid {t.rule}; \
                      display: flex; align-items: center; gap: 8px; padding: 0 12px; font-family: {t.font};",
            Search { size: 16, color: t.ink_3 }
            span { style: "font-size: 14px; color: {t.ink_3};", "Search" }
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

/// The right side: the selection's details.
#[component]
pub fn RightSide() -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let item = match (lab.route)() {
        Route::Item(_, id) => lab.lists.read().get(id).cloned(),
        _ => None,
    };
    let Some(item) = item else {
        return rsx! {
            div { style: "display: flex; flex-direction: column; gap: 8px; padding: 12px 20px; font-family: {t.font};",
                span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3};", "Details" }
                span { style: "font-size: 15px; font-weight: 650; color: {t.ink_2};", "Nothing selected" }
                span { style: "font-size: 13px; line-height: 1.5; color: {t.ink_3};",
                    "Open an item to see its project, its date and who has it."
                }
            }
        };
    };
    let rows = [
        ("In", title(item.tab).to_owned()),
        ("Project", item.project.unwrap_or("None").to_owned()),
        ("When", item.detail.clone()),
        ("Owner", "Cody".to_owned()),
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
        }
    }
}
