//! The shell's regions as slots (`docs/content/architecture/interface-files.md`):
//! the desk's ribbon, the left and right sidebars' content, the title
//! row's tab, the status bar and the title row. Each is filled by the
//! shell's own component unless a [`Chrome`] above [`AppShell`](super::AppShell)
//! says otherwise: a [`Filler`] (an extension's view, a layout file's
//! assignment), hidden, or the shell's own drawn in a look of its own
//! (token overrides for that region, the cascade's nearest node).
//!
//! [`Chrome`] also carries what others add to the shell beside the app's
//! own: tabs, status items, a phone's action-bar buttons. Nothing in the
//! default interface is special — the shell's components are the
//! fillers a slot has when nobody else fills it.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use dioxus::prelude::*;

use super::{BarAction, IconFn, StatusItem, Tab};

/// A region of the shell that a view can fill.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    /// A desk's ribbon at the window's left edge.
    Ribbon,
    /// The left sidebar's content.
    Left,
    /// The right side's content.
    Right,
    /// The page's tab in a desk's title row.
    Tabs,
    /// A desk's status bar.
    Status,
    /// A desk's title row.
    Title,
}

impl Slot {
    pub const ALL: [Self; 6] = [
        Self::Ribbon,
        Self::Left,
        Self::Right,
        Self::Tabs,
        Self::Status,
        Self::Title,
    ];

    /// Its name in a layout file and a manifest (`status`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ribbon => "ribbon",
            Self::Left => "left",
            Self::Right => "right",
            Self::Tabs => "tabs",
            Self::Status => "status",
            Self::Title => "title",
        }
    }

    /// The slot named `name`.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// What fills a slot instead of the shell's own component: who put it
/// there, and what draws it.
#[derive(Clone)]
pub struct Filler {
    /// Who fills it (`ext:lab.board`, `layout:main`), for a reader.
    pub owner: String,
    /// What it shows (`lab.board/status-bar`), for a reader.
    pub view: String,
    pub render: Rc<dyn Fn() -> Element>,
}

impl Filler {
    pub fn new(
        owner: impl Into<String>,
        view: impl Into<String>,
        render: impl Fn() -> Element + 'static,
    ) -> Self {
        Self {
            owner: owner.into(),
            view: view.into(),
            render: Rc::new(render),
        }
    }
}

impl PartialEq for Filler {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.view == other.view
            && Rc::ptr_eq(&self.render, &other.render)
    }
}

impl std::fmt::Debug for Filler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filler")
            .field("owner", &self.owner)
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

/// What others add to the shell, and how its slots are filled.
#[derive(Clone, Default, PartialEq)]
pub struct Chrome {
    /// Tabs after the app's.
    pub tabs: Vec<Tab>,
    /// Status items after the app's.
    pub status: Vec<StatusItem>,
    /// Action-bar buttons after the app's.
    pub bar: Vec<BarAction>,
    /// The slots filled by someone else.
    pub fillers: BTreeMap<Slot, Filler>,
    /// The slots shown empty.
    pub hidden: BTreeSet<Slot>,
    /// The slots drawn by the shell's own component, whatever fills them
    /// under this chrome ([`Self::over`]): a layout saying `shell/status`.
    pub own: BTreeSet<Slot>,
    /// A slot's own look: token overrides over the workspace's (each a
    /// token's name and value, a literal or an expression).
    pub looks: BTreeMap<Slot, BTreeMap<String, String>>,
}

impl std::fmt::Debug for Chrome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chrome")
            .field("tabs", &self.tabs.iter().map(|t| t.id).collect::<Vec<_>>())
            .field(
                "status",
                &self.status.iter().map(|s| &s.id).collect::<Vec<_>>(),
            )
            .field("bar", &self.bar.len())
            .field("fillers", &self.fillers)
            .field("hidden", &self.hidden)
            .field("own", &self.own)
            .field("looks", &self.looks)
            .finish()
    }
}

impl Chrome {
    /// This chrome over `under`: its fillers, hidden slots and looks win;
    /// the tabs, status items and bar buttons of both, `under`'s first.
    #[must_use]
    pub fn over(&self, under: &Self) -> Self {
        let mut out = under.clone();
        out.tabs.extend(self.tabs.iter().cloned());
        out.status.extend(self.status.iter().cloned());
        out.bar.extend(self.bar.iter().cloned());
        for slot in &self.own {
            out.fillers.remove(slot);
        }
        out.own.extend(self.own.iter().copied());
        out.fillers
            .extend(self.fillers.iter().map(|(k, v)| (*k, v.clone())));
        out.hidden.extend(self.hidden.iter().copied());
        out.looks
            .extend(self.looks.iter().map(|(k, v)| (*k, v.clone())));
        out
    }

