//! The Vault tab: the workspace host, Obsidian's shape — files and search
//! in the left dock behind a ribbon, notes in the center (more than one
//! may be open), the outline and backlinks on the right (a console opens
//! at the bottom from "Open a view"). Drag a tab anywhere, hold W to grab a pane without
//! tabs, right-click (or ⋯) a tab for the rest.

use architect_shell::{Action, FormFactor, PageBar, Screen, StatusItem, Toolbar, icon, use_shell};
use architect_workspace::{
    Command, Pane, Ribbon, Side, Target, TitleRow, View, ViewKind, Views, Workspace, WorkspaceView,
    use_pane,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    CalendarCheck, FileText, Files, Layers, Link, ListTree, Network, Search, SquarePen, Terminal,
};

/// The lab's views.
fn views() -> Views {
    Views::new()
        .view(
            View::new("files", "Files", |_| rsx! { FilesView {} })
                .icon(icon!(Files))
                .home(Target::Dock(Side::Left)),
        )
        .view(
            View::new(
                "search",
                "Search",
                |_| rsx! { Stand { what: "Search the vault", next: "Type to find a note by its words." } },
            )
            .icon(icon!(Search))
            .home(Target::Dock(Side::Left)),
        )
        .view(
            View::new(
                "outline",
                "Outline",
                |_| rsx! { Stand { what: "No headings yet", next: "Start a line with # in the note to give it one." } },
            )
            .icon(icon!(ListTree))
            .home(Target::Dock(Side::Right)),
        )
        .view(
            View::new(
                "backlinks",
                "Backlinks",
                |_| rsx! { Stand { what: "No links here yet", next: "Link to this note from another with [[its name]]." } },
            )
            .icon(icon!(Link))
            .home(Target::Dock(Side::Right)),
        )
        .view(
            View::new("console", "Console", |_| rsx! { Stand { what: "Nothing logged", next: "What the vault does shows here." } })
                .icon(icon!(Terminal))
                .home(Target::Dock(Side::Bottom)),
        )
        .view(
            View::new(
                "graph",
                "Graph",
                |_| rsx! { Stand { what: "Every note and its links", next: "Link notes together and the graph fills in." } },
            )
            .icon(icon!(Network))
            .home(Target::Split(Side::Right)),
        )
        .view(
            View::new("note", "Untitled", |_| rsx! { NoteView {} })
                .icon(icon!(FileText))
                .many(),
        )
}

fn note(id: &str, title: &str) -> Pane {
    Pane::new(id, "note", title)
}

/// Where the lab starts.
fn first_workspace() -> Workspace {
    let mut w = Workspace::new();
    let opens = [
        (note("note-welcome", "Welcome"), Target::Tab),
        (note("note-shell", "The app shell"), Target::Tab),
        (
            Pane::new("graph", "graph", "Graph"),
            Target::Split(Side::Right),
        ),
        (
            Pane::new("files", "files", "Files"),
            Target::Dock(Side::Left),
        ),
        (
            Pane::new("search", "search", "Search"),
            Target::Dock(Side::Left),
        ),
        (
            Pane::new("outline", "outline", "Outline"),
            Target::Dock(Side::Right),
        ),
        (
            Pane::new("backlinks", "backlinks", "Backlinks"),
            Target::Dock(Side::Right),
        ),
    ];
    for (pane, target) in opens {
        let _ = w.open(pane, target);
    }
    let _ = w.activate(&architect_workspace::PaneId::new("files"));
    let _ = w.activate(&architect_workspace::PaneId::new("note-welcome"));
    if let Some(main) = w.windows.first_mut() {
        main.docks.left.ribbon = Some(Ribbon {
            views: vec![ViewKind::new("files"), ViewKind::new("search")],
            commands: vec!["new-note".to_owned(), "daily".to_owned()],
            hidden: Vec::new(),
        });
    }
    w
}

