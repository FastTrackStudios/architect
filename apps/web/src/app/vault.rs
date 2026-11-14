//! The Vault tab in a browser: the workspace host, as the desktop's
//! (`apps/ui/src/vault.rs` `VaultPage`): the navigator in the left dock
//! behind a ribbon, notes in the center (more than one may be open, in
//! tabs and splits), a phone's tab switcher. The vault is a
//! `MemorySource`: the demo vault ([`demo_vault`]) in the tab's memory.
//!
//! A note is edited as its markdown in a text area: the editor's web view
//! (`architect_editor`, feature `web`) runs its DOM work through
//! `document::eval`, which this page's Content-Security-Policy refuses
//! (`page`). Its properties show above it, as they are.

use std::sync::Arc;

use architect_navigator::markdown::note_from_markdown;
use architect_navigator::source::FileOp;
use architect_navigator::ui::{NavigatorView, Open, use_navigator};
use architect_navigator::{MemorySource, Millis, NavigatorSource, NoteFile, OpenIn, PropertyValue};
use architect_shell::{Action, FormFactor, PageBar, Screen, Toolbar, icon, use_shell};
use architect_workspace::{
    Command, FormClass, Pane, Ribbon, Screensets, Side, Target, TitleRow, View, ViewKind, Views,
    Workspace, WorkspaceView, use_pane,
};
use dioxus::prelude::*;
use lucide_dioxus::{FileText, Files, Layers, Puzzle, SquarePen};

/// ⌘K's (and a phone's bar's) New note.
pub const NEW_NOTE: &str = "VAULT_NEW_NOTE";
/// A phone's Files: the navigator, in the shell's left drawer.
pub const FILES: &str = "VAULT_FILES";

/// The view a note of the vault is; its pane's state is the note's path.
const NOTE: &str = "vault.note";

/// The view an island's view on a note is (a favourite's `{view}`, #607);
/// its pane's state is `crate::favourites::frame_state`.
const FRAME: &str = "ext.frame";

/// Whether a note of the demo was edited or made since the page loaded:
/// leaving the demo then asks first (`super::sign_in`).
static EDITED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the demo vault was changed here.
pub fn demo_edited() -> bool {
    EDITED.load(std::sync::atomic::Ordering::Relaxed)
}

fn edited() {
    EDITED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The vault the app shows, in context above the Vault tab: the demo's,
/// or the signed-in account's org vaults (`org`, which reads notes not yet
/// read and writes edits back to the node).
#[derive(Clone)]
pub struct WebVault {
    pub source: MemorySource,
    pub org: Option<super::org::OrgVaults>,
}

/// Unix milliseconds now.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)] // a time in ms, well inside i64
pub fn now() -> Millis {
    js_sys::Date::now() as Millis
}

/// The demo vault (`apps/ui/demo-vault`) as an in-memory source: every
/// folder and file, each note's text, each image's `data:` URL.
#[must_use]
pub fn demo_vault() -> MemorySource {
    use base64::Engine as _;
    let now = now();
    let day: Millis = 86_400_000;
    let mut source = MemorySource::new(crate::demo::NAME);
    for folder in crate::demo::folders() {
        source = source.with_folder(folder);
    }
    let mut files = Vec::new();
    for (n, f) in crate::demo::files().into_iter().enumerate() {
        // A day apart, newest first by path order's end: the lists sort.
        let age = day.saturating_mul(Millis::try_from(n % 60).unwrap_or(0));
        let (created, modified) = (
            now.saturating_sub(age.saturating_mul(2)),
            now.saturating_sub(age),
        );
        if let Some(text) = f.text() {
            if is_note(f.path) {
                files.push(note_from_markdown(f.path, text, created, modified));
            } else {
                files.push(NoteFile::new(f.path, created, modified));
            }
            source = source.with_text(f.path, text);
        } else {
            files.push(NoteFile::new(f.path, created, modified));
        }
        if let Some(mime) = f.media_type() {
            let data = base64::engine::general_purpose::STANDARD.encode(f.bytes);
            source = source.with_resource(f.path, format!("data:{mime};base64,{data}"));
        }
    }
    source.with_files(files)
}

/// The Vault's workspace, in context above the shell (a desk's left dock
/// is drawn in the shell's sidebar, outside the page): what the navigator
/// opens notes into.
#[derive(Clone, Copy)]
pub struct VaultWorkspace(Signal<Workspace>);

/// The Vault's workspace over `vault`, in context; call above the shell.
pub fn use_vault_workspace(vault: &MemorySource) -> VaultWorkspace {
    use_context_provider(|| VaultWorkspace(Signal::new(first_workspace(vault))))
}

impl VaultWorkspace {
    /// The workspace itself.
    #[must_use]
    pub const fn signal(&self) -> Signal<Workspace> {
        self.0
    }
}

/// The note on show: the document on show's note (a note's pane, or an
/// island's view on one), its path and its `type` property.
#[must_use]
pub fn note_on_show(w: &Workspace, vault: &MemorySource) -> Option<(String, Option<String>)> {
    let pane = w.current_document().and_then(|id| w.pane(id))?;
    let path = match pane.view.0.as_str() {
        NOTE => pane.state.clone(),
        FRAME => crate::favourites::read_frame_state(&pane.state)?
            .2
            .to_owned(),
        _ => return None,
    };
    if !is_note(&path) {
        return None;
    }
    let ty = type_of(vault, &path)?;
    Some((path, ty))
}

/// Note `path`'s `type` as the vault has it now: `None` when it is not in
/// the vault, `Some(None)` when it has no type.
#[allow(
    clippy::option_option,
    reason = "not in the vault and untyped are different answers"
)]
pub fn type_of(vault: &MemorySource, path: &str) -> Option<Option<String>> {
    let text = vault.read_text(path)?;
    Some(
        note_from_markdown(path, &text, 0, 0)
            .properties
            .into_iter()
            .find(|(k, _)| k == "type")
            .and_then(|(_, v)| match v {
                PropertyValue::Text(t) => Some(t.trim().to_owned()),
                _ => None,
            })
            .filter(|t| !t.is_empty()),
    )
}

