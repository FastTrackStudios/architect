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
mod settings;
mod vault;

use architect_orgs::{Capabilities, Org, Orgs, Role};
use architect_shell::icon;
use architect_shell::{self as shell, AppShell, ShellStyle};
use dioxus::prelude::*;
use lucide_dioxus::{
    CalendarDays, CheckCheck, CircleUser, Command, Feather, Inbox, Keyboard, ListTodo, LogOut,
    NotebookText, Plus, Settings2, Users,
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

/// The lab's tabs: the command that goes to each, its label, its id.
const TABS: [(&str, &str, &str); 4] = [
    ("LAB_GO_INBOX", "Go to Inbox", "inbox"),
    ("LAB_GO_TASKS", "Go to Tasks", "tasks"),
    ("LAB_GO_SCHEDULE", "Go to Schedule", "schedule"),
    ("LAB_GO_VAULT", "Go to Vault", "vault"),
];

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
    settings: Option<architect_settings::SettingsHandle>,
}

impl Lab {
    /// Sets a text setting (the lab's signals follow the settings).
    pub fn put(self, key: &str, id: &str) {
        if let Some(s) = self.settings {
            s.set(key, architect_settings::Value::Text(id.to_owned()));
        }
    }

    pub fn put_bool(self, key: &str, on: bool) {
        if let Some(s) = self.settings {
            s.set(key, architect_settings::Value::Bool(on));
        }
    }

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
    // Settings: the lab's sections, kept on this device.
    let store = use_hook(|| {
        architect_settings::DeviceValues::open("architect-lab")
            .ok()
            .map(std::rc::Rc::new)
    });
    let save = store.clone();
    let settings = architect_settings::use_settings_provider(
        settings::registry,
        || store.as_ref().map(|s| s.load()).unwrap_or_default(),
        Some(Callback::new(move |values: architect_settings::Values| {
            if let Some(s) = save.as_ref() {
                let _ = s.save(&values);
            }
        })),
    );
    let lab = use_context_provider(|| Lab {
        route: Signal::new(Route::Tab("inbox")),
        back: Signal::new(Vec::new()),
        style: Signal::new(ShellStyle::FloatingPill),
        right: Signal::new(true),
        status: Signal::new(shell::StatusStyle::Bar),
        settings: Some(settings),
    });
    // The lab's signals follow the settings.
    {
        let text = |key: &str| {
            architect_settings::use_setting(key)
                .and_then(|v| v.as_text().map(str::to_owned))
                .unwrap_or_default()
        };
        let style = settings::shell_style(&text("workspace.shell"));
        let status = settings::status_style(&text("workspace.status"));
        let right = architect_settings::use_setting("workspace.details")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let (mut a, mut b, mut c) = (lab.style, lab.status, lab.right);
        if *a.peek() != style {
            a.set(style);
        }
        if *b.peek() != status {
            b.set(status);
        }
        if *c.peek() != right {
            c.set(right);
        }
    }
    let metronome = architect_settings::use_setting("workspace.metronome")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Commands: the palette (⌘K), quick open (⌘O), and every command's key.
    let commands = architect_commands::use_commands_provider();
    for (id, label, to) in TABS {
        architect_commands::use_command(
            architect_commands::Command::new(id, label).group("Go"),
            move || lab.jump(Route::Tab(to)),
        );
    }
    architect_commands::use_command(
        architect_commands::Command::new("LAB_TOGGLE_DETAILS", "Details")
            .group("View")
            .checked((lab.right)()),
        move || lab.put_bool("workspace.details", !(lab.right)()),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_CAPTURE", "Capture")
            .group("Inbox")
            .shortcut("⇧⌘N"),
        move || lab.go(Route::Item("inbox", 0)),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_SHOW_COMMANDS", "Show every command")
            .group("Help")
            .shortcut("⌘K"),
        move || commands.show(architect_commands::Overlay::Palette),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_OPEN_ANYTHING", "Open…")
            .group("File")
            .shortcut("⌘O"),
        move || commands.show(architect_commands::Overlay::QuickOpen),
    );
    // The menu bar: a view of the commands, labels and keys and all.
    let menus = architect_commands::MenuBar::standard("Architect Lab")
        .menu("File", |m| {
            m.commands(["LAB_CAPTURE", "LAB_OPEN_ANYTHING"])
                .separator()
                .commands([
                    "WORKSPACE_CLOSE_TAB",
                    "WORKSPACE_REOPEN_TAB",
                    "WORKSPACE_CLOSE_OTHER_TABS",
                ])
        })
        .menu("View", |m| {
            m.command("LAB_TOGGLE_DETAILS")
                .separator()
                .group("Panes")
                .separator()
                .group("Docks")
        })
        .menu("Go", |m| {
            // The tabs, the one showing ticked: a live section, read again
            // whenever the bar is drawn.
            m.section("Tabs", move || {
                let on = (lab.route)().tab();
                TABS.iter()
                    .map(|(id, label, to)| {
                        architect_commands::Live::new(*label, *id).checked(*to == on)
                    })
                    .collect()
            })
            .separator()
            .commands([
                "WORKSPACE_NEXT_TAB",
                "WORKSPACE_PREVIOUS_TAB",
                "WORKSPACE_GO_BACK",
                "WORKSPACE_GO_FORWARD",
            ])
        })
        .menu("Help", |m| m.command("LAB_SHOW_COMMANDS"));
    let route = (lab.route)();
    let tabs = vec![
        shell::Tab::new("inbox", "Inbox", icon!(Inbox)).badge(5),
        shell::Tab::new("tasks", "Tasks", icon!(ListTodo)),
        shell::Tab::new("schedule", "Schedule", icon!(CalendarDays)),
        shell::Tab::new("vault", "Vault", icon!(NotebookText)),
    ];
    // Which workspace the lab shows; `None` is all of them.
    // The person's organizations (the identity server's, here made up):
    // the switcher, and what each role may do.
    let orgs = architect_orgs::use_orgs_provider(
        || {
            Orgs::new(vec![
                Org::new("me", "Personal", Role::Owner)
                    .personal()
                    .color("#047857"),
                Org::new("fts", "FastTrackStudio", Role::Admin)
                    .members(12)
                    .color("#1d4ed8"),
                Org::new("hp", "Harbor & Pine", Role::Viewer)
                    .members(4)
                    .color("#b45309"),
            ])
        },
        Capabilities::new()
            .allow(&Role::Viewer, ["notes.read"])
            .allow(&Role::Member, ["notes.write"])
            .allow(&Role::Admin, ["members.invite"])
            .allow(&Role::Owner, ["org.delete"])
            .inherit(),
        None,
    );
    let workspaces = orgs
        .switcher(Some("All organizations"))
        .action("New organization", icon!(Plus), |()| {})
        .action("Manage organizations", icon!(Settings2), |()| {});
    // A command that is the role's to allow: off with the reason elsewhere.
    let here = orgs
        .current()
        .map_or_else(|| "every organization".to_owned(), |o| o.name);
    let invite =
        architect_commands::Command::new("ORG_INVITE", "Invite people").group("Organization");
    architect_commands::use_command(
        if orgs.can("members.invite") {
            invite
        } else {
            invite.disabled(format!("only an admin can invite to {here}"))
        },
        || {},
    );
    let window_chrome = screen::use_window_chrome();
    let capture = shell::Action::new("Capture", icon!(Feather), move |()| {
        lab.go(Route::Item("inbox", 0));
    });
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; overflow: hidden;",
            document::Style { {CSS} }
            // The window's chords (⌘K, ⌘W, …), whatever has the focus.
            architect_commands::WindowKeys {}
            NativeMenus { bar: menus }
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
                            .item("Settings", icon!(Settings2), move |()| settings.show(None))
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
                    shell::StatusItem::new("commands", "⌘K")
                        .icon(icon!(Command))
                        .hint("Every command")
                        .on_press(move |()| commands.show(architect_commands::Overlay::Palette)),
                    shell::StatusItem::new("sync", "Synced")
                        .icon(icon!(CheckCheck)).hint("Every change is saved and synced"),
                    shell::StatusItem::new("people", "3 online").icon(icon!(Users)),
                ],
                status_style: (lab.status)(),
                match route {
                    Route::Tab("vault") => rsx! { vault::VaultPage {} },
                    Route::Tab(tab) => rsx! { pages::ListPage { key: "{tab}", tab } },
                    Route::Item(tab, index) => rsx! { pages::ItemPage { tab, index } },
                }
            }
            architect_commands::CommandLayer {}
            architect_settings::SettingsPage {}
            if metronome {
                settings::Metronome {}
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

/// The menu bar: the platform's where there is one (macOS, a Linux shell
/// that takes dbusmenu, iPadOS), else the app's own in the window.
#[component]
fn NativeMenus(bar: architect_commands::MenuBar) -> Element {
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "ios"))]
    return rsx! {
        architect_commands::NativeMenus { bar: bar.clone() }
        architect_commands::InWindowMenus { bar }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "ios")))]
    rsx! { architect_commands::InWindowMenus { bar } }
}
