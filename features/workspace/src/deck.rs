//! Decks: one loaded context each (a song, a project), shown in a
//! screenset, with that context's own changes to it.
//!
//! A person sees a deck as a **scope tab** at the leading edge of the tab
//! bar (a scope tab kind's, [`crate::TabKind`]); the tabs they open while
//! it shows are its.
//!
//! Two primitives, kept apart (`docs/content/architecture/workspace.md`,
//! "Decks"):
//!
//! - a [`Screenset`](crate::Screenset) is a named, reusable layout
//!   ("Mixing"): splits, docks, which views go where; it names no song;
//! - a [`Deck`] is one [`DeckContext`] plus the screenset it is shown in,
//!   plus the context's **override** of it: what the person rearranged
//!   inside the deck, kept per context so it opens again as it was left.
//!
//! [`DeckState`] is what the operations change: the [`Decks`] (the scope
//! tabs, and the general tabs' own deck, [`Decks::general`], on show when
//! no scope tab is), the screensets, and the live [`Workspace`] the deck
//! on show draws into.
//! Switching records the live layout into the outgoing deck
//! ([`DeckState::record`]) and puts the incoming deck's in its place
//! ([`DeckState::show`]). Pure data: no Dioxus, no I/O, no clock.
//!
//! "Deck" is the code's name only: every word a person reads ("scope
//! tab") and every action id (`SCOPE_*`) comes from the crate's one
//! `scope_word!` (its words here: [`NOUN`], [`NOUN_TITLE`],
//! [`NOUN_PLURAL`]; the actions' prefix:
//! [`crate::control::deck::ids::PREFIX`]).

use facet::Facet;

use crate::model::Workspace;
use crate::screenset::{FormClass, Layout, Screensets};

/// What a person calls a deck, in a sentence ("Open in a new scope tab").
pub const NOUN: &str = scope_word!(noun);
/// The same at the start of a label ("Scope tab").
pub const NOUN_TITLE: &str = scope_word!(title);
/// More than one ("scope tabs").
pub const NOUN_PLURAL: &str = scope_word!(plural);
/// More than one, at the start of a label ("Scope tabs").
pub const NOUN_PLURAL_TITLE: &str = scope_word!(plural_title);
/// What a person calls the tabs on show when no scope tab is ("the
/// general tabs").
pub const GENERAL_NOUN: &str = scope_word!(general);

/// The general tabs' deck's id ([`Decks::general`]).
pub const GENERAL: &str = "general";

/// One of a context's properties, as its tab kind's subtitle reads it
/// (`key` `G`, `tempo` `96`).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ContextProperty {
    pub key: String,
    pub value: String,
}

/// What a deck holds open: a song, a project, a vault, an org root.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckContext {
    /// Its type (`song`, `project`, `vault`): which default screenset it
    /// gets ([`DeckTemplate`]).
    pub kind: String,
    /// Unique among its type's (a song's id, a project's folder).
    pub id: String,
    /// What its deck says on the strip.
    pub title: String,
    /// A Lucide icon's name.
    #[facet(default)]
    pub icon: Option<String>,
    /// Its icon's colour, a CSS colour (a project's own `color`).
    #[facet(default)]
    pub colour: Option<String>,
    /// Its own folder, when it has one (a project's): its override is kept
    /// in the folder's `.architect/` (`home::ContextFiles`).
    #[facet(default)]
    pub folder: Option<String>,
    /// Its properties (a song's `key` and `tempo`): what its tab kind's
    /// subtitle is made of ([`crate::TabKind::subtitle`]).
    #[facet(default)]
    pub properties: Vec<ContextProperty>,
}

impl DeckContext {
    pub fn new(kind: impl Into<String>, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            title: title.into(),
            icon: None,
            colour: None,
            folder: None,
            properties: Vec::new(),
        }
    }

    /// With property `key` (another value for it replaces the old one).
    #[must_use]
    pub fn property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        self.properties.retain(|p| p.key != key);
        self.properties.push(ContextProperty {
            key,
            value: value.into(),
        });
        self
    }

    /// Property `key`'s value.
    #[must_use]
    pub fn get_property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.value.as_str())
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn colour(mut self, colour: impl Into<String>) -> Self {
        self.colour = Some(colour.into());
        self
    }

    #[must_use]
    pub fn folder(mut self, folder: impl Into<String>) -> Self {
        self.folder = Some(folder.into());
        self
    }

    /// `kind:id`: what names the context in a store.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }

    /// The same context (its title may have changed).
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

