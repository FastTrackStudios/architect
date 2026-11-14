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

use std::sync::Arc;

use architect_extensions::contrib::NavigatorProviderDecl;
use architect_navigator::ui::{NavigatorView, Open, use_navigator};
use architect_navigator::{
    MemorySource, Millis, Mount, Mounts, NavTarget, NavigatorSource, NoteFile, OpenIn, Place,
    ProviderRequest, ProviderSource, SourceEvent, VaultSource, ViewState,
};
use architect_navigator_api::{ActionProvider, NavigatorProvider};
use architect_shell::{Action, FormFactor, PageBar, Screen, StatusItem, Toolbar, icon, use_shell};
use architect_workspace::{
    Command, FormClass, Pane, Ribbon, Screenset, Screensets, Side, Target, TitleRow, View,
    ViewKind, Views, Workspace, WorkspaceView, use_pane, use_pane_status,
};
use dioxus::prelude::*;
use lucide_dioxus::{
    AudioLines, Bot, CalendarCheck, Cpu, FileText, Files, Hourglass, Layers, Link, ListTree,
    MousePointerClick, Network, Puzzle, Search, SlidersHorizontal, SquarePen, SquareTerminal,
    Table, Terminal,
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
        // A note of the vault on disk, in the editor; its path is the
        // pane's state (a rename keeps the pane).
        .view(
            View::new(VAULT_NOTE, "Untitled", |_| rsx! { VaultNoteView {} })
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
        // (`apps/sandbox/clicker`).
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
        // A WASI component's view (`apps/sandbox/wasi-counter`, feature
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
        // A .base file: Bases' table, cards and list, and the types loaded
        // extensions add (`architect_bases::ui`). Last, so the Open menu's
        // other rows keep their places.
        .view(architect_bases::ui::view().icon(icon!(Table)))
}

/// The vault's notes (the lab has no files yet).
const NOTES: [(&str, &str); 8] = [
    ("note-welcome", "Welcome"),
    ("note-shell", "The app shell"),
    ("note-workspace", "The workspace"),
    ("note-actions", "Actions everywhere"),
    ("note-extensions", "Extensions"),
    ("note-chart", "A chart in a note"),
    ("note-song", "Amazing Grace"),
    ("note-base", "A base in a note"),
];

/// The notes written in Markdown (the rest say [`WELCOME`]): a ```` ```kf ````
/// fence and a `type: song` note, which the Board extension draws inline
/// (`crate::note`).
const MARKDOWN: [(&str, &str); 4] = [
    (
        "note-base",
        "The fence below is a base: Bases claims its language (base) and runs its YAML over the lab's vault, \
here as a list. Turn Bases off and the fence is plain text again.

```base
filters: 'status != null'
views:
  - type: list
    name: Notes
    order: [file.name, status]
```
",
    ),
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

/// The lab vault's base (`apps/sandbox/vault`).
const BASE: &str = "Bases/Notes by status.base";

fn base_title(path: &str) -> &str {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
}

/// The pane a base opens in: the one `BASES_OPEN` opens.
fn base_pane(path: &str) -> Pane {
    let mut pane = Pane::new(
        architect_bases::actions::pane_id(path),
        architect_bases::actions::VIEW_KIND,
        base_title(path),
    );
    pane.state = serde_json::json!({ "path": path }).to_string();
    pane
}

/// The view a note of the vault on disk is.
const VAULT_NOTE: &str = "vault.note";

/// A pane for the vault's note at `path`: a new id each time (a note
/// renamed keeps its pane, and a new note may take its old name).
fn vault_note(path: &str) -> Pane {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut pane = Pane::new(format!("vault-{n}"), VAULT_NOTE, tab_title(path));
    path.clone_into(&mut pane.state);
    pane
}

/// Whether `path`'s extension is `ext`, in any case.
fn has_ext(path: &str, ext: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The pane showing the vault's note at `path`, if one does.
fn vault_pane_of(w: &Workspace, path: &str) -> Option<architect_workspace::PaneId> {
    w.panes
        .iter()
        .find(|p| p.view.0 == VAULT_NOTE && p.state == path)
        .map(|p| p.id.clone())
}

/// The vault's note at `path` on show: its pane forward when one is open,
/// else a new one at `target`, in the main area (beside the note on show;
/// never in the dock the navigator is in, even with no note open).
/// Shows note `path` of the vault on disk (`target`): what loading a
/// project onto a deck opens (`crate::decks`, `on_fresh`).
pub fn show_note(workspace: Signal<Workspace>, path: &str, target: Target) {
    show_vault_note(workspace, path, target);
}

fn show_vault_note(workspace: Signal<Workspace>, path: &str, target: Target) {
    let mut workspace = workspace;
    let mut w = workspace.write();
    if let Some(id) = vault_pane_of(&w, path) {
        let _ = w.activate(&id);
        return;
    }
    drop(w);
    open_in_main(workspace, vault_note(path), target);
}

/// Opens `pane` at `target` in the main area: beside the note on show, and
/// never in the dock the navigator is in, even with no note open.
fn open_in_main(workspace: Signal<Workspace>, pane: Pane, target: Target) {
    let mut workspace = workspace;
    let mut w = workspace.write();
    if let Some(doc) = w.current_document().cloned() {
        let _ = w.activate(&doc);
    }
    let Ok(id) = w.open(pane, target) else {
        return;
    };
    let docked = w
        .locate(&id)
        .is_some_and(|l| l.window != 0 || matches!(l.area, architect_workspace::Area::Dock(_)));
    let main = w
        .windows
        .first()
        .and_then(|m| m.center.as_ref())
        .and_then(architect_workspace::Node::first_group)
        .map(|g| g.id);
    if docked && let Some(group) = main {
        let _ = w.move_pane(&id, architect_workspace::Dest::Group { group, index: None });
        let _ = w.activate(&id);
    }
}

/// A new note in `folder` of the vault on disk, opened (⌘N, the ribbon's
/// and the empty main area's New note, the navigator's).
fn new_vault_note(workspace: Signal<Workspace>, disk: &VaultSource, folder: &str) {
    match disk.create_note(folder) {
        Ok(path) => show_vault_note(workspace, &path, Target::Tab),
        Err(e) => tracing::warn!(folder, error = %e, "lab.vault.new_note_failed"),
    }
}

/// Where the lab starts: on a vault on disk, its Welcome note.
fn first_workspace() -> Workspace {
    let on_disk = crate::notes::open_vault().is_some();
    let mut w = Workspace::new();
    let (first, second) = if on_disk {
        (
            (vault_note("Welcome.md"), Target::Tab),
            (note("note-shell", "The app shell"), Target::Tab),
        )
    } else {
        (
            (note("note-welcome", "Welcome"), Target::Tab),
            (note("note-shell", "The app shell"), Target::Tab),
        )
    };
    let opens = [
        first,
        second,
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
    // The Architect app opens the person's vault alone: its Welcome note
    // when it has one, the sidebars' views; the made-up notes, the
    // arrangement, the graph and the Clicker are the sandbox's.
    let sandbox = crate::distribution().is_sandbox();
    let has_welcome =
        crate::notes::open_vault().is_some_and(|v| v.root().join("Welcome.md").is_file());
    let opens = opens.into_iter().filter(|(pane, _)| {
        sandbox
            || match pane.view.0.as_str() {
                VAULT_NOTE => has_welcome,
                "files" | "search" | "outline" | "backlinks" => true,
                _ => false,
            }
    });
    for (pane, target) in opens {
        let _ = w.open(pane, target);
    }
    let _ = w.activate(&architect_workspace::PaneId::new("files"));
    if let Some(welcome) = w.panes.first().map(|p| p.id.clone()) {
        let _ = w.activate(&welcome);
    }
    if let Some(main) = w.windows.first_mut() {
        main.docks.left.ribbon = Some(Ribbon {
            views: vec![ViewKind::new("files"), ViewKind::new("search")],
            // Today's note only opened a note: it is the palette's
            // (`NAVIGATOR_OPEN_DAILY_NOTE`) and the Calendar row's (#443).
            commands: vec!["new-note".to_owned()],
            hidden: Vec::new(),
        });
    }
    w
}

/// Tells the app's root (`crate::favourites::VaultFocus`) the note on
/// show: the document on show's file when it is a note of the vault on
/// disk (a note's pane, or an extension's view opened on one), the view
/// it is in, and its `type`, read again whenever the vault says the file
/// changed (its `type` edited, here or outside); nothing once the page
/// goes. A note a `{place}` tab opened keeps its set only while it, or the
/// note it was opened from, is on show.
fn use_vault_focus(workspace: Signal<Workspace>) {
    let focus = try_use_context::<crate::favourites::VaultFocus>();
    let on_show: Option<(String, String)> = {
        let w = workspace.read();
        w.current_document()
            .and_then(|id| w.pane(id))
            .map(|p| (p.state.clone(), p.view.0.clone()))
            .filter(|(s, _)| {
                std::path::Path::new(s)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            })
    };
    // Bumped when the vault changes the file on show.
    let mut changed = use_signal(|| 0_u64);
    let disk = use_lab_vault().disk;
    let at = use_signal(|| None::<String>);
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<SourceEvent>();
        let sub = disk.peek().as_ref().map(|d| {
            d.subscribe(Box::new(move |e| {
                let _ = tx.unbounded_send(e.clone());
            }))
        });
        spawn(async move {
            use futures_util::StreamExt as _;
            while let Some(event) = rx.next().await {
                // The note on show, or the note a place on show was opened
                // from (its set ends once that is gone or retyped).
                let from =
                    focus.and_then(|f| f.opened.peek().as_ref().map(|(o, _)| o.from.clone()));
                if at.peek().as_deref().is_some_and(|p| event.touches(p))
                    || from.is_some_and(|p| event.touches(&p))
                {
                    let n = changed.peek().wrapping_add(1);
                    changed.set(n);
                }
            }
        });
        std::rc::Rc::new(sub)
    });
    let revision = changed();
    use_effect(use_reactive!(|on_show, revision| {
        let _ = revision;
        let Some(mut focus) = focus else { return };
        if focus.workspace.peek().is_none() {
            focus.workspace.set(Some(workspace));
        }
        let mut at = at;
        let path = on_show.as_ref().map(|(p, _)| p.clone());
        if *at.peek() != path {
            at.set(path.clone());
        }
        let text = path.as_ref().and_then(|path| {
            crate::notes::open_vault()
                .and_then(|v| std::fs::read_to_string(v.root().join(path)).ok())
        });
        let note = on_show
            .zip(text)
            .map(|((path, view), text)| crate::favourites::OpenNote {
                path,
                note_type: crate::favourites::note_type(&text),
                // An extension's view's kind is `<extension>/<view>`.
                view: view.contains('/').then_some(view),
            });
        // A place's set lasts while it or its note is on show, and its note
        // is still there with that type (`architect_shell::place_kept`).
        let kept = focus.opened.peek().as_ref().is_some_and(|(o, ty)| {
            // No vault to read it from: nothing says it changed.
            let now = crate::notes::open_vault().map_or_else(
                || Some(ty.clone()),
                |v| {
                    std::fs::read_to_string(v.root().join(&o.from))
                        .ok()
                        .map(|t| crate::favourites::note_type(&t))
                },
            );
            architect_shell::place_kept(
                o,
                ty.as_deref(),
                note.as_ref().map(|n| n.path.as_str()),
                now.as_ref().map(Option::as_deref),
            )
        });
        if !kept && focus.opened.peek().is_some() {
            focus.opened.set(None);
        }
        if *focus.note.peek() != note {
            focus.note.set(note);
        }
    }));
    use_drop(move || {
        if let Some(mut focus) = focus {
            focus.workspace.set(None);
            focus.note.set(None);
            focus.opened.set(None);
        }
    });
}

/// Shows the vault's notes: the last note pane opened, else the files.
fn show_notes(mut workspace: Signal<Workspace>) {
    let mut w = workspace.write();
    let note = w
        .panes
        .iter()
        .rev()
        .find(|p| p.view.0 == VAULT_NOTE)
        .map(|p| p.id.clone());
    let _ = w.activate(&note.unwrap_or_else(|| architect_workspace::PaneId::new("files")));
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

/// "Project": a project's deck starts in it: the navigator and search on
/// the left, the outline and backlinks on the right, the project's note
/// (opened when it loads) in the middle.
fn project_workspace() -> Workspace {
    let mut w = Workspace::new();
    for (pane, side) in [
        (Pane::new("search", "search", "Search"), Side::Left),
        (Pane::new("files", "files", "Files"), Side::Left),
        (Pane::new("outline", "outline", "Outline"), Side::Right),
        (
            Pane::new("backlinks", "backlinks", "Backlinks"),
            Side::Right,
        ),
    ] {
        let _ = w.open(pane, Target::Dock(side));
    }
    let _ = w.activate(&architect_workspace::PaneId::new("files"));
    if let Some(main) = w.windows.first_mut() {
        main.docks.left.ribbon = Some(Ribbon {
            views: vec![ViewKind::new("files"), ViewKind::new("search")],
            // Today's note only opened a note: it is the palette's
            // (`NAVIGATOR_OPEN_DAILY_NOTE`) and the Calendar row's (#443).
            commands: vec!["new-note".to_owned()],
            hidden: Vec::new(),
        });
    }
    w
}

/// The vault's screensets: "Write" (where it starts), "Mix" and
/// "Embedded views"; "Project", a project's deck's.
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
        Screenset::new("Project")
            .slot(5)
            .layout(FormClass::Desk, project_workspace()),
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

#[component]
pub fn VaultPage() -> Element {
    let screen = use_context::<Signal<Screen>>();
    let screen = screen();
    let workspace = use_signal(first_workspace);
    let sets = use_signal(screensets);
    let pane_status = use_signal(Vec::new);
    // What an extension's view in a pop-out draws with: its page, its web
    // view (#222), its frame times.
    let bridge = use_hook(|| {
        architect_workspace::Bridge::new()
            .context::<architect_extensions::interface::ViewPages>()
            .context::<architect_extensions::interface::WebViews>()
            .context::<architect_workspace::Meters>()
    });
    // The vault the navigator shows (the ribbon's "New note" adds to it).
    let lab = use_lab_vault();
    // The Vault's panes are its sidebar: on a vault on disk, two columns
    // (the navigator's folders beside its notes); a film's in-memory one
    // opens it folded to the rail. Every layout stays a press away.
    architect_shell::use_sidebar_layout(if lab.disk.peek().is_some() {
        architect_shell::SidebarLayout::Two
    } else {
        architect_shell::SidebarLayout::Rail
    });
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
    // Cursors come with the panes (#509), and `PRESENCE_*` is on the list.
    let _cursors = crate::cursors::use_app_cursors(&actions.0);
    // Decks (#441): the vault, each project, each song loaded onto a deck
    // of its own, shown in its screenset; the session and each context's
    // layout kept (`crate::decks`). The workspace and screensets above are
    // the deck on show's.
    let _decks = crate::decks::use_lab_decks(
        workspace,
        sets,
        screen_class(&screen),
        &views,
        actions.clone(),
    );
    // The tab kinds (#476): a project's compact scope tab, and every
    // extension's (the sandbox's wide song tabs, its setlist tab).
    architect_workspace::use_tab_kinds(crate::decks::tab_kinds(), Some(actions.clone()));
    // A phone's action bar: the workspace's tabs button, counted.
    architect_workspace::use_tabs_button(workspace);
    use_vault_focus(workspace);
    // The Vault tab's surface (`crate::favourites`): back to the notes from
    // the projects list or the calendar.
    architect_commands::use_command(
        architect_commands::Command::new("VAULT_NOTES", "Show the vault's notes").group("Go"),
        move || show_notes(workspace),
    );
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
        } else if let Some(disk) = lab.disk.cloned() {
            // A note on disk: a file in the vault, opened in the editor.
            new_vault_note(workspace, &disk, "");
            return;
        } else {
            // A new note is a file in the vault: the navigator lists it.
            made.with_mut(|n| *n = n.saturating_add(1));
            let path = format!("Untitled {}.md", made());
            let now = chrono_now();
            if let Some(memory) = lab.memory.cloned() {
                memory.put(NoteFile::new(path, now, now));
            }
            (format!("note-new-{}", made()), "Untitled".to_owned())
        };
        // In the main area, never the dock the navigator is in (#442).
        open_in_main(workspace, note(&pane_id, &title), Target::Tab);
    };
    let new_note = on_command.clone();
    architect_commands::use_command(
        architect_commands::Command::new("LAB_NEW_NOTE", "New note")
            .group("Vault")
            .shortcut("⌘N"),
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
            move |_| {
                lab.disk.cloned().map_or_else(
                    || {
                        NOTES
                            .iter()
                            .map(|(id, title)| {
                                architect_commands::OpenItem::new("note", *id, *title)
                                    .detail("Vault")
                            })
                            .collect()
                    },
                    // Go to file: every note of the vault on disk, by its path.
                    |disk| {
                        disk.vault().with(|v| {
                            v.pages
                                .iter()
                                .map(|p| {
                                    architect_commands::OpenItem::new(
                                        "note",
                                        p.rel_path.clone(),
                                        p.basename.clone(),
                                    )
                                    .detail(p.folder.clone())
                                })
                                .collect()
                        })
                    },
                )
            },
            move |item, open_in| {
                let target = match open_in {
                    architect_commands::OpenIn::Here => Target::Here,
                    architect_commands::OpenIn::Tab => Target::Tab,
                    architect_commands::OpenIn::Split => Target::Split(Side::Right),
                };
                if lab.disk.peek().is_some() {
                    show_vault_note(workspace, &item.id, target);
                    return;
                }
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
                // With nothing open, the main area offers a new note.
                empty: vec!["new-note".to_owned()],
                phone: screen.form() == FormFactor::Phone,
                coarse: screen.touch,
                title_row,
                // One left side: the shell's ribbon and sidebar are the dock's.
                in_shell: shell.desk(),
                phone_header: !phone,
                switcher,
                screensets: sets,
                // The decks keep the layouts and the screensets.
                status: pane_status,
                form: screen_class(&screen),
                actions,
                bridge,
            }
        }
    }
}

