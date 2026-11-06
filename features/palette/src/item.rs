//! What a row is: an [`Item`], from whichever [`Source`](crate::Source)
//! gave it — a command, a note to open, an app (the launcher), an
//! extension's contributed row.

/// What running a command does to the app.
///
/// The Action List's three (`architect::action::Effect`), kept here so
/// the engine needs no `architect`. A caller from outside runs a command through `COMMANDS_RUN`
/// with this effect, not a blanket one: a destructive command waits for a
/// person's yes when the list asks for confirmations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Effect {
    /// Reads only, or changes only what is on show (a menu, an overlay).
    ReadOnly,
    /// Changes the app, undoably (a closed tab reopens).
    #[default]
    Mutating,
    /// Changes the app in a way an undo may not bring back.
    Destructive,
}

impl Effect {
    /// Its name, as the Action List writes it (`read-only`, `mutating`,
    /// `destructive`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Mutating => "mutating",
            Self::Destructive => "destructive",
        }
    }
}

/// One row, from any source: a command (from the Action List, or a scoped
/// action a view registered while it is mounted), something to open, an
/// app.
///
/// `architect-commands` calls it `Command`, the palette's name for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The source it came from (`commands`, `note`, `theme`); empty for a
    /// command, which is the palette's own.
    pub source: String,
    /// What its source runs or opens it by: a command's action id
    /// (`WORKSPACE_CLOSE_TAB`), a note's path.
    pub id: String,
    pub label: String,
    /// Where it belongs (`Workspace`, `Tabs`), shown beside it.
    pub group: String,
    /// What it says more, searched too.
    pub detail: String,
    /// How a person reaches it (`⌘W`), for display.
    pub shortcut: String,
    pub enabled: bool,
    /// Why not, when it cannot run now.
    pub disabled_reason: Option<String>,
    /// A toggle's state, the tick a menu shows; `None` when it is not one.
    pub checked: Option<bool>,
    /// What running it does ([`Effect::Mutating`] unless it says).
    pub effect: Effect,
    /// Higher is more recent (a timestamp, or an order); 0 when unknown.
    /// What an opened item is ordered by after its match.
    pub recency: u64,
}

impl Item {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            source: String::new(),
            id: id.into(),
            label: label.into(),
            group: String::new(),
            detail: String::new(),
            shortcut: String::new(),
            enabled: true,
            disabled_reason: None,
            checked: None,
            effect: Effect::Mutating,
            recency: 0,
        }
    }

    /// The source it came from.
    #[must_use]
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// How recent it is (higher is more recent).
    #[must_use]
    pub const fn recency(mut self, recency: u64) -> Self {
        self.recency = recency;
        self
    }

    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    #[must_use]
    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = shortcut.into();
        self
    }

    /// A toggle, on or off: a menu ticks it when on.
    #[must_use]
    pub const fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    /// What running it does to the app.
    #[must_use]
    pub const fn effect(mut self, effect: Effect) -> Self {
        self.effect = effect;
        self
    }

    /// It cannot be taken back (deletes, closes what does not reopen):
    /// [`Effect::Destructive`].
    #[must_use]
    pub const fn destructive(self) -> Self {
        self.effect(Effect::Destructive)
    }

    /// It only reads, or changes only what is on show: [`Effect::ReadOnly`].
    #[must_use]
    pub const fn read_only(self) -> Self {
        self.effect(Effect::ReadOnly)
    }

    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.enabled = false;
        self.disabled_reason = Some(reason.into());
        self
    }
}