/// A deck's id: opaque, never handed out twice in a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
#[repr(transparent)]
pub struct DeckId(pub String);

impl DeckId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// One deck: a context, the screenset it is shown in, and its override of
/// that screenset (per class of screen; none: shown as saved).
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Deck {
    pub id: DeckId,
    pub context: DeckContext,
    /// The [`Screenset`](crate::Screenset)'s id; `""` for none (a blank
    /// workspace).
    pub screenset: String,
    /// What the person changed inside this deck, per class of screen.
    #[facet(default)]
    pub overrides: Vec<Layout>,
    /// A word the app puts on the deck's tab (a song's unsaved session).
    #[facet(default)]
    pub badge: Option<String>,
}

impl Default for Deck {
    /// The general tabs' deck: [`GENERAL`], on a `general` context, shown
    /// in no screenset until its type's default is given.
    fn default() -> Self {
        Self::general(DeckContext::new(
            GENERAL,
            GENERAL,
            scope_word!(general_title),
        ))
    }
}

impl Deck {
    /// The general tabs' deck over `context` (its type picks its default
    /// screenset: the app's `vault`).
    #[must_use]
    pub fn general(context: DeckContext) -> Self {
        Self {
            id: DeckId::new(GENERAL),
            context,
            screenset: String::new(),
            overrides: Vec::new(),
            badge: None,
        }
    }

    /// The general tabs' own deck, not a scope tab.
    #[must_use]
    pub fn is_general(&self) -> bool {
        self.id.0 == GENERAL
    }

    /// Its override for `class`.
    #[must_use]
    pub fn override_for(&self, class: FormClass) -> Option<&Workspace> {
        self.overrides
            .iter()
            .find(|l| l.class == class)
            .map(|l| &l.workspace)
    }

    /// Its layout differs from its screenset's on `class`: the strip's
    /// marker.
    #[must_use]
    pub fn modified(&self, class: FormClass) -> bool {
        self.override_for(class).is_some()
    }

    fn set_override(&mut self, class: FormClass, workspace: Option<Workspace>) -> bool {
        let at = self.overrides.iter().position(|l| l.class == class);
        match (at, workspace) {
            (Some(i), None) => {
                self.overrides.remove(i);
                true
            }
            (Some(i), Some(w)) => match self.overrides.get_mut(i) {
                Some(l) if l.workspace != w => {
                    l.workspace = w;
                    true
                }
                _ => false,
            },
            (None, Some(w)) => {
                self.overrides.push(Layout {
                    class,
                    workspace: w,
                });
                true
            }
            (None, None) => false,
        }
    }

    /// What is kept with its context: the screenset and the override.
    #[must_use]
    pub fn context_layout(&self) -> ContextLayout {
        ContextLayout {
            context: self.context.clone(),
            screenset: self.screenset.clone(),
            layouts: self.overrides.clone(),
        }
    }
}

/// What is kept with a context ([`crate::store::ContextStore`]): the
/// screenset its deck was shown in and its override, so it opens again as
/// it was left.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ContextLayout {
    pub context: DeckContext,
    pub screenset: String,
    #[facet(default)]
    pub layouts: Vec<Layout>,
}

impl ContextLayout {
    /// As JSON.
    ///
    /// # Errors
    ///
    /// When it cannot be written (it always can).
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// From JSON; every layout repaired.
    ///
    /// # Errors
    ///
    /// JSON that is not a context layout.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut out: Self = facet_json::from_str(json).map_err(|e| e.to_string())?;
        for l in &mut out.layouts {
            l.workspace.repair();
        }
        Ok(out)
    }
}

/// A context type's default screenset, as the app or an extension
/// (`[[deck_templates]]`) gives it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DeckTemplate {
    /// The context type (`song`).
    pub kind: String,
    /// The screenset a deck of it is shown in.
    pub screenset: String,
    /// Who gave it (an extension's id; `app` for the app).
    #[facet(default)]
    pub by: String,
}

