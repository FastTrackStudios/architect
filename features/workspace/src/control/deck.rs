//! The scope tab actions (`SCOPE_*`), over the decks.
//!
//! Open a context in a scope tab, close, switch (the general tabs too),
//! move, list, and a scope tab's screenset (switch it, save its layout as
//! one, reset to it); and `TAB_KINDS`, every tab kind registered. Each is
//! a function over a [`DeckState`] here (no Dioxus, no I/O); feature
//! `actions` puts them on an Action List ([`register`]), run where the
//! decks live: [`HeadlessDecks`] for a test or a server, the app's window
//! for the tab bar (`host/deck.rs`).
//!
//! The prefix and the words a person reads come from the crate's one
//! `scope_word!`.

// The facet derive's generated code trips the arithmetic lint.
#![allow(clippy::arithmetic_side_effects)]

use facet::Facet;

use crate::deck::{ContextLayout, DeckContext, DeckId, DeckState, Load, Outcome};

/// The actions' ids.
pub mod ids {
    /// What every deck action's id starts with.
    pub const PREFIX: &str = scope_word!(prefix);
    pub const LIST: &str = concat!(scope_word!(prefix), "LIST");
    pub const OPEN: &str = concat!(scope_word!(prefix), "OPEN");
    pub const CLOSE: &str = concat!(scope_word!(prefix), "CLOSE");
    pub const NEXT: &str = concat!(scope_word!(prefix), "NEXT");
    pub const PREV: &str = concat!(scope_word!(prefix), "PREV");
    pub const MOVE: &str = concat!(scope_word!(prefix), "MOVE");
    pub const ACTIVATE: &str = concat!(scope_word!(prefix), "ACTIVATE");
    pub const DUPLICATE: &str = concat!(scope_word!(prefix), "DUPLICATE");
    pub const REOPEN: &str = concat!(scope_word!(prefix), "REOPEN");
    pub const SET_SCREENSET: &str = concat!(scope_word!(prefix), "SET_SCREENSET");
    pub const SAVE_AS_SCREENSET: &str = concat!(scope_word!(prefix), "SAVE_AS_SCREENSET");
    pub const RESET: &str = concat!(scope_word!(prefix), "RESET");
    pub const SET_DEFAULT: &str = concat!(scope_word!(prefix), "SET_DEFAULT");
    pub const GENERAL: &str = concat!(scope_word!(prefix), "GENERAL");
    /// Every tab kind (not a deck action: on the list beside them).
    pub const TAB_KINDS: &str = "TAB_KINDS";

    /// Every id [`super::register`] puts on a list.
    pub const ALL: &[&str] = &[
        LIST,
        OPEN,
        CLOSE,
        NEXT,
        PREV,
        MOVE,
        ACTIVATE,
        DUPLICATE,
        REOPEN,
        SET_SCREENSET,
        SAVE_AS_SCREENSET,
        RESET,
        SET_DEFAULT,
        GENERAL,
    ];
}

/// The change event's topic: each write that changed something.
pub const TOPIC: &str = scope_word!(plural);

/// One deck, as an answer lists it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckRow {
    pub id: String,
    pub index: u32,
    pub context: DeckContext,
    /// Its screenset's id and name.
    pub screenset: String,
    pub screenset_name: String,
    /// Its layout differs from its screenset's (on this class of screen).
    pub modified: bool,
    #[facet(default)]
    pub badge: Option<String>,
    pub active: bool,
}

/// A screenset a deck can be shown in.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ScreensetRow {
    pub id: String,
    pub name: String,
}

/// A context type's default screenset, and who says so.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DefaultRow {
    pub kind: String,
    pub screenset: String,
    /// `person`, or the template's giver (`app`, an extension's id).
    pub by: String,
}

/// `SCOPE_LIST`: the scope tabs, and the general tabs.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckList {
    /// The scope tab on show; none while the general tabs show.
    pub active: Option<String>,
    pub decks: Vec<DeckRow>,
    /// The general tabs (id `general`): what shows with no scope tab
    /// active.
    pub general: DeckRow,
    pub screensets: Vec<ScreensetRow>,
    /// Each context type's default screenset.
    pub defaults: Vec<DefaultRow>,
    /// How many closed decks `SCOPE_REOPEN` can bring back.
    pub closed: u32,
}

