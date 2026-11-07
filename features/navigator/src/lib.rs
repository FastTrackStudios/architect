//! The navigator: a port of [Notebook Navigator] (an Obsidian plugin by
//! Johan Sanneblad, GPL-3.0) as architect's sidebar.
//!
//! **Licence.** This crate is GPL-3.0-or-later: it carries Notebook
//! Navigator's design and code. With Navigator, the built-in extension that
//! links it (`architect-navigator-extension`), it is the only GPL code in
//! architect. An app that links it ships under the GPL. Nothing else in
//! architect depends on it.
//!
//! A navigation pane (the folder tree) beside a list pane (the selected
//! folder's notes), side by side, stacked, or one at a time with a slide
//! between them; both virtualized, so a vault of 100,000 notes draws as
//! fast as one of ten. It reads a vault through [`NavigatorSource`]
//! ([`MemorySource`] keeps one in memory; feature `vault`'s
//! `VaultSource` is architect's vault, live); the app opens the notes it is
//! asked to (`ui::NavigatorView`'s `on_open`), in its workspace.
//!
//! The model is plain data, native only ([`NavigatorState`]: the index,
//! the rows, the selection and history, the keys, the scroll). Feature
//! `ui` is the Dioxus view on architect's shell: its commands on the
//! palette (`NAVIGATOR_*`), its section on the settings page
//! ([`NavigatorSettings`]), its colours as theme tokens (`navigator.*`).
//!
//! Under the folders, a calendar of daily notes ([`calendar`]), and the
//! periodic notes, daily to yearly, each a Moment pattern under one folder
//! ([`PeriodicNotes`], [`pattern`]). A [`Period`] is a day, a week, a
//! month, a quarter or a year; a search's [`DateFilter`] is made from the
//! same days.
//!
//! [Notebook Navigator]: https://github.com/johansan/notebook-navigator

pub mod appearance;
pub mod calendar;
pub mod hidden;
pub mod icons;
pub mod index;
pub mod keys;
pub mod layout;
pub mod list;
pub mod mounts;
pub mod pattern;
pub mod period;
pub mod profiles;
pub mod properties;
pub mod provider_source;
pub mod query;
pub mod rainbow;
pub mod search;
pub mod selection;
pub mod settings;
pub mod source;
pub mod state;
pub mod tags;
pub mod template;
pub mod tree;
pub mod virtualize;

#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "vault")]
pub mod vault_source;

pub use appearance::Appearance;
pub use calendar::{CalendarDay, CalendarGrid, CalendarWeek, HeaderPeriod, PeriodicNotes};
pub use hidden::{FileVisibility, Hidden, HiddenRules};
pub use icons::Icon;
pub use index::{NoteCount, VaultIndex};
pub use keys::{Hotkey, KeyAction, KeyBinding, KeyEvent, KeyMap, KeyScope};
pub use layout::{Pane, PaneLayout};
pub use list::{Clock, Lines, ListContext, ListRow, ListRowKind, PinIn, Thumb};
pub use mounts::{Mount, MountKind, Mounts, OrgBadge, Place};
pub use pattern::Pattern;
pub use period::{DateField, DateFilter, DayMonthOrder, Period, PeriodKind, WeekRules};
pub use profiles::{Profiles, Shortcut, ShortcutKind, VaultProfile};
pub use properties::PropertyTree;
pub use provider_source::{ProviderRequest, ProviderSource};
pub use query::{Join, Operand, Query};
pub use search::{FilterSearch, SearchContext, SearchMatch};
pub use selection::{NavTarget, Selection};
pub use settings::{
    CalendarPlacement, CountPlacement, FolderNoteOpen, Grouping, ListMode, MonthFormat,
    NarrowLayout, NavigatorSettings, Orientation, SearchProvider, SortOption, StartPane, TextCount,
    TreeSort, WeekStart, WeekendDays,
};
pub use source::{
    FileOp, FolderMeta, Listener, MemorySource, Millis, NavigatorSource, NoteFile, PropertyValue,
    SourceEvent, Subscription, TaskCounts,
};
pub use state::{Colour, HideTarget, Key, Mods, NavigatorState, OpenIn, Outcome, Pin, ViewState};
pub use tags::TagTree;
pub use tree::{NavRow, NavRowKind};
#[cfg(feature = "vault")]
pub use vault_source::VaultSource;
