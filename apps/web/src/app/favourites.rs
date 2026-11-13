//! The bottom slots in a browser as favourites (#607), on the shell's one
//! path (`architect_shell::Favourites`, `resolve_favourites`, `declare`),
//! as the desktop and phone app's (`apps/ui/src/favourites.rs`).
//!
//! The web app's own set is what it can open today: the Vault (the note
//! on show), Files (the navigator), Search (quick open), and New note as
//! the round button. While the note on show has a `type` some extension
//! of the node declares favourites for (`/_ext/favourites.json`,
//! `crate::favourites`), its tabs take the slots: `page` the note, an
//! island's view (a view its own island lists, by name or as `<its id>/<view>`)
//! that view in a frame of its own (`super::frames`, a tab of the
//! workspace beside the note), a command its extension may run, or a note
//! beside it; a tab the browser cannot open, or that names what its
//! extension may not, is left out. The tab current is the one whose place
//! is on show; a `{place}` keeps the set while its note is on show. Its
//! primary action shows its first face: the browser knows no state yet.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use architect_commands::{Commands, Overlay};
use architect_favourites::{Effect, Listed};
use architect_navigator::{MemorySource, NavigatorSource as _, SourceEvent};
use architect_shell::{self as shell, Action, Declared, Favourites, PlaceOpened, Shows, Tab, icon};
use architect_workspace::Workspace;
use dioxus::prelude::*;
use lucide_dioxus::{Files, NotebookText, Search, SquarePen};

use super::{frames, vault};
use crate::favourites::{Opens, Reach, label_for, opens};

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
///
/// The set is the note on show's type's, or, while a note a `{place}` tab
/// opened is on show, the type of the note it was opened from
/// (`shell::set_note`). The note's `type` is read again whenever the vault
/// says the file changed.
pub fn use_web_favourites(
    workspace: Signal<Workspace>,
    source: &MemorySource,
    commands: Commands,
) -> Slots {
    let declared_list = use_resource(frames::favourites);
    let islands = use_resource(frames::island_views);
    // A place a tab opened, the note whose set it keeps and that note's
    // type.
    let mut opened = use_signal(|| None::<(PlaceOpened, Option<String>)>);
    // The declared tabs' presses, made once each (`shell::Presses`).
    let presses = shell::use_presses();
    let revision = use_file_changes(source, workspace, opened);
    let on_show = {
        let source = source.clone();
        use_memo(move || {
            let _ = revision();
            vault::note_on_show(&workspace.read(), &source)
        })
    };
    let framed = use_memo(move || vault::frame_on_show(&workspace.read()));
    // A place's set lasts while it or its note is on show, and its note
    // is still there with that type (`shell::place_kept`).
    {
        let source = source.clone();
        use_effect(move || {
            let path = on_show().map(|(p, _)| p);
            let stale = opened.peek().as_ref().is_some_and(|(o, ty)| {
                let now = vault::type_of(&source, &o.from);
                !shell::place_kept(
                    o,
                    ty.as_deref(),
                    path.as_deref(),
                    now.as_ref().map(Option::as_deref),
                )
            });
            if stale {
                opened.set(None);
            }
        });
    }
    let default = own_set(commands);
    let vault_slots = |set| Slots {
        set,
        current: Some("vault"),
    };
    let Some((shown, shown_type)) = on_show() else {
        return vault_slots(default);
    };
    let (path, ty) = match opened.read().clone() {
        Some((o, ty)) if o.place == shown => (o.from, ty),
        _ => (shown.clone(), shown_type),
    };
    let Some(ty) = ty else {
        return vault_slots(default);
    };
    let list: Rc<Vec<_>> = declared_list.read().clone().unwrap_or_default();
    let islands = islands.read().clone().unwrap_or_default();
    let decls: Vec<&Listed> = list.iter().filter(|d| d.favourites.is_for(&ty)).collect();
    if decls.is_empty() {
        return vault_slots(default);
    }
    let actions: BTreeMap<String, Effect> = commands
        .all()
        .into_iter()
        .filter(|c| c.enabled)
        .map(|c| (c.id, effect(c.effect)))
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
    for d in decls {
        let made = shell::declare(
            &d.favourites,
            |target| {
                let what = opens(&d.by, target, &reach)?;
                let shows = shows(&what, &path);
                let label = label_for(&d.favourites, target);
                let key = format!("{path}\0{ty}\0{}\0{label}\0{what:?}", d.by.extension);
                let (path, ty) = (path.clone(), ty.clone());
                let press = presses.get(key, || {
                    Callback::new(move |()| {
                        if let Opens::Path(place) = &what {
                            opened.set(Some((
                                PlaceOpened {
                                    place: place.clone(),
                                    from: path.clone(),
                                },
                                Some(ty.clone()),
                            )));
                        }
                        run(&what, &label, &path, workspace, commands);
                    })
                });
                Some((press, shows))
            },
            |_| false,
        );
        if let Some(made) = made {
            declared.push(made);
        }
    }
    if declared.is_empty() {
        return vault_slots(default);
    }
    let set = shell::resolve_favourites(&default, &declared, Some(&ty));
    // The tab whose place is on show: an island's view in its frame, else
    // the note itself.
    let on = match framed() {
        Some((island, view, note)) => Shows::View {
            extension: island,
            view,
            note,
        },
        None => Shows::Note(shown),
    };
    let current = set.current(&on);
    Slots { set, current }
}

/// What is on show once `what` opened, on note `note`.
fn shows(what: &Opens, note: &str) -> Shows {
    match what {
        Opens::Note => Shows::Note(note.to_owned()),
        Opens::Path(p) => Shows::Note(p.clone()),
        Opens::View { island, view } => Shows::View {
            extension: island.clone(),
            view: view.clone(),
            note: note.to_owned(),
        },
        Opens::QuickOpen | Opens::Files | Opens::Action(_) => Shows::Elsewhere,
    }
}

/// How much a command changes, as a manifest names it.
const fn effect(e: architect_commands::Effect) -> Effect {
    match e {
        architect_commands::Effect::ReadOnly => Effect::ReadOnly,
        architect_commands::Effect::Mutating => Effect::Mutating,
        architect_commands::Effect::Destructive => Effect::Destructive,
    }
}

/// A count bumped whenever the vault says the note on show's file, or the
/// note a place on show was opened from, changed (`SourceEvent::touches`):
/// their `type` is read again then.
fn use_file_changes(
    source: &MemorySource,
    workspace: Signal<Workspace>,
    opened: Signal<Option<(PlaceOpened, Option<String>)>>,
) -> Signal<u64> {
    let mut changed = use_signal(|| 0_u64);
    let source = source.clone();
    use_hook(move || {
        let (tx, mut rx) = futures_channel::mpsc::unbounded::<SourceEvent>();
        let sub = source.subscribe(Box::new(move |e| {
            let _ = tx.unbounded_send(e.clone());
        }));
        let source = source.clone();
        spawn(async move {
            use futures_util::StreamExt as _;
            while let Some(event) = rx.next().await {
                let on_show = vault::note_on_show(&workspace.peek(), &source).map(|(p, _)| p);
                let from = opened.peek().as_ref().map(|(o, _)| o.from.clone());
                if on_show.is_some_and(|p| event.touches(&p))
                    || from.is_some_and(|p| event.touches(&p))
                {
                    let n = changed.peek().wrapping_add(1);
                    changed.set(n);
                }
            }
        });
        Rc::new(sub)
    });
    changed
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