/// What every deck write answers.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckAnswer {
    pub changed: bool,
    /// Why nothing changed: `already_open`, `already_active`, `same`,
    /// `only_deck`, `not_modified`, `nothing_closed`.
    #[facet(default)]
    pub reason: Option<String>,
    /// The deck acted on.
    #[facet(default)]
    pub deck: Option<String>,
    /// A screenset made (`SCOPE_SAVE_AS_SCREENSET`).
    #[facet(default)]
    pub screenset: Option<String>,
    pub list: DeckList,
}

/// `SCOPE_OPEN`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckOpenArgs {
    /// The context: `kind` (`song`, `project`), `id`, `title`, and
    /// optionally `icon` and `folder`.
    pub context: DeckContext,
    /// Onto a new deck after the one on show; else onto the one on show
    /// (a context open already brings its deck forward).
    #[facet(default)]
    pub new: bool,
    /// The screenset to show it in; else what its context kept, else its
    /// type's default.
    #[facet(default)]
    pub screenset: Option<String>,
}

/// A deck by id; the one on show when unset.
#[derive(Debug, Clone, PartialEq, Eq, Default, Facet)]
pub struct DeckArgs {
    #[facet(default)]
    pub deck: Option<String>,
}

/// `SCOPE_ACTIVATE`: a deck by `deck`, or by place (`index`, from 0).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckActivateArgs {
    #[facet(default)]
    pub deck: Option<String>,
    #[facet(default)]
    pub index: Option<u32>,
}

/// `SCOPE_MOVE`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckMoveArgs {
    pub deck: String,
    /// Its new place among the scope tabs, from 0.
    pub index: u32,
}

/// `SCOPE_SET_SCREENSET`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckScreensetArgs {
    #[facet(default)]
    pub deck: Option<String>,
    /// The screenset's id.
    pub screenset: String,
}

/// `SCOPE_SAVE_AS_SCREENSET`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckSaveAsArgs {
    #[facet(default)]
    pub deck: Option<String>,
    /// The new screenset's name.
    pub name: String,
}

/// `SCOPE_SET_DEFAULT`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckDefaultArgs {
    /// The context type.
    pub kind: String,
    /// The screenset's id; unset goes back to the templates'.
    #[facet(default)]
    pub screenset: Option<String>,
}

/// What a context kept, when a deck opens on it: the app's
/// [`crate::store::ContextStore`], read where the decks live.
pub trait Kept {
    fn kept(&self, context: &DeckContext) -> Option<ContextLayout>;

    /// `context` as the app knows it: what a context given by its type and
    /// id alone lacks, filled in (a project's folder).
    fn resolve(&self, context: DeckContext) -> DeckContext {
        context
    }
}

impl Kept for () {
    fn kept(&self, _: &DeckContext) -> Option<ContextLayout> {
        None
    }
}