impl DeckTemplate {
    pub fn new(kind: impl Into<String>, screenset: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            screenset: screenset.into(),
            by: "app".to_owned(),
        }
    }
}

/// The person's pick of a context type's default screenset.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct KindDefault {
    pub kind: String,
    pub screenset: String,
}

/// The scope tabs: every open deck in order, and the one on show.
///
/// The active one is none while the general tabs show; then the general
/// tabs' own deck, and the person's defaults. Kept per device as the
/// session ([`Decks::session`]); each scope tab's override is kept with
/// its context, the general tabs' in the session.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct Decks {
    /// The scope tabs, in the bar's order.
    pub decks: Vec<Deck>,
    /// The scope tab on show; none: the general tabs.
    pub active: Option<DeckId>,
    /// The general tabs: what shows with no scope tab active (its id
    /// [`GENERAL`]). Not a scope tab: it has no tab and never closes.
    #[facet(default)]
    pub general: Deck,
    /// The next deck id's number.
    #[facet(default)]
    pub next_id: u64,
    /// The person's default screenset per context type, over templates.
    #[facet(default)]
    pub defaults: Vec<KindDefault>,
    /// Closed decks, newest last, to reopen.
    #[facet(default)]
    pub closed: Vec<Deck>,
}

impl Decks {
    /// How many closed decks are kept to reopen.
    pub const CLOSED_KEPT: usize = 20;

    /// Scope tab `id`, or the general tabs' deck for [`GENERAL`].
    #[must_use]
    pub fn get(&self, id: &DeckId) -> Option<&Deck> {
        if id.0 == GENERAL {
            return Some(&self.general);
        }
        self.decks.iter().find(|d| &d.id == id)
    }

    pub fn get_mut(&mut self, id: &DeckId) -> Option<&mut Deck> {
        if id.0 == GENERAL {
            return Some(&mut self.general);
        }
        self.decks.iter_mut().find(|d| &d.id == id)
    }

    /// The deck on show: the active scope tab's, else the general tabs'.
    #[must_use]
    pub fn shown(&self) -> &Deck {
        self.active_deck().unwrap_or(&self.general)
    }

    /// The id of the deck on show ([`GENERAL`] for the general tabs).
    #[must_use]
    pub fn shown_id(&self) -> DeckId {
        self.shown().id.clone()
    }

    /// The general tabs show: no scope tab is active.
    #[must_use]
    pub fn general_shown(&self) -> bool {
        self.active_deck().is_none()
    }

    /// Folds an older session's decks on contexts of type `kind` (the
    /// app's vault, a deck before the general tabs were apart) into the
    /// general tabs: the first becomes them, layout and all; the rest go.
    /// True when there were any.
    pub fn fold_general(&mut self, kind: &str) -> bool {
        let Some(at) = self.decks.iter().position(|d| d.context.kind == kind) else {
            return false;
        };
        let was = self.decks.remove(at);
        let folded: Vec<DeckId> = std::iter::once(was.id.clone())
            .chain(
                self.decks
                    .iter()
                    .filter(|d| d.context.kind == kind)
                    .map(|d| d.id.clone()),
            )
            .collect();
        self.decks.retain(|d| d.context.kind != kind);
        self.general = Deck {
            id: DeckId::new(GENERAL),
            badge: None,
            ..was
        };
        if self.active.as_ref().is_some_and(|a| folded.contains(a)) {
            self.active = None;
        }
        true
    }

    #[must_use]
    pub fn index_of(&self, id: &DeckId) -> Option<usize> {
        self.decks.iter().position(|d| &d.id == id)
    }

    /// The scope tab on show (none while the general tabs show).
    #[must_use]
    pub fn active_deck(&self) -> Option<&Deck> {
        self.active.as_ref().and_then(|id| self.get(id))
    }

    /// The open deck showing `context`, the first if there are more.
    #[must_use]
    pub fn find(&self, context: &DeckContext) -> Option<&Deck> {
        self.decks.iter().find(|d| d.context.same(context))
    }

    /// Every open context, once each, the general tabs' first, then the
    /// scope tabs' in the bar's order: what the app keeps loaded.
    #[must_use]
    pub fn contexts(&self) -> Vec<&DeckContext> {
        let mut out: Vec<&DeckContext> = vec![&self.general.context];
        for d in &self.decks {
            if !out.iter().any(|c| c.same(&d.context)) {
                out.push(&d.context);
            }
        }
        out
    }