/// The island view on show, when the document on show is one:
/// `(island, view, note)`.
#[must_use]
pub fn frame_on_show(w: &Workspace) -> Option<(String, String, String)> {
    let pane = w.current_document().and_then(|id| w.pane(id))?;
    (pane.view.0 == FRAME)
        .then(|| crate::favourites::read_frame_state(&pane.state))
        .flatten()
        .map(|(i, v, n)| (i.to_owned(), v.to_owned(), n.to_owned()))
}

/// Island `island`'s view `view` on note `note`, titled `title`: its pane
/// forward when one is open, else a new tab beside the note.
pub fn open_frame(
    mut workspace: Signal<Workspace>,
    island: &str,
    view: &str,
    note: &str,
    title: &str,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let state = crate::favourites::frame_state(island, view, note);
    let open = workspace
        .read()
        .panes
        .iter()
        .find(|p| p.view.0 == FRAME && p.state == state)
        .map(|p| p.id.clone());
    if let Some(id) = open {
        let _ = workspace.write().activate(&id);
        return;
    }
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut pane = Pane::new(
        format!("frame-{n}"),
        FRAME,
        format!("{title} · {}", title_of(note)),
    );
    pane.state = state;
    open_in_main(workspace, pane, Target::Tab);
}

/// Note `path` on show (a favourite's `page` or `{place}`).
pub fn show(workspace: Signal<Workspace>, path: &str) {
    show_note(workspace, path, Target::Tab);
}

fn note_pane(path: &str) -> Pane {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut pane = Pane::new(format!("note-{n}"), NOTE, title_of(path));
    path.clone_into(&mut pane.state);
    pane
}

/// Whether `path` is a markdown note (`.md`, any case).
fn is_note(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case("md"))
}

/// A note's tab title: its name without `.md`.
fn title_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name).to_owned()
}

