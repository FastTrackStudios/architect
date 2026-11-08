//! The Vault page's scope tabs (#441, #476; the code's decks): each
//! project and (in the sandbox) each setlist's song in a scope tab of its
//! own, shown in its type's screenset, and the vault's general tabs with
//! none active (`docs/content/architecture/workspace.md`, "Scope tabs").
//!
//! - The tab kinds: Architect's compact `project` kind (the project's
//!   glyph and colour, #471) and every loaded extension's
//!   (`[[tab_kinds]]`: the sandbox's wide `song` kind, its custom
//!   `setlist` tab drawn by [`crate::setlists::setlist_tab`]).
//!
//! - The session (which decks are open, the one on show) and the saved
//!   screensets are this device's, in the app's folder: `decks/session.json`
//!   and `decks/screensets.json`.
//! - Each deck's layout is kept with its context: a project's in its
//!   folder's `.architect/deck.json`, any other's under the vault's
//!   `.architect/decks/` (`home::ContextFiles`).
//! - The templates: the vault's deck is shown in "Write", a project's in
//!   "Project"; an extension adds its own (`[[deck_templates]]`, the
//!   sandbox's Setlists: a song's in "Song").
//!
//! The sandbox keeps nothing unless it shows a vault on disk
//! (`ARCHITECT_VAULT`): its films and goldens start the same each run.

use std::rc::Rc;

use architect_workspace::home::{ContextFiles, FileStore};
use architect_workspace::{
    Bridge, DeckContext, DeckHandle, DeckStores, DeckTemplate, DecksConfig, FormClass, Screensets,
    Target, Views, Workspace, WorkspaceActions, WorkspaceStore, use_decks,
};
use dioxus::prelude::*;

/// A project's context type, and its scope tab kind's id.
pub const PROJECT: &str = "project";
/// The whole vault's.
pub const VAULT: &str = "vault";

