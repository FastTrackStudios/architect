//! The navigator as one value, and what each input does to it.
//!
//! The vault, the settings, what is open and selected, both panes' rows
//! and scroll. The view (`ui`) draws it and feeds it clicks, keys and wheels; the tests
//! drive it the same way without a window.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use facet::Facet;

use crate::index::VaultIndex;
use crate::layout::{NAV_HEIGHT, NAV_WIDTH, Pane};
use crate::list::{self, Clock, ListMetrics, ListRow, ListRowKind};
use crate::selection::{NavTarget, Selection};
use crate::settings::NavigatorSettings;
use crate::source::{NoteFile, SourceEvent, ancestors, is_within, parent};
use crate::tree::{self, NavRow, NavRowKind, TreeOptions};
use crate::virtualize::{Align, OVERSCAN, Offsets};

/// A key the navigator answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Tab,
    Escape,
    /// `A`, for select all with ⌘/Ctrl.
    A,
}

/// The modifiers held with a key or a click. `command` is ⌘ on a Mac and
/// Ctrl elsewhere (Notebook Navigator's `Mod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub command: bool,
}

impl Mods {
    pub const NONE: Self = Self {
        shift: false,
        command: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        command: false,
    };
    pub const COMMAND: Self = Self {
        shift: false,
        command: true,
    };
}

/// Where a note opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenIn {
    /// In the pane on show (a click, Return).
    #[default]
    Here,
    /// A new tab (⇧Return).
    Tab,
    /// Beside it (⌘Return).
    Split,
}

/// What the app is asked to do after an input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Open a note in the workspace. `settle` when the keyboard moved to
    /// it: wait for the keys to settle first (Notebook Navigator's 500 ms
    /// `DEBOUNCE_KEYBOARD_FILE_OPEN`).
    Open {
        path: String,
        how: OpenIn,
        settle: bool,
    },
    /// Give the keyboard to the note on show (→ from the list).
    FocusEditor,
}

/// A pane's scroll: how far, how tall the view.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scroll {
    pub offset: f64,
    pub view: f64,
}

/// A scroll asked for before the pane could do it (no size yet). Ranked:
/// a later ask replaces it only if it ranks at least as high
/// (`docs/scroll-orchestration.md`: top < structure < visibility < folder
/// navigation < reveal).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    key: String,
    align: Align,
    rank: u8,
}

const RANK_TOP: u8 = 0;
const RANK_STRUCTURE: u8 = 1;
const RANK_NAVIGATION: u8 = 3;
const RANK_REVEAL: u8 = 4;

/// What the navigator keeps between runs: the open folders, the manual
/// order, the selected folder, the panes' sizes (Notebook Navigator keeps
/// them in local storage).
#[derive(Debug, Clone, PartialEq, Facet, Default)]
pub struct ViewState {
    pub expanded: Vec<String>,
    /// Each parent's children by name, as a person ordered them.
    pub child_order: Vec<ChildOrder>,
    pub folder: Option<String>,
    pub nav_width: f64,
    pub nav_height: f64,
}

/// One parent's manual order.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ChildOrder {
    pub parent: String,
    pub names: Vec<String>,
}

/// The navigator.
#[derive(Debug, Clone)]
pub struct NavigatorState {
    index: VaultIndex,
    settings: NavigatorSettings,
    expanded: BTreeSet<String>,
    child_order: BTreeMap<String, Vec<String>>,
    selection: Selection,
    /// The pane the keyboard is in.
    pub focus: Pane,
    /// The pane on show when one pane shows at a time.
    pub single: Pane,
    /// The note open in the workspace.
    active: Option<String>,
    clock: Clock,
    metrics: ListMetrics,
    /// Whether a phone's (taller rows).
    touch: bool,
    /// The folders' pane's size, side by side and stacked.
    pub nav_width: f64,
    pub nav_height: f64,
    nav_rows: Vec<NavRow>,
    nav_offsets: Offsets,
    list_rows: Vec<ListRow>,
    list_offsets: Offsets,
    nav_scroll: Scroll,
    list_scroll: Scroll,
    nav_pending: Option<Pending>,
    list_pending: Option<Pending>,
    /// Whether the root reorder handles show.
    pub reordering: bool,
}