/// Opens `pane` at `target` in the main area: beside the note on show,
/// never in the dock the navigator is in (`apps/ui/src/vault.rs`
/// `open_in_main`).
fn open_in_main(mut workspace: Signal<Workspace>, pane: Pane, target: Target) {
    let mut w = workspace.write();
    if let Some(doc) = w.current_document().cloned() {
        let _ = w.activate(&doc);
    }
    // No document yet (the org's vaults have no `Welcome.md` at their
    // root to open first), so the navigator's pane is the active one:
    // opened from there, the note would go into its dock group and leave
    // the dock showing nothing. With nothing active it opens in the main
    // area.
    let docked_active = w
        .active
        .as_ref()
        .and_then(|a| w.locate(a))
        .is_some_and(|l| l.window != 0 || matches!(l.area, architect_workspace::Area::Dock(_)));
    if docked_active {
        w.active = None;
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

/// Note `path` on show: its pane forward when one is open, else a new one
/// at `target`.
fn show_note(mut workspace: Signal<Workspace>, path: &str, target: Target) {
    let open = workspace
        .read()
        .panes
        .iter()
        .find(|p| p.view.0 == NOTE && p.state == path)
        .map(|p| p.id.clone());
    if let Some(id) = open {
        let _ = workspace.write().activate(&id);
        return;
    }
    open_in_main(workspace, note_pane(path), target);
}

/// A new note in `folder`, opened. In the demo it is made in the tab's
/// memory; on the org's vaults it is written to the node first (inside a
/// vault: `crate::org_note_folder`), and opens once the node has it.
fn new_note(workspace: Signal<Workspace>, web: &WebVault, folder: &str) {
    let vault = web.source.clone();
    let names = vault.files();
    let taken = |p: &str| names.iter().any(|f| f.path == p);
    let Some(org) = web.org.clone() else {
        let Some(path) = crate::untitled_path(folder, taken) else {
            return;
        };
        if vault
            .write(&[FileOp::CreateFile {
                path: path.clone(),
                text: String::new(),
            }])
            .is_ok()
        {
            edited();
            show_note(workspace, &path, Target::Tab);
        }
        return;
    };
    let active = active_note(workspace);
    let Some(folder) = crate::org_note_folder(folder, &org.vaults(), active.as_deref()) else {
        org.report("There is no vault to put a new note in.");
        return;
    };
    let paths: Vec<String> = crate::untitled_paths(&folder)
        .filter(|p| !taken(p))
        .collect();
    spawn(async move {
        match org.create(paths).await {
            Ok(path) => {
                let now = now();
                vault.set_text(note_from_markdown(&path, "", now, now), String::new());
                show_note(workspace, &path, Target::Tab);
            }
            Err(e) => org.report(format!("A new note was not made: {e}")),
        }
    });
}

/// The path of the note on show, if a note is.
fn active_note(workspace: Signal<Workspace>) -> Option<String> {
    let w = workspace.peek();
    w.active
        .as_ref()
        .and_then(|id| w.pane(id))
        .filter(|p| p.view.0 == NOTE)
        .map(|p| p.state.clone())
}

/// Where the Vault starts: the navigator in the left dock, the Welcome
/// note in the middle.
fn first_workspace(vault: &MemorySource) -> Workspace {
    let mut w = Workspace::new();
    if vault.read_text("Welcome.md").is_some() {
        let _ = w.open(note_pane("Welcome.md"), Target::Tab);
    }
    let _ = w.open(
        Pane::new("files", "files", "Files"),
        Target::Dock(Side::Left),
    );
    let _ = w.activate(&architect_workspace::PaneId::new("files"));
    if let Some(welcome) = w.panes.first().map(|p| p.id.clone()) {
        let _ = w.activate(&welcome);
    }
    if let Some(main) = w.windows.first_mut() {
        main.docks.left.ribbon = Some(Ribbon {
            views: vec![ViewKind::new("files")],
            commands: vec!["new-note".to_owned()],
            hidden: Vec::new(),
        });
    }
    w
}

fn views() -> Views {
    Views::new()
        .view(
            View::new("files", "Files", |_| rsx! { FilesView {} })
                .icon(icon!(Files))
                .home(Target::Dock(Side::Left)),
        )
        .view(View::new(NOTE, "Note", |_| rsx! { NoteView {} }).icon(icon!(FileText)))
        .view(View::new(FRAME, "Extension view", |_| rsx! { FrameView {} }).icon(icon!(Puzzle)))
}

fn screen_class(screen: &Screen) -> FormClass {
    let (w, h) = (narrow(screen.width), narrow(screen.height));
    FormClass::of(w, h, screen.form() != FormFactor::Desk)
}

/// A screen length as the workspace's points (window sizes fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

#[component]
pub fn VaultPage() -> Element {
    let screen = use_context::<Signal<Screen>>()();
    let vault = use_context::<WebVault>();
    let VaultWorkspace(workspace) = use_context::<VaultWorkspace>();
    let sets = use_signal(Screensets::default);
    let pane_status = use_signal(Vec::new);
    architect_shell::use_sidebar_layout(architect_shell::SidebarLayout::Two);
    let views = use_hook(views);
    let actions = architect_workspace::WorkspaceActions::new(use_context::<
        Arc<architect::action::ActionList>,
    >());
    architect_workspace::use_tabs_button(workspace);
    let commands = vec![Command::new("new-note", "New note", icon!(SquarePen))];
    let on_command = {
        let web = vault.clone();
        move |id: String| {
            if id == "new-note" {
                new_note(workspace, &web, "");
            }
        }
    };
    {
        let web = vault.clone();
        // The palette and the shortcut run this outside any scope: back
        // into this one, or the org's `spawn` finds no runtime and the app
        // stops.
        let (runtime, scope) = (
            dioxus::core::Runtime::current(),
            dioxus::core::current_scope_id(),
        );
        architect_commands::use_command(
            architect_commands::Command::new(NEW_NOTE, "New note")
                .group("Vault")
                .shortcut("⌃⌘N"),
            move || runtime.in_scope(scope, || new_note(workspace, &web, "")),
        );
    }
    // The navigator in a phone's drawer, from its bar.
    {
        let shell = use_shell();
        architect_commands::use_command(
            architect_commands::Command::new(FILES, "Files").group("Vault"),
            move || shell.toggle(architect_shell::Side::Left),
        );
    }
    // ⌘O finds the vault's notes.
    {
        let source = vault.source;
        architect_commands::use_opener(move || {
            architect_commands::Opener::new(
                "note",
                "Notes",
                move |_| {
                    source
                        .files()
                        .into_iter()
                        .filter(|f| is_note(&f.path))
                        .map(|f| {
                            let folder = f.path.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
                            architect_commands::OpenItem::new(
                                "note",
                                f.path.clone(),
                                title_of(&f.path),
                            )
                            .detail(folder)
                        })
                        .collect()
                },
                move |item, open_in| {
                    let target = match open_in {
                        architect_commands::OpenIn::Here => Target::Here,
                        architect_commands::OpenIn::Tab => Target::Tab,
                        architect_commands::OpenIn::Split => Target::Split(Side::Right),
                    };
                    show_note(workspace, &item.id, target);
                },
            )
        });
    }
    architect_shell::use_status(pane_status());
    let shell = use_shell();
    let title_row = shell.desk().then(|| {
        let (left, right) = shell.title_clearance();
        TitleRow {
            height: narrow(shell.title_row()),
            left: narrow(left),
            right: narrow(right),
            sidebar_left: narrow(shell.sidebar_title_left()),
            on_drag: None,
            on_zoom: None,
        }
    });
    let phone = screen.form() == FormFactor::Phone;
    let mut switcher = use_signal(|| false);
    let pane_title = {
        let w = workspace.read();
        w.active
            .as_ref()
            .and_then(|id| w.pane(id))
            .map_or_else(|| "Vault".to_owned(), |p| p.title.clone())
    };
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
                empty: vec!["new-note".to_owned()],
                phone,
                coarse: screen.touch,
                title_row,
                in_shell: shell.desk(),
                phone_header: !phone,
                switcher,
                screensets: sets,
                status: pane_status,
                form: screen_class(&screen),
                actions,
            }
        }
    }
}