/// The vault's own deck: the first a first run opens.
fn vault_context() -> DeckContext {
    let name = crate::notes::open_vault()
        .and_then(|v| {
            v.root()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .filter(|_| !crate::Mode::current().is_sandbox())
        .unwrap_or_else(|| "Vault".to_owned());
    DeckContext::new(VAULT, "vault", name).icon("library")
}

/// The project whose folder is `folder` (vault-relative), when it has a
/// `project.md`: its title is the project's name (`crate::vault::tab_title`).
#[must_use]
pub fn project_context(folder: &str) -> Option<DeckContext> {
    let vault = crate::notes::open_vault()?;
    let root = vault.root();
    let folder = folder.trim_matches('/');
    if folder.is_empty() || !root.join(folder).join("project.md").is_file() {
        return None;
    }
    // Its own icon and colour (its `project.md`'s), else the project
    // glyph: as the navigator draws it.
    let text = std::fs::read_to_string(root.join(folder).join("project.md")).unwrap_or_default();
    let (props, _) = crate::note::frontmatter(&text);
    let prop = |k: &str| {
        props
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
    };
    // Its name: its `title`, else its folder's (as its tab and Recent
    // Files say it).
    let title = crate::vault::tab_title(&format!("{folder}/project.md"));
    let mut context = DeckContext::new(PROJECT, folder, title)
        .icon(prop("icon").unwrap_or_else(|| architect_navigator_api::PROJECT_ICON.to_owned()))
        .folder(folder);
    context.colour = prop("color").or_else(|| prop("colour"));
    Some(context)
}

/// The decks kept in memory for the run (the sandbox without a vault on
/// disk): a page switch and back finds them as they were left.
#[derive(Clone)]
struct InMemory(DeckStores);

/// Where the decks are kept: in memory for the run in the sandbox without
/// a vault on disk.
fn stores() -> DeckStores {
    let vault = crate::notes::open_vault();
    if vault.is_none() && crate::Mode::current().is_sandbox() {
        return use_root_context(|| {
            let contexts: Rc<dyn architect_workspace::ContextStore> =
                Rc::new(architect_workspace::MemoryContextStore::new());
            InMemory(DeckStores {
                session: Some(WorkspaceStore::new(architect_workspace::MemoryStore::new())),
                screensets: None,
                contexts: Some(contexts),
            })
        })
        .0;
    }
    let dir = crate::app_dir().map(|d| d.join("decks"));
    DeckStores {
        session: dir
            .as_ref()
            .map(|d| WorkspaceStore::new(FileStore(d.join("session.json")))),
        screensets: dir.map(|d| WorkspaceStore::new(FileStore(d.join("screensets.json")))),
        contexts: vault.map(|v| {
            let files: Rc<dyn architect_workspace::ContextStore> =
                Rc::new(ContextFiles::new(v.root()));
            files
        }),
    }
}

/// The app's templates and every loaded extension's (`[[deck_templates]]`);
/// an extension's shipped layout becomes a built-in screenset in `sets`.
fn templates(views: &Views, sets: &mut Screensets) -> Vec<DeckTemplate> {
    let mut out = vec![
        DeckTemplate::new(VAULT, "write"),
        DeckTemplate::new(PROJECT, "project"),
    ];
    let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>() else {
        return out;
    };
    let specs: Vec<architect_workspace::control::ViewSpec> = views
        .iter()
        .map(|v| architect_workspace::control::ViewSpec {
            kind: v.kind.0.clone(),
            title: v.title.clone(),
            single: v.single,
        })
        .collect();
    for by in ui.host().contributions().deck_templates() {
        let decl = by.item;
        let made = decl.layout_text.as_deref().and_then(|text| {
            architect_workspace::control::deck::template_screenset(
                &decl.screenset,
                text,
                specs.clone(),
            )
            .map_err(|e| tracing::warn!(error = %e, extension = %by.owner, "decks.template_layout"))
            .ok()
        });
        let id = made
            .as_ref()
            .map_or_else(|| slug(&decl.screenset), |s| s.id.clone());
        if let Some(set) = made.filter(|s| sets.get(&s.id).is_none()) {
            sets.sets.push(set);
        }
        out.push(DeckTemplate {
            kind: decl.context,
            screenset: id,
            by: by.owner,
        });
    }
    out
}

/// A screenset's id from its name, as the workspace makes it.
fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Architect's tab kinds and every loaded extension's (`[[tab_kinds]]`),
/// with the renderers the app links in.
pub fn tab_kinds() -> architect_workspace::TabKindsUi {
    use architect_workspace::{TabKind, TabKinds, TabKindsUi, TabWidth};
    // A project's scope tab: compact, its own icon and colour (its
    // `project.md`'s), else the project glyph.
    let mut kinds = TabKinds::new()
        .with(TabKind::scope(PROJECT, "Project").icon(architect_navigator_api::PROJECT_ICON));
    if let Some(ui) = try_consume_context::<architect_extensions::interface::ExtensionUi>() {
        for by in ui.host().contributions().tab_kinds() {
            let d = by.item;
            let mut k = if d.scope {
                TabKind::scope(d.id, d.title)
            } else {
                TabKind::new(d.id, d.title)
            }
            .width(
                d.width
                    .as_deref()
                    .map_or(TabWidth::Compact, TabWidth::parse),
            )
            .by(by.owner.clone());
            k.icon = d.icon;
            k.colour = d.colour;
            k.subtitle = d.subtitle;
            // Its views as panes name them: `<extension>/<view>`.
            k.views = d
                .views
                .iter()
                .map(|v| format!("{}/{v}", by.owner))
                .collect();
            for m in d.menu {
                k = k.item(m.label, m.action);
            }
            kinds.register(k);
        }
    }
    TabKindsUi::new(kinds).renderer(
        crate::setlists::SETLIST_TAB,
        std::rc::Rc::new(crate::setlists::setlist_tab),
    )
}

/// The Vault page's decks over its workspace and screensets.
pub fn use_lab_decks(
    workspace: Signal<Workspace>,
    sets: Signal<Screensets>,
    form: FormClass,
    views: &Views,
    actions: WorkspaceActions,
) -> DeckHandle {
    let stores = stores();
    let templates = use_hook(|| {
        let mut s = sets;
        let mut all = s.peek().clone();
        let templates = templates(views, &mut all);
        if *s.peek() != all {
            s.set(all);
        }
        templates
    });
    use_decks(DecksConfig {
        workspace,
        screensets: sets,
        form,
        stores,
        templates,
        // No scope tab on a first run: the general tabs, as without them.
        start: Vec::new(),
        // The general tabs are the vault's (shown in "Write"); an older
        // session's vault deck folds into them.
        general: Some(vault_context()),
        views: Some(views.clone()),
        bridge: Bridge::default(),
        actions: Some(actions),
        resolve: Some(Rc::new(resolve)),
        // A project with nothing kept starts on its page: its
        // `project.md` in the main area, in a tab (never in place of what
        // is active: the dock's navigator, when the screenset's main area
        // starts empty).
        on_fresh: Some(Callback::new(move |context: DeckContext| {
            if let Some(folder) = context.folder.filter(|_| context.kind == PROJECT) {
                crate::vault::show_note(workspace, &format!("{folder}/project.md"), Target::Tab);
            }
        })),
    })
}

/// A context as the app knows it: a project given by its id alone
/// (`SCOPE_OPEN {context: {kind: project, id: "Projects/Album"}}`) gets
/// its folder, icon and, when it has none, its title.
fn resolve(mut context: DeckContext) -> DeckContext {
    if context.kind != PROJECT || context.folder.is_some() {
        return context;
    }
    if let Some(p) = project_context(&context.id) {
        context.id = p.id;
        context.folder = p.folder;
        if context.icon.is_none() {
            context.icon = p.icon;
        }
        if context.colour.is_none() {
            context.colour = p.colour;
        }
        if context.title.trim().is_empty() {
            context.title = p.title;
        }
    }
    context
}