/// The scope tabs and the general tabs as they are.
#[must_use]
pub fn list(st: &DeckState) -> DeckList {
    let active = st.decks.active.clone();
    let name = |id: &str| {
        st.screensets
            .get(id)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    };
    let mut kinds: Vec<String> = st.templates.iter().map(|t| t.kind.clone()).collect();
    kinds.extend(st.decks.defaults.iter().map(|d| d.kind.clone()));
    kinds.extend(st.decks.decks.iter().map(|d| d.context.kind.clone()));
    kinds.sort();
    kinds.dedup();
    let g = &st.decks.general;
    DeckList {
        active: active.as_ref().map(|a| a.0.clone()),
        general: DeckRow {
            id: g.id.0.clone(),
            index: 0,
            context: g.context.clone(),
            screenset: g.screenset.clone(),
            screenset_name: name(&g.screenset),
            modified: g.modified(st.form),
            badge: None,
            active: active.is_none(),
        },
        decks: st
            .decks
            .decks
            .iter()
            .enumerate()
            .map(|(i, d)| DeckRow {
                id: d.id.0.clone(),
                index: u32::try_from(i).unwrap_or(u32::MAX),
                context: d.context.clone(),
                screenset: d.screenset.clone(),
                screenset_name: name(&d.screenset),
                modified: d.modified(st.form),
                badge: d.badge.clone(),
                active: active.as_ref() == Some(&d.id),
            })
            .collect(),
        screensets: st
            .screensets
            .sets
            .iter()
            .map(|s| ScreensetRow {
                id: s.id.clone(),
                name: s.name.clone(),
            })
            .collect(),
        defaults: kinds
            .into_iter()
            .map(|kind| {
                let by = if st.decks.default_for(&kind).is_some() {
                    "person".to_owned()
                } else {
                    st.templates
                        .iter()
                        .find(|t| t.kind == kind)
                        .map_or_else(|| "app".to_owned(), |t| t.by.clone())
                };
                DefaultRow {
                    screenset: st.screenset_for(&kind),
                    kind,
                    by,
                }
            })
            .collect(),
        closed: u32::try_from(st.decks.closed.len()).unwrap_or(u32::MAX),
    }
}

fn answer(st: &DeckState, out: Outcome) -> DeckAnswer {
    let (changed, reason, deck) = match out {
        Ok(id) => (true, None, id),
        Err((id, why)) => (false, Some(why.as_str().to_owned()), id),
    };
    DeckAnswer {
        changed,
        reason,
        deck: Some(deck.0),
        screenset: None,
        list: list(st),
    }
}

/// `deck`, else the scope tab on show; else (for what the general tabs
/// can take too: a screenset, a reset) the general tabs.
fn target(st: &DeckState, deck: Option<String>) -> Result<DeckId, String> {
    deck.map(DeckId)
        .or_else(|| st.decks.active.clone())
        .ok_or_else(|| format!("no {} is open", crate::deck::NOUN))
}

fn target_or_general(st: &DeckState, deck: Option<String>) -> DeckId {
    deck.map_or_else(|| st.decks.shown_id(), DeckId)
}

/// `SCOPE_GENERAL`.
///
/// # Errors
///
/// Never.
pub fn general(st: &mut DeckState) -> Result<DeckAnswer, String> {
    let out = st.show_general();
    Ok(answer(st, out))
}

