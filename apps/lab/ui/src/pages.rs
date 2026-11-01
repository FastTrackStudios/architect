//! The lab's pages: a list per tab, an item, and the two sides — each
//! written the way an app writes a page on the shell.

use architect_shell::icon;
use architect_shell::{
    Action, Crumb, Menu, Page, ShellStyle, StatusStyle, Tool, Toolbar, use_shell,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowUpDown, Check, Copy, Funnel, Hash, Inbox, Plus, Search, Share, Smartphone, Trash2,
};

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

fn items(tab: &str) -> &'static [(&'static str, &'static str)] {
    match tab {
        "tasks" => &[
            ("Mix the Sunday set", "Studio · due Friday"),
            ("Send stems to Jess", "Harbor & Pine · today"),
            ("Book the Grey Eagle", "Album release · next week"),
            ("Rewire the keys rig", "Signal · someday"),
            ("Chart Late Train in C", "Charts · Thursday"),
            ("Renew the Task plan", "Admin · Nov 3"),
        ],
        "schedule" => &[
            ("Rehearsal", "Today · 7:30 PM · Crossroads"),
            ("Mix review with Sam", "Tomorrow · 11:00"),
            ("Sunday Night Worship", "Sun · 6:00 PM"),
            ("Album Release Show", "Sat Nov 14 · 8:00 PM"),
        ],
        "vault" => &[
            ("Setlist ideas", "Edited 2h ago"),
            ("Gear list", "Edited yesterday"),
            ("Lyrics — Paper Boats", "Edited Oct 22"),
            ("Shell design notes", "Edited Oct 30"),
        ],
        _ => &[
            ("Call back the venue", "Captured 10 min ago"),
            ("Idea: a capo chart view", "Captured 1h ago"),
            ("Bass DI is buzzing", "Captured yesterday"),
            ("Ask Cody about the click", "Captured yesterday"),
            ("Swap the opener and closer", "Captured Oct 28"),
        ],
    }
}

/// What every lab page adds to its ⋯: the shell's style, the right side,
/// and (off a real device) which device the window stands in for.
// `Lab` is a handful of signals, copied into every closure below.
#[allow(clippy::large_types_passed_by_value)]
fn lab_menu(menu: Menu, lab: Lab) -> Menu {
    let style = (lab.style)();
    // The settings are the truth; the lab's signals follow them.
    let pick =
        move |s: ShellStyle| move |()| lab.put("workspace.shell", crate::settings::shell_id(s));
    let mut menu = menu
        .divider()
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
        .check("Right side", (lab.right)(), move |()| {
            lab.put_bool("workspace.details", !(lab.right)());
        })
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
            menu = menu.item(d.label(), icon!(Smartphone), move |()| set_device(d));
            if d == device {
                menu = menu.note("✓");
            }
        }
    }
    menu
}

/// A tab's page: its list.
#[component]
pub fn ListPage(tab: &'static str) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let menu = Menu::new()
        .item("Sort", icon!(ArrowUpDown), |()| {})
        .note("Newest");
    // T3 Code's: the scope and the search each on its own, the page's
    // actions joined in one pill.
    let mut scope = use_signal(|| "All");
    let pick = move |s: &'static str| move |()| scope.set(s);
    let scope_menu = Menu::new()
        .heading("Show")
        .check("All", scope() == "All", pick("All"))
        .check("Mine", scope() == "Mine", pick("Mine"))
        .check("Flagged", scope() == "Flagged", pick("Flagged"));
    let toolbar = Toolbar::new()
        .separate(
            Tool::menu("Scope", icon!(Funnel), scope_menu).badge(usize::from(scope() != "All")),
        )
        .separate(Action::new("Search", icon!(Search), |()| {}))
        .joined([
            Tool::action(Action::new("New", icon!(Plus), move |()| {
                lab.go(Route::Item(tab, 0));
            })),
            Tool::more(lab_menu(menu, lab)),
        ]);
    rsx! {
        Page {
            title: title(tab),
            crumbs: vec![Crumb::new("FastTrackStudio").icon(icon!(Hash))],
            toolbar,
            span { style: "font-size: 13px; color: {t.ink_3}; font-family: {t.font}; padding-bottom: 10px;",
                "{items(tab).len()} items"
            }
            div { style: "display: flex; flex-direction: column; gap: 10px;",
                for (i, (name, detail)) in items(tab).iter().enumerate() {
                    Row { key: "{i}", name, detail, on_press: move |()| lab.go(Route::Item(tab, i)) }
                }
            }
        }
    }
}

