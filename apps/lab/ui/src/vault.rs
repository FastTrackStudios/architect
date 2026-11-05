//! The Vault tab: the workspace host, Obsidian's shape — the navigator
//! (`architect-navigator`, a port of Notebook Navigator: folders beside
//! notes) and search in the left dock behind a ribbon, notes in the center (more than one
//! may be open), the outline and backlinks on the right (a console opens
//! at the bottom from "Open a view"). Drag a tab anywhere — out of the
//! window for a window of its own, on a desk — hold W to grab a pane
//! without tabs, right-click (or ⋯, or hold a finger on) a tab for the
//! rest. ⌘1 recalls the "Write" screenset, ⌘2 "Mix" (the notes stacked),
//! ⌘3 "Embedded views" (a pane of each kind of view: `crate::embedded`).
//! The arrangement tab has a script's marker lane over it, and the
//! script's tool window floats.

use architect_navigator::ui::{NavigatorView, Open, use_navigator};
use architect_navigator::{MemorySource, Millis, NoteFile, OpenIn, ViewState};
use architect_shell::{Action, FormFactor, PageBar, Screen, StatusItem, Toolbar, icon, use_shell};
use architect_workspace::{
    Command, FormClass, MemoryStore, Pane, Ribbon, Screenset, Screensets, Side, Target, TitleRow,
    View, ViewKind, Views, Workspace, WorkspaceStore, WorkspaceView, use_pane, use_pane_status,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    AudioLines, Bot, CalendarCheck, Cpu, FileText, Files, Hourglass, Layers, Link, ListTree,
    MousePointerClick, Network, Puzzle, Search, SlidersHorizontal, SquarePen, SquareTerminal,
    Terminal,
};

