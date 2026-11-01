//! The lab's pages: a list per tab, an item, and the two sides — each
//! written the way an app writes a page on the shell.

use architect_shell::icon;
use architect_shell::{
    Action, Crumb, FormFactor, Menu, Page, ShellStyle, StatusStyle, Tool, Toolbar, use_shell,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowUpDown, Check, Copy, Funnel, Hash, Inbox, Plus, Search, Share, Smartphone, Trash2,
};

use crate::screen::{Device, set_device};
use crate::{Lab, Route};

const INK: &str = "#e4e4e7";
const INK_2: &str = "#a1a1aa";
const INK_3: &str = "#80808a";
const SHEET_2: &str = "#16161a";
const RULE: &str = "#222228";
const FONT: &str = "-apple-system, 'SF Pro Text', system-ui, 'Segoe UI', Roboto, sans-serif";

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
fn lab_menu(menu: Menu, lab: Lab) -> Menu {
    let style = (lab.style)();
    let pick = move |s: ShellStyle| {
        move |()| {
            let mut style = lab.style;
            style.set(s);
        }
    };
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
            let mut right = lab.right;
            right.toggle();
        })
        .heading("Status")
        .check(
            "Bar along the foot",
            (lab.status)() == StatusStyle::Bar,
            move |()| {
                let mut s = lab.status;
                s.set(StatusStyle::Bar);
            },
        )
        .check(
            "Badge in the corner",
            (lab.status)() == StatusStyle::Badge,
            move |()| {
                let mut s = lab.status;
                s.set(StatusStyle::Badge);
            },
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
    let lab = use_context::<Lab>();
    let shell = use_shell();
    let form = match shell.form() {
        FormFactor::Phone => "Phone",
        FormFactor::Tablet => "Tablet",
        FormFactor::Desk => "Desk",
    };
    let screen = shell.screen;
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
            span { style: "font-size: 13px; color: {INK_3}; font-family: {FONT}; padding-bottom: 10px;",
                "{screen.width:.0} × {screen.height:.0} · {form}"
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
    rsx! {
        button {
            class: "lp",
            style: "display: flex; flex-direction: column; align-items: flex-start; gap: 4px; padding: 14px 16px; \
                    border-radius: 14px; border: 1px solid {RULE}; background: #0e0e11; color: {INK}; \
                    font-family: {FONT}; text-align: left;",
            onclick: move |_| on_press.call(()),
            span { style: "font-size: 16px; font-weight: 650;", "{name}" }
            span { style: "font-size: 13px; color: {INK_3};", "{detail}" }
        }
    }
}

/// One item: the bar has the way back, the page says what it is.
#[component]
pub fn ItemPage(tab: &'static str, index: usize) -> Element {
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
            div { style: "display: flex; flex-direction: column; gap: 14px; font-family: {FONT}; color: {INK};",
                span { style: "font-size: 14px; color: {INK_3};", "{detail} · in {title(tab)}" }
                div { style: "height: 1px; background: {RULE};" }
                span { style: "font-size: 15px; line-height: 1.5; color: #c4c4cc;",
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
    let projects = [
        ("Studio", 12),
        ("Harbor & Pine", 4),
        ("Album release", 9),
        ("Signal", 3),
        ("Admin", 2),
    ];
    let mut here = use_signal(|| "Studio");
    rsx! {
        div { style: "margin: 0 12px 10px; height: 38px; border-radius: 10px; background: {SHEET_2}; border: 1px solid {RULE}; \
                      display: flex; align-items: center; gap: 8px; padding: 0 12px; font-family: {FONT};",
            Search { size: 16, color: INK_3 }
            span { style: "font-size: 14px; color: {INK_3};", "Search" }
        }
        div { style: "display: flex; flex-direction: column; gap: 2px; padding: 4px 8px; font-family: {FONT};",
            SideRow { label: "Inbox", count: 5, on: false, icon: rsx! { Inbox { size: 18, color: INK_2 } }, on_press: |()| {} }
            span { style: "font-size: 12px; font-weight: 650; color: {INK_3}; padding: 14px 12px 6px;", "Projects" }
            for (p, n) in projects {
                SideRow { key: "{p}", label: p, count: n, on: here() == p,
                    icon: rsx! { Hash { size: 18, color: if here() == p { INK } else { INK_3 } } },
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
    let (bg, colour, weight) = if on {
        ("rgba(255,255,255,0.08)", INK, 650)
    } else {
        ("transparent", INK_2, 500)
    };
    rsx! {
        button {
            class: "lp",
            style: "height: 40px; display: flex; align-items: center; justify-content: flex-start; gap: 12px; \
                    padding: 0 12px; border: none; border-radius: 10px; background: {bg}; color: {colour}; \
                    font-family: {FONT}; font-size: 15px; font-weight: {weight}; text-align: left;",
            onclick: move |_| on_press.call(()),
            {icon}
            span { style: "flex: 1; text-align: left;", "{label}" }
            span { style: "font-size: 12px; color: {INK_3};", "{count}" }
        }
    }
}

/// The right side: the selection's details.
#[component]
pub fn RightSide() -> Element {
    let lab = use_context::<Lab>();
    let route = (lab.route)();
    let what = match route {
        Route::Item(tab, i) => items(tab).get(i).map_or("Nothing selected", |(n, _)| n),
        Route::Tab(_) => "Nothing selected",
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; padding: 12px 20px; font-family: {FONT}; color: {INK};",
            span { style: "font-size: 12px; font-weight: 650; color: {INK_3};", "Details" }
            span { style: "font-size: 18px; font-weight: 700;", "{what}" }
            for (k, v) in [("Status", "In progress"), ("Project", "Studio"), ("Due", "Friday"), ("Owner", "Cody")] {
                div { key: "{k}", style: "display: flex; justify-content: space-between; font-size: 14px; padding: 10px 0; \
                                          border-bottom: 1px solid {RULE};",
                    span { style: "color: {INK_3};", "{k}" }
                    span { "{v}" }
                }
            }
        }
    }
}