/// `SCOPE_OPEN`.
///
/// # Errors
///
/// A screenset that is not there.
pub fn open(st: &mut DeckState, kept: &dyn Kept, a: DeckOpenArgs) -> Result<DeckAnswer, String> {
    let load = if a.new { Load::NewDeck } else { Load::Here };
    let context = kept.resolve(a.context);
    let found = kept.kept(&context);
    let out = st
        .open(context, load, a.screenset.as_deref(), found)
        .map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_CLOSE`.
///
/// # Errors
///
/// No such deck.
pub fn close(st: &mut DeckState, a: DeckArgs) -> Result<DeckAnswer, String> {
    let id = target(st, a.deck)?;
    let out = st.close(&id).map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_NEXT` (`by` 1) and `SCOPE_PREV` (-1).
///
/// # Errors
///
/// No deck is open.
pub fn cycle(st: &mut DeckState, by: isize) -> Result<DeckAnswer, String> {
    let out = st.cycle(by).map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_ACTIVATE`.
///
/// # Errors
///
/// No such deck.
pub fn activate(st: &mut DeckState, a: DeckActivateArgs) -> Result<DeckAnswer, String> {
    let out = match (a.deck, a.index) {
        (Some(id), _) => st.activate(&DeckId(id)),
        (None, Some(i)) => st.activate_nth(usize::try_from(i).unwrap_or(usize::MAX)),
        (None, None) => return Err("give `deck` or `index`".to_owned()),
    }
    .map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_MOVE`.
///
/// # Errors
///
/// No such deck.
pub fn move_deck(st: &mut DeckState, a: DeckMoveArgs) -> Result<DeckAnswer, String> {
    let index = usize::try_from(a.index).unwrap_or(usize::MAX);
    let out = st
        .move_to(&DeckId(a.deck), index)
        .map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_DUPLICATE`.
///
/// # Errors
///
/// No such deck.
pub fn duplicate(st: &mut DeckState, a: DeckArgs) -> Result<DeckAnswer, String> {
    let id = target(st, a.deck)?;
    let out = st.duplicate(&id).map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_REOPEN`.
///
/// # Errors
///
/// Never: nothing closed answers `nothing_closed`.
pub fn reopen(st: &mut DeckState, (): ()) -> Result<DeckAnswer, String> {
    let reopened = st.reopen();
    Ok(reopened.map_or_else(
        || DeckAnswer {
            changed: false,
            reason: Some("nothing_closed".to_owned()),
            deck: None,
            screenset: None,
            list: list(st),
        },
        |id| answer(st, Ok(id)),
    ))
}

/// `SCOPE_SET_SCREENSET`.
///
/// # Errors
///
/// No such deck or screenset.
pub fn set_screenset(st: &mut DeckState, a: DeckScreensetArgs) -> Result<DeckAnswer, String> {
    let id = target_or_general(st, a.deck);
    let out = st
        .set_screenset(&id, &a.screenset)
        .map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_SAVE_AS_SCREENSET`.
///
/// # Errors
///
/// No such deck, or an empty name.
pub fn save_as_screenset(st: &mut DeckState, a: DeckSaveAsArgs) -> Result<DeckAnswer, String> {
    if a.name.trim().is_empty() {
        return Err("give the screenset a `name`".to_owned());
    }
    let id = target_or_general(st, a.deck);
    let made = st
        .save_as_screenset(&id, a.name.trim())
        .map_err(|e| e.to_string())?;
    let mut out = answer(st, Ok(id));
    out.screenset = Some(made);
    Ok(out)
}

/// `SCOPE_RESET`.
///
/// # Errors
///
/// No such deck.
pub fn reset(st: &mut DeckState, a: DeckArgs) -> Result<DeckAnswer, String> {
    let id = target_or_general(st, a.deck);
    let out = st.reset(&id).map_err(|e| e.to_string())?;
    Ok(answer(st, out))
}

/// `SCOPE_SET_DEFAULT`.
///
/// # Errors
///
/// A screenset that is not there.
pub fn set_default(st: &mut DeckState, a: &DeckDefaultArgs) -> Result<DeckAnswer, String> {
    if let Some(s) = a.screenset.as_deref() {
        if st.screensets.get(s).is_none() {
            return Err(format!("no screenset is called `{s}`"));
        }
    }
    let before = st.screenset_for(&a.kind);
    st.decks.set_default(&a.kind, a.screenset.as_deref());
    let changed = st.screenset_for(&a.kind) != before;
    Ok(DeckAnswer {
        changed,
        reason: (!changed).then(|| "same".to_owned()),
        deck: None,
        screenset: Some(st.screenset_for(&a.kind)),
        list: list(st),
    })
}

/// A deck template's shipped layout (`[[deck_templates]] layout`:
/// `WORKSPACE_EXPORT`'s tree, as JSON) as a built-in screenset called
/// `name`, for a desk; every view it names must be one of `views`.
///
/// # Errors
///
/// JSON that is not a layout, or a layout that does not fit
/// (`WORKSPACE_APPLY`'s errors).
pub fn template_screenset(
    name: &str,
    layout_json: &str,
    views: Vec<super::ViewSpec>,
) -> Result<crate::Screenset, String> {
    let layout: super::PortableLayout =
        facet_json::from_str(layout_json).map_err(|e| e.to_string())?;
    let mut st = super::State::new(crate::Workspace::new());
    super::apply(
        &mut st,
        &super::Env::new(views),
        super::ApplyArgs {
            layout,
            if_revision: None,
            keep_unplaced: false,
        },
    )?;
    Ok(crate::Screenset::new(name).layout(crate::FormClass::Desk, st.workspace))
}

#[cfg(feature = "actions")]
pub use on_list::{DeckExec, DeckJob, HeadlessDecks, register, register_tab_kinds};

#[cfg(feature = "actions")]
mod on_list {
    use std::sync::{Arc, Mutex, PoisonError};

    use architect::action::{ActionList, ActionMeta, Effect};
    use facet::Facet;

    use super::{
        DeckActivateArgs, DeckAnswer, DeckArgs, DeckDefaultArgs, DeckList, DeckMoveArgs,
        DeckOpenArgs, DeckSaveAsArgs, DeckScreensetArgs, Kept, ids,
    };
    use crate::deck::{ContextLayout, DeckContext, DeckState};

    /// A write, sent to where the decks live.
    pub type DeckJob = Box<dyn FnOnce(&mut DeckState, &dyn Kept) + Send>;

    /// Where the decks live, as the actions reach them.
    pub trait DeckExec: Send + Sync + 'static {
        /// Runs `job` there (in its turn); false when the decks are gone.
        fn exec(&self, job: DeckJob) -> bool;
        /// Reads the decks as they are now.
        fn read(&self, f: &mut dyn FnMut(&DeckState));
    }

    const GONE: &str = "the decks are gone";

    macro_rules! metas {
        ($($name:ident = $id:ident, $display:expr, $desc:expr;)*) => {
            $(static $name: ActionMeta = ActionMeta {
                id: ids::$id,
                trait_name: "",
                method_name: "",
                display_name: $display,
                description: $desc,
                category: "Workspace",
                group: scope_word!(plural_title),
                shortcut: "",
                toggleable: false,
                undo: false,
            };)*
        };
    }

    metas! {
        LIST = LIST, scope_word!(plural_title),
            concat!("Every open ", scope_word!(noun), " in the tab bar's order (its id, context, screenset, whether its layout differs from the screenset's, its badge), the one on show (none: the ", scope_word!(general), "), the ", scope_word!(general), " (`general`), the screensets one can be shown in, each context type's default screenset, and how many closed ones can be reopened.");
        OPEN = OPEN, concat!("Load onto a ", scope_word!(noun)),
            concat!("Opens a context (`context`: kind, id, title, and optionally icon, colour, folder, properties) in the ", scope_word!(noun), " on show, or with `new: true` (or while the ", scope_word!(general), " show) in a new ", scope_word!(noun), " after it. A context open already brings its ", scope_word!(noun), " forward (`already_open` when it shows). It is shown in `screenset`, else the screenset and changes its context kept, else its type's default.");
        CLOSE = CLOSE, concat!("Close a ", scope_word!(noun)),
            concat!("Closes `deck` (the one on show when unset); its neighbour shows, else the ", scope_word!(general), ". Its context keeps its layout, and ", scope_word!(prefix), "REOPEN brings it back.");
        NEXT = NEXT, concat!("Next ", scope_word!(noun)),
            concat!("Shows the next along the ", scope_word!(general), " and then each ", scope_word!(noun), " (round the end). With none open it answers `only_deck`.");
        PREV = PREV, concat!("Previous ", scope_word!(noun)),
            concat!("Shows the previous along the ", scope_word!(general), " and each ", scope_word!(noun), " (round the start).");
        MOVE = MOVE, concat!("Move a ", scope_word!(noun)),
            concat!("Moves `deck` to place `index` (from 0) among the ", scope_word!(plural), ".");
        ACTIVATE = ACTIVATE, concat!("Show a ", scope_word!(noun)),
            "Shows `deck`, or the one at `index` (from 0; 8 and past is the last, as ⌘9 is).";
        DUPLICATE = DUPLICATE, concat!("Duplicate a ", scope_word!(noun)),
            concat!("A second ", scope_word!(noun), " on the same context, laid out as `deck` is now, after it and on show.");
        REOPEN = REOPEN, concat!("Reopen a closed ", scope_word!(noun)),
            concat!("Brings back the last closed ", scope_word!(noun), " as it was, on show.");
        SET_SCREENSET = SET_SCREENSET, concat!("Switch this ", scope_word!(noun), "'s screenset"),
            concat!("Shows `deck` (the one on show when unset; `general` for the ", scope_word!(general), ") in screenset `screenset` (an id from ", scope_word!(prefix), "LIST); its own changes to the old one go.");
        SAVE_AS_SCREENSET = SAVE_AS_SCREENSET, "Save as screenset",
            concat!("Saves `deck`'s layout as a new screenset called `name`, which the ", scope_word!(noun), " is then shown in; other ", scope_word!(plural), " can use it.");
        RESET = RESET, "Reset to the screenset",
            "Drops `deck`'s own changes: it shows its screenset as saved. One with none answers `not_modified`.";
        GENERAL = GENERAL, concat!("Show the ", scope_word!(general)),
            concat!("Shows the ", scope_word!(general), ": no ", scope_word!(noun), " active. Answers `already_active` when they show.");
        SET_DEFAULT = SET_DEFAULT, "Set a context type's default screenset",
            concat!("New ", scope_word!(plural), " on contexts of type `kind` (`song`, `project`) are shown in `screenset`; unset goes back to the app's or an extension's template.");
    }

    fn write<A>(
        list: &ActionList,
        exec: &Arc<dyn DeckExec>,
        meta: &'static ActionMeta,
        effect: Effect,
        f: fn(&mut DeckState, &dyn Kept, A) -> Result<DeckAnswer, String>,
    ) -> Result<(), architect::action::ActionError>
    where
        A: Facet<'static> + Send + 'static,
    {
        let exec = exec.clone();
        list.register_async_with(
            meta,
            effect,
            move |_: &architect::action::ActionCaller, args: A| {
                let (tx, rx) = futures_channel::oneshot::channel();
                let sent = exec.exec(Box::new(move |st, kept| {
                    let _ = tx.send(f(st, kept, args));
                }));
                async move {
                    if !sent {
                        return Err(GONE.to_owned());
                    }
                    rx.await.map_err(|_| GONE.to_owned())?
                }
            },
        )
    }

    /// Puts every deck action (`ids::ALL`) on `list`, run through `exec`.
    ///
    /// # Errors
    ///
    /// An id already on `list`.
    pub fn register(
        list: &ActionList,
        exec: &Arc<dyn DeckExec>,
    ) -> Result<(), architect::action::ActionError> {
        use Effect::{Destructive, Mutating};
        {
            let exec = exec.clone();
            list.register(&LIST, Effect::ReadOnly, move |(): ()| {
                let mut out: Result<DeckList, String> = Err(GONE.to_owned());
                exec.read(&mut |st| out = Ok(super::list(st)));
                out
            })?;
        }
        let e = exec;
        write::<DeckOpenArgs>(list, e, &OPEN, Mutating, super::open)?;
        write::<DeckArgs>(list, e, &CLOSE, Destructive, |st, _, a| super::close(st, a))?;
        write::<()>(list, e, &NEXT, Mutating, |st, _, ()| super::cycle(st, 1))?;
        write::<()>(list, e, &PREV, Mutating, |st, _, ()| super::cycle(st, -1))?;
        write::<DeckMoveArgs>(list, e, &MOVE, Mutating, |st, _, a| super::move_deck(st, a))?;
        write::<DeckActivateArgs>(list, e, &ACTIVATE, Mutating, |st, _, a| {
            super::activate(st, a)
        })?;
        write::<DeckArgs>(list, e, &DUPLICATE, Mutating, |st, _, a| {
            super::duplicate(st, a)
        })?;
        write::<()>(list, e, &REOPEN, Mutating, |st, _, ()| {
            super::reopen(st, ())
        })?;
        write::<DeckScreensetArgs>(list, e, &SET_SCREENSET, Mutating, |st, _, a| {
            super::set_screenset(st, a)
        })?;
        write::<DeckSaveAsArgs>(list, e, &SAVE_AS_SCREENSET, Mutating, |st, _, a| {
            super::save_as_screenset(st, a)
        })?;
        write::<DeckArgs>(list, e, &RESET, Mutating, |st, _, a| super::reset(st, a))?;
        write::<DeckDefaultArgs>(list, e, &SET_DEFAULT, Mutating, |st, _, a| {
            super::set_default(st, &a)
        })?;
        write::<()>(list, e, &GENERAL, Mutating, |st, _, ()| super::general(st))?;
        Ok(())
    }

    static TAB_KINDS: ActionMeta = ActionMeta {
        id: ids::TAB_KINDS,
        trait_name: "",
        method_name: "",
        display_name: "Tab kinds",
        description: concat!(
            "Every tab kind the app and its extensions registered: its id, title, whether its tabs are ",
            scope_word!(plural),
            " (`scope`), its width (`compact`, `wide`), icon, colour, subtitle template, the views a custom kind's tabs are, its menu rows, and who registered it."
        ),
        category: "Workspace",
        group: scope_word!(plural_title),
        shortcut: "",
        toggleable: false,
        undo: false,
    };

    /// Puts `TAB_KINDS` on `list`, answering what `kinds` reads.
    ///
    /// # Errors
    ///
    /// The id already on `list`.
    pub fn register_tab_kinds(
        list: &ActionList,
        kinds: Arc<dyn Fn() -> crate::TabKinds + Send + Sync>,
    ) -> Result<(), architect::action::ActionError> {
        list.register(&TAB_KINDS, Effect::ReadOnly, move |(): ()| {
            Ok::<_, String>(kinds())
        })
    }

    /// Decks with no window, behind a lock: for a test, a server, a model.
    /// What each context keeps is kept here too, by its key.
    #[derive(Clone)]
    pub struct HeadlessDecks {
        shared: Arc<Mutex<(DeckState, Vec<ContextLayout>)>>,
    }

    struct Memory<'a>(&'a [ContextLayout]);

    impl Kept for Memory<'_> {
        fn kept(&self, context: &DeckContext) -> Option<ContextLayout> {
            self.0.iter().find(|k| k.context.same(context)).cloned()
        }
    }

    impl HeadlessDecks {
        #[must_use]
        pub fn new(state: DeckState) -> Self {
            Self {
                shared: Arc::new(Mutex::new((state, Vec::new()))),
            }
        }

        /// Puts the deck actions on `list`.
        ///
        /// # Errors
        ///
        /// As [`register`].
        pub fn install(&self, list: &ActionList) -> Result<(), architect::action::ActionError> {
            let exec: Arc<dyn DeckExec> = Arc::new(Headless {
                shared: self.shared.clone(),
                list: list.clone(),
            });
            register(list, &exec)
        }

        /// A copy of the decks' state now.
        #[must_use]
        pub fn state(&self) -> DeckState {
            self.shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .0
                .clone()
        }

        /// The live layout changed (a person arranged the deck on show).
        pub fn arrange(&self, f: impl FnOnce(&mut crate::Workspace)) {
            let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let st = &mut shared.0;
            f(&mut st.live);
            let (form, live) = (st.form, st.live.clone());
            st.screensets.set_working(form, live);
            st.record();
            let kept: Vec<ContextLayout> = shared
                .0
                .decks
                .decks
                .iter()
                .map(crate::Deck::context_layout)
                .collect();
            remember(&mut shared.1, kept);
        }
    }

    fn remember(into: &mut Vec<ContextLayout>, kept: Vec<ContextLayout>) {
        for k in kept {
            match into.iter_mut().find(|o| o.context.same(&k.context)) {
                Some(o) => *o = k,
                None => into.push(k),
            }
        }
    }

    struct Headless {
        shared: Arc<Mutex<(DeckState, Vec<ContextLayout>)>>,
        list: ActionList,
    }

    impl DeckExec for Headless {
        fn exec(&self, job: DeckJob) -> bool {
            let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let (st, kept) = &mut *shared;
            let before = st.decks.clone();
            job(st, &Memory(kept));
            let mut all: Vec<ContextLayout> = st
                .decks
                .decks
                .iter()
                .map(crate::Deck::context_layout)
                .collect();
            all.extend(st.decks.closed.iter().map(crate::Deck::context_layout));
            remember(kept, all);
            let changed = st.decks != before;
            let active = st.decks.active.clone();
            drop(shared);
            if changed {
                self.list
                    .changed(super::TOPIC, &active.map(|a| a.0).unwrap_or_default());
            }
            true
        }

        fn read(&self, f: &mut dyn FnMut(&DeckState)) {
            let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            f(&shared.0);
        }
    }
}