/// The folder of a project row: the folder itself, or a `project.md`'s.
fn project_folder(path: &str) -> &str {
    path.strip_suffix("/project.md").unwrap_or(path)
}

/// The lab's vaults and what its navigator had open, for the run: at the
/// root, as the sidebar that shows them comes and goes.
///
/// Org = vault (#443): the app's vault is its org's, and every other org
/// vault this device holds (`crate::vaults::catalog`) opens beside it,
/// all in one navigator ([`Mounts`]), with the providers' mounts.
#[derive(Clone, Copy)]
struct LabVault {
    /// What the navigator reads: every vault and provider.
    /// `CopyValue`s, so the ribbon's handler stays `Copy`.
    source: CopyValue<Arc<dyn NavigatorSource>>,
    mounts: CopyValue<Mounts>,
    /// The app's vault's mount id (`@<org>`).
    own: CopyValue<String>,
    /// The vault on disk (`crate::notes`), live, when the lab has one.
    disk: CopyValue<Option<VaultSource>>,
    /// The other orgs' vaults, by mount id.
    others: CopyValue<std::collections::BTreeMap<String, VaultSource>>,
    /// The in-memory vault a film's run shows instead.
    memory: CopyValue<Option<MemorySource>>,
    kept: Signal<Option<ViewState>>,
}

