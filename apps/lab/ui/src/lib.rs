//! The app shell's lab: a made-up app on [`architect_shell`] that
//! `apps/architect-desktop` and `apps/architect-mobile` mount, so the shell
//! can be designed on a phone, a tablet and a desk with `dx serve`'s hot
//! reload — an `rsx!` edit here or in `architect-shell` shows in the running
//! window at once.
//!
//! The app is Task's shape (the shell's reference): Inbox, Tasks, Schedule
//! and Vault as the tabs, Capture as the separate button, projects in the
//! left side and the selection's details in the right. Its ⋯ menu switches
//! the shell's style live.

mod pages;
mod screen;
mod vault;

use architect_shell::icon;
use architect_shell::{self as shell, AppShell, ShellStyle};
use dioxus::prelude::*;
use lucide_dioxus::{
    CalendarDays, CircleUser, Feather, Inbox, Keyboard, ListTodo, LogOut, NotebookText, Plus,
    Settings2, Users,
};

pub use screen::{Device, use_window_chrome, use_window_screen};

/// Where the lab is: a tab's list, or one item of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Tab(&'static str),
    Item(&'static str, usize),
}

impl Route {
    const fn tab(self) -> &'static str {
        match self {
            Self::Tab(t) | Self::Item(t, _) => t,
        }
    }
}

/// The lab's state, shared with its pages.
#[derive(Clone, Copy)]
pub struct Lab {
    pub route: Signal<Route>,
    back: Signal<Vec<Route>>,
    pub style: Signal<ShellStyle>,
    /// Whether the app fills the right side.
    pub right: Signal<bool>,
    /// How a desk shows the status items.
    pub status: Signal<shell::StatusStyle>,
}

impl Lab {
    pub fn go(mut self, to: Route) {
        let here = *self.route.peek();
        self.back.write().push(here);
        self.route.set(to);
    }

    pub fn jump(mut self, to: Route) {
        self.back.write().clear();
        self.route.set(to);
    }

    #[must_use]
    pub fn can_go_back(self) -> bool {
        !self.back.read().is_empty()
    }

    pub fn go_back(mut self) {
        let prev = self.back.write().pop();
        if let Some(prev) = prev {
            self.route.set(prev);
        }
    }
}

/// The stylesheet: the page's ground, and press feedback (`.lp`).
const CSS: &str = "\
html, body { margin: 0; padding: 0; background: #09090b; }\
button { cursor: pointer; }\
.lp:active { background-color: rgba(255,255,255,0.07); }\
";

/// The lab's root. The host publishes the window as a
/// `Signal<shell::Screen>` ([`use_window_screen`]) above it.
#[component]
pub fn LabApp() -> Element {
    let screen = use_context::<Signal<shell::Screen>>();
    let lab = use_context_provider(|| Lab {
        route: Signal::new(Route::Tab("inbox")),
        back: Signal::new(Vec::new()),
        style: Signal::new(ShellStyle::FloatingPill),
        right: Signal::new(true),
        status: Signal::new(shell::StatusStyle::Bar),
    });
    let route = (lab.route)();
    let tabs = vec![
        shell::Tab::new("inbox", "Inbox", icon!(Inbox)).badge(5),
        shell::Tab::new("tasks", "Tasks", icon!(ListTodo)),
        shell::Tab::new("schedule", "Schedule", icon!(CalendarDays)),
        shell::Tab::new("vault", "Vault", icon!(NotebookText)),
    ];
    // Which workspace the lab shows; `None` is all of them.
    let mut workspace = use_signal(|| Some("fts".to_owned()));
    let workspaces = shell::Workspaces::new("Organization", move |id| workspace.set(id))
        .all("All organizations")
        .with(
            shell::Workspace::new("fts", "FastTrackStudio")
                .detail("12 members")
                .color("#1d4ed8"),
        )
        .with(
            shell::Workspace::new("hp", "Harbor & Pine")
                .detail("Band · 4 members")
                .color("#b45309"),
        )
        .with(
            shell::Workspace::new("me", "Personal")
                .detail("Just you")
                .color("#047857"),
        )
        .current(workspace())
        .action("New organization", icon!(Plus), |()| {})
        .action("Manage organizations", icon!(Settings2), |()| {});
    let window_chrome = screen::use_window_chrome();
    let capture = shell::Action::new("Capture", icon!(Feather), move |()| {
        lab.go(Route::Item("inbox", 0));
    });
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; overflow: hidden;",
            document::Style { {CSS} }
            AppShell {
                screen: screen(),
                style: (lab.style)(),
                title: "Lab",
                tabs,
                current: Some(route.tab()),
                on_tab: move |id| lab.jump(Route::Tab(id)),
                separate: capture,
                brand: rsx! { Brand {} },
                account: shell::Account::new("Cody Wright")
                    .detail("cody@fasttrackstudio.app")
                    .presence("#22c55e")
                    .menu(
                        shell::Menu::new()
                            .item("Profile", icon!(CircleUser), |()| {})
                            .item("Settings", icon!(Settings2), |()| {})
                            .item("Keyboard shortcuts", icon!(Keyboard), |()| {})
                            .divider()
                            .item("Sign out", icon!(LogOut), |()| {}),
                    ),
                workspaces,
                left: rsx! { pages::LeftSide {} },
                right: if (lab.right)() { Some(rsx! { pages::RightSide {} }) } else { None },
                on_pull: move |()| lab.jump(Route::Tab("vault")),
                window: window_chrome,
                status: vec![
                    shell::StatusItem::new("sync", "Synced").dot("#22c55e").hint("Every change is saved and synced"),
                    shell::StatusItem::new("people", "3 online").icon(icon!(Users)),
                ],
                status_style: (lab.status)(),
                match route {
                    Route::Tab("vault") => rsx! { vault::VaultPage {} },
                    Route::Tab(tab) => rsx! { pages::ListPage { key: "{tab}", tab } },
                    Route::Item(tab, index) => rsx! { pages::ItemPage { tab, index } },
                }
            }
            screen::DeviceChrome {}
        }
    }
}

/// The lab's brand, T3 Code's way: a mark and a two-tone wordmark.
#[component]
fn Brand() -> Element {
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; font-family: -apple-system, system-ui, sans-serif;",
            span { style: "width: 26px; height: 26px; border-radius: 7px; background: #e4e4e7; display: flex; \
                           align-items: center; justify-content: center; color: #09090b; font-size: 14px; font-weight: 800;",
                "A"
            }
            span { style: "font-size: 21px; font-weight: 750; letter-spacing: -0.03em; color: #e4e4e7;", "Architect" }
            span { style: "font-size: 21px; font-weight: 500; letter-spacing: -0.03em; color: #80808a;", "Lab" }
        }
    }
}
