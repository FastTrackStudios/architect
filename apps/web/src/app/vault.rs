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

/// The vault the app shows, in context above the Vault tab.
#[derive(Clone)]
pub struct WebVault {
    pub source: MemorySource,
}

/// Unix milliseconds now.
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)] // a time in ms, well inside i64
fn now() -> Millis {
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
    let text = vault.read_text(&path)?;
    let ty = note_from_markdown(&path, &text, 0, 0)
        .properties
        .into_iter()
        .find(|(k, _)| k == "type")
        .and_then(|(_, v)| match v {
            PropertyValue::Text(t) => Some(t.trim().to_owned()),
            _ => None,
        })
        .filter(|t| !t.is_empty());
    Some((path, ty))
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
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
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

/// A new note in `folder`, written to the vault and opened.
fn new_note(workspace: Signal<Workspace>, vault: &MemorySource, folder: &str) {
    let names = vault.files();
    let path = (1..=999)
        .map(|n: u32| {
            let name = if n == 1 {
                "Untitled.md".to_owned()
            } else {
                format!("Untitled {n}.md")
            };
            if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            }
        })
        .find(|p| !names.iter().any(|f| f.path == *p));
    let Some(path) = path else { return };
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
        let source = vault.source.clone();
        move |id: String| {
            if id == "new-note" {
                new_note(workspace, &source, "");
            }
        }
    };
    {
        let source = vault.source.clone();
        architect_commands::use_command(
            architect_commands::Command::new(NEW_NOTE, "New note")
                .group("Vault")
                .shortcut("⌃⌘N"),
            move || new_note(workspace, &source, ""),
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
        let source: Arc<dyn NavigatorSource> = Arc::new(vault.source);
        source
    });
    let phone = use_context::<Signal<Screen>>()().form() == FormFactor::Phone;
    let source = use_context::<WebVault>().source;
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
            on_new_note: move |folder: String| new_note(workspace, &source, &folder),
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
    let bundle = super::frames::use_island_of(Some(island.clone()));
    let state = bundle.read().clone();
    match state {
        None => rsx! {},
        Some(Ok(b)) if b.island.views().contains(&view) => {
            let note = crate::ext_host::note_props(
                audience,
                &b.island,
                &crate::ext_host::Note {
                    path: &path,
                    title: &title,
                    note_type: &note_type,
                    properties: &properties,
                    body: &body,
                },
            );
            rsx! {
                super::embed::Embedded { bundle: b, view, note, pane: true }
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

/// One note: an image or a PDF drawn from its URL, a markdown note in the
/// editor (its properties above it, as they are; editing them is the
/// desktop's for now), anything else said so.
#[component]
fn WebNote(path: String) -> Element {
    let vault = use_context::<WebVault>();
    let t = use_shell().theme;
    let source = vault.source;
    let text = use_hook(|| source.read_text(&path));
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
    let Some(text) = text.filter(|_| is_note(&path)) else {
        return rsx! {
            div { style: "flex: 1; padding: 32px; color: {t.ink_3};",
                p { "{title_of(&path)} opens in the desktop app; the browser shows notes and images." }
            }
        };
    };
    let (front, body) = architect_navigator::markdown::split_block(&text);
    let front = front.into_owned();
    let properties = note_from_markdown(&path, &text, 0, 0).properties;
    let body = body.to_owned();
    let created = use_hook(|| {
        source
            .files()
            .into_iter()
            .find(|f| f.path == path)
            .map_or_else(now, |f| f.created)
    });
    let on_input = {
        let (path, front) = (path.clone(), front);
        move |e: FormEvent| {
            let full = format!("{front}{}", e.value());
            let file = note_from_markdown(&path, &full, created, now());
            source.set_text(file, full);
            edited();
        }
    };
    let editing = architect_shell::use_editing_actions("editor:note", &[]);
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
    rsx! {
        document::Style { {LAYOUT} }
        div { style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 24px 28px 96px; color: {t.ink};",
            div { class: "web-note", "data-note": "{path}",
                if !note_type.is_empty() {
                    super::embed::NoteEmbed {
                        // The signed-in person's own vault, or the demo; a
                        // share's guest is `Audience::Guest` (#629, #648).
                        audience: crate::ext_host::Audience::Member,
                        path: path.clone(),
                        title: prop("title").unwrap_or_else(|| title_of(&path)),
                        note_type: note_type.clone(),
                        properties: shown_properties,
                        body: body.clone(),
                    }
                }
                if !properties.is_empty() {
                    dl { "data-testid": "note-properties",
                        style: "display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; margin: 0 0 16px; \
                                font-size: 13px; color: {t.ink_3};",
                        for (k, v) in properties {
                            Property { key: "{k}", value: shown(&v), name: k }
                        }
                    }
                }
                textarea {
                    "data-testid": "note-text",
                    "aria-label": "{title_of(&path)}",
                    spellcheck: "true",
                    initial_value: "{body}",
                    oninput: on_input,
                    onfocusin: move |_| editing.focus(),
                    onfocusout: move |_| editing.blur(),
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