#[component]
pub fn VaultPage() -> Element {
    let screen = use_context::<Signal<Screen>>();
    let screen = screen();
    let workspace = use_signal(first_workspace);
    let views = use_hook(views);
    let mut made = use_signal(|| 0_u32);
    let commands = vec![
        Command::new("new-note", "New note", icon!(SquarePen)),
        Command::new("daily", "Today's note", icon!(CalendarCheck)),
    ];
    let on_command = move |id: String| {
        let (pane_id, title) = if id == "daily" {
            ("note-daily".to_owned(), "2026-10-31".to_owned())
        } else {
            made.with_mut(|n| *n = n.saturating_add(1));
            (format!("note-new-{}", made()), "Untitled".to_owned())
        };
        let mut workspace = workspace;
        let _ = workspace.write().open(note(&pane_id, &title), Target::Tab);
    };
    // What is open, on the status line (Obsidian's counts).
    let open = workspace.read().panes.len();
    let edits: u32 = workspace
        .read()
        .panes
        .iter()
        .filter_map(|p| p.state.parse::<u32>().ok())
        .sum();
    architect_shell::use_status(vec![
        StatusItem::new("backlinks", "0 backlinks").left(),
        StatusItem::new("panes", format!("{open} panes")).left(),
        StatusItem::new("edits", format!("{edits} edits")).left(),
    ]);
    // On a desk the workspace's tabs are the title row (Obsidian's): no page
    // bar, the top tab bars as tall, clear of the window's buttons.
    let shell = use_shell();
    let title_row = shell.desk().then(|| {
        let (left, right) = shell.title_clearance();
        TitleRow {
            height: narrow(shell.title_row()),
            left: narrow(left),
            right: narrow(right),
            sidebar_left: narrow(shell.sidebar_title_left()),
            on_drag: shell.window_chrome().map(|w| w.on_drag),
            on_zoom: shell.window_chrome().map(|w| w.on_zoom),
        }
    });
    // On a phone the page's own bar says what is open and opens the
    // switcher (one title, on the page's edge); the pane bar goes.
    let phone = screen.form() == FormFactor::Phone;
    let mut switcher = use_signal(|| false);
    let pane_title = {
        let w = workspace.read();
        w.active
            .as_ref()
            .and_then(|id| w.pane(id))
            .map_or_else(|| "Vault".to_owned(), |p| p.title.clone())
    };
    let panes = Toolbar::new().separate(
        Action::new(format!("{open} open"), icon!(Layers), move |()| {
            switcher.set(true);
        })
        .with_label(),
    );
    rsx! {
        if phone {
            PageBar { title: pane_title, toolbar: panes }
        } else if title_row.is_none() {
            PageBar { title: "Vault" }
        }
        div { style: "flex: 1 1 0; min-height: 0; display: flex; position: relative;",
            WorkspaceView {
                workspace,
                views,
                commands,
                on_command,
                phone: screen.form() == FormFactor::Phone,
                coarse: screen.touch,
                title_row,
                // One left side: the shell's ribbon and sidebar are the dock's.
                in_shell: shell.desk(),
                phone_header: !phone,
                switcher,
            }
        }
    }
}

#[component]
fn FilesView() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let files = [
        ("note-welcome", "Welcome"),
        ("note-shell", "The app shell"),
        ("note-workspace", "The workspace"),
        ("note-actions", "Actions everywhere"),
        ("note-extensions", "Extensions"),
    ];
    // The note on show is lit, as Obsidian lights the open file.
    let open_now = pane
        .workspace()
        .read()
        .current_document()
        .map(|p| p.0.clone());
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; padding: 8px; gap: 2px; overflow-y: scroll;",
            for (id, title) in files {
                button {
                    key: "{id}",
                    class: "lp",
                    "aria-current": if open_now.as_deref() == Some(id) { "page" } else { "false" },
                    style: "display: flex; align-items: center; justify-content: flex-start; min-height: 30px; padding: 0 10px; border: none; \
                            border-radius: 6px; background: {lit(open_now.as_deref() == Some(id), &t)}; color: {t.ink}; \
                            font-size: 13px; text-align: left; cursor: pointer;",
                    onclick: {
                        let pane = pane.clone();
                        // The id is the file: a note open already comes
                        // forward instead of opening twice.
                        move |_| {
                            let _ = pane.workspace().write().open(note(id, title), Target::Here);
                        }
                    },
                    "{title}"
                }
            }
        }
    }
}

#[component]
fn NoteView() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let edits: u32 = pane.state().parse().unwrap_or(0);
    let title = pane.title();
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; gap: 12px; padding: 28px 36px; overflow-y: scroll;",
            div { style: "font-size: 26px; font-weight: 750; color: {t.ink};", "{title}" }
            div { style: "max-width: 560px; font-size: 14px; line-height: 1.6; color: {t.ink_2};",
                "Notes open as tabs. Drag a tab to a pane's edge to split it, or to the window's edge to dock it; \
                 right-click a tab for the rest. What you arrange comes back the next time you open the vault."
            }
            button {
                class: "lp",
                style: "align-self: flex-start; min-height: 32px; padding: 0 14px; border-radius: 8px; \
                        border: 1px solid {t.rule}; background: {t.pill}; color: {t.ink}; cursor: pointer;",
                onclick: move |_| pane.set_state((edits + 1).to_string()),
                if edits == 0 { "Mark as edited" } else { "Edited {edits} times" }
            }
        }
    }
}

#[component]
fn Stand(what: &'static str, next: &'static str) -> Element {
    let t = use_shell().theme;
    // Centred in the pane itself, so empty panes line up across groups.
    rsx! {
        // Laid against the pane's own edges: in a dock, a flex child's
        // wrapping text measured narrower than the pane.
        div { style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; flex-direction: column; \
                      justify-content: center; padding: 16px 24px;",
            p { style: "margin: 0; text-align: center; color: {t.ink_3}; font-size: 12px; line-height: 1.6;",
                span { style: "color: {t.ink_2}; font-size: 13px; font-weight: 600;", "{what}" }
                br {}
                "{next}"
            }
        }
    }
}

/// A screen length as the workspace's points (window sizes fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// A file row's fill: the accent, faint, behind the note on show.
const fn lit(on: bool, t: &architect_shell::ShellTheme) -> &'static str {
    if on { t.accent_soft } else { "transparent" }
}