impl LabVault {
    /// The vault a note pane's state is in, and its path there: the app's
    /// for a plain path, another org's for `@<org>/<path>`.
    fn vault_of(&self, state: &str) -> (Option<VaultSource>, String) {
        match architect_navigator::mounts::split(state) {
            Some((m, rest)) if *self.own.peek() == m => (self.disk.peek().clone(), rest.to_owned()),
            Some((m, rest)) => (self.others.peek().get(m).cloned(), rest.to_owned()),
            None => (self.disk.peek().clone(), state.to_owned()),
        }
    }
}

/// How the Now Playing card shows an item's art (a vault path): the
/// app's vault's image as a `data:` URL (`crate::collection::art_url`).
///
/// The vault is asked for at each call, not held: the card is at the
/// app's root, mounted before the first run's card opens a vault, and
/// holding [`LabVault`] there would make the Vault's (a root context)
/// before it, without the vault (every note "No vault is open" until
/// the app was opened again).
pub fn use_now_playing_art() -> Callback<String, Option<String>> {
    use_callback(move |p: String| {
        crate::notes::open_vault().and_then(|v| crate::collection::art_url(&v.root(), &p))
    })
}

/// A vault on disk as the navigator reads it, watched.
fn watched(open: vault::actions::OpenVault, name: String) -> VaultSource {
    let source = VaultSource::named(open, name);
    if let Err(e) = source.watch() {
        tracing::warn!(error = %e, "lab.vault.watch_failed");
    }
    source
}

fn use_lab_vault() -> LabVault {
    use_root_context(|| {
        let disk = crate::notes::open_vault().map(|open| {
            // The sandbox's is the demo vault; the Architect app's is the
            // person's, by its folder's name.
            let name = if crate::distribution().is_sandbox() {
                "Demo vault".to_owned()
            } else {
                let root = open.root();
                root.file_name()
                    .map_or_else(|| "Vault".to_owned(), |n| n.to_string_lossy().into_owned())
            };
            watched(open, name)
        });
        let memory = disk.is_none().then(lab_vault);
        let catalog = crate::vaults::catalog();
        let dir = disk.as_ref().map(|d| d.vault().root());
        let own = crate::vaults::app_vault_org(&catalog, dir.as_deref());
        let own_mount = Mount::vault(own.badge());
        let own_source: Arc<dyn NavigatorSource> = match (&disk, &memory) {
            (Some(d), _) => Arc::new(d.clone()),
            (None, Some(m)) => Arc::new(m.clone()),
            (None, None) => Arc::new(MemorySource::new("Lab")),
        };
        let mounts = Mounts::new("Navigator");
        let mut others = std::collections::BTreeMap::new();
        let mut vaults: Vec<(Mount, Arc<dyn NavigatorSource>)> =
            vec![(own_mount.clone(), own_source)];
        for v in catalog.iter().filter(|v| v.org != own.org) {
            match vault::actions::OpenVault::open(&v.dir) {
                Ok(open) => {
                    let source = watched(open, v.name.clone());
                    let m = Mount::vault(v.badge());
                    others.insert(m.id.clone(), source.clone());
                    vaults.push((m, Arc::new(source)));
                }
                Err(e) => tracing::warn!(org = %v.org, error = %e, "lab.vault.org_vault_unopened"),
            }
        }
        for (m, s) in vaults {
            mounts.mount(m, s);
        }
        let source: Arc<dyn NavigatorSource> = Arc::new(mounts.clone());
        LabVault {
            source: CopyValue::new_in_scope(source, ScopeId::ROOT),
            mounts: CopyValue::new_in_scope(mounts, ScopeId::ROOT),
            own: CopyValue::new_in_scope(own_mount.id, ScopeId::ROOT),
            disk: CopyValue::new_in_scope(disk, ScopeId::ROOT),
            others: CopyValue::new_in_scope(others, ScopeId::ROOT),
            memory: CopyValue::new_in_scope(memory, ScopeId::ROOT),
            kept: Signal::new_in_scope(None, ScopeId::ROOT),
        }
    })
}

/// Where the navigator keeps what it had open between runs: the app's
/// folder's `navigator-view` (the Architect app's, or the sandbox's
/// throwaway one); none for an in-memory vault.
fn view_store() -> Option<architect::settings::Store<ViewState>> {
    let dir = crate::app_dir()?;
    Some(architect::settings::Store::in_dir(
        crate::distribution().folder_name(),
        "navigator-view",
        dir,
    ))
}

