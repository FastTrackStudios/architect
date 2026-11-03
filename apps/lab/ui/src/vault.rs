//! The Vault tab: the workspace host, Obsidian's shape — files and search
//! in the left dock behind a ribbon, notes in the center (more than one
//! may be open), the outline and backlinks on the right (a console opens
//! at the bottom from "Open a view"). Drag a tab anywhere — out of the
//! window for a window of its own, on a desk — hold W to grab a pane
//! without tabs, right-click (or ⋯, or hold a finger on) a tab for the
//! rest. ⌘1 recalls the "Write" screenset, ⌘2 "Mix" (the notes stacked),
//! ⌘3 "Embedded views" (a pane of each kind of view: `crate::embedded`).
//! The arrangement tab has a script's marker lane over it, and the
//! script's tool window floats.

use architect_shell::{Action, FormFactor, PageBar, Screen, StatusItem, Toolbar, icon, use_shell};
use architect_workspace::{
    Command, FormClass, MemoryStore, Pane, Ribbon, Screenset, Screensets, Side, Target, TitleRow,
    View, ViewKind, Views, Workspace, WorkspaceStore, WorkspaceView, use_pane, use_pane_status,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    AudioLines, Bot, CalendarCheck, FileText, Files, Layers, Link, ListTree, MousePointerClick,
    Network, Search, SlidersHorizontal, SquarePen, SquareTerminal, Terminal,
};

/// The lab's views, and the kinds that draw them.
fn views() -> Views {
    let views = Views::new().with_kinds(crate::embedded::kinds());
    crate::embedded::views()
        .into_iter()
        .fold(views, Views::view)
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
            View::new("actions", "Action list", |_| {
                rsx! { architect_commands::ActionListView {} }
            })
            .icon(icon!(SquareTerminal)),
        )
        .view(
            View::new("arrange", "Arrangement", |_| rsx! { crate::arrange::ArrangeView {} })
                .icon(icon!(AudioLines))
                // It publishes its spaces for the marker lane as it mounts.
                .still(),
        )
        .view(architect_canvas::ui::window_view().icon(icon!(SlidersHorizontal)).still())
        .view(
            View::new("note", "Untitled", |_| rsx! { NoteView {} })
                .icon(icon!(FileText))
                .many(),
        )
        // An agent's generated UI: every A2UI surface rendered with
        // `A2UI_RENDER` (from MCP or `architect-desktop actions run`),
        // drawn with architect-ui and patched live (`architect-a2ui`).
        .view(
            architect_a2ui::ui::surfaces_view()
                .icon(icon!(Bot))
                .home(Target::Split(Side::Right)),
        )
        // A process extension's view, drawn by another process
        // (`apps/lab/clicker`).
        .view(
            View::new("clicker", "Clicker", |_| {
                rsx! {
                    architect_extensions::remote::RemoteView {
                        processes: crate::extensions::processes(),
                        extension: crate::extensions::CLICKER,
                        view: "counter",
                    }
                }
            })
            .icon(icon!(MousePointerClick))
            .home(Target::Split(Side::Right))
            .still(),
        )
}

/// The vault's notes (the lab has no files yet).
const NOTES: [(&str, &str); 5] = [
    ("note-welcome", "Welcome"),
    ("note-shell", "The app shell"),
    ("note-workspace", "The workspace"),
    ("note-actions", "Actions everywhere"),
    ("note-extensions", "Extensions"),
];

fn note(id: &str, title: &str) -> Pane {
    Pane::new(id, "note", title)
}

