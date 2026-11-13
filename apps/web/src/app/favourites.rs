//! The bottom slots in a browser as favourites (#607), on the shell's one
//! path (`architect_shell::Favourites`, `resolve_favourites`, `declare`),
//! as the desktop and phone app's (`apps/ui/src/favourites.rs`).
//!
//! The web app's own set is what it can open today: the Vault (the note
//! on show), Files (the navigator), Search (quick open), and New note as
//! the round button. While the note on show has a `type` some extension
//! of the node declares favourites for (`/_ext/favourites.json`,
//! `crate::favourites`), its tabs take the slots: `page` the note, an
//! island's view (`<extension id>/<view>`) that view in a frame of its own
//! (`super::frames`, a tab of the workspace beside the note), a command,
//! or a note beside it; a tab the browser cannot open is left out. Its
//! primary action shows its first face: the browser knows no state yet.

use std::collections::BTreeSet;
use std::rc::Rc;

use architect_commands::{Commands, Overlay};
use architect_navigator::{MemorySource, NavigatorSource as _};
use architect_shell::{self as shell, Action, Declared, Favourites, Tab, icon};
use architect_workspace::Workspace;
use dioxus::prelude::*;
use lucide_dioxus::{Files, NotebookText, Search, SquarePen};

use super::{frames, vault};
use crate::favourites::{Opens, Reach, opens};

/// The slots to show and the tab that is current.
pub struct Slots {
    pub set: Favourites,
    pub current: Option<&'static str>,
}

/// The web app's own set.
fn own_set(commands: Commands) -> Favourites {
    Favourites::new(
        vec![
            Tab::new("vault", "Vault", icon!(NotebookText)),
            Tab::new("files", "Files", icon!(Files)),
            Tab::new("search", "Search", icon!(Search)),
        ],
        Some(Action::new("New note", icon!(SquarePen), move |()| {
            let _ = commands.run(vault::NEW_NOTE);
        })),
    )
}

/// What the web app's own tab `id` opens.
pub fn own_tab(id: &str, workspace: Signal<Workspace>, commands: Commands) {
    match id {
        "vault" => {
            let note = vault::frame_on_show(&workspace.peek()).map(|(_, _, n)| n);
            if let Some(note) = note {
                vault::show(workspace, &note);
            }
        }
        "files" => {
            let _ = commands.run(vault::FILES);
        }
        "search" => commands.show(Overlay::QuickOpen),
        _ => {}
    }
}

/// The favourites for the note on show, and which tab is current.
pub fn use_web_favourites(
    workspace: Signal<Workspace>,
    source: &MemorySource,
    commands: Commands,
) -> Slots {
    let declared_list = use_resource(frames::favourites);
    let islands = use_resource(frames::island_views);
    let on_show = {
        let source = source.clone();
        use_memo(move || vault::note_on_show(&workspace.read(), &source))
    };
    let framed = use_memo(move || vault::frame_on_show(&workspace.read()));
    let default = own_set(commands);
    let open = on_show();
    let (path, ty) = match open {
        Some((path, Some(ty))) => (path, ty),
        _ => {
            return Slots {
                set: default,
                current: Some("vault"),
            };
        }
    };
    let list: Rc<Vec<_>> = declared_list.read().clone().unwrap_or_default();
    let islands = islands.read().clone().unwrap_or_default();
    let decls: Vec<_> = list.iter().filter(|d| d.is_for(&ty)).collect();
    if decls.is_empty() {
        return Slots {
            set: default,
            current: Some("vault"),
        };
    }
    let actions: BTreeSet<String> = commands
        .all()
        .into_iter()
        .filter(|c| c.enabled)
        .map(|c| c.id)
        .collect();
    let files: BTreeSet<String> = source.files().into_iter().map(|f| f.path).collect();
    let exists = |p: &str| files.contains(p);
    let reach = Reach {
        islands: &islands,
        actions: &actions,
        note: &path,
        exists: &exists,
    };
    let mut declared: Vec<Declared> = Vec::new();
    let mut marks: Vec<(&'static str, Opens)> = Vec::new();
    for d in decls {
        let made = shell::declare(
            d,
            |target| {
                let what = opens(target, &reach)?;
                let label = d
                    .tabs
                    .iter()
                    .find(|f| &f.target == target)
                    .map_or_else(String::new, |f| f.label.clone());
                let path = path.clone();
                Some(Callback::new(move |()| {
                    run(&what, &label, &path, workspace, commands);
                }))
            },
            |_| false,
        );
        if let Some(made) = made {
            if declared.is_empty() {
                marks = d
                    .tabs
                    .iter()
                    .filter_map(|f| {
                        opens(&f.target, &reach)
                            .map(|o| (shell::declared_tab_id(&d.note_type, &f.id), o))
                    })
                    .take(shell::SLOTS)
                    .collect();
            }
            declared.push(made);
        }
    }
    let set = shell::resolve_favourites(&default, &declared, Some(&ty));
    // The tab whose place is on show: an island's view in its frame, else
    // the note itself.
    let current = match framed() {
        Some((island, view, _)) => marks
            .iter()
            .find(|(_, o)| matches!(o, Opens::View { island: i, view: v } if *i == island && *v == view))
            .map(|(id, _)| *id),
        None => marks
            .iter()
            .find(|(_, o)| *o == Opens::Note)
            .map(|(id, _)| *id),
    };
    let current = if declared.is_empty() {
        Some("vault")
    } else {
        current
    };
    Slots { set, current }
}

/// Opens what a declared tab (`label`) opens, on note `note`.
fn run(what: &Opens, label: &str, note: &str, workspace: Signal<Workspace>, commands: Commands) {
    match what {
        Opens::Note => vault::show(workspace, note),
        Opens::Path(p) => vault::show(workspace, p),
        Opens::QuickOpen => commands.show(Overlay::QuickOpen),
        Opens::Files => {
            let _ = commands.run(vault::FILES);
        }
        Opens::Action(id) => {
            let _ = commands.run(id);
        }
        Opens::View { island, view } => vault::open_frame(workspace, island, view, note, label),
    }
}