/// The demo vault's periodic notes (`apps/ui/demo-vault.py`'s journal):
/// set when the five patterns and the folder are still the defaults, so a
/// person's own are left alone; and the properties its browser shows.
fn demo_vault_periodic_notes(s: &mut architect_navigator::NavigatorSettings) {
    let defaults = architect_navigator::NavigatorSettings::default();
    let untouched = s.periodic_folder == defaults.periodic_folder
        && s.daily_pattern == defaults.daily_pattern
        && s.weekly_pattern == defaults.weekly_pattern
        && s.monthly_pattern == defaults.monthly_pattern
        && s.quarterly_pattern == defaults.quarterly_pattern
        && s.yearly_pattern == defaults.yearly_pattern;
    if untouched {
        "Journal".clone_into(&mut s.periodic_folder);
        "[Daily]/YYYY-MM-DD".clone_into(&mut s.daily_pattern);
        "[Weekly]/GGGG-[W]WW".clone_into(&mut s.weekly_pattern);
        "[Monthly]/YYYY-MM".clone_into(&mut s.monthly_pattern);
        "[Quarterly]/YYYY-[Q]Q".clone_into(&mut s.quarterly_pattern);
        "[Yearly]/YYYY".clone_into(&mut s.yearly_pattern);
    }
    // The property browser shows the keys named (none by default, as
    // Notebook Navigator's `propertyKeys`): the demo vault's own.
    if s.property_keys.trim().is_empty() {
        "status, type, mood".clone_into(&mut s.property_keys);
    }
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
/// its preview, a canvas, and notes with tags, a status and tasks. The
/// demo vault's own files (`apps/ui/demo-vault`, made by
/// `apps/ui/demo-vault.py`'s `navigator_samples`), read in as the vault on
/// disk reads them (`architect_navigator::vault_source::note_from_text`):
/// one source for both.
fn navigator_samples(source: MemorySource, now: Millis) -> MemorySource {
    use architect_navigator::vault_source::note_from_text;
    use base64::Engine as _;
    let day: Millis = 86_400_000;
    let ago = |d: i64| now.saturating_sub(day.saturating_mul(d));
    let logo = include_str!("../demo-vault/Attachments/Launch logo.svg");
    let sketch = include_bytes!("../demo-vault/Attachments/Sketch.excalidraw.png");
    let svg_url = format!(
        "data:image/svg+xml;utf8,{}",
        logo.trim().replace('#', "%23")
    );
    let png_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(sketch)
    );
    // (path, text, made and last changed, days ago)
    let notes: [(&str, &str, i64, i64); 4] = [
        (
            "Projects/Launch plan.md",
            include_str!("../demo-vault/Projects/Launch plan.md"),
            9,
            0,
        ),
        (
            "Projects/Retro.md",
            include_str!("../demo-vault/Projects/Retro.md"),
            30,
            4,
        ),
        (
            "Projects/Vault/Vault.md",
            include_str!("../demo-vault/Projects/Vault/Vault.md"),
            40,
            6,
        ),
        (
            "Attachments/Sketch.excalidraw.md",
            include_str!("../demo-vault/Attachments/Sketch.excalidraw.md"),
            1,
            1,
        ),
    ];
    let files = [
        ("Attachments/Launch logo.svg", 3),
        ("Attachments/Brief.pdf", 12),
        ("Attachments/Sketch.excalidraw.png", 1),
        ("Attachments/Board.canvas", 20),
    ];
    let source = source
        .with_resource("Attachments/Launch logo.svg", svg_url)
        .with_resource("Attachments/Sketch.excalidraw.png", png_url);
    let source = files.into_iter().fold(source, |s, (path, days)| {
        s.with_file(NoteFile::new(path, ago(days), ago(days)))
    });
    notes
        .into_iter()
        .fold(source, |s, (path, text, made, changed)| {
            s.with_file(note_from_text(path, text, ago(made), ago(changed)))
        })
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
const FILES: [(&str, &str); 16] = [
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
    ("note-base", "A base in a note.md"),
    // The lab vault's base: opens as a base (`architect_bases::ui::view`).
    ("base-status", BASE),
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

/// What note `path`'s tab says: a project's note (`<folder>/project.md`)
/// its project's name (its `title`, else its folder's: "Album", never
/// "project"); any other its file's name.
#[must_use]
pub fn tab_title(path: &str) -> String {
    let Some((folder, name)) = path.rsplit_once('/') else {
        return title_of(path);
    };
    if !name.eq_ignore_ascii_case("project.md") {
        return title_of(path);
    }
    let text = crate::notes::open_vault()
        .and_then(|v| std::fs::read_to_string(v.root().join(path)).ok())
        .unwrap_or_default();
    let (props, _) = crate::note::frontmatter(&text);
    props
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.trim().trim_matches('"').to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| folder.rsplit('/').next().unwrap_or(folder).to_owned())
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
        .or_else(|| pane.strip_prefix("base:").map(str::to_owned))
}

/// A loaded provider: its key (`<extension>/<id>`), its owner and its
/// declaration.
#[derive(Clone, PartialEq)]
struct LoadedProvider {
    key: String,
    extension: String,
    decl: NavigatorProviderDecl,
}

impl LoadedProvider {
    /// Where the left list shows it (`[[navigator_providers]] section`).
    fn place(&self) -> Place {
        if !self.decl.shown() {
            return Place::Hidden;
        }
        match self.decl.section.as_str() {
            "top" => Place::Top,
            "assets" => Place::Assets,
            _ => Place::Section,
        }
    }

    /// Its mount in the navigator: `~<extension>:<id>`.
    fn mount(&self) -> Mount {
        let mut m = Mount::provider(&self.key, self.decl.title.clone(), self.place());
        if !self.decl.icon.is_empty() {
            m = m.icon(self.decl.icon.clone());
        }
        if self.decl.nest == Some(false) {
            m = m.flat();
        }
        m
    }
}

/// The providers the loaded extensions declare, in the left list's order:
/// by section, then `position`, then title.
fn loaded_providers(
    ui: Option<architect_extensions::interface::ExtensionUi>,
) -> Vec<LoadedProvider> {
    let mut out: Vec<LoadedProvider> = ui
        .map(|ui| {
            let _ = ui.revision();
            ui.host()
                .contributions()
                .navigator_providers()
                .into_iter()
                .map(|by| LoadedProvider {
                    key: format!("{}/{}", by.owner, by.item.id),
                    extension: by.owner,
                    decl: by.item,
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| {
        (a.decl.position.unwrap_or(i32::MAX), &a.decl.title)
            .cmp(&(b.decl.position.unwrap_or(i32::MAX), &b.decl.title))
    });
    out
}

/// A provider in the navigator: its source (in its mount) and how it is
/// asked to open an item, by mount id. At the root: the sidebar comes and
/// goes, the feeds stay.
#[derive(Clone, Copy)]
struct Feeds(Signal<std::collections::HashMap<String, Feed>>);

#[derive(Clone)]
struct Feed {
    source: ProviderSource,
    provider: Arc<ActionProvider>,
}

fn use_feeds() -> Feeds {
    use_root_context(|| {
        Feeds(Signal::new_in_scope(
            std::collections::HashMap::new(),
            ScopeId::ROOT,
        ))
    })
}

/// The left dock's files: the navigator, one over every vault the person
/// reaches and every provider (#443): the providers' rows (Inbox,
/// Calendar, Projects) at the top, Recent Files/, Vaults/, Assets/,
/// Tags/. A page with a provider (one of its extension's
/// `[[navigator_providers]] views`) selects that provider's row.
#[component]
fn FilesView() -> Element {
    let ui = try_use_context::<architect_extensions::interface::ExtensionUi>();
    let providers = loaded_providers(ui);
    rsx! {
        // A provider left out of the list is not read at all.
        for l in providers.iter().filter(|l| l.place() != Place::Hidden).cloned() {
            ProviderFeed { key: "{l.key}", provider: l }
        }
        VaultFiles { providers }
    }
}

/// Runs the host's actions: how the app reads every provider
/// (`ActionProvider`), linked in or a process.
fn host_runner(host: architect_extensions::host::Host) -> architect_navigator_api::Run {
    Arc::new(move |id: String, args: serde_json::Value| {
        let list = host.list().clone();
        Box::pin(async move {
            let bytes = serde_json::to_vec(&args).map_err(|e| e.to_string())?;
            let out = list
                .run_json_async(&id, &bytes)
                .await
                .map_err(|e| e.to_string())?;
            if out.is_empty() {
                return Ok(serde_json::Value::Null);
            }
            serde_json::from_slice(&out).map_err(|e| e.to_string())
        })
    })
}

/// What a provider's snapshot says of its row: its count, its groups.
fn describe(m: &mut Mount, snap: &architect_navigator_api::Snapshot) {
    m.count = snap.count.and_then(|n| usize::try_from(n).ok());
    if snap.groups {
        m.nests = false;
        m.list = Some(architect_navigator::Appearance {
            descendants: Some(true),
            grouping: Some(architect_navigator::Grouping::Folder),
            ..architect_navigator::Appearance::default()
        });
    }
    m.group_by = snap.group_by.clone().map(|k| (k, snap.group_order.clone()));
    // A calendar (#444): a period lists what is in its days, by day, by
    // time.
    if snap.calendar.is_some() {
        m.list = Some(architect_navigator::Appearance {
            descendants: Some(true),
            grouping: Some(architect_navigator::Grouping::Folder),
            sort: Some(architect_navigator::SortOption::CreatedAsc),
            ..architect_navigator::Appearance::default()
        });
    }
    // A place in its own groups (a project's contents) lists itself, not
    // what is below it (a sub-project's are its own).
    if m.group_by.is_some() {
        m.list = Some(architect_navigator::Appearance {
            descendants: Some(false),
            ..architect_navigator::Appearance::default()
        });
    }
}

/// A provider's feed: its snapshot read through its actions now and every
/// few seconds into its mount of the navigator; a drag and New sent back
/// to it. Draws nothing.
#[component]
fn ProviderFeed(provider: LoadedProvider) -> Element {
    let pane = use_pane();
    let lab = use_lab_vault();
    let feeds = use_feeds();
    let ui = use_context::<architect_extensions::interface::ExtensionUi>();
    let mount = use_hook(|| provider.mount());
    let id = mount.id.clone();
    let remote = use_hook({
        let prefix = provider.decl.actions.clone();
        move || Arc::new(ActionProvider::new(prefix, host_runner(ui.host())))
    });
    let (requests, mut rx) = use_hook(|| {
        let (tx, rx) = futures_channel::mpsc::unbounded::<ProviderRequest>();
        (tx, CopyValue::new(Some(rx)))
    });
    let source = use_hook({
        let (id, title, remote) = (id.clone(), provider.decl.title, Arc::clone(&remote));
        move || {
            let kept = feeds.0.peek().get(&id).map(|f| f.source.clone());
            let source = kept.unwrap_or_else(|| {
                ProviderSource::with(architect_navigator_api::Snapshot::new(title))
            });
            // This feed's: a source kept from before sends here now.
            let source = source.on_request(move |r| {
                let _ = requests.unbounded_send(r);
            });
            lab.mounts
                .peek()
                .mount(mount.clone(), Arc::new(source.clone()));
            let mut feeds = feeds.0;
            feeds.write().insert(
                id,
                Feed {
                    source: source.clone(),
                    provider: remote,
                },
            );
            source
        }
    });
    use_drop({
        let id = id.clone();
        move || lab.mounts.peek().unmount(&id)
    });
    // Read now, then every three seconds: an extension's data changes
    // under it (a task written elsewhere).
    let reload = {
        let (remote, source, id) = (Arc::clone(&remote), source, id);
        move || {
            let (remote, source, id) = (Arc::clone(&remote), source.clone(), id.clone());
            async move {
                match remote.snapshot().await {
                    Ok(snap) => {
                        lab.mounts.peek().describe(&id, |m| describe(m, &snap));
                        source.load(snap);
                        // Its month grid's days, each its node's folder.
                        let grid = source.section_calendar(&id);
                        lab.mounts.peek().describe(&id, |m| m.calendar = grid);
                    }
                    Err(e) => {
                        tracing::warn!(provider = %id, error = %e, "lab.navigator.provider_unread");
                    }
                }
            }
        }
    };
    use_future({
        let reload = reload.clone();
        move || {
            let reload = reload.clone();
            async move {
                loop {
                    reload().await;
                    futures_timer::Delay::new(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    });
    // What the navigator asked of it (a drag, New): sent, then read again.
    let workspace = pane.workspace();
    let decks = architect_workspace::try_use_decks();
    let app_list = try_use_context::<Arc<architect::action::ActionList>>();
    use_future({
        let (remote, reload) = (Arc::clone(&remote), reload);
        move || {
            let (remote, reload) = (Arc::clone(&remote), reload.clone());
            let (decks, app_list) = (decks.clone(), app_list.clone());
            async move {
                let Some(mut rx) = rx.write().take() else {
                    return;
                };
                while let Some(r) = futures_util::StreamExt::next(&mut rx).await {
                    let done = match r {
                        ProviderRequest::Move { items, to } => {
                            remote.move_items(items, to).await.map(|()| None)
                        }
                        ProviderRequest::Create { container, title } => {
                            remote.create(container, title).await.map(Some)
                        }
                    };
                    match done {
                        Ok(Some(open)) => follow(
                            open,
                            &Opening {
                                workspace,
                                decks: decks.clone(),
                                ui,
                                app: app_list.clone(),
                                lab,
                                target: Target::Tab,
                                new_deck: false,
                            },
                        ),
                        Ok(None) => {}
                        Err(e) => tracing::warn!(error = %e, "lab.navigator.provider_request"),
                    }
                    reload().await;
                }
            }
        }
    });
    rsx! {}
}

/// Where and how an item opens.
#[derive(Clone)]
struct Opening {
    workspace: Signal<Workspace>,
    decks: Option<architect_workspace::DeckHandle>,
    ui: architect_extensions::interface::ExtensionUi,
    /// The app's own list (the inbox's actions are on it).
    app: Option<Arc<architect::action::ActionList>>,
    lab: LabVault,
    target: Target,
    new_deck: bool,
}

/// Opens what a provider says an item opens.
fn follow(open: architect_navigator_api::Open, how: &Opening) {
    use architect_navigator_api::Open as O;
    let Opening {
        workspace,
        ref decks,
        ref ui,
        ref app,
        lab,
        target,
        new_deck,
    } = *how;
    match open {
        O::Nothing => {}
        // A song, a project: loaded onto the deck on show, or a new one.
        O::Context {
            context_type,
            id,
            title,
            folder,
            properties,
        } => {
            let mut context = architect_workspace::DeckContext::new(context_type, id, title);
            context.folder = folder;
            // Its properties make its tab (a song's key and tempo); its
            // `color` is its tab's colour.
            for (k, v) in properties {
                if k == "color" || k == "colour" {
                    context.colour = Some(v.clone());
                }
                context = context.property(k, v);
            }
            let how = if new_deck {
                architect_workspace::Load::NewDeck
            } else {
                architect_workspace::Load::Here
            };
            if let Some(decks) = decks {
                decks.load(context, how);
            }
        }
        O::File { path } => {
            if lab.disk.peek().is_some() {
                show_vault_note(workspace, &path, target);
            } else {
                let _ = ui.open_path(&path);
            }
        }
        O::View {
            extension,
            view,
            state,
        } => {
            let list = ui.host().list().clone();
            let args = serde_json::json!({ "extension": extension, "view": view, "state": state });
            spawn(async move {
                let _ = list
                    .run_json_async("EXT_VIEW_OPEN", args.to_string().as_bytes())
                    .await;
            });
        }
        // A capture of the Queue's Inbox: the app's triage card.
        O::Action { id, args } if id == crate::capture::PROCESS_CARD => {
            let path = args
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let list = app
                .clone()
                .unwrap_or_else(|| Arc::new(ui.host().list().clone()));
            crate::capture::process_card(list, path);
        }
        O::Action { id, args } => {
            let list = ui.host().list().clone();
            spawn(async move {
                let _ = list.run_json_async(&id, args.to_string().as_bytes()).await;
            });
        }
    }
}

/// A note pane's state as the navigator's path: the app's vault's notes
/// are kept by their vault path, another org's as `@<org>/<path>`.
fn nav_path(own: &str, state: &str) -> String {
    if architect_navigator::mounts::split(state).is_some() {
        state.to_owned()
    } else if state.is_empty() {
        own.to_owned()
    } else {
        format!("{own}/{state}")
    }
}

/// A view kept before the navigator held every vault: its folders, as
/// the app's vault's.
fn mounted_view(mut v: ViewState, own: &str) -> ViewState {
    let fix = |p: &mut String| {
        if architect_navigator::mounts::split(p).is_none() {
            *p = nav_path(own, p);
        }
    };
    if !v.expanded.iter().any(|p| p.starts_with('@')) {
        v.expanded.iter_mut().for_each(fix);
        v.expanded.push(String::new());
        if let Some(f) = v.folder.as_mut() {
            fix(f);
        }
    }
    v
}

/// Each deck's navigator selection, for the run (#468). The sidebar is
/// shared, one navigator for every deck (its settings, pins, folds and
/// width); what it has selected is each deck's own.
#[derive(Default)]
struct Picks {
    /// The deck the navigator last followed.
    last: Option<architect_workspace::DeckId>,
    /// What each deck had selected when it was last on show.
    by: std::collections::HashMap<architect_workspace::DeckId, NavTarget>,
    /// A deck whose context's row is not in the navigator yet (its
    /// provider's snapshot is on its way): selected once it is.
    pending: Option<architect_workspace::DeckContext>,
}

#[derive(Clone, Copy)]
struct DeckPicks(CopyValue<Picks>);

/// At the root: the sidebar is drawn again with each deck's layout.
fn use_deck_picks() -> DeckPicks {
    use_root_context(|| DeckPicks(CopyValue::new_in_scope(Picks::default(), ScopeId::ROOT)))
}

/// The navigator row a deck's context begins on:
/// - the vault's: the root, every vault and provider;
/// - a project's: its row in the Projects provider (projects | contents,
///   with its Songs, Notes and Tasks);
/// - a provider's context (a setlist's song): the container it is in (its
///   setlist).
///
/// `None` while that row is not in the navigator yet.
fn deck_home(feeds: Feeds, context: &architect_workspace::DeckContext) -> Option<String> {
    if context.kind == crate::decks::VAULT {
        return Some(String::new());
    }
    let feeds = feeds.0.peek();
    if context.kind == crate::decks::PROJECT {
        let page = format!("{}/project.md", context.folder.as_deref()?);
        return feeds.iter().find_map(|(mount, f)| {
            f.source
                .folder_of(&page)
                .map(|folder| format!("{mount}/{folder}"))
        });
    }
    feeds.iter().find_map(|(mount, f)| {
        let item = f.source.snapshot().items.into_iter().find(|i| {
            matches!(&i.open, Some(architect_navigator_api::Open::Context { context_type, id, .. })
                if *context_type == context.kind && *id == context.id)
        })?;
        let folder = f.source.folder_of(&item.container)?;
        Some(format!("{mount}/{folder}"))
    })
}

/// The project a provider's row at `rest` in mount `mount` is (Projects'
/// row for it, its node a `project.md`), as a deck's context.
fn provider_project(
    feeds: Feeds,
    mount: &str,
    rest: &str,
) -> Option<architect_workspace::DeckContext> {
    let node = feeds.0.peek().get(mount)?.source.node_at(rest)?;
    let folder = node.id.strip_suffix("/project.md")?;
    crate::decks::project_context(folder)
}

/// Selects in the navigator what deck `on_deck` had selected, or its
/// context's row ([`deck_home`]); while the same deck stays on show, its
/// selection is kept as it changes.
fn follow_deck(
    nav: &architect_navigator::ui::Navigator,
    picks: DeckPicks,
    feeds: Feeds,
    on_deck: Option<(
        architect_workspace::DeckId,
        architect_workspace::DeckContext,
    )>,
) {
    // Read, so a change of selection (or a provider's snapshot arriving)
    // runs this again.
    let target = nav.state.read().target();
    let Some((id, context)) = on_deck else {
        return;
    };
    let mut cell = picks.0;
    let mut p = cell.write();
    let show = |want: &str| {
        nav.state.peek().index().has_folder(want).then(|| {
            let want = want.to_owned();
            move || nav.update(|s| s.show_folder(&want))
        })
    };
    if p.last.as_ref() == Some(&id) {
        if let Some(c) = p.pending.clone() {
            if let Some(go) = deck_home(feeds, &c).as_deref().and_then(show) {
                p.pending = None;
                drop(p);
                go();
            }
            return;
        }
        p.by.insert(id, target);
        return;
    }
    let first = p.last.is_none();
    p.last = Some(id.clone());
    p.pending = None;
    let kept = p.by.get(&id).cloned();
    if let Some(t) = kept {
        drop(p);
        nav.update(|s| s.select_nav(&t));
        return;
    }
    // The app's first deck keeps what the navigator had open last run.
    if first && context.kind == crate::decks::VAULT {
        p.by.insert(id, target);
        return;
    }
    match deck_home(feeds, &context).as_deref().and_then(show) {
        Some(go) => {
            drop(p);
            go();
        }
        None => p.pending = Some(context),
    }
}

/// The navigator over every vault and provider. A click opens a note here
/// (one open already comes forward); the note on show is revealed; a
/// right-click is the file's context menu, with what extensions add
/// (`[[context]] target = "file"`).
#[component]
fn VaultFiles(providers: Vec<LoadedProvider>) -> Element {
    let pane = use_pane();
    // The dock is drawn by the shell, above the page: the vaults and what
    // the navigator had open are the app's, kept at its root.
    let lab = use_lab_vault();
    let LabVault {
        source, kept, disk, ..
    } = lab;
    let own = lab.own.peek().clone();
    let feeds = use_feeds();
    let nav = use_navigator(move || source.cloned());
    use_hook({
        let own = own.clone();
        move || {
            // What the navigator had open last run (its search, profiles,
            // shortcuts and recent notes too), from the app's folder.
            if kept.peek().is_none()
                && disk.peek().is_some()
                && let Some(v) = view_store().and_then(|s| s.load().ok())
            {
                let mut kept = kept;
                kept.set(Some(mounted_view(v, &own)));
            }
            if let Some(v) = kept.peek().as_ref() {
                nav.restore(v);
            }
            // The demo vault keeps its periodic notes in `Journal/Daily`,
            // `Journal/Weekly` (ISO weeks), …: the calendar reads them there,
            // while the periodic notes are still Notebook Navigator's defaults.
            if disk.peek().is_some() {
                nav.settings.update(demo_vault_periodic_notes);
            }
        }
    });
    // Kept as it changes, every two seconds at most: an app that quits
    // runs no drop.
    use_future(move || async move {
        let Some(store) = disk.peek().as_ref().and_then(|_| view_store()) else {
            return;
        };
        let mut saved: Option<ViewState> = None;
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(2)).await;
            let view = nav.view_state();
            if saved.as_ref() != Some(&view) {
                let _ = store.save(&view);
                saved = Some(view);
            }
        }
    });
    // A vault note opened by path from anywhere (an extension's `open`, a
    // notification's "go to"): its pane in the main area, as a click here.
    let path_pane = pane.clone();
    let opener = use_hook(move || {
        let (tx, rx) = futures_channel::mpsc::unbounded::<String>();
        if let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>() {
            ui.host().set_path_opener(move |path| {
                let markdown = std::path::Path::new(path)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"));
                if !markdown {
                    return Err(format!("nothing opens {path}"));
                }
                tx.unbounded_send(path.to_owned())
                    .map_err(|_| "the window is gone".to_owned())?;
                Ok(serde_json::json!({ "opened_by": "app", "path": path }))
            });
        }
        std::rc::Rc::new(std::cell::RefCell::new(Some(rx)))
    });
    use_future(move || {
        let taken = opener.borrow_mut().take();
        let workspace = path_pane.workspace();
        async move {
            use futures_util::StreamExt as _;
            let Some(mut rx) = taken else { return };
            while let Some(path) = rx.next().await {
                // Only a vault on disk has notes by path.
                if disk.peek().is_some() {
                    show_vault_note(workspace, &path, Target::Tab);
                }
            }
        }
    });
    use_drop(move || {
        let mut kept = kept;
        let view = nav.view_state();
        if disk.peek().is_some()
            && let Some(store) = view_store()
        {
            let _ = store.save(&view);
        }
        kept.set(Some(view));
    });
    let decks = architect_workspace::try_use_decks();
    // The navigator follows the deck on show (#468): each deck has its own
    // selection, its context's row to begin with.
    // The general tabs are the vault's: their selection is theirs too.
    let on_deck = decks.map(|d| {
        let decks = d.decks.read();
        let x = decks.shown();
        (x.id.clone(), x.context.clone())
    });
    // A page with a provider selects its row: once each time it comes on
    // show, so a click elsewhere holds. Not on a context's deck: there the
    // context's row is selected (`follow_deck`).
    let on_vault = on_deck
        .as_ref()
        .is_none_or(|(_, c)| c.kind == crate::decks::VAULT);
    let want: Option<String> = {
        let workspace = pane.workspace();
        let w = workspace.read();
        w.current_document()
            .and_then(|id| w.pane(id))
            .and_then(|p| {
                let (extension, view) = p.view.0.split_once('/')?;
                providers
                    .iter()
                    .find(|l| {
                        l.place() != Place::Hidden
                            && l.extension == extension
                            && l.decl.views.iter().any(|v| v == view)
                    })
                    .map(|l| l.mount().id)
            })
            .filter(|_| on_vault)
    };
    use_effect(use_reactive!(|want| {
        if let Some(id) = want {
            nav.update(|s| s.select_folder(&id));
        }
    }));
    // A pop-out window has no screen of the lab's: a desk's.
    let phone =
        try_use_context::<Signal<Screen>>().is_some_and(|s| s().form() == FormFactor::Phone);
    // The note on show, lit and revealed.
    let open_now = {
        let workspace = pane.workspace();
        let w = workspace.read();
        w.current_document().and_then(|p| {
            w.pane(p)
                .filter(|p| p.view.0 == VAULT_NOTE)
                .map(|p| nav_path(&own, &p.state))
                .or_else(|| path_of(&p.0).map(|p| nav_path(&own, &p)))
        })
    };
    // In the shell's sidebar on a desk, as many columns as it has. A memo
    // of that alone: reading the shell itself here would draw the view
    // again each time the shell draws, and the shell again with it.
    let shell = try_use_context::<Signal<architect_shell::Shell>>();
    let sidebar_columns = use_memo(move || {
        shell.and_then(|s| {
            let s = s.read();
            s.desk().then(|| s.sidebar_layout().columns())
        })
    });
    let in_main = pane
        .workspace()
        .read()
        .locate(&pane.id)
        .is_some_and(|l| l.window == 0);
    let columns = sidebar_columns().filter(|_| in_main);
    let picks = use_deck_picks();
    use_effect(use_reactive!(|on_deck| follow_deck(
        &nav, picks, feeds, on_deck
    )));
    // A project deck's own note on show (its page, its songs) leaves the
    // project selected rather than revealing the note in its vault folder.
    let held = on_deck
        .as_ref()
        .filter(|(_, c)| c.kind == crate::decks::PROJECT)
        .and_then(|(_, c)| c.folder.as_deref())
        .zip(open_now.as_deref())
        .is_some_and(|(folder, path)| path.starts_with(&format!("{own}/{folder}/")));
    use_effect(use_reactive!(|open_now, held| {
        let on_provider = matches!(
            nav.state.peek().target(),
            NavTarget::Folder(f) if f.starts_with('~')
        );
        nav.set_active_with(open_now.as_deref(), !(held && on_provider));
    }));
    let open_pane = pane.clone();
    let menu_pane = pane;
    // A project's folder (or its project.md) in the app's vault, or a
    // provider's item that is a context, loads as a deck: a ⌘-click or a
    // middle-click on one loads it onto a new deck.
    {
        let own = own.clone();
        use_context_provider(move || {
            architect_navigator::ui::DeckRows(std::rc::Rc::new(move |path: &str| {
                match architect_navigator::mounts::split(path) {
                    Some((m, rest)) if m == own => {
                        crate::decks::project_context(project_folder(rest)).is_some()
                    }
                    Some((m, rest)) if m.starts_with('~') => {
                        provider_project(feeds, m, rest).is_some()
                            || feeds.0.peek().get(m).is_some_and(|f| {
                                f.source.item_at(rest).is_some_and(|i| {
                                    matches!(
                                        i.open,
                                        Some(architect_navigator_api::Open::Context { .. })
                                    )
                                })
                            })
                    }
                    _ => false,
                }
            }))
        });
    }
    let ui = try_use_context::<architect_extensions::interface::ExtensionUi>();
    let app_list = try_use_context::<Arc<architect::action::ActionList>>();
    let on_open = EventHandler::new({
        let own = own.clone();
        move |o: Open| {
            let target = match (o.how, o.browsing) {
                (OpenIn::Split, _) => Target::Split(Side::Right),
                (_, true) => Target::Here,
                (OpenIn::Here | OpenIn::Tab | OpenIn::NewDeck, false) => Target::Tab,
            };
            let new_deck = o.how == OpenIn::NewDeck;
            let (mount, rest) =
                architect_navigator::mounts::split(&o.path).unwrap_or(("", &o.path));
            // A provider's item: as it says, else as the provider says.
            // A project's row in Projects: onto a deck of its own.
            if new_deck
                && let (Some(decks), Some(context)) = (decks, provider_project(feeds, mount, rest))
            {
                decks.load(context, architect_workspace::Load::NewDeck);
                return;
            }
            if mount.starts_with('~') {
                let (Some(ui), Some(feed)) = (ui, feeds.0.peek().get(mount).cloned()) else {
                    return;
                };
                let Some(item) = feed.source.item_at(rest) else {
                    return;
                };
                let how = Opening {
                    workspace: open_pane.workspace(),
                    decks: decks.clone(),
                    ui,
                    app: app_list.clone(),
                    lab,
                    target,
                    new_deck,
                };
                if let Some(open) = item.open {
                    follow(open, &how);
                } else {
                    spawn(async move {
                        match feed.provider.open(item.id).await {
                            Ok(open) => follow(open, &how),
                            Err(e) => nav.say(e),
                        }
                    });
                }
                return;
            }
            let markdown = has_ext(rest, "md");
            // Another org's note: opened from its own vault, without
            // switching the app's.
            if mount != own {
                if markdown && disk.peek().is_some() {
                    show_vault_note(open_pane.workspace(), &o.path, target);
                }
                return;
            }
            let path = rest.to_owned();
            if new_deck
                && let (Some(decks), Some(context)) =
                    (decks, crate::decks::project_context(project_folder(&path)))
            {
                decks.load(context, architect_workspace::Load::NewDeck);
                return;
            }
            if has_ext(&path, "base") {
                // A base: its pane (one per base), in the main area beside
                // the note on show; open already, it comes forward.
                let workspace = open_pane.workspace();
                let id = architect_workspace::PaneId::new(architect_bases::actions::pane_id(&path));
                let open = workspace.peek().pane(&id).is_some();
                if open {
                    let mut workspace = workspace;
                    let _ = workspace.write().activate(&id);
                } else {
                    open_in_main(workspace, base_pane(&path), target);
                }
            } else if markdown && disk.peek().is_some() {
                // Beside the note on show, not in the dock pressed.
                show_vault_note(open_pane.workspace(), &path, target);
            } else if let Some((id, title)) = note_of(&path).filter(|_| disk.peek().is_none()) {
                // A click opens a tab in the group of the note on show; the
                // arrow keys show each note in its place.
                let mut workspace = open_pane.workspace();
                let mut w = workspace.write();
                if let Some(doc) = w.current_document().cloned() {
                    let _ = w.activate(&doc);
                }
                let _ = w.open(note(&id, &title), target);
            } else if let Some(ui) =
                try_consume_context::<architect_extensions::interface::ExtensionUi>()
            {
                let _ = ui.open_path(&path);
            }
        }
    });
    let on_context_menu = EventHandler::new({
        let own = own;
        move |(path, e): (String, MouseEvent)| {
            // The app's vault's files by their vault path, as every
            // `[[context]] target = "file"` takes them.
            let path = match architect_navigator::mounts::split(&path) {
                Some((m, rest)) if m == own => rest.to_owned(),
                _ => path,
            };
            menu_pane.context_menu("file", path, &e);
        }
    });
    // New note in the list's header is the navigator's own: from the
    // folder's template, opened through `on_open` (#339); in a provider's
    // container, an item it makes.
    let on_new_note: Option<EventHandler<String>> = None;
    // Extensions' rows in the navigator's own menus (NN's MenusAPI): their
    // `[[context]]` on `navigator:folder`, `navigator:tag`,
    // `navigator:property`, run as a context menu's row is, told the
    // target and its id.
    let entries = try_use_context::<architect_workspace::ContextEntries>();
    let commands = architect_commands::try_use_commands();
    use_context_provider(move || architect_navigator::ui::MenuExtra {
        rows: std::rc::Rc::new(move |target: &str, id: &str| {
            entries
                .as_ref()
                .map(|e| {
                    (e.0)(target, id)
                        .into_iter()
                        .map(|x| (x.label, x.command))
                        .collect()
                })
                .unwrap_or_default()
        }),
        run: std::rc::Rc::new(move |command: &str, target: &str, id: &str| {
            if let Some(c) = &commands {
                let invocation = architect::action::Invocation::new(
                    architect::action::InvocationSource::ContextMenu,
                )
                .on(target, id);
                let _ = c.run_invoked(command, None, invocation);
            }
        }),
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
            columns,
            on_new_note,
            on_vault: None,
        })
    });
    if mount.peek().phone != phone || mount.peek().columns != columns {
        let mut m = mount.write();
        m.phone = phone;
        m.columns = columns;
    }
    try_use_context::<architect_extensions::interface::ExtensionUi>().map_or_else(
        || {
            rsx! {
                NavigatorView { navigator: nav, phone, on_open, on_context_menu, columns, on_new_note }
            }
        },
        |ui| {
            rsx! {
                architect_extensions::interface::ExtensionView {
                    ui,
                    extension: architect_navigator_extension::ID.to_owned(),
                    view: "files".to_owned(),
                    bare: true,
                }
            }
        },
    )
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

/// How long typing rests before it is saved.
const SAVE_AFTER_MS: u64 = 400;

/// A note of the vault on disk, its path the pane's state: its title (a
/// rename) over the note in the editor (`crate::note_editor`), saved to its
/// file as it is typed, an outside change merged in.
#[component]
fn VaultNoteView() -> Element {
    let pane = use_pane();
    let id = pane.id.0.clone();
    let path = pane.state();
    if use_lab_vault().vault_of(&path).0.is_none() || path.is_empty() {
        return rsx! { Stand { what: "No vault is open", next: "Open the lab on a folder of notes (just lab)." } };
    }
    // Keyed by the pane (in a list, where keys count: a lone root's key
    // does not remount it, and the next note would get this one's
    // editor): a rename keeps the editor and its typing.
    rsx! {
        for id in [id] {
            VaultNote { key: "{id}", path: path.clone() }
        }
    }
}

#[component]
fn VaultNote(path: String) -> Element {
    let pane = use_pane();
    let t = use_shell().theme;
    // The vault it is in (the app's, or another org's for `@<org>/…`),
    // and its path there: its links resolve in that vault (#443).
    let lab = use_lab_vault();
    let (vault, path) = lab.vault_of(&path);
    let disk = use_hook(|| CopyValue::new(vault));
    let mount = use_hook(|| {
        CopyValue::new(
            architect_navigator::mounts::split(&pane.state())
                .map(|(m, _)| m.to_owned())
                .filter(|m| *m != *lab.own.peek()),
        )
    });
    // A path in its vault as this app keeps a note pane's state.
    let state_of = move |p: &str| {
        mount
            .peek()
            .as_ref()
            .map_or_else(|| p.to_owned(), |m| format!("{m}/{p}"))
    };
    // The note's path now (a rename moves it).
    let mut at = use_signal(|| path.clone());
    use_effect(use_reactive!(|path| {
        if *at.peek() != path {
            at.set(path);
        }
    }));
    let first = use_hook(|| {
        disk.peek()
            .as_ref()
            .and_then(|d| d.vault().text(&path))
            .unwrap_or_default()
    });
    // What the file held when last read or saved, and what is typed: its
    // properties (the Properties bar's) and its body (the editor's).
    let base = use_hook(|| CopyValue::new(first.clone()));
    let ours = use_hook(|| CopyValue::new(first.clone()));
    let (block0, body0) = crate::properties::split(&first);
    let mut block = use_signal(|| block0.to_owned());
    let body = use_hook(|| CopyValue::new(body0.to_owned()));
    let body0 = body0.to_owned();
    let types = use_hook(|| crate::properties::Types {
        declared: std::sync::Arc::new(
            disk.peek()
                .as_ref()
                .map(|d| crate::properties::declared_types(&d.vault().root()))
                .unwrap_or_default(),
        ),
    });
    let mut incoming = use_signal(|| None::<(u64, String)>);
    let mut gone = use_signal(|| false);
    let serial = use_hook(|| CopyValue::new(0_u64));
    // The typing written to the file, when it differs from what is there.
    let save = move || {
        let (mut base, ours) = (base, ours);
        let text = ours.cloned();
        if text == *base.peek() {
            return;
        }
        let Some(d) = disk.peek().clone() else { return };
        let p = at.peek().clone();
        match d.save(&p, &text) {
            Ok(()) => base.set(text),
            Err(e) => tracing::warn!(path = %p, error = %e, "lab.vault.save_failed"),
        }
    };
    let later = move || {
        let mut serial = serial;
        let n = serial.cloned().wrapping_add(1);
        serial.set(n);
        spawn(async move {
            futures_timer::Delay::new(std::time::Duration::from_millis(SAVE_AFTER_MS)).await;
            if serial.cloned() == n {
                save();
            }
        });
    };
    // Never lose a keystroke: what is typed is written as the note goes.
    use_drop(save);
    // The vault's changes: an outside edit merged into the typing, a
    // rename followed, a delete said.
    let sub = use_hook(|| {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<SourceEvent>();
        let sub = disk.peek().as_ref().map(|d| {
            d.subscribe(Box::new(move |e| {
                let _ = tx.unbounded_send(e.clone());
            }))
        });
        let pane = pane.clone();
        spawn(async move {
            use futures_util::StreamExt as _;
            let mut n = 0_u64;
            while let Some(event) = rx.next().await {
                let here = at.peek().clone();
                match event {
                    SourceEvent::FileModified(f) if f.path == here => {
                        let Some(theirs) = disk.peek().as_ref().and_then(|d| d.vault().text(&here))
                        else {
                            continue;
                        };
                        let (mut base, mut ours) = (base, ours);
                        if theirs == *base.peek() {
                            continue; // its own save
                        }
                        let merged = vault_doc::merge3(&base.peek(), &ours.peek(), &theirs);
                        base.set(theirs.clone());
                        if merged != *ours.peek() {
                            ours.set(merged.clone());
                            let (b, rest) = crate::properties::split(&merged);
                            if *block.peek() != b {
                                block.set(b.to_owned());
                            }
                            let mut body = body;
                            if *body.peek() != rest {
                                body.set(rest.to_owned());
                                n = n.wrapping_add(1);
                                incoming.set(Some((n, rest.to_owned())));
                            }
                        }
                        if merged != theirs {
                            later();
                        }
                    }
                    SourceEvent::FileRenamed { from, file } if from == here => {
                        at.set(file.path.clone());
                        pane.set_state(state_of(&file.path));
                        pane.set_title(tab_title(&file.path));
                    }
                    SourceEvent::FileDeleted(p) if p == here => gone.set(true),
                    _ => {}
                }
            }
        });
        std::rc::Rc::new(sub)
    });
    let _ = &sub;
    let here = at();
    let title = title_of(&here);
    // Its tab says its name (a project's note: its project's), a pane
    // kept from before too.
    let tab = tab_title(&here);
    let titled = pane.clone();
    use_effect(use_reactive!(|tab| {
        if titled.title() != tab {
            titled.set_title(tab);
        }
    }));
    let mut draft = use_signal(|| None::<String>);
    let rename = move || {
        let mut draft = draft;
        let Some(name) = draft.take() else { return };
        let name = name.trim().replace('/', " ");
        let from = at.peek().clone();
        if name.is_empty() || name == title_of(&from) {
            return;
        }
        save();
        let folder = from.rsplit_once('/').map(|(f, _)| f.to_owned());
        let to = folder.map_or_else(|| format!("{name}.md"), |f| format!("{f}/{name}.md"));
        if let Some(d) = disk.peek().as_ref()
            && let Err(e) = d.rename(&from, &to)
        {
            tracing::warn!(from = %from, to = %to, error = %e, "lab.vault.rename_failed");
        }
    };
    let words = ours.cloned().split_whitespace().count();
    use_pane_status(vec![
        StatusItem::new("words", crate::count(words, "word")).left(),
    ]);
    let shown = draft().unwrap_or_else(|| title.clone());
    // A link followed: the vault's note it names, in the main area; a
    // web link, by the extension that claims it.
    let workspace = pane.workspace();
    let open_link = move |href: String| {
        if href.contains("://") {
            if let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>()
            {
                let _ = ui.open_link(&href);
            }
            return;
        }
        let name = crate::properties::link_target(&href);
        let path = disk
            .peek()
            .clone()
            .map(crate::note_editor::NoteVault)
            .and_then(|v| v.file_path(&name));
        match path {
            Some(path) if has_ext(&path, "md") => {
                show_vault_note(workspace, &state_of(&path), Target::Tab);
            }
            Some(path) if has_ext(&path, "base") => {
                open_in_main(workspace, base_pane(&path), Target::Tab);
            }
            Some(path) => tracing::info!(link = %path, "lab.vault.link_not_a_note"),
            None => tracing::info!(link = %name, "lab.vault.link_unresolved"),
        }
    };
    // A collection (#668) draws as the core list at its head, as the
    // fallback of its type's embed: an extension that claims the type
    // draws instead. While the list draws, its own rows (`sets`, `songs`,
    // …) leave the Properties bar and its listing fences fold.
    let fields = architect_media::front::fields(&block());
    let note_type = fields
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let is_list = architect_media::Collection::read(&fields).is_some();
    let claimed = !note_type.is_empty()
        && try_consume_context::<architect_extensions::interface::ExtensionUi>().is_some_and(
            |ui| {
                let _ = ui.revision();
                ui.host()
                    .embeds()
                    .resolve(&architect_extensions::contrib::Claim::NoteType(
                        note_type.clone(),
                    ))
                    .is_some()
            },
        );
    let core_list = is_list && !claimed;
    let hidden: Vec<String> = if core_list {
        architect_media::Collection::fields_in(&fields)
            .into_iter()
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    let head = is_list.then(|| disk.peek().clone()).flatten().map(|d| {
        let list = rsx! {
            crate::collection::CollectionHead {
                vault: crate::collection::VaultHandle(d),
                path: here.clone(),
                title: shown.clone(),
                text: block(),
                on_open: move |p: String| show_vault_note(workspace, &state_of(&p), Target::Tab),
            }
        };
        if note_type.is_empty() {
            list
        } else {
            rsx! {
                architect_extensions::embeds::EmbedView {
                    claim: architect_extensions::contrib::Claim::NoteType(note_type.clone()),
                    source: body.cloned(),
                    props: serde_json::Value::Object(fields.clone()),
                    fallback: list,
                }
            }
        }
    });
    rsx! {
        div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column; gap: 10px; padding: 24px 36px; overflow-y: scroll;",
            "data-vault-note": "{here}",
            input {
                "data-testid": "vault-note-title",
                "aria-label": "Note title",
                style: "font-size: 26px; font-weight: 750; color: {t.ink}; background: transparent; border: none; outline: none; padding: 0;",
                value: "{shown}",
                oninput: move |e| draft.set(Some(e.value())),
                onkeydown: move |e: KeyboardEvent| {
                    if e.key().to_string() == "Enter" {
                        rename();
                    }
                },
                onblur: move |_| rename(),
            }
            if gone() {
                div { style: "color: {t.ink_3}; font-size: 13px;", "This note's file was deleted." }
            }
            {head}
            crate::properties::PropertiesBar {
                block: block(),
                types,
                hidden,
                on_change: move |b: String| {
                    let mut ours = ours;
                    ours.set(format!("{b}{}", body.peek()));
                    block.set(b);
                    later();
                },
                on_link: move |name: String| open_link(name),
            }
            crate::note_editor::NoteEditor {
                id: here.clone(),
                text: body0,
                incoming: incoming(),
                vault: disk.peek().clone().map(crate::note_editor::NoteVault),
                on_link: move |href: String| open_link(href),
                fold_listing: core_list,
                on_change: move |value: String| {
                    let (mut ours, mut body) = (ours, body);
                    ours.set(format!("{}{value}", block.peek()));
                    body.set(value);
                    later();
                },
            }
        }
    }
}

#[cfg(test)]
mod start_tests {
    use architect_workspace::Docks;

    /// The Vault starts with every dock in the main window: no layout it
    /// ships pops one out (#353: the sidebar came up as a window of its
    /// own), and none does once the workspace repairs it.
    #[test]
    fn the_vault_starts_with_every_dock_in_the_main_window() {
        let mut checked = 0;
        for set in super::screensets().sets {
            for layout in set.layouts {
                let mut w = layout.workspace;
                w.repair();
                assert_eq!(w.windows.len(), 1, "{}: one window", set.name);
                for side in Docks::SIDES {
                    assert!(
                        w.dock_window(side).is_none(),
                        "{}: {side:?} dock at home",
                        set.name
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 3, "Write, Mix and Embedded views");
        let first = super::first_workspace();
        assert_eq!(first.windows.len(), 1);
        assert!(
            first.windows[0].docks.left.root.is_some(),
            "the navigator's dock is the main window's"
        );
    }

    /// The film's in-memory vault shows the list pane's samples from the
    /// demo vault's own files: one source of sample content.
    #[test]
    fn the_in_memory_samples_are_the_demo_vaults_files() {
        use architect_navigator::{NavigatorSource, TaskCounts};
        let s = super::navigator_samples(architect_navigator::MemorySource::new("Lab"), 0);
        let plan = s.file("Projects/Launch plan.md").expect("the launch plan");
        assert_eq!(
            plan.tasks,
            Some(TaskCounts {
                total: 6,
                unfinished: 2
            })
        );
        assert!(
            plan.tags.contains(&"projects/launch".to_owned()),
            "{:?}",
            plan.tags
        );
        assert!(plan.words.is_some_and(|w| w > 1000), "{:?}", plan.words);
        let image = plan.feature_image.expect("a feature image");
        assert!(image.contains("Attachments/Launch logo.svg"), "{image}");
        assert!(
            s.resource_url("Attachments/Launch logo.svg")
                .is_some_and(|u| u.starts_with("data:image/svg+xml"))
        );
        assert!(
            s.resource_url("Attachments/Sketch.excalidraw.png")
                .is_some_and(|u| u.starts_with("data:image/png;base64,"))
        );
        for path in [
            "Attachments/Brief.pdf",
            "Attachments/Board.canvas",
            "Projects/Vault/Vault.md",
        ] {
            assert!(s.file(path).is_some(), "{path}");
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("demo-vault")
                    .join(path)
                    .is_file(),
                "{path} is a demo vault file"
            );
        }
    }
}