    /// The person's default for `kind`, if they picked one.
    #[must_use]
    pub fn default_for(&self, kind: &str) -> Option<&str> {
        self.defaults
            .iter()
            .find(|d| d.kind == kind)
            .map(|d| d.screenset.as_str())
    }

    /// The person picks `screenset` as `kind`'s default (`None` goes back
    /// to the templates').
    pub fn set_default(&mut self, kind: &str, screenset: Option<&str>) {
        self.defaults.retain(|d| d.kind != kind);
        if let Some(s) = screenset {
            self.defaults.push(KindDefault {
                kind: kind.to_owned(),
                screenset: s.to_owned(),
            });
        }
    }

    fn fresh_id(&mut self) -> DeckId {
        loop {
            self.next_id = self.next_id.saturating_add(1);
            let id = DeckId(format!("deck-{}", self.next_id));
            if self.get(&id).is_none() && id.0 != GENERAL {
                return id;
            }
        }
    }

    /// What is kept per device: the scope tabs without their overrides
    /// (those are their contexts'), the general tabs with theirs (they
    /// have no context of their own to keep them), and nothing closed.
    #[must_use]
    pub fn session(&self) -> Self {
        let mut out = self.clone();
        for d in &mut out.decks {
            d.overrides.clear();
            d.badge = None;
        }
        out.general.badge = None;
        out.closed.clear();
        out
    }

    /// As JSON.
    ///
    /// # Errors
    ///
    /// When it cannot be written (it always can).
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// From JSON; an active id that names no scope tab is dropped (the
    /// general tabs show).
    ///
    /// # Errors
    ///
    /// JSON that is not decks.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut out: Self = facet_json::from_str(json).map_err(|e| e.to_string())?;
        if out
            .active
            .as_ref()
            .is_some_and(|a| a.0 == GENERAL || out.get(a).is_none())
        {
            out.active = None;
        }
        out.general.id = DeckId::new(GENERAL);
        for l in &mut out.general.overrides {
            l.workspace.repair();
        }
        for d in &mut out.decks {
            for l in &mut d.overrides {
                l.workspace.repair();
            }
        }
        Ok(out)
    }
}

/// Where a context opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Load {
    /// Onto the scope tab on show (its context replaced); a context open
    /// in another scope tab already brings that one forward instead. With
    /// the general tabs on show, a new scope tab (they are never
    /// replaced).
    #[default]
    Here,
    /// Onto a new scope tab, after the one on show (⌘-click, middle-click).
    NewDeck,
}

/// Why an operation changed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unchanged {
    /// The context is on show in that deck already.
    AlreadyOpen,
    /// The deck is on show already.
    AlreadyActive,
    /// Its place or screenset is the one asked for.
    Same,
    /// There is one deck, or none, to move between.
    OnlyDeck,
    /// The deck shows its screenset as saved: nothing to reset.
    NotModified,
}

impl Unchanged {
    /// As an action's answer says it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyOpen => "already_open",
            Self::AlreadyActive => "already_active",
            Self::Same => "same",
            Self::OnlyDeck => "only_deck",
            Self::NotModified => "not_modified",
        }
    }
}

/// What an operation did: the deck it acted on, or why nothing changed.
pub type Outcome = Result<DeckId, (DeckId, Unchanged)>;

/// A deck operation that names something that is not there.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeckError {
    #[error("no {noun} is called `{0}`", noun = NOUN)]
    NoDeck(String),
    #[error("no screenset is called `{0}`")]
    NoScreenset(String),
    #[error("no {noun} is open", noun = NOUN)]
    NoActive,
    /// The general tabs are not a scope tab: they do not close, move or
    /// duplicate.
    #[error("the {general} are not a {noun}", general = GENERAL_NOUN, noun = NOUN)]
    General,
}

/// What the deck operations change: the decks, the screensets, and the
/// live workspace the active deck draws into, for one class of screen.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckState {
    pub decks: Decks,
    pub screensets: Screensets,
    /// The active deck's arrangement as it is now (the host's).
    pub live: Workspace,
    pub form: FormClass,
    /// The app's and the extensions' defaults per context type.
    pub templates: Vec<DeckTemplate>,
}