/// The navigator over the vault: in the workspace's left dock (a desk's
/// sidebar), or in the shell's left drawer on a phone.
#[component]
pub fn FilesView() -> Element {
    let vault = use_context::<WebVault>();
    let VaultWorkspace(workspace) = use_context::<VaultWorkspace>();
    let nav = use_navigator(move || {
        // On the org's vaults the layout is read-only (`super::org::NodeLayout`).
        let source: Arc<dyn NavigatorSource> = match vault.org {
            Some(_) => Arc::new(super::org::NodeLayout(vault.source)),
            None => Arc::new(vault.source),
        };
        source
    });
    let phone = use_context::<Signal<Screen>>()().form() == FormFactor::Phone;
    let web = use_context::<WebVault>();
    rsx! {
        NavigatorView {
            navigator: nav,
            phone,
            on_open: move |o: Open| {
                let target = match o.how {
                    OpenIn::Tab => Target::Tab,
                    OpenIn::Split => Target::Split(Side::Right),
                    _ => Target::Here,
                };
                show_note(workspace, &o.path, target);
                // A phone's drawer goes, the note on show.
                if let Some(shell) = architect_shell::try_use_shell()
                    && shell.form() == FormFactor::Phone
                {
                    shell.open(None);
                }
            },
            on_new_note: move |folder: String| new_note(workspace, &web, &folder),
        }
    }
}

/// A note pane: its path's note, its editor kept while the pane lives.
#[component]
fn NoteView() -> Element {
    let pane = use_pane();
    let id = pane.id.0.clone();
    let path = pane.state();
    rsx! {
        for id in [id] {
            WebNote { key: "{id}", path: path.clone() }
        }
    }
}