impl NavigatorState {
    /// The navigator over `index`, the root selected and open.
    #[must_use]
    pub fn new(index: VaultIndex, settings: NavigatorSettings, clock: Clock) -> Self {
        let single = match settings.start_pane {
            crate::settings::StartPane::Navigation => Pane::Navigation,
            crate::settings::StartPane::Files => Pane::Files,
        };
        let mut s = Self {
            index,
            settings,
            expanded: std::iter::once(String::new()).collect(),
            child_order: BTreeMap::new(),
            selection: Selection::default(),
            focus: Pane::Navigation,
            single,
            active: None,
            clock,
            metrics: ListMetrics::DESK,
            touch: false,
            nav_width: NAV_WIDTH,
            nav_height: NAV_HEIGHT,
            nav_rows: Vec::new(),
            nav_offsets: Offsets::default(),
            list_rows: Vec::new(),
            list_offsets: Offsets::default(),
            nav_scroll: Scroll::default(),
            list_scroll: Scroll::default(),
            nav_pending: None,
            list_pending: None,
            reordering: false,
        };
        s.selection
            .select_target(NavTarget::Folder(String::new()), None);
        s.rebuild();
        s
    }

    // ── Reading ─────────────────────────────────────────────────────────

    #[must_use]
    pub const fn index(&self) -> &VaultIndex {
        &self.index
    }

    #[must_use]
    pub const fn settings(&self) -> &NavigatorSettings {
        &self.settings
    }

    #[must_use]
    pub const fn selection(&self) -> &Selection {
        &self.selection
    }

    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    #[must_use]
    pub const fn clock(&self) -> Clock {
        self.clock
    }

    #[must_use]
    pub const fn expanded(&self) -> &BTreeSet<String> {
        &self.expanded
    }

    #[must_use]
    pub fn nav_rows(&self) -> &[NavRow] {
        &self.nav_rows
    }

    #[must_use]
    pub fn list_rows(&self) -> &[ListRow] {
        &self.list_rows
    }

    #[must_use]
    pub const fn nav_offsets(&self) -> &Offsets {
        &self.nav_offsets
    }

    #[must_use]
    pub const fn list_offsets(&self) -> &Offsets {
        &self.list_offsets
    }

    #[must_use]
    pub const fn scroll(&self, pane: Pane) -> Scroll {
        match pane {
            Pane::Navigation => self.nav_scroll,
            Pane::Files => self.list_scroll,
        }
    }

    /// The rows of `pane` to draw now.
    #[must_use]
    pub fn window(&self, pane: Pane) -> Range<usize> {
        let (offsets, s) = self.pane(pane);
        // Before the view is measured, a screenful's worth.
        let view = if s.view > 0.0 { s.view } else { 800.0 };
        offsets.window(s.offset, view, OVERSCAN)
    }

    const fn pane(&self, pane: Pane) -> (&Offsets, Scroll) {
        match pane {
            Pane::Navigation => (&self.nav_offsets, self.nav_scroll),
            Pane::Files => (&self.list_offsets, self.list_scroll),
        }
    }

    /// The list's notes, in order.
    #[must_use]
    pub fn list_order(&self) -> Vec<&str> {
        self.list_rows.iter().filter_map(ListRow::file).collect()
    }