/// The lab's views, and the kinds that draw them, with the views loaded
/// extensions add (`extra`).
fn views(extra: Vec<View>) -> Views {
    let views = Views::new().with_kinds(crate::embedded::kinds());
    crate::embedded::views()
        .into_iter()
        .chain(extra)
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
        // The extensions page, in a pane (it is a settings section too).
        .view(
            View::new("extensions", "Extensions", |_| {
                rsx! { architect_extensions::page::ManagerView {} }
            })
            .icon(icon!(Puzzle))
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
        // The clicker's heavy job: primes counted in a worker, the
        // window animating all the while; and a naughty window that
        // spins on its own thread, marked busy.
        .view(
            View::new("clicker.heavy", "Heavy job", |_| {
                rsx! { crate::extensions::HeavyWindow {} }
            })
            .icon(icon!(Cpu))
            .home(Target::Split(Side::Right))
            .still(),
        )
        .view(
            View::new("clicker.naughty", "Naughty view", |_| {
                rsx! { crate::extensions::NaughtyWindow {} }
            })
            .icon(icon!(Hourglass))
            .home(Target::Split(Side::Right))
            .still(),
        )
        // A WASI component's view (`apps/lab/wasi-counter`, feature
        // `wasi`), drawn as a process's is.
        .view(
            View::new("wasi.counter", "WASI Counter", |_| {
                rsx! {
                    architect_extensions::remote::RemoteView {
                        processes: crate::extensions::processes(),
                        extension: crate::extensions::WASI_COUNTER,
                        view: "counter",
                    }
                }
            })
            .icon(icon!(Cpu))
            .home(Target::Split(Side::Right))
            .still(),
        )
        // The clicker's ImGui window: a Luau script from its package,
        // in the app's theme (`theme`, the clicker's own tokens).
        .view(
            View::new("clicker.window", "Clicker window", |_| {
                rsx! { crate::extensions::ClickerWindow {} }
            })
            .icon(icon!(SlidersHorizontal))
            .home(Target::Split(Side::Right))
            .still(),
        )
}

/// The vault's notes (the lab has no files yet).
const NOTES: [(&str, &str); 7] = [
    ("note-welcome", "Welcome"),
    ("note-shell", "The app shell"),
    ("note-workspace", "The workspace"),
    ("note-actions", "Actions everywhere"),
    ("note-extensions", "Extensions"),
    ("note-chart", "A chart in a note"),
    ("note-song", "Amazing Grace"),
];

/// The notes written in Markdown (the rest say [`WELCOME`]): a ```` ```kf ````
/// fence and a `type: song` note, which the Board extension draws inline
/// (`crate::note`).
const MARKDOWN: [(&str, &str); 3] = [
    (
        "note-chart",
        "The fence below is Keyflow. The Board extension claims its language (kf), so it is drawn as a chart, \
inline, as tall as it needs to be. Edit the note and the chart follows; turn the Board off and the fence is \
plain text again.

```kf
[Intro]
| C | G | Am | F |
[Verse]
| C | C | G | G |
| Am | Am | F | F |
```

A fence no extension claims stays as it is written:

```rust
fn main() { println!(\"plain\"); }
```",
    ),
    (
        "note-song",
        "---
type: song
title: Amazing Grace
artist: John Newton
key: G
tempo: 72
---
Amazing grace, how sweet the sound
That saved a wretch like me
I once was lost, but now am found
Was blind, but now I see

```kf
[Verse]
| G | G7 | C | G |
| G | Em | D | D7 |
```",
    ),
    (
        "note-outline",
        "Plans for the week. TODO book the hall

- Rehearsal
- Setlist
  - Opener
- Gear

| Day | Room |
|-----|------|
| Mon | A |
| Tue | B |",
    ),
];

/// Each Markdown note's text as edited, by its id.
#[derive(Clone, Copy)]
struct NoteTexts(Signal<std::collections::BTreeMap<String, String>>);

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
    let builtins = vec![
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
    let builtins = {
        let mut builtins = builtins;
        builtins.push(
            Screenset::new("Chart")
                .slot(4)
                .layout(FormClass::Desk, crate::keyflow::chart_workspace()),
        );
        builtins
    };
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
    // The vault the navigator shows (the ribbon's "New note" adds to it).
    let vault = use_lab_vault().source;
    // The Vault's panes are its sidebar: a desk opens it folded, the
    // switcher and the account on the ribbon.
    architect_shell::use_sidebar_default(architect_shell::SidebarDefault::Collapsed);
    // The lab's views, and every view a loaded extension adds (made again
    // when one loads or unloads).
    let ui = try_use_context::<architect_extensions::interface::ExtensionUi>();
    let views = use_memo(move || views(ui.map(|u| u.views()).unwrap_or_default()))();
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
            // A new note is a file in the vault: the navigator lists it.
            made.with_mut(|n| *n = n.saturating_add(1));
            let path = format!("Untitled {}.md", made());
            let now = chrono_now();
            vault.read().put(NoteFile::new(path, now, now));
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

/// The lab's vault and what its navigator had open, for the run: at the
/// root, as the sidebar that shows them comes and goes.
#[derive(Clone, Copy)]
struct LabVault {
    /// A `CopyValue`, so the ribbon's handler stays `Copy`.
    source: CopyValue<MemorySource>,
    kept: Signal<Option<ViewState>>,
}

fn use_lab_vault() -> LabVault {
    use_root_context(|| LabVault {
        source: CopyValue::new_in_scope(lab_vault(), ScopeId::ROOT),
        kept: Signal::new_in_scope(None, ScopeId::ROOT),
    })
}

/// The vault's files as the navigator sees them: the lab's notes in
/// folders, each with the start of its text, a few days apart, and a file
/// of a kind the lab has no view for (an extension's opener claims it: the
/// Board's, for `.kf`).
fn lab_vault() -> MemorySource {
    let now = chrono_now();
    let hour: Millis = 3_600_000;
    let mut age: Millis = 0;
    let files = FILES.iter().map(|(id, path)| {
        age = age.saturating_add(hour.saturating_mul(17));
        let text = MARKDOWN
            .iter()
            .find(|(m, _)| m == id)
            .map_or(WELCOME, |(_, t)| *t);
        NoteFile::new(
            *path,
            now.saturating_sub(age.saturating_mul(3)),
            now.saturating_sub(age),
        )
        .preview(preview_of(text))
    });
    let source = MemorySource::new("Lab").with_folder("Archive");
    navigator_samples(source, now).with_files(files)
}

/// What shows the list pane in full (#335): an SVG, a PDF, a drawing and
/// its preview, a canvas, and notes with tags, a status and tasks.
fn navigator_samples(source: MemorySource, now: Millis) -> MemorySource {
    let day: Millis = 86_400_000;
    let ago = |d: i64| now.saturating_sub(day.saturating_mul(d));
    let text = |t: &str| architect_navigator::PropertyValue::Text(t.to_owned());
    let logo = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>\
        <rect width='64' height='64' rx='12' fill='%234f7cff'/><circle cx='32' cy='32' r='16' fill='white'/></svg>";
    let sketch = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'>\
        <rect width='64' height='64' fill='%23fff7e0'/><path d='M8 48 L24 20 L36 40 L44 28 L56 48 Z' fill='none' stroke='%23d9822b' stroke-width='3'/></svg>";
    source
        .with_resource("Attachments/Logo.svg", logo)
        .with_resource("Attachments/Sketch.excalidraw.png", sketch)
        .with_file(NoteFile::new("Attachments/Logo.svg", ago(3), ago(3)))
        .with_file(NoteFile::new("Attachments/Brief.pdf", ago(12), ago(12)))
        .with_file(NoteFile::new("Attachments/Sketch.excalidraw.md", ago(1), ago(1)))
        .with_file(NoteFile::new("Attachments/Sketch.excalidraw.png", ago(1), ago(1)))
        .with_file(NoteFile::new("Attachments/Board.canvas", ago(20), ago(20)))
        .with_file(
            NoteFile::new("Projects/Launch plan.md", ago(9), ago(0))
                .preview("What ships, in what order, and who signs it off.")
                .tag("work")
                .tag("projects/launch")
                .property("status", text("doing"))
                .feature_image("Attachments/Logo.svg")
                .tasks(6, 2)
                .words(1840),
        )
        .with_file(
            NoteFile::new("Projects/Retro.md", ago(30), ago(4))
                .preview("What went well, what to change next time.")
                .tag("work")
                .property("status", text("done"))
                .tasks(3, 0)
                .words(620),
        )
        .with_file(
            NoteFile::new("Projects/Vault/Vault.md", ago(40), ago(6))
                .preview("The Vault folder's own note: what lives here.")
                .tag("projects/vault"),
        )
}

/// Now, in milliseconds since the epoch.
fn chrono_now() -> Millis {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| Millis::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// A note's text as a preview: its words, without its frontmatter and
/// fences' markers.
fn preview_of(text: &str) -> String {
    let body = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(text, |(_, body)| body);
    body.lines()
        .filter(|l| !l.starts_with("```") && !l.starts_with('|'))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each note's file in the vault, by its pane id: the lab's notes at the
/// root (where the scenarios find them), more in folders.
const FILES: [(&str, &str); 14] = [
    ("note-welcome", "Welcome.md"),
    ("note-shell", "The app shell.md"),
    ("note-workspace", "The workspace.md"),
    ("note-actions", "Actions everywhere.md"),
    ("note-extensions", "Extensions.md"),
    ("note-chart", "A chart in a note.md"),
    ("note-song", "Amazing Grace.md"),
    ("note-outline", "Journal/Plans for the week.md"),
    ("note-journal-1", "Journal/2026-11-04.md"),
    ("note-nav-panes", "Projects/Navigator/Two panes.md"),
    ("note-nav-scroll", "Projects/Navigator/Virtual scrolling.md"),
    ("note-nav-keys", "Projects/Navigator/Keys.md"),
    ("note-vault-index", "Projects/Vault/The index.md"),
    ("kf-intro", "Songs/Intro.kf"),
];

/// The note a file is (its pane id and title), if the lab draws it.
fn note_of(path: &str) -> Option<(String, String)> {
    let markdown = std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !markdown {
        return None;
    }
    if let Some((id, _)) = FILES.iter().find(|(_, p)| *p == path) {
        let title = NOTES
            .iter()
            .find(|(n, _)| n == id)
            .map_or_else(|| title_of(path), |(_, t)| (*t).to_owned());
        return Some(((*id).to_owned(), title));
    }
    // A note made here ("New note", `Untitled N.md`) is `note-new-N`.
    let made = path
        .strip_prefix("Untitled ")
        .and_then(|rest| rest.strip_suffix(".md"))
        .filter(|n| n.parse::<u32>().is_ok())?;
    Some((format!("note-new-{made}"), "Untitled".to_owned()))
}

fn title_of(path: &str) -> String {
    let name = path.rsplit_once('/').map_or(path, |(_, n)| n);
    name.rsplit_once('.').map_or(name, |(n, _)| n).to_owned()
}

/// The file a note pane is, if it is one of the vault's.
fn path_of(pane: &str) -> Option<String> {
    FILES
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, p)| (*p).to_owned())
        .or_else(|| {
            pane.strip_prefix("note-new-")
                .map(|n| format!("Untitled {n}.md"))
        })
}

/// The left dock's files: the navigator over the lab's vault. A click
/// opens a note here (one open already comes forward); the note on show is
/// revealed; a right-click is the file's context menu, with what extensions
/// add (`[[context]] target = "file"`).
#[component]
fn FilesView() -> Element {
    let pane = use_pane();
    // The dock is drawn by the shell, above the page: the vault and what
    // the navigator had open are the app's, kept at its root.
    let LabVault { source, kept } = use_lab_vault();
    let nav = use_navigator(move || std::sync::Arc::new(source.cloned()));
    use_hook(move || {
        if let Some(v) = kept.peek().as_ref() {
            nav.restore(v);
        }
    });
    use_drop(move || {
        let mut kept = kept;
        kept.set(Some(nav.view_state()));
    });
    // A pop-out window has no screen of the lab's: a desk's.
    let phone =
        try_use_context::<Signal<Screen>>().is_some_and(|s| s().form() == FormFactor::Phone);
    // The note on show, lit and revealed.
    let open_now = pane
        .workspace()
        .read()
        .current_document()
        .and_then(|p| path_of(&p.0));
    use_effect(use_reactive!(|open_now| nav.set_active(open_now.as_deref())));
    let open_pane = pane.clone();
    let menu_pane = pane;
    let on_open = EventHandler::new(move |o: Open| {
        if let Some((id, title)) = note_of(&o.path) {
            // A click opens a tab in the group of the note on
            // show; the arrow keys show each note in its place.
            let target = match (o.how, o.browsing) {
                (OpenIn::Split, _) => Target::Split(Side::Right),
                (_, true) => Target::Here,
                (OpenIn::Here | OpenIn::Tab, false) => Target::Tab,
            };
            // Beside the note on show, not in the dock pressed.
            let mut workspace = open_pane.workspace();
            let mut w = workspace.write();
            if let Some(doc) = w.current_document().cloned() {
                let _ = w.activate(&doc);
            }
            let _ = w.open(note(&id, &title), target);
        } else if let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>() {
            let _ = ui.open_path(&o.path);
        }
    });
    let on_context_menu = EventHandler::new(move |(path, e): (String, MouseEvent)| {
        menu_pane.context_menu("file", path, &e);
    });
    // Navigator, the built-in extension, draws it: its view over this
    // navigator (or a development copy's, `EXT_CLONE {"id":"navigator"}`).
    let mut mount = use_context_provider(|| {
        Signal::new(architect_navigator_extension::NavigatorMount {
            navigator: nav,
            on_open,
            on_context_menu: Some(on_context_menu),
            on_focus_editor: None,
            phone,
        })
    });
    if mount.peek().phone != phone {
        mount.write().phone = phone;
    }
    match try_use_context::<architect_extensions::interface::ExtensionUi>() {
        Some(ui) => rsx! {
            architect_extensions::interface::ExtensionView {
                ui,
                extension: architect_navigator_extension::ID.to_owned(),
                view: "files".to_owned(),
                bare: true,
            }
        },
        None => rsx! {
            NavigatorView { navigator: nav, phone, on_open, on_context_menu }
        },
    }
}

/// The class of screen the lab stands in for.
fn screen_class(screen: &Screen) -> FormClass {
    let (w, h) = (narrow(screen.width), narrow(screen.height));
    FormClass::of(w, h, screen.form() != FormFactor::Desk)
}

/// The issue a note links to.
const ISSUE: &str = "https://github.com/FastTrackStudios/architect/issues/179";

/// A note's body (the lab's notes are all this one).
const WELCOME: &str = "Notes open as tabs. Drag a tab to a pane's edge to split it, or to the window's edge to dock it; \
                       right-click a tab for the rest. What you arrange comes back the next time you open the vault.";

#[component]
fn NoteView() -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    let edits: u32 = pane.state().parse().unwrap_or(0);
    let title = pane.title();
    let texts = use_root_context(|| {
        NoteTexts(Signal::new_in_scope(
            MARKDOWN
                .iter()
                .map(|(id, text)| ((*id).to_owned(), (*text).to_owned()))
                .collect(),
            ScopeId::ROOT,
        ))
    })
    .0;
    let id = pane.id.0.clone();
    let markdown = texts.read().get(&id).cloned();
    // The note's own fact on the status line, while it is the one on show.
    let words = markdown
        .as_deref()
        .unwrap_or(WELCOME)
        .split_whitespace()
        .count();
    use_pane_status(vec![
        StatusItem::new("words", crate::count(words, "word")).left(),
    ]);
    if let Some(text) = markdown {
        // Keyed by the note: a note's editor and its embeds are its own.
        return rsx! {
            MarkdownNote { key: "{id}", id, title, text, texts }
        };
    }
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; gap: 12px; padding: 28px 36px; overflow-y: scroll;",
            div { style: "font-size: 26px; font-weight: 750; color: {t.ink};", "{title}" }
            div { style: "max-width: 560px; font-size: 14px; line-height: 1.6; color: {t.ink_2};", "{WELCOME}" }
            // A link: Ctrl- or Cmd-click opens it with the extension that
            // claims its pattern (the Board's, for a GitHub issue).
            div { style: "font-size: 13px; color: {t.ink_3};",
                "Tracked in "
                a {
                    style: "color: {t.accent}; text-decoration: underline; cursor: pointer;",
                    onclick: move |e: MouseEvent| {
                        if architect_extensions::interface::link_press(&e)
                            && let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>()
                        {
                            let _ = ui.open_link(ISSUE);
                        }
                    },
                    "{ISSUE}"
                }
                " (Ctrl- or Cmd-click)"
            }
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

/// A note written in Markdown: its title, an editor for its source
/// ("Edit"), and the note drawn (`crate::note::NoteBody`), claimed blocks
/// by the extensions that claim them.
#[component]
fn MarkdownNote(
    id: String,
    title: String,
    text: String,
    texts: Signal<std::collections::BTreeMap<String, String>>,
) -> Element {
    let t = use_shell().theme;
    let mut editing = use_signal(|| false);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; gap: 12px; padding: 28px 36px; overflow-y: scroll;",
            div { style: "display: flex; align-items: center; gap: 12px;",
                div { style: "font-size: 26px; font-weight: 750; color: {t.ink};", "{title}" }
                button {
                    id: "note-edit",
                    class: "lp",
                    style: "min-height: 28px; padding: 0 12px; border-radius: 8px; border: 1px solid {t.rule}; \
                            background: {t.pill}; color: {t.ink}; cursor: pointer;",
                    onclick: move |_| editing.toggle(),
                    if editing() { "Done" } else { "Edit" }
                }
            }
            if editing() {
                crate::note_editor::NoteEditor {
                    id: id.clone(),
                    text: text.clone(),
                    on_change: move |value: String| {
                        let mut texts = texts;
                        texts.write().insert(id.clone(), value);
                    },
                }
            }
            crate::note::NoteBody { text }
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