/// Where the lab starts.
fn first_workspace() -> Workspace {
    let mut w = Workspace::new();
    let opens = [
        (note("note-welcome", "Welcome"), Target::Tab),
        (note("note-shell", "The app shell"), Target::Tab),
        (Pane::new("arrange", "arrange", "Arrangement"), Target::Tab),
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
        (Pane::new("clicker", "clicker", "Clicker"), Target::Tab),
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

/// "Mix": every note stacked in one strip, the graph beside it.
fn mix_workspace() -> Workspace {
    let mut w = Workspace::new();
    for (id, title) in NOTES {
        let _ = w.open(note(id, title), Target::Tab);
    }
    if let Some(g) = w.active_group() {
        let _ = w.set_stacked(g, true);
    }
    let _ = w.open(
        Pane::new("graph", "graph", "Graph"),
        Target::Split(Side::Right),
    );
    w
}

/// The vault's screensets: "Write" (where it starts), "Mix" and
/// "Embedded views".
fn screensets() -> Screensets {
    let mut builtins = vec![
        Screenset::new("Write")
            .slot(1)
            .layout(FormClass::Desk, first_workspace()),
        Screenset::new("Mix")
            .slot(2)
            .layout(FormClass::Desk, mix_workspace()),
        Screenset::new("Embedded views")
            .slot(3)
            .layout(FormClass::Desk, crate::embedded::workspace()),
    ];
    // "New Chart" (⌘K): the source editor beside its preview.
    #[cfg(feature = "keyflow-lab")]
    builtins.push(
        Screenset::new("Chart")
            .slot(4)
            .layout(FormClass::Desk, crate::keyflow::chart_workspace()),
    );
    Screensets::default().with_builtins(builtins)
}

/// Where the lab keeps its workspace: in memory, for the run (a page
/// switch and back finds it as it was left).
#[derive(Clone)]
struct VaultStore(WorkspaceStore);

#[component]
pub fn VaultPage() -> Element {
    let screen = use_context::<Signal<Screen>>();
    let screen = screen();
    let workspace = use_signal(first_workspace);
    let sets = use_signal(screensets);
    let store = use_root_context(|| VaultStore(WorkspaceStore::new(MemoryStore::new()))).0;
    let pane_status = use_signal(Vec::new);
    let views = use_hook(views);
    // The workspace on the lab's Action List: every operation, run by the
    // host below; a scenario's checks read the arrangement, each change
    // is heard.
    let actions = architect_workspace::WorkspaceActions::new(use_context::<
        std::sync::Arc<architect::action::ActionList>,
    >());
    // A phone's action bar: the workspace's tabs button, counted.
    architect_workspace::use_tabs_button(workspace);
    // The marker lane draws over the arrangement; its tool window floats
    // as a pane, to dock or tab like any other.
    let _canvases = architect_canvas::ui::use_canvases();
    crate::arrange::use_marker_lane();
    architect_canvas::ui::use_tool_windows(workspace);
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
    let new_note = on_command.clone();
    architect_commands::use_command(
        architect_commands::Command::new("LAB_NEW_NOTE", "New note").group("Vault"),
        move || {
            // A copy of the handler (its signals are shared): `Fn`.
            let mut open = new_note;
            open("new-note".to_owned());
        },
    );
    // ⌘O finds the vault's notes (and the workspace's views and tabs).
    architect_commands::use_opener(move || {
        architect_commands::Opener::new(
            "note",
            "Notes",
            |_| {
                NOTES
                    .iter()
                    .map(|(id, title)| {
                        architect_commands::OpenItem::new("note", *id, *title).detail("Vault")
                    })
                    .collect()
            },
            move |item, open_in| {
                let target = match open_in {
                    architect_commands::OpenIn::Here => Target::Here,
                    architect_commands::OpenIn::Tab => Target::Tab,
                    architect_commands::OpenIn::Split => Target::Split(Side::Right),
                };
                let mut w = workspace;
                let _ = w.write().open(note(&item.id, &item.title), target);
            },
        )
    });
    let set = {
        let s = sets.read();
        s.active
            .as_deref()
            .and_then(|id| s.get(id))
            .map(|set| set.name.clone())
    };
    // The note on show says its own (its words, Obsidian's way); the
    // screenset is the vault's.
    let mut items = pane_status();
    if let Some(set) = set {
        items.push(StatusItem::new("screenset", set).hint("⌘1 Write, ⌘2 Mix, ⌘3 Embedded views"));
    }
    architect_shell::use_status(items);
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
    // A phone's tab switcher: how many tabs there are to switch between.
    let open = workspace.read().tab_count();
    let panes = Toolbar::new().separate(
        Action::new(format!("{open} tabs"), icon!(Layers), move |()| {
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
                screensets: sets,
                store,
                status: pane_status,
                form: screen_class(&screen),
                actions,
            }
        }
    }
}

#[component]
fn FilesView() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let files = NOTES;
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

/// The class of screen the lab stands in for.
fn screen_class(screen: &Screen) -> FormClass {
    let (w, h) = (narrow(screen.width), narrow(screen.height));
    FormClass::of(w, h, screen.form() != FormFactor::Desk)
}

/// A note's body (the lab's notes are all this one).
const WELCOME: &str = "Notes open as tabs. Drag a tab to a pane's edge to split it, or to the window's edge to dock it; \
                       right-click a tab for the rest. What you arrange comes back the next time you open the vault.";

#[component]
fn NoteView() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let edits: u32 = pane.state().parse().unwrap_or(0);
    let title = pane.title();
    // The note's own fact on the status line, while it is the one on show.
    let words = WELCOME.split_whitespace().count();
    use_pane_status(vec![
        StatusItem::new("words", crate::count(words, "word")).left(),
    ]);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; gap: 12px; padding: 28px 36px; overflow-y: scroll;",
            div { style: "font-size: 26px; font-weight: 750; color: {t.ink};", "{title}" }
            div { style: "max-width: 560px; font-size: 14px; line-height: 1.6; color: {t.ink_2};", "{WELCOME}" }
            button {
                class: "lp",
                style: "align-self: flex-start; min-height: 32px; padding: 0 14px; border-radius: 8px; \
                        border: 1px solid {t.rule}; background: {t.pill}; color: {t.ink}; cursor: pointer;",
                onclick: move |_| pane.set_state((edits + 1).to_string()),
                match edits {
                    0 => "Mark as edited".to_owned(),
                    1 => "Edited once".to_owned(),
                    n => format!("Edited {n} times"),
                }
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
fn lit(on: bool, t: &architect_shell::ShellTheme) -> &str {
    if on { &t.accent_soft } else { "transparent" }
}