    /// The note at `path`.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&NoteFile> {
        self.index.file(path)
    }

    /// The tree's options from the settings and the manual order.
    #[must_use]
    pub fn tree_options(&self) -> TreeOptions {
        TreeOptions {
            show_root: self.settings.show_root,
            child_order: self.child_order.clone(),
        }
    }

    /// Whether anything is expanded but the root (what the collapse/expand
    /// all button shows).
    #[must_use]
    pub fn any_expanded(&self) -> bool {
        self.expanded.iter().any(|p| !p.is_empty())
    }

    // ── Changing ───────────────────────────────────────────────────────

    /// Draws again from the vault and the settings.
    fn rebuild(&mut self) {
        self.rebuild_nav();
        self.rebuild_list();
    }

    fn rebuild_nav(&mut self) {
        let mut expanded = self.expanded.clone();
        if !self.settings.show_root {
            // A hidden root is always open.
            expanded.insert(String::new());
        }
        self.nav_rows = tree::flatten(&self.index, &expanded, &self.tree_options());
        let h = self.nav_item_height();
        self.nav_offsets = Offsets::new(self.nav_rows.iter().map(|r| match r.kind {
            NavRowKind::TopSpacer => tree::TOP_SPACER,
            NavRowKind::BottomSpacer => tree::BOTTOM_SPACER,
            _ => h,
        }));
        self.clamp_scroll(Pane::Navigation);
        self.settle(Pane::Navigation);
    }

    fn rebuild_list(&mut self) {
        let folder = self.selection.folder().unwrap_or("").to_owned();
        self.list_rows = list::rows(&self.index, &folder, &self.settings, self.clock);
        let (settings, metrics) = (&self.settings, self.metrics);
        self.list_offsets = Offsets::new(
            self.list_rows
                .iter()
                .map(|r| list::height(r, settings, metrics)),
        );
        self.clamp_scroll(Pane::Files);
        self.settle(Pane::Files);
    }

    /// A folder row's height: the setting, 12 more on a phone.
    #[must_use]
    pub fn nav_item_height(&self) -> f64 {
        let h = f64::from(self.settings.item_height.clamp(20, 40));
        if self.touch { h + 12.0 } else { h }
    }

    /// New settings.
    pub fn set_settings(&mut self, settings: NavigatorSettings) {
        if settings != self.settings {
            self.settings = settings;
            self.rebuild();
        }
    }

    /// A phone's measures (`true`) or a desk's.
    pub fn set_touch(&mut self, touch: bool) {
        if touch != self.touch {
            self.touch = touch;
            self.metrics = if touch {
                ListMetrics::PHONE
            } else {
                ListMetrics::DESK
            };
            self.rebuild();
        }
    }

    /// The time now, for the list's date groups.
    pub fn set_clock(&mut self, clock: Clock) {
        let day_moved = self.clock.day(self.clock.now) != clock.day(clock.now)
            || self.clock.offset != clock.offset;
        self.clock = clock;
        if day_moved {
            self.rebuild_list();
        }
    }

    /// A change in the source.
    pub fn apply(&mut self, event: &SourceEvent) {
        let moved = self.index.apply(event);
        let index = &self.index;
        self.selection
            .follow(&moved, |p| index.file(p).is_some(), |f| index.has_folder(f));
        let renamed: BTreeSet<String> = self
            .expanded
            .iter()
            .map(|p| {
                moved
                    .iter()
                    .find(|(from, _)| from == p)
                    .map_or_else(|| p.clone(), |(_, to)| to.clone())
            })
            .filter(|p| index.has_folder(p))
            .collect();
        self.expanded = renamed;
        if let Some(a) = &self.active
            && let Some((_, to)) = moved.iter().find(|(from, _)| from == a)
        {
            self.active = Some(to.clone());
        }
        self.rebuild();
    }

    /// Everything read again from a fresh index (a [`SourceEvent::Reset`]).
    pub fn replace_index(&mut self, index: VaultIndex) {
        self.index = index;
        self.apply(&SourceEvent::Reset);
    }

    /// Selects `folder` in the navigation pane, recorded in the history.
    pub fn select_folder(&mut self, folder: &str) {
        if !self.index.has_folder(folder) {
            return;
        }
        // The note open in the workspace stays selected when it is in the
        // new folder's list (Notebook Navigator's auto-selected file).
        let keep = self.active.clone().filter(|a| {
            let p = parent(a);
            p == folder || (self.settings.include_descendants && is_within(p, folder))
        });
        self.selection
            .select_target(NavTarget::Folder(folder.to_owned()), keep.clone());
        self.rebuild_list();
        match keep {
            Some(file) => self.ask_scroll(Pane::Files, file, Align::Auto, RANK_NAVIGATION),
            None => self.ask_scroll_top(),
        }
        let key = folder_key(folder);
        self.ask_scroll(Pane::Navigation, key, Align::Auto, RANK_NAVIGATION);
    }

    /// Opens or closes `folder`.
    pub fn toggle(&mut self, folder: &str) {
        if !self.expanded.remove(folder) {
            self.expanded.insert(folder.to_owned());
        }
        self.rebuild_nav();
    }

    pub fn set_expanded(&mut self, folder: &str, open: bool) {
        let changed = if open {
            self.expanded.insert(folder.to_owned())
        } else {
            self.expanded.remove(folder)
        };
        if changed {
            self.rebuild_nav();
        }
    }

    /// Collapse all, or expand all when nothing is expanded (Notebook
    /// Navigator's one button). With smart collapse, collapsing keeps the
    /// selected folder's chain open, and pressing again from that state
    /// expands all.
    pub fn collapse_or_expand_all(&mut self) {
        let keep: BTreeSet<String> = if self.settings.smart_collapse {
            self.selection
                .folder()
                .map(|f| {
                    ancestors(f)
                        .map(str::to_owned)
                        .chain(std::iter::once(String::new()))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            std::iter::once(String::new()).collect()
        };
        let collapsed = self.expanded == keep || !self.any_expanded();
        if collapsed {
            self.expanded = self.index.expandable().into_iter().collect();
            self.expanded.insert(String::new());
        } else {
            self.expanded = keep;
        }
        self.rebuild_nav();
        if let Some(f) = self.selection.folder().map(folder_key) {
            self.ask_scroll(Pane::Navigation, f, Align::Auto, RANK_STRUCTURE);
        }
    }

    /// Collapses every folder (the selected one's chain kept open with
    /// smart collapse).
    pub fn collapse_all(&mut self) {
        let mut keep: BTreeSet<String> = std::iter::once(String::new()).collect();
        if self.settings.smart_collapse
            && let Some(f) = self.selection.folder()
        {
            keep.extend(ancestors(f).map(str::to_owned));
        }
        self.expanded = keep;
        self.rebuild_nav();
    }

    pub fn expand_all(&mut self) {
        self.expanded = self.index.expandable().into_iter().collect();
        self.expanded.insert(String::new());
        self.rebuild_nav();
    }

    /// The note open in the workspace changed: lit, and revealed when the
    /// settings say so.
    pub fn set_active(&mut self, path: Option<&str>) {
        if self.active.as_deref() == path {
            return;
        }
        self.active = path.map(str::to_owned);
        if let Some(p) = path
            && self.settings.auto_reveal
            && self.index.file(p).is_some()
        {
            self.reveal(p, false);
        }
    }

    /// Shows `path`: selects it, in its folder (the folders above it
    /// opened), and scrolls both panes to it. An automatic reveal (`manual`
    /// false) with notes from folders below and the shortest path keeps a
    /// folder that already lists the note, or stops at the nearest open
    /// one, without opening any (`src/utils/autoRevealUtils.ts`).
    pub fn reveal(&mut self, path: &str, manual: bool) {
        let Some(file) = self.index.file(path) else {
            return;
        };
        let home = file.folder().to_owned();
        let folder =
            if !manual && self.settings.include_descendants && self.settings.reveal_shortest_path {
                let current = self.selection.folder().unwrap_or("");
                if is_within(&home, current) {
                    current.to_owned()
                } else {
                    // The highest folder of its chain whose parents are all
                    // open: the nearest one in view.
                    let chain: Vec<&str> = std::iter::once(home.as_str())
                        .chain(ancestors(&home))
                        .collect();
                    chain
                        .iter()
                        .rev()
                        .find(|f| {
                            ancestors(f).all(|a| {
                                self.expanded.contains(a)
                                    || (a.is_empty() && !self.settings.show_root)
                            }) && (!f.is_empty() || self.settings.show_root)
                        })
                        .map_or_else(|| home.clone(), |f| (*f).to_owned())
                }
            } else {
                for a in ancestors(&home) {
                    self.expanded.insert(a.to_owned());
                }
                home
            };
        if self.selection.folder() == Some(folder.as_str()) {
            self.selection.set_file(Some(path.to_owned()));
        } else {
            self.selection
                .select_target(NavTarget::Folder(folder.clone()), Some(path.to_owned()));
        }
        self.rebuild();
        self.ask_scroll(
            Pane::Navigation,
            folder_key(&folder),
            Align::Auto,
            RANK_REVEAL,
        );
        self.ask_scroll(Pane::Files, path.to_owned(), Align::Auto, RANK_REVEAL);
    }

    /// Reveals the note open in the workspace (the reveal command).
    pub fn reveal_active(&mut self) {
        if let Some(a) = self.active.clone() {
            self.reveal(&a, true);
        }
    }

    /// Back in the history of folders.
    pub fn back(&mut self) {
        if self.selection.back().is_some() {
            self.after_history();
        }
    }

    pub fn forward(&mut self) {
        if self.selection.forward().is_some() {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        if let Some(f) = self.selection.folder().map(str::to_owned) {
            for a in ancestors(&f) {
                self.expanded.insert(a.to_owned());
            }
            self.rebuild();
            self.ask_scroll(
                Pane::Navigation,
                folder_key(&f),
                Align::Auto,
                RANK_NAVIGATION,
            );
        }
        self.ask_scroll_top();
    }

    /// A click on a note, with `mods` held (no multi-select on a phone).
    pub fn click_file(&mut self, path: &str, mods: Mods) -> Option<Outcome> {
        self.focus = Pane::Files;
        let owned: Vec<String> = self.list_order().into_iter().map(str::to_owned).collect();
        let order: Vec<&str> = owned.iter().map(String::as_str).collect();
        let open = if mods.command && !self.touch {
            self.selection.toggle(path, &order)
        } else if mods.shift && !self.touch {
            Some(self.selection.range(path, &order))
        } else {
            Some(self.selection.click(path))
        };
        open.map(|path| Outcome::Open {
            path,
            how: OpenIn::Here,
            settle: false,
        })
    }

    /// A click on a folder.
    pub fn click_folder(&mut self, folder: &str) {
        self.focus = Pane::Navigation;
        self.select_folder(folder);
        if self.single == Pane::Navigation {
            self.single = Pane::Files;
        }
    }

    /// Selects every note in the list.
    pub fn select_all(&mut self) {
        let order: Vec<String> = self.list_order().into_iter().map(str::to_owned).collect();
        let order: Vec<&str> = order.iter().map(String::as_str).collect();
        self.selection.select_all(&order);
    }

    /// Moves `name`, a child of `parent`, `by` places in the manual order.
    pub fn move_child(&mut self, parent: &str, name: &str, by: isize) {
        let order = tree::reorder(&self.index, parent, &self.tree_options(), name, by);
        self.child_order.insert(parent.to_owned(), order);
        self.rebuild_nav();
    }

    /// Forgets `parent`'s manual order (A to Z again).
    pub fn reset_order(&mut self, parent: &str) {
        if self.child_order.remove(parent).is_some() {
            self.rebuild_nav();
        }
    }

    // ── Scrolling ──────────────────────────────────────────────────────

    /// The pane's view is `view` tall.
    pub fn set_view(&mut self, pane: Pane, view: f64) {
        let s = self.scroll_mut(pane);
        if (s.view - view).abs() > 0.5 {
            s.view = view;
            self.clamp_scroll(pane);
            self.settle(pane);
        }
    }

    /// Scrolls `pane` by `dy`.
    pub fn scroll_by(&mut self, pane: Pane, dy: f64) {
        let to = self.scroll(pane).offset + dy;
        self.scroll_to(pane, to);
    }

    /// Scrolls `pane` to `y`.
    pub fn scroll_to(&mut self, pane: Pane, y: f64) {
        self.scroll_mut(pane).offset = y;
        self.clamp_scroll(pane);
    }

    const fn scroll_mut(&mut self, pane: Pane) -> &mut Scroll {
        match pane {
            Pane::Navigation => &mut self.nav_scroll,
            Pane::Files => &mut self.list_scroll,
        }
    }

    fn clamp_scroll(&mut self, pane: Pane) {
        let max = match pane {
            Pane::Navigation => self.nav_offsets.max_scroll(self.nav_scroll.view),
            Pane::Files => self.list_offsets.max_scroll(self.list_scroll.view),
        };
        let s = self.scroll_mut(pane);
        s.offset = s.offset.clamp(0.0, max);
    }

    fn ask_scroll_top(&mut self) {
        self.list_pending = Some(Pending {
            key: String::new(),
            align: Align::Start,
            rank: RANK_TOP,
        });
        // A new list starts at its top, whatever was asked of the last.
        self.list_scroll.offset = 0.0;
        self.settle(Pane::Files);
    }

    fn ask_scroll(&mut self, pane: Pane, key: String, align: Align, rank: u8) {
        let slot = match pane {
            Pane::Navigation => &mut self.nav_pending,
            Pane::Files => &mut self.list_pending,
        };
        if slot.as_ref().is_none_or(|p| rank >= p.rank) {
            *slot = Some(Pending { key, align, rank });
        }
        self.settle(pane);
    }

    /// Does a pending scroll once the pane has a size.
    fn settle(&mut self, pane: Pane) {
        let (pending, s) = match pane {
            Pane::Navigation => (&mut self.nav_pending, self.nav_scroll),
            Pane::Files => (&mut self.list_pending, self.list_scroll),
        };
        if s.view <= 0.0 {
            return;
        }
        let Some(p) = pending.take() else {
            return;
        };
        if p.key.is_empty() {
            self.scroll_mut(pane).offset = 0.0;
            return;
        }
        let found = match pane {
            Pane::Navigation => self.nav_rows.iter().position(|r| r.key == p.key),
            Pane::Files => self.list_rows.iter().position(|r| r.key == p.key),
        };
        let Some(mut i) = found else {
            return;
        };
        // A heading right over the note comes into view with it.
        if pane == Pane::Files
            && p.align != Align::Auto
            && let Some(prev) = i.checked_sub(1)
            && matches!(
                self.list_rows.get(prev).map(|r| &r.kind),
                Some(ListRowKind::Header { .. })
            )
        {
            i = prev;
        }
        let offsets = match pane {
            Pane::Navigation => &self.nav_offsets,
            Pane::Files => &self.list_offsets,
        };
        let to = offsets.scroll_to(i, p.align, s.offset, s.view);
        self.scroll_mut(pane).offset = to;
    }

    /// Brings row `i` of `pane` into view.
    fn show_row(&mut self, pane: Pane, i: usize) {
        let s = self.scroll(pane);
        let to = match pane {
            Pane::Navigation => self.nav_offsets.scroll_to(i, Align::Auto, s.offset, s.view),
            Pane::Files => self
                .list_offsets
                .scroll_to(i, Align::Auto, s.offset, s.view),
        };
        self.scroll_to(pane, to);
    }

    // ── Keys ───────────────────────────────────────────────────────────

    /// A key, in the pane with the keyboard (`focus`). Returns what the app
    /// should do, if anything.
    pub fn key(&mut self, key: Key, mods: Mods) -> Option<Outcome> {
        match self.focus {
            Pane::Navigation => {
                self.nav_key(key, mods);
                None
            }
            Pane::Files => self.list_key(key, mods),
        }
    }

    /// Rows a page key moves: the view over the average row, less one
    /// (at least one).
    fn page(&self, pane: Pane) -> usize {
        let (offsets, s) = self.pane(pane);
        let n = offsets.len().max(1);
        let avg = offsets.total() / f64::from(u32::try_from(n).unwrap_or(u32::MAX));
        if avg <= 0.0 {
            return 1;
        }
        whole((s.view / avg).floor() - 1.0).max(1)
    }

    fn nav_key(&mut self, key: Key, mods: Mods) {
        let selectable: Vec<usize> = self
            .nav_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect();
        let current = self
            .selection
            .folder()
            .and_then(|f| self.nav_rows.iter().position(|r| r.folder() == Some(f)));
        let at = current.and_then(|c| selectable.iter().position(|i| *i == c));
        let go = |s: &mut Self, to: Option<usize>| {
            if let Some(row) = to.and_then(|t| selectable.get(t)).copied()
                && let Some(f) = s
                    .nav_rows
                    .get(row)
                    .and_then(NavRow::folder)
                    .map(str::to_owned)
            {
                s.select_folder(&f);
                s.show_row(Pane::Navigation, row);
            }
        };
        let last = selectable.len().saturating_sub(1);
        match key {
            Key::Down => go(self, Some(at.map_or(0, |a| a.saturating_add(1).min(last)))),
            Key::Up => go(self, Some(at.map_or(0, |a| a.saturating_sub(1)))),
            Key::Home => go(self, Some(0)),
            Key::End => go(self, Some(last)),
            Key::PageDown => {
                let p = self.page(Pane::Navigation);
                go(self, Some(at.map_or(0, |a| a.saturating_add(p).min(last))));
            }
            Key::PageUp => {
                let p = self.page(Pane::Navigation);
                go(self, Some(at.map_or(0, |a| a.saturating_sub(p))));
            }
            Key::Right => {
                let row = current.and_then(|c| self.nav_rows.get(c)).cloned();
                if let Some(NavRow {
                    kind:
                        NavRowKind::Folder {
                            path,
                            has_children: true,
                            expanded: false,
                            ..
                        },
                    ..
                }) = row
                {
                    self.set_expanded(&path, true);
                } else {
                    self.focus_pane(Pane::Files);
                }
            }
            Key::Left => {
                let row = current.and_then(|c| self.nav_rows.get(c)).cloned();
                if let Some(NavRow {
                    kind: NavRowKind::Folder { path, expanded, .. },
                    ..
                }) = row
                {
                    if expanded {
                        self.set_expanded(&path, false);
                    } else if !path.is_empty() {
                        let up = parent(&path).to_owned();
                        if self
                            .nav_rows
                            .iter()
                            .any(|r| r.folder() == Some(up.as_str()))
                        {
                            self.select_folder(&up);
                        }
                    }
                }
            }
            Key::Tab if !mods.shift => self.focus_pane(Pane::Files),
            _ => {}
        }
    }

    fn list_key(&mut self, key: Key, mods: Mods) -> Option<Outcome> {
        let order: Vec<String> = self.list_order().into_iter().map(str::to_owned).collect();
        let refs: Vec<&str> = order.iter().map(String::as_str).collect();
        let at = self
            .selection
            .cursor()
            .and_then(|c| refs.iter().position(|p| *p == c));
        let last = refs.len().checked_sub(1)?;
        let moved = |s: &mut Self, path: Option<String>| -> Option<Outcome> {
            let path = path?;
            if let Some(i) = s
                .list_rows
                .iter()
                .position(|r| r.file() == Some(path.as_str()))
            {
                s.show_row(Pane::Files, i);
            }
            Some(Outcome::Open {
                path,
                how: OpenIn::Here,
                settle: true,
            })
        };
        let to = |i: usize| refs.get(i).map(|p| (*p).to_owned());
        match key {
            Key::Down | Key::Up if mods.shift && !self.touch => {
                let step = if key == Key::Down { 1 } else { -1 };
                let next = self.selection.extend(step, &refs);
                moved(self, next)
            }
            Key::Home | Key::End if mods.shift && !self.touch => {
                let next = self.selection.extend_to_edge(key == Key::End, &refs);
                moved(self, next)
            }
            Key::Down | Key::Up | Key::Home | Key::End | Key::PageDown | Key::PageUp => {
                let page = self.page(Pane::Files);
                let i = match (key, at) {
                    (Key::Down, Some(a)) => a.saturating_add(1).min(last),
                    (Key::Up, Some(a)) => a.saturating_sub(1),
                    (Key::PageDown, Some(a)) => a.saturating_add(page).min(last),
                    (Key::PageUp, Some(a)) => a.saturating_sub(page),
                    (Key::End, _) => last,
                    _ => 0,
                };
                let path = to(i)?;
                self.selection.click(&path);
                moved(self, Some(path))
            }
            Key::A if mods.command && !self.touch => {
                self.selection.select_all(&refs);
                None
            }
            Key::Left => {
                self.focus_pane(Pane::Navigation);
                None
            }
            Key::Tab if mods.shift => {
                self.focus_pane(Pane::Navigation);
                None
            }
            Key::Right | Key::Tab => Some(Outcome::FocusEditor),
            Key::Enter => {
                let path = self.selection.cursor()?.to_owned();
                let how = if mods.command {
                    OpenIn::Split
                } else if mods.shift {
                    OpenIn::Tab
                } else {
                    OpenIn::Here
                };
                Some(Outcome::Open {
                    path,
                    how,
                    settle: false,
                })
            }
            _ => None,
        }
    }

    /// Gives `pane` the keyboard, and shows it when one pane shows at a
    /// time.
    pub fn focus_pane(&mut self, pane: Pane) {
        self.focus = pane;
        self.single = pane;
        if pane == Pane::Files && self.selection.cursor().is_none() {
            // Something to move from: the first note.
            if let Some(first) = self.list_order().first().map(|p| (*p).to_owned()) {
                self.selection.set_file(Some(first));
            }
        }
    }

    // ── Kept between runs ──────────────────────────────────────────────

    #[must_use]
    pub fn view_state(&self) -> ViewState {
        ViewState {
            expanded: self.expanded.iter().cloned().collect(),
            child_order: self
                .child_order
                .iter()
                .map(|(parent, names)| ChildOrder {
                    parent: parent.clone(),
                    names: names.clone(),
                })
                .collect(),
            folder: self.selection.folder().map(str::to_owned),
            nav_width: self.nav_width,
            nav_height: self.nav_height,
        }
    }

    /// Puts back what [`view_state`](Self::view_state) kept; folders that
    /// are gone are let go.
    pub fn restore(&mut self, state: &ViewState) {
        self.expanded = state
            .expanded
            .iter()
            .filter(|p| self.index.has_folder(p))
            .cloned()
            .collect();
        self.child_order = state
            .child_order
            .iter()
            .map(|c| (c.parent.clone(), c.names.clone()))
            .collect();
        if state.nav_width > 0.0 {
            self.nav_width = state.nav_width;
        }
        if state.nav_height > 0.0 {
            self.nav_height = state.nav_height;
        }
        if let Some(f) = state.folder.as_deref().filter(|f| self.index.has_folder(f)) {
            self.selection
                .select_target(NavTarget::Folder(f.to_owned()), None);
        }
        self.rebuild();
    }
}

/// A folder row's key.
#[must_use]
pub fn folder_key(path: &str) -> String {
    format!("folder:{path}")
}

/// A non-negative count from a float, cut to a whole number.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#[must_use]
pub const fn whole(v: f64) -> usize {
    if v.is_nan() || v <= 0.0 {
        0
    } else {
        v as usize
    }
}
