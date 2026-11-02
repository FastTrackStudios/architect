//! The ⌘K palette's model: what a row is, what a typed query asks for,
//! and the order the rows come in.

use crate::fuzzy::{Match, fuzzy};
use crate::recent::Recents;

/// What running a command does to the app.
///
/// The Action List's three (`architect::action::Effect`), kept here so
/// the model needs no `architect`. A caller from outside runs a command through `COMMANDS_RUN`
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

/// One command as the palette shows it: from the Action List, or a
/// scoped action a view registered while it is mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The action id it runs (`WORKSPACE_CLOSE_TAB`).
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
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            group: String::new(),
            detail: String::new(),
            shortcut: String::new(),
            enabled: true,
            disabled_reason: None,
            checked: None,
            effect: Effect::Mutating,
        }
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

/// What a query asks for, by its first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `>` or nothing: commands.
    Commands,
    /// `@`: this page's targets (a track, a section).
    Targets,
    /// `#`: settings.
    Settings,
    /// `?`: what a key does.
    Help,
}

/// A typed query: its scope and the text to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub scope: Scope,
    pub text: String,
}

impl Query {
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim_start();
        let (scope, rest) = match trimmed.chars().next() {
            Some('>') => (Scope::Commands, trimmed.get(1..).unwrap_or("")),
            Some('@') => (Scope::Targets, trimmed.get(1..).unwrap_or("")),
            Some('#') => (Scope::Settings, trimmed.get(1..).unwrap_or("")),
            Some('?') => (Scope::Help, trimmed.get(1..).unwrap_or("")),
            _ => (Scope::Commands, trimmed),
        };
        Self {
            scope,
            text: rest.trim().to_owned(),
        }
    }
}

/// A row in order, with what lit up in its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub command: Command,
    /// Matched characters of the label (empty when it matched elsewhere,
    /// or the query is empty).
    pub positions: Vec<usize>,
    pub score: i64,
}

/// Bonus for having been run recently, most recent the most.
const RECENT: i64 = 40;

/// The commands matching `text`, best first.
///
/// Enabled before disabled, then by match (the label counts most, the
/// group and the id less), then by how recently each ran. An empty `text`
/// lists the recent ones first, then the rest by group and label.
#[must_use]
pub fn rank(commands: &[Command], text: &str, recents: &Recents) -> Vec<Ranked> {
    let mut rows: Vec<Ranked> = commands
        .iter()
        .filter_map(|c| {
            let label = fuzzy(text, &c.label);
            let other = || {
                fuzzy(text, &format!("{} {}", c.group, c.label))
                    .or_else(|| fuzzy(text, &c.detail))
                    .or_else(|| fuzzy(text, &c.id))
                    .map(|m| Match {
                        score: m.score / 2,
                        positions: Vec::new(),
                    })
            };
            let m = label.or_else(other)?;
            let recent = recents.rank(&c.id).map_or(0, |r| {
                RECENT.saturating_sub(i64::try_from(r).unwrap_or(RECENT).saturating_mul(4))
            });
            Some(Ranked {
                command: c.clone(),
                positions: m.positions,
                score: m.score.saturating_add(recent.max(0)),
            })
        })
        .collect();
    if text.trim().is_empty() {
        rows.sort_by(|a, b| {
            let ra = recents.rank(&a.command.id);
            let rb = recents.rank(&b.command.id);
            b.command
                .enabled
                .cmp(&a.command.enabled)
                .then(match (ra, rb) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.command.group.cmp(&b.command.group))
                .then_with(|| a.command.label.cmp(&b.command.label))
        });
    } else {
        rows.sort_by(|a, b| {
            b.command
                .enabled
                .cmp(&a.command.enabled)
                .then(b.score.cmp(&a.score))
                .then_with(|| a.command.label.cmp(&b.command.label))
        });
    }
    rows
}