/// An island's view on a note (a favourite's `{view}`, #607), filling its
/// pane: any view the island lists (`Island::views`), handed the note and
/// its source's playhead state as a claimed view is (`super::embed`).
#[component]
fn FrameView() -> Element {
    let pane = use_pane();
    let state = pane.state();
    let vault = use_context::<WebVault>();
    let t = use_shell().theme;
    let Some((island, view, note)) = crate::favourites::read_frame_state(&state) else {
        return rsx! {};
    };
    let Some(text) = vault.source.read_text(note) else {
        return rsx! {
            div { style: "flex: 1; padding: 32px; color: {t.ink_3};", p { "{title_of(note)} is no longer in the vault." } }
        };
    };
    let (_, body) = architect_navigator::markdown::split_block(&text);
    let properties: Vec<(String, String)> = note_from_markdown(note, &text, 0, 0)
        .properties
        .iter()
        .map(|(k, v)| (k.clone(), shown(v)))
        .collect();
    let prop = |name: &str| {
        properties
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let key = state.clone();
    rsx! {
        div { key: "{key}", style: "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; color: {t.ink};",
            FramePane {
                island: island.to_owned(),
                view: view.to_owned(),
                // The signed-in person's own vault, or the demo; a share's
                // guest is `Audience::Guest` (#629, #648).
                audience: crate::ext_host::Audience::Member,
                path: note.to_owned(),
                title: prop("title").unwrap_or_else(|| title_of(note)),
                note_type: prop("type").unwrap_or_default(),
                properties: properties.clone(),
                body: body.to_owned(),
            }
        }
    }
}

/// Island `island`'s view `view`, once its bundle is checked and only when
/// the island lists it; a word in its place otherwise. The note reaches it
/// through the gate (`ext_host::note_props`) for `audience`.
#[component]
fn FramePane(
    island: String,
    view: String,
    audience: crate::ext_host::Audience,
    path: String,
    title: String,
    note_type: String,
    properties: Vec<(String, String)>,
    body: String,
) -> Element {
    let t = use_shell().theme;
    let vault = use_context::<WebVault>();
    let bundle = super::frames::use_island_of(Some(island));
    let state = bundle.read().clone();
    match state {
        None => rsx! {},
        Some(Ok(b)) if b.island.views().contains(&view) => {
            let files = match audience {
                crate::ext_host::Audience::Member => {
                    super::embed::member_files(&vault, &b.island, &note_type, &path)
                }
                crate::ext_host::Audience::Guest => Vec::new(),
            };
            let note = crate::ext_host::note_props(
                audience,
                &b.island,
                &crate::ext_host::Note {
                    path: &path,
                    title: &title,
                    note_type: &note_type,
                    properties: &properties,
                    body: &body,
                    files: &files,
                },
            );
            // Keyed by island and view, in a list (the only place a key
            // remounts): a pane that goes from one view to another gets a
            // new frame, never the last view's frame under the new name.
            let id = format!("{}/{view}", b.island.id);
            rsx! {
                for k in [id] {
                    super::embed::Embedded { key: "{k}", bundle: b.clone(), view: view.clone(), note: note.clone(), pane: true }
                }
            }
        }
        Some(_) => rsx! {
            div { "data-testid": "ext-view-missing", style: "flex: 1; padding: 32px; color: {t.ink_3};",
                p { "This view is not on this server." }
            }
        },
    }
}

/// The note's frame: the text area fills the pane's column, as the
/// desktop's editor does (`apps/ui/src/note_editor.rs` `LAYOUT`).
const LAYOUT: &str = "
.web-note { box-sizing: border-box; max-width: 760px; width: 100%; margin: 0 auto; display: flex; flex-direction: column; min-height: 100%; }
.web-note textarea { flex: 1 1 auto; min-height: 60vh; width: 100%; box-sizing: border-box; resize: none; border: 0; outline: none;
  padding: 0; margin: 0; background: transparent; color: inherit; font: inherit; font-size: 15px; line-height: 1.65; tab-size: 4; }
";

/// A property's value as text.
fn shown(v: &PropertyValue) -> String {
    match v {
        PropertyValue::Text(t) => t.clone(),
        PropertyValue::Number(n) => n.to_string(),
        PropertyValue::Bool(b) => if *b { "Yes" } else { "No" }.to_owned(),
        PropertyValue::List(l) => l.join(", "),
    }
}

/// One note: an image drawn from its URL, a markdown note as its text
/// (read from the node first when it has not been yet; why not, when the
/// read fails), anything else said so.
#[component]
fn WebNote(path: String) -> Element {
    let vault = use_context::<WebVault>();
    let t = use_shell().theme;
    let source = vault.source.clone();
    let mut text = use_signal(|| source.read_text(&path).map(Ok::<String, String>));
    {
        let (path, org, source) = (path.clone(), vault.org, source.clone());
        use_hook(move || {
            if text.peek().is_none()
                && is_note(&path)
                && let Some(org) = org
            {
                spawn(async move {
                    match org.read(&path).await {
                        Ok(read) => {
                            source.set_text(
                                note_from_markdown(&path, &read, now(), now()),
                                read.clone(),
                            );
                            text.set(Some(Ok(read)));
                        }
                        Err(e) => text.set(Some(Err(e))),
                    }
                });
            }
        });
    }
    if let Some(url) = source
        .resource_url(&path)
        .filter(|u| u.starts_with("data:image/"))
    {
        return rsx! {
            div { style: "flex: 1; overflow: auto; display: flex; justify-content: center; padding: 24px;",
                img { src: "{url}", alt: "{title_of(&path)}", style: "max-width: 100%; height: auto; object-fit: contain;" }
            }
        };
    }
    if !is_note(&path) {
        return rsx! {
            div { style: "flex: 1; padding: 32px; color: {t.ink_3};",
                p { "{title_of(&path)} opens in the desktop app; the browser shows notes and images." }
            }
        };
    }
    match text() {
        Some(Ok(text)) => rsx! { NoteText { path, text } },
        Some(Err(why)) => rsx! {
            div { style: "flex: 1; padding: 32px; color: {t.ink_3};", "data-testid": "note-failed",
                p { style: "color: {t.ink};", "{title_of(&path)} could not be opened." }
                p { "{why}" }
            }
        },
        None => rsx! {
            div { style: "flex: 1; padding: 32px; color: {t.ink_3};", p { "Opening {title_of(&path)}…" } }
        },
    }
}

/// A note's text area, reached through the DOM: its value is set once
/// and then only from the live document, keeping the caret, never while
/// an IME composes (`crate::live_text`). Not a controlled input: setting
/// a text area's value moves its caret to the end and ends a composition.
struct Editor {
    id: String,
    /// What the text area showed when it was last read or written (the
    /// demo's; on the org's vaults `text` keeps it, with its version).
    shown: std::cell::RefCell<String>,
    composing: std::cell::Cell<bool>,
    live: Option<std::rc::Rc<super::org::LiveNote>>,
    /// The live note's text against the document (`live`'s).
    text: Option<std::cell::RefCell<crate::live_text::LiveText>>,
    /// The note's text into the vault (the navigator's preview follows).
    keep: Box<dyn Fn(&str)>,
    /// Why typing did not reach the document, said in the note.
    failed: Signal<Option<String>>,
}

impl Editor {
    fn area(&self) -> Option<web_sys::HtmlTextAreaElement> {
        use wasm_bindgen::JsCast as _;
        web_sys::window()?
            .document()?
            .get_element_by_id(&self.id)?
            .dyn_into()
            .ok()
    }

    /// What was typed since the text area was last read: into the live
    /// document, merged by Loro at the version the text area showed
    /// (`crate::live_text::LiveText::typed`), or into the vault in the
    /// demo.
    fn absorb(&self) {
        let Some(area) = self.area() else { return };
        let typed = area.value();
        match (&self.live, &self.text) {
            (Some(live), Some(text)) => match text.borrow_mut().typed(&typed) {
                Ok(crate::live_text::Typed::Merged { relay }) => {
                    if let Some(update) = relay {
                        live.relay(update);
                    }
                }
                Ok(crate::live_text::Typed::NotLive | crate::live_text::Typed::Nothing) => {}
                Err(why) => {
                    let mut failed = self.failed;
                    failed.set(Some(why));
                }
            },
            _ => {
                if *self.shown.borrow() != typed {
                    edited();
                    (self.keep)(&typed);
                    *self.shown.borrow_mut() = typed;
                }
            }
        }
    }

    /// `body` into the text area, the caret kept with its text.
    fn write(&self, body: &str) {
        if let Some(area) = self.area() {
            let now = area.value();
            if now != body {
                let caret =
                    |at: Option<u32>| at.map(|at| crate::live_text::caret_after(at, &now, body));
                let (start, end) = (
                    caret(area.selection_start().ok().flatten()),
                    caret(area.selection_end().ok().flatten()),
                );
                area.set_value(body);
                if let (Some(start), Some(end)) = (start, end) {
                    let _ = area.set_selection_range(start, end);
                }
            }
        }
        (self.keep)(body);
    }

    /// The session attached: the text area shows the document, and is
    /// typed into from now on (what it held before is dropped: it was
    /// read-only).
    fn attached(&self) {
        let Some(text) = &self.text else { return };
        let body = text.borrow_mut().attach();
        self.write(&body);
    }

    /// The text area brought to the live document: typing first taken
    /// in, then the document's text shown with the caret where its text
    /// went. Not while an IME composes (after it ends).
    fn show_document(&self) {
        if self.composing.get() {
            return;
        }
        self.absorb();
        let Some(text) = &self.text else { return };
        if !text.borrow().is_live() {
            return;
        }
        let body = text.borrow_mut().show();
        self.write(&body);
    }
}

/// A markdown note's text, editable: its properties above it, as they are.
/// On the org's vaults it is live with the node while it is open
/// (`super::org::LiveNote`): typing goes to the node's document, and
/// another's typing comes into this one. It may be typed in from the
/// first attach on, once the node's copy is in the document: typing into
/// the empty document before that would add the note a second time.
#[component]
fn NoteText(path: String, text: String) -> Element {
    let vault = use_context::<WebVault>();
    let t = use_shell().theme;
    let source = vault.source.clone();
    let (front, body0) = architect_navigator::markdown::split_block(&text);
    let front = front.into_owned();
    let properties = note_from_markdown(&path, &text, 0, 0).properties;
    // The text area's first value; the DOM's from then on.
    let initial = use_hook(|| body0.to_owned());
    let mut ready = use_signal(|| vault.org.is_none());
    let mut lost = use_signal(|| None::<String>);
    let failed = use_signal(|| None::<String>);
    // The text area's fold, held from the person's first focus in it: a
    // list made or unmade by what they type never folds it under them.
    let mut fold_held = use_signal(|| None::<bool>);
    let created = use_hook(|| {
        source
            .files()
            .into_iter()
            .find(|f| f.path == path)
            .map_or_else(now, |f| f.created)
    });
    let editor = use_hook(|| {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let (source, path_kept, front) = (source.clone(), path.clone(), front.clone());
        let keep = Box::new(move |b: &str| {
            let full = format!("{front}{b}");
            source.set_text(note_from_markdown(&path_kept, &full, created, now()), full);
        });
        let live = vault.org.as_ref().and_then(|org| org.live(&path));
        let (live, changes) = match live {
            Some((live, changes)) => (Some(std::rc::Rc::new(live)), Some(changes)),
            None => (None, None),
        };
        let text = live.as_ref().map(|live| {
            std::cell::RefCell::new(crate::live_text::LiveText::new(
                live.doc(),
                vault_proto::COLLAB_BODY_CONTAINER,
                body0,
            ))
        });
        let editor = std::rc::Rc::new(Editor {
            id: format!("note-text-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
            shown: std::cell::RefCell::new(body0.to_owned()),
            composing: std::cell::Cell::new(false),
            live,
            text,
            keep,
            failed,
        });
        if let Some(mut changes) = changes {
            let heard = std::rc::Rc::clone(&editor);
            spawn(async move {
                use super::org::Heard;
                use futures_util::StreamExt as _;
                while let Some(h) = changes.next().await {
                    match h {
                        Heard::Attached => {
                            // The first attach makes it editable; a later
                            // one (after a drop or a re-dial) catches up.
                            if *ready.peek() {
                                heard.show_document();
                            } else {
                                heard.attached();
                                ready.set(true);
                            }
                            lost.set(None);
                        }
                        Heard::Changed => {
                            if *ready.peek() {
                                heard.show_document();
                            }
                        }
                        Heard::Lost(why) => lost.set(Some(why)),
                    }
                }
            });
        }
        editor
    });
    let editing = architect_shell::use_editing_actions("editor:note", &[]);
    let on_input = {
        let editor = std::rc::Rc::clone(&editor);
        move |_: FormEvent| {
            if !editor.composing.get() {
                editor.show_document();
            }
        }
    };
    let on_compose = {
        let editor = std::rc::Rc::clone(&editor);
        move |_: CompositionEvent| editor.composing.set(true)
    };
    let on_composed = {
        let editor = std::rc::Rc::clone(&editor);
        move |_: CompositionEvent| {
            editor.composing.set(false);
            editor.show_document();
        }
    };
    let id = editor.id.clone();
    // What the text area holds now, else the note as it opened: Dioxus
    // writes a `value` on every render (it is volatile), so a render for
    // anything else (the extension listing arriving, a signal) writes back
    // what the person sees, never the note as it was when it opened.
    let current = editor
        .area()
        .map_or_else(|| initial.clone(), |area| area.value());
    let read_only = !ready();
    let prop = |name: &str| {
        properties
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| shown(v))
    };
    // An extension's view claiming the note's type, at its head (#495).
    let note_type = prop("type").unwrap_or_default();
    let shown_properties: Vec<(String, String)> = properties
        .iter()
        .map(|(k, v)| (k.clone(), shown(v)))
        .collect();
    // A collection (#668) draws as the core list at its head, unless an
    // island claims its type: then the island's view draws instead. The
    // list's own rows (`sets`, `songs`, …) leave the property table, and
    // its words are read with the fences that repeat the list folded out.
    let fields = architect_media::front::fields(&text);
    let listing = super::frames::use_listing();
    let claimed = listing
        .read()
        .as_ref()
        .is_some_and(|l| crate::ext_host::claimed(l, &note_type).is_some());
    let core_list = !claimed && super::collection::is_collection(&fields);
    let listed_keys = if core_list {
        architect_media::Collection::fields_in(&fields)
    } else {
        Vec::new()
    };
    let table: Vec<(String, PropertyValue)> = properties
        .iter()
        .filter(|(k, _)| !listed_keys.contains(&k.as_str()))
        .cloned()
        .collect();
    // A note's recording (its `recording` property, a vault path) plays on
    // the app's own source in the Now Playing bar (#589).
    let recording = prop("recording");
    let note_title = prop("title").unwrap_or_else(|| title_of(&path));
    // The claimed view gets the body as the note last read it; the text
    // area is the live copy.
    let body = body0.to_owned();
    rsx! {
        document::Style { {LAYOUT} }
        div { style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 24px 28px 96px; color: {t.ink};",
            div { class: "web-note", "data-note": "{path}",
                if core_list {
                    super::collection::CollectionHead {
                        path: path.clone(),
                        title: note_title.clone(),
                        text: text.clone(),
                    }
                }
                if !note_type.is_empty() {
                    super::embed::NoteEmbed {
                        // The signed-in person's own vault, or the demo; a
                        // share's guest is `Audience::Guest` (#629, #648).
                        audience: crate::ext_host::Audience::Member,
                        path: path.clone(),
                        title: note_title.clone(),
                        note_type,
                        properties: shown_properties,
                        body,
                    }
                }
                if let Some(r) = recording {
                    super::recording::RecordingRow {
                        path: path.clone(), title: note_title, recording: r,
                        artist: prop("artist"), cover: prop("cover"),
                    }
                }
                if !table.is_empty() {
                    dl { "data-testid": "note-properties",
                        style: "display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; margin: 0 0 16px; \
                                font-size: 13px; color: {t.ink_3};",
                        for (k, v) in table {
                            Property { key: "{k}", value: shown(&v), name: k }
                        }
                    }
                }
                if read_only {
                    p { "data-testid": "note-connecting", style: "margin: 0 0 12px; font-size: 13px; color: {t.ink_3};",
                        "Connecting to the node… The note can be edited once it is live."
                    }
                }
                if let Some(why) = failed() {
                    p { "data-testid": "note-failed-edit", role: "alert",
                        style: "margin: 0 0 12px; font-size: 13px; color: {t.ink};",
                        "An edit did not reach the note ({why}). Copy what you typed before reloading."
                    }
                }
                if let Some(why) = lost() {
                    p { "data-testid": "note-lost", role: "status",
                        style: "margin: 0 0 12px; font-size: 13px; color: {t.ink_3};",
                        "Not live ({why}). Reconnecting; what you type is kept and sent then."
                    }
                }
                if core_list {
                    super::collection::ReadingBody { body: body0.to_owned() }
                }
                // One text area whatever `core_list` is (the extension
                // listing can load after the note opens): folded under
                // its summary for a list, open otherwise, until the person
                // first focuses it (then as it was), never rebuilt, so
                // what was typed, the caret and focus stay.
                details { "data-testid": "note-text-fold", open: fold_held().unwrap_or(!core_list),
                    summary {
                        style: if core_list {
                            "cursor: pointer; font-size: 13px; color: {t.ink_3}; margin: 0 0 10px;"
                        } else {
                            "display: none;"
                        },
                        "Edit the note's text"
                    }
                    textarea {
                        id: "{id}",
                        "data-testid": "note-text",
                        "data-live": if read_only { "connecting" } else { "ready" },
                        "aria-label": "{title_of(&path)}",
                        spellcheck: "true",
                        readonly: read_only,
                        value: "{current}",
                        oninput: on_input,
                        oncompositionstart: on_compose,
                        oncompositionend: on_composed,
                        onfocusin: move |_| {
                            if fold_held.peek().is_none() {
                                fold_held.set(Some(!core_list));
                            }
                            editing.focus();
                        },
                        onfocusout: move |_| editing.blur(),
                    }
                }
            }
        }
    }
}

/// One property: its name and its value as text.
#[component]
fn Property(name: String, value: String) -> Element {
    let t = use_shell().theme;
    rsx! {
        dt { style: "margin: 0;", "{name}" }
        dd { style: "margin: 0; color: {t.ink};", "{value}" }
    }
}