#[component]
fn Row(name: &'static str, detail: &'static str, on_press: EventHandler<()>) -> Element {
    let t = use_shell().theme;
    rsx! {
        button {
            class: "lp",
            style: "display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 14px 16px; \
                    border-radius: 14px; border: 1px solid {t.rule}; background: {t.sheet}; color: {t.ink}; \
                    font-family: {t.font}; text-align: left;",
            onclick: move |_| on_press.call(()),
            span { style: "font-size: 16px; font-weight: 650;", "{name}" }
            span { style: "font-size: 13px; color: {t.ink_3};", "{detail}" }
        }
    }
}

/// One item: the bar has the way back, the page says what it is.
#[component]
pub fn ItemPage(tab: &'static str, index: usize) -> Element {
    let t = use_shell().theme;
    let lab = use_context::<Lab>();
    let (name, detail) = items(tab)
        .get(index)
        .copied()
        .unwrap_or(("New item", "Just now"));
    let menu = Menu::new()
        .item("Duplicate", icon!(Copy), |()| {})
        .divider()
        .item("Delete", icon!(Trash2), move |()| lab.go_back())
        .danger();
    rsx! {
        Page {
            title: name,
            crumbs: vec![
                Crumb::new("FastTrackStudio").icon(icon!(Hash)),
                Crumb::new(title(tab)).on_press(move |()| lab.jump(Route::Tab(tab))),
            ],
            on_back: if lab.can_go_back() { Some(EventHandler::new(move |()| lab.go_back())) } else { None },
            // T3 Code's thread: three buttons in one pill.
            toolbar: Toolbar::new().joined([
                Tool::action(Action::new("Share", icon!(Share), |()| {})),
                Tool::action(Action::new("Done", icon!(Check), move |()| lab.go_back())),
                Tool::more(lab_menu(menu, lab)),
            ]),
            div { style: "display: flex; flex-direction: column; gap: 14px; font-family: {t.font}; color: {t.ink};",
                span { style: "font-size: 14px; color: {t.ink_3};", "{detail} · in {title(tab)}" }
                div { style: "height: 1px; background: {t.rule};" }
                span { style: "font-size: 15px; line-height: 1.5; color: {t.ink_2};",
                    "Swipe right for the projects, left for the details. Pull down from the top for the vault. \
                     The ⋯ switches the shell's style, and on a desk the device this window stands in for."
                }
            }
        }
    }
}

/// The left side: a search and the projects (the shell puts the account
/// at its foot).
#[component]
pub fn LeftSide() -> Element {
    let t = use_shell().theme;
    let projects = [
        ("Studio", 12),
        ("Harbor & Pine", 4),
        ("Album release", 9),
        ("Signal", 3),
        ("Admin", 2),
    ];
    let mut here = use_signal(|| "Studio");
    rsx! {
        div { style: "margin: 0 12px 10px; height: 38px; border-radius: 10px; background: {t.pill}; border: 1px solid {t.rule}; \
                      display: flex; align-items: center; gap: 8px; padding: 0 12px; font-family: {t.font};",
            Search { size: 16, color: t.ink_3 }
            span { style: "font-size: 14px; color: {t.ink_3};", "Search" }
        }
        div { style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 8px; font-family: {t.font};",
            SideRow { label: "Inbox", count: 5, on: false, icon: rsx! { Inbox { size: 18, color: t.ink_2 } }, on_press: |()| {} }
            span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3}; padding: 14px 12px 6px;", "Projects" }
            for (p, n) in projects {
                SideRow { key: "{p}", label: p, count: n, on: here() == p,
                    icon: rsx! { Hash { size: 18, color: if here() == p { t.ink } else { t.ink_3 } } },
                    on_press: move |()| here.set(p) }
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
    let route = (lab.route)();
    let what = match route {
        Route::Item(tab, i) => items(tab).get(i).map_or("Nothing selected", |(n, _)| n),
        Route::Tab(_) => "Nothing selected",
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; padding: 12px 20px; font-family: {t.font}; color: {t.ink};",
            span { style: "font-size: 12px; font-weight: 650; color: {t.ink_3};", "Details" }
            span { style: "font-size: 18px; font-weight: 700;", "{what}" }
            for (k, v) in [("Status", "In progress"), ("Project", "Studio"), ("Due", "Friday"), ("Owner", "Cody")] {
                div { key: "{k}", style: "display: flex; justify-content: space-between; font-size: 14px; padding: 10px 0; \
                                          border-bottom: 1px solid {t.rule};",
                    span { style: "color: {t.ink_3};", "{k}" }
                    span { "{v}" }
                }
            }
        }
    }
}