    /// Who fills `slot`, when not the shell.
    #[must_use]
    pub fn filler(&self, slot: Slot) -> Option<&Filler> {
        self.fillers.get(&slot)
    }

    #[must_use]
    pub fn is_hidden(&self, slot: Slot) -> bool {
        self.hidden.contains(&slot)
    }
}

/// The chrome the shell below reads: the one an ancestor provided, else a
/// new one provided here. Call it above [`AppShell`](super::AppShell)
/// (the app's root) to fill its slots and add to it.
#[must_use]
pub fn use_chrome() -> Signal<Chrome> {
    try_use_context::<Signal<Chrome>>()
        .unwrap_or_else(|| use_context_provider(|| Signal::new(Chrome::default())))
}

/// `id` kept for the app's life: a contributed tab's id, so it can be a
/// [`Tab::id`] like the app's own. The same text is kept once.
#[must_use]
pub fn intern(id: &str) -> &'static str {
    static KEPT: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(k) = kept.get(id) {
        return k;
    }
    let k: &'static str = Box::leak(id.to_owned().into_boxed_str());
    kept.insert(k);
    k
}

/// A lucide icon by its name (`layout-dashboard`), for what is named in
/// data (a manifest, a layout file); a puzzle piece for a name it does
/// not know.
#[must_use]
// `icon!` casts its closure to the fn pointer, as an app's does.
#[allow(clippy::as_conversions)]
pub fn named_icon(name: &str) -> IconFn {
    use lucide_dioxus::{
        AudioLines, Bell, Bookmark, Bug, Calendar, Check, Clock, Cloud, Code, File, FileText,
        Folder, GitPullRequest, Globe, Heart, House, Inbox, Info, LayoutDashboard, Link, List,
        Lock, Mail, MessageSquare, Mic, Monitor, Moon, Package, Pin, Play, Puzzle, Search,
        Settings, SlidersHorizontal, Star, Sun, Tag, Terminal, Timer, User, Users, Zap,
    };
    match name {
        "bell" => crate::icon!(Bell),
        "bookmark" => crate::icon!(Bookmark),
        "bug" => crate::icon!(Bug),
        "calendar" => crate::icon!(Calendar),
        "check" => crate::icon!(Check),
        "clock" => crate::icon!(Clock),
        "cloud" => crate::icon!(Cloud),
        "code" => crate::icon!(Code),
        "file" => crate::icon!(File),
        "file-text" => crate::icon!(FileText),
        "folder" => crate::icon!(Folder),
        "git-pull-request" => crate::icon!(GitPullRequest),
        "globe" => crate::icon!(Globe),
        "heart" => crate::icon!(Heart),
        "home" | "house" => crate::icon!(House),
        "inbox" => crate::icon!(Inbox),
        "info" => crate::icon!(Info),
        "layout-dashboard" => crate::icon!(LayoutDashboard),
        "link" => crate::icon!(Link),
        "list" => crate::icon!(List),
        "lock" => crate::icon!(Lock),
        "mail" => crate::icon!(Mail),
        "message-square" => crate::icon!(MessageSquare),
        "mic" => crate::icon!(Mic),
        "monitor" => crate::icon!(Monitor),
        "moon" => crate::icon!(Moon),
        "music" => crate::icon!(AudioLines),
        "package" => crate::icon!(Package),
        "pin" => crate::icon!(Pin),
        "play" => crate::icon!(Play),
        "search" => crate::icon!(Search),
        "settings" => crate::icon!(Settings),
        "sliders-horizontal" => crate::icon!(SlidersHorizontal),
        "star" => crate::icon!(Star),
        "sun" => crate::icon!(Sun),
        "tag" => crate::icon!(Tag),
        "terminal" => crate::icon!(Terminal),
        "timer" => crate::icon!(Timer),
        "user" => crate::icon!(User),
        "users" => crate::icon!(Users),
        "zap" => crate::icon!(Zap),
        _ => crate::icon!(Puzzle),
    }
}

/// The theme store's look for one slot: the workspace layer with the
/// slot's overrides on top.
pub(super) fn slot_theme(
    store: Option<&architect_theme::ThemeStore>,
    look: &BTreeMap<String, String>,
) -> Option<super::ShellTheme> {
    let store = store?;
    let mut layer = store.layer(architect_theme::LayerKind::Workspace);
    for (k, v) in look {
        layer = layer.token(k.clone(), v.clone());
    }
    Some(super::ShellTheme::from_resolved(
        &store.resolve_with(architect_theme::LayerKind::Workspace, layer),
    ))
}