impl DeckState {
    #[must_use]
    pub const fn new(decks: Decks, screensets: Screensets, live: Workspace) -> Self {
        Self {
            decks,
            screensets,
            live,
            form: FormClass::Desk,
            templates: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_templates(mut self, templates: Vec<DeckTemplate>) -> Self {
        self.templates = templates;
        self
    }

    #[must_use]
    pub const fn with_form(mut self, form: FormClass) -> Self {
        self.form = form;
        self
    }

    /// The screenset a new deck of `kind` is shown in: the person's
    /// default, else the first template's that exists, else the active
    /// screenset, else the first; `""` when there are none.
    #[must_use]
    pub fn screenset_for(&self, kind: &str) -> String {
        let exists = |id: &str| self.screensets.get(id).is_some();
        self.decks
            .default_for(kind)
            .filter(|s| exists(s))
            .map(str::to_owned)
            .or_else(|| {
                self.templates
                    .iter()
                    .filter(|t| t.kind == kind)
                    .map(|t| t.screenset.clone())
                    .find(|s| exists(s))
            })
            .or_else(|| self.screensets.active.clone().filter(|s| exists(s)))
            .or_else(|| self.screensets.sets.first().map(|s| s.id.clone()))
            .unwrap_or_default()
    }

    /// `screenset`'s layout for this class, as saved.
    fn saved(&self, screenset: &str) -> Option<&Workspace> {
        self.screensets
            .get(screenset)
            .and_then(|s| s.layout_for(self.form))
    }

    /// What deck `id` draws: its override for this class, else its
    /// screenset's layout, else a blank workspace.
    #[must_use]
    pub fn arrangement(&self, id: &DeckId) -> Workspace {
        let Some(deck) = self.decks.get(id) else {
            return Workspace::new();
        };
        deck.override_for(self.form)
            .or_else(|| self.saved(&deck.screenset))
            .cloned()
            .unwrap_or_default()
    }

    /// Keeps the live layout in the deck on show (a scope tab's, else the
    /// general tabs'): as its override, or none
    /// when it is the screenset's as saved. A screenset loaded while the
    /// deck showed (⌘⌥1–9, `WORKSPACES_LOAD`) becomes the deck's, its old
    /// override gone. True when the deck changed.
    pub fn record(&mut self) -> bool {
        let id = self.decks.shown_id();
        let loaded = self
            .screensets
            .active
            .clone()
            .filter(|s| self.screensets.get(s).is_some());
        let Some(deck) = self.decks.get(&id) else {
            return false;
        };
        let mut changed = false;
        let mut screenset = deck.screenset.clone();
        let mut overrides = deck.overrides.clone();
        if let Some(loaded) = loaded.filter(|l| *l != screenset) {
            screenset = loaded;
            overrides.clear();
            changed = true;
        }
        let as_saved = self
            .saved(&screenset)
            .is_some_and(|s| s.same_arrangement(&self.live));
        let mut probe = Deck {
            overrides,
            ..deck.clone()
        };
        changed |= probe.set_override(self.form, (!as_saved).then(|| self.live.clone()));
        if let Some(deck) = self.decks.get_mut(&id) {
            deck.screenset = screenset;
            deck.overrides = probe.overrides;
        }
        changed
    }

    /// Keeps `live` as deck `id`'s layout while it is not the one on show
    /// (a deck in a window of its own): its override, or none when it is
    /// its screenset as saved. True when the deck changed.
    pub fn keep(&mut self, id: &DeckId, live: &Workspace) -> bool {
        let Some(screenset) = self.decks.get(id).map(|d| d.screenset.clone()) else {
            return false;
        };
        let as_saved = self
            .saved(&screenset)
            .is_some_and(|s| s.same_arrangement(live));
        let form = self.form;
        self.decks
            .get_mut(id)
            .is_some_and(|d| d.set_override(form, (!as_saved).then(|| live.clone())))
    }

    /// Shows the active deck: its arrangement live, its screenset the
    /// active one, the screensets' working copy for this class the live
    /// layout (so their dirty mark and Reset speak for the deck).
    pub fn show(&mut self) {
        let id = self.decks.shown_id();
        self.live = self.arrangement(&id);
        let screenset = self
            .decks
            .get(&id)
            .map(|d| d.screenset.clone())
            .filter(|s| self.screensets.get(s).is_some());
        self.screensets.active = screenset;
        self.screensets.set_working(self.form, self.live.clone());
    }

    fn require(&self, id: &DeckId) -> Result<(), DeckError> {
        self.decks
            .get(id)
            .map(|_| ())
            .ok_or_else(|| DeckError::NoDeck(id.0.clone()))
    }

    /// `id` is a scope tab (not the general tabs).
    fn require_scope(&self, id: &DeckId) -> Result<(), DeckError> {
        if id.0 == GENERAL {
            return Err(DeckError::General);
        }
        self.require(id)
    }

    /// The general tabs' type and its default screenset: what an app sets
    /// before the first show (the app's `vault`, shown in "Write"). A
    /// screenset already picked stays.
    pub fn set_general(&mut self, context: DeckContext) {
        if self.decks.general.screenset.is_empty() {
            self.decks.general.screenset = self.screenset_for(&context.kind);
        }
        self.decks.general.context = context;
    }

    /// Shows the general tabs (no scope tab active).
    pub fn show_general(&mut self) -> Outcome {
        let general = DeckId::new(GENERAL);
        if self.decks.general_shown() {
            return Err((general, Unchanged::AlreadyActive));
        }
        self.record();
        self.decks.active = None;
        self.show();
        Ok(general)
    }

    /// Loads `context` ([`Load`]): onto the deck on show, or a new deck.
    /// `kept` is what its context kept ([`ContextLayout`]): its deck opens
    /// in that screenset with that override. Else `screenset`, else its
    /// type's default.
    ///
    /// # Errors
    ///
    /// `screenset` names none.
    pub fn open(
        &mut self,
        context: DeckContext,
        load: Load,
        screenset: Option<&str>,
        kept: Option<ContextLayout>,
    ) -> Result<Outcome, DeckError> {
        if let Some(s) = screenset.filter(|s| self.screensets.get(s).is_none()) {
            return Err(DeckError::NoScreenset(s.to_owned()));
        }
        if self.decks.general.context.same(&context) {
            return Ok(self.show_general());
        }
        self.record();
        if load == Load::Here {
            if let Some(open) = self.decks.find(&context).map(|d| d.id.clone()) {
                if self.decks.active.as_ref() == Some(&open) {
                    return Ok(Err((open, Unchanged::AlreadyOpen)));
                }
                self.decks.active = Some(open.clone());
                self.show();
                return Ok(Ok(open));
            }
        }
        let kept = kept
            .filter(|k| k.context.same(&context))
            .filter(|k| self.screensets.get(&k.screenset).is_some());
        let (set, overrides) = match (screenset, kept) {
            (Some(s), _) => (s.to_owned(), Vec::new()),
            (None, Some(k)) => (k.screenset, k.layouts),
            (None, None) => (self.screenset_for(&context.kind), Vec::new()),
        };
        let here = self
            .decks
            .active
            .clone()
            .filter(|_| load == Load::Here)
            .and_then(|a| self.decks.index_of(&a));
        let id = if let Some(i) = here {
            let deck = self.decks.decks.get_mut(i).ok_or(DeckError::NoActive)?;
            deck.context = context;
            deck.screenset = set;
            deck.overrides = overrides;
            deck.badge = None;
            deck.id.clone()
        } else {
            let id = self.decks.fresh_id();
            let at = self
                .decks
                .active
                .as_ref()
                .and_then(|a| self.decks.index_of(a))
                .map_or(self.decks.decks.len(), |i| i.saturating_add(1));
            self.decks.decks.insert(
                at,
                Deck {
                    id: id.clone(),
                    context,
                    screenset: set,
                    overrides,
                    badge: None,
                },
            );
            id
        };
        self.decks.active = Some(id.clone());
        self.show();
        Ok(Ok(id))
    }

    /// Shows deck `id` ([`GENERAL`]: the general tabs).
    ///
    /// # Errors
    ///
    /// No deck `id`.
    pub fn activate(&mut self, id: &DeckId) -> Result<Outcome, DeckError> {
        self.require(id)?;
        if id.0 == GENERAL {
            return Ok(self.show_general());
        }
        if self.decks.active.as_ref() == Some(id) {
            return Ok(Err((id.clone(), Unchanged::AlreadyActive)));
        }
        self.record();
        self.decks.active = Some(id.clone());
        self.show();
        Ok(Ok(id.clone()))
    }

    /// Shows the deck `by` places along the ring of the general tabs and
    /// then each scope tab (round the ends): ⌃Tab is 1, ⌃⇧Tab -1.
    ///
    /// # Errors
    ///
    /// Never; with no scope tab open it answers `only_deck`.
    pub fn cycle(&mut self, by: isize) -> Result<Outcome, DeckError> {
        let ring: Vec<DeckId> = std::iter::once(DeckId::new(GENERAL))
            .chain(self.decks.decks.iter().map(|d| d.id.clone()))
            .collect();
        let shown = self.decks.shown_id();
        if ring.len() < 2 {
            return Ok(Err((shown, Unchanged::OnlyDeck)));
        }
        let at = ring.iter().position(|d| *d == shown).unwrap_or(0);
        let len = isize::try_from(ring.len()).unwrap_or(isize::MAX);
        let next = isize::try_from(at)
            .unwrap_or(0)
            .saturating_add(by)
            .rem_euclid(len.max(1));
        let id = ring
            .get(usize::try_from(next).unwrap_or(0))
            .cloned()
            .ok_or(DeckError::NoActive)?;
        self.activate(&id)
    }

    /// Shows the deck at `index` (⌘1–9 is 0–8; ⌘9 the last, as a
    /// browser's).
    ///
    /// # Errors
    ///
    /// No deck is open.
    pub fn activate_nth(&mut self, index: usize) -> Result<Outcome, DeckError> {
        let n = self.decks.decks.len();
        let i = if index >= 8 {
            n.saturating_sub(1)
        } else {
            index
        };
        let id = self
            .decks
            .decks
            .get(i)
            .map(|d| d.id.clone())
            .ok_or(DeckError::NoActive)?;
        self.activate(&id)
    }

    /// Closes scope tab `id`: its neighbour on the right (else the left,
    /// else the general tabs) shows if it was on show. It is kept to
    /// reopen.
    ///
    /// # Errors
    ///
    /// No scope tab `id` (the general tabs do not close).
    pub fn close(&mut self, id: &DeckId) -> Result<Outcome, DeckError> {
        self.require_scope(id)?;
        let showing = self.decks.shown_id() == *id;
        if showing {
            self.record();
        }
        let at = self.decks.index_of(id).unwrap_or(0);
        let deck = self.decks.decks.remove(at);
        self.decks.closed.push(deck);
        if self.decks.closed.len() > Decks::CLOSED_KEPT {
            self.decks.closed.remove(0);
        }
        if showing {
            let next = self
                .decks
                .decks
                .get(at)
                .or_else(|| self.decks.decks.get(at.wrapping_sub(1)))
                .map(|d| d.id.clone());
            self.decks.active = next;
            self.show();
        }
        Ok(Ok(id.clone()))
    }

    /// The last closed deck, open again where the strip ends and on show.
    pub fn reopen(&mut self) -> Option<DeckId> {
        let deck = self.decks.closed.pop()?;
        self.record();
        let id = if self.decks.get(&deck.id).is_some() {
            self.decks.fresh_id()
        } else {
            deck.id.clone()
        };
        self.decks.decks.push(Deck {
            id: id.clone(),
            ..deck
        });
        self.decks.active = Some(id.clone());
        self.show();
        Some(id)
    }

    /// Moves deck `id` to `index` in the strip (a drag).
    ///
    /// # Errors
    ///
    /// No deck `id`.
    pub fn move_to(&mut self, id: &DeckId, index: usize) -> Result<Outcome, DeckError> {
        self.require_scope(id)?;
        let from = self.decks.index_of(id).unwrap_or(0);
        let to = index.min(self.decks.decks.len().saturating_sub(1));
        if from == to {
            return Ok(Err((id.clone(), Unchanged::Same)));
        }
        let deck = self.decks.decks.remove(from);
        self.decks.decks.insert(to, deck);
        Ok(Ok(id.clone()))
    }

    /// A second deck on the same context, as `id` is now, after it and on
    /// show.
    ///
    /// # Errors
    ///
    /// No deck `id`.
    pub fn duplicate(&mut self, id: &DeckId) -> Result<Outcome, DeckError> {
        self.require_scope(id)?;
        self.record();
        let at = self.decks.index_of(id).unwrap_or(0);
        let mut copy = self
            .decks
            .decks
            .get(at)
            .cloned()
            .ok_or(DeckError::NoActive)?;
        let new = self.decks.fresh_id();
        copy.id = new.clone();
        copy.badge = None;
        self.decks.decks.insert(at.saturating_add(1), copy);
        self.decks.active = Some(new.clone());
        self.show();
        Ok(Ok(new))
    }

    /// Shows deck `id` in another screenset: its override goes.
    ///
    /// # Errors
    ///
    /// No deck `id`, or no screenset `screenset`.
    pub fn set_screenset(&mut self, id: &DeckId, screenset: &str) -> Result<Outcome, DeckError> {
        self.require(id)?;
        if self.screensets.get(screenset).is_none() {
            return Err(DeckError::NoScreenset(screenset.to_owned()));
        }
        let showing = self.decks.shown_id() == *id;
        if showing {
            self.record();
        }
        let Some(deck) = self.decks.get_mut(id) else {
            return Err(DeckError::NoDeck(id.0.clone()));
        };
        if deck.screenset == screenset && deck.overrides.is_empty() {
            return Ok(Err((id.clone(), Unchanged::Same)));
        }
        screenset.clone_into(&mut deck.screenset);
        deck.overrides.clear();
        if showing {
            self.show();
        }
        Ok(Ok(id.clone()))
    }

    /// Deck `id`'s layout as a new screenset called `name`, which it then
    /// shows in (its override gone: it is the screenset now). The new
    /// set's id.
    ///
    /// # Errors
    ///
    /// No deck `id`.
    pub fn save_as_screenset(&mut self, id: &DeckId, name: &str) -> Result<String, DeckError> {
        self.require(id)?;
        if self.decks.shown_id() == *id {
            self.record();
        }
        let deck = self
            .decks
            .get(id)
            .cloned()
            .ok_or_else(|| DeckError::NoDeck(id.0.clone()))?;
        // Its layout on every class: its own where it changed the set's,
        // the set's elsewhere.
        let mut layouts = self
            .screensets
            .get(&deck.screenset)
            .map(|s| s.layouts.clone())
            .unwrap_or_default();
        for o in &deck.overrides {
            match layouts.iter_mut().find(|l| l.class == o.class) {
                Some(l) => l.workspace = o.workspace.clone(),
                None => layouts.push(o.clone()),
            }
        }
        if !layouts.iter().any(|l| l.class == self.form) {
            layouts.push(Layout {
                class: self.form,
                workspace: self.arrangement(id),
            });
        }
        let working = std::mem::replace(&mut self.screensets.working, layouts);
        let active = self.screensets.active.clone();
        let new = self.screensets.save_as(name, None);
        self.screensets.working = working;
        self.screensets.active = active;
        if let Some(d) = self.decks.get_mut(id) {
            d.screenset.clone_from(&new);
            d.overrides.clear();
        }
        if self.decks.shown_id() == *id {
            self.show();
        }
        Ok(new)
    }

    /// Deck `id` back to its screenset as saved, on this class of screen.
    ///
    /// # Errors
    ///
    /// No deck `id`.
    pub fn reset(&mut self, id: &DeckId) -> Result<Outcome, DeckError> {
        self.require(id)?;
        let showing = self.decks.shown_id() == *id;
        if showing {
            self.record();
        }
        let form = self.form;
        let changed = self
            .decks
            .get_mut(id)
            .is_some_and(|d| d.set_override(form, None));
        if !changed {
            return Ok(Err((id.clone(), Unchanged::NotModified)));
        }
        if showing {
            self.show();
        }
        Ok(Ok(id.clone()))
    }

    /// The app's badge on deck `id` (`None` takes it off).
    pub fn set_badge(&mut self, id: &DeckId, badge: Option<String>) {
        if let Some(d) = self.decks.get_mut(id) {
            d.badge = badge;
        }
    }
}
