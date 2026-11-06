//! The navigator as one value, and what each input does to it.
//!
//! The vault, the settings, what is open and selected, both panes' rows
//! and scroll. The view (`ui`) draws it and feeds it clicks, keys and wheels; the tests
//! drive it the same way without a window.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use facet::Facet;

use chrono::NaiveDate;

use crate::appearance::{Appearance, target_key};
use crate::calendar::{self, CalendarContext, CalendarGrid, PeriodicNotes};
use crate::hidden::Hidden;
use crate::index::VaultIndex;
use crate::keys::{KeyAction, KeyBinding, KeyMap};
use crate::layout::{NAV_HEIGHT, NAV_WIDTH, Pane};
use crate::list::{self, Clock, ListContext, ListMetrics, ListRow, ListRowKind, PinIn};
use crate::period::{DayMonthOrder, Period, PeriodKind, day_of};
use crate::profiles::{Profiles, Shortcut, ShortcutKind, VaultProfile};
use crate::properties::{self, PropertyTree, ROOT};
use crate::query::{Join, Operand, Query};
use crate::search::{FilterSearch, toggle_term};
use crate::selection::{NavTarget, Selection};
use crate::settings::{FolderNoteOpen, NavigatorSettings, SearchProvider, SortOption};
use crate::source::{FileOp, NoteFile, PropertyValue, SourceEvent, ancestors, is_within, parent};
use crate::tags::{self, TAGGED, TagTree, UNTAGGED, is_or_below, tag_key};
use crate::template::Templates;
use crate::tree::{self, NavRow, NavRowKind, RECENT, SHORTCUTS, TagRowOptions, TreeOptions};
use crate::virtualize::{Align, OVERSCAN, Offsets};

mod files;
mod looks;

pub use files::{
    Made, MergeSeparator, NameCheck, NewKind, NewNote, OPEN_ALL_WARNING, Reader, check_name,
};
pub use looks::Paint;

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
    /// `P`, for pin with ⌘/Ctrl-⇧.
    Pin,
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Change the vault ([`crate::NavigatorSource::write`]): a folder note
    /// made, set or let go, notes moved in a manual order. Then open
    /// `open`, if any.
    Write {
        ops: Vec<FileOp>,
        open: Option<(String, OpenIn)>,
    },
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
    /// The notes pinned, and where.
    pub pinned: Vec<Pin>,
    /// The list's groups folded, by their collapse keys.
    pub collapsed: Vec<String>,
    /// Each folder's and tag's own list.
    pub appearances: Vec<Appearance>,
    /// Tags' and properties' colours.
    pub colours: Vec<Colour>,
    /// The calendar is on show (Notebook Navigator's `showCalendar`).
    pub calendar: bool,
    /// The open tags' keys, and `__tagged__` for the "Tags" row
    /// (`expandedTags`, `expandedVirtualFolders`).
    pub expanded_tags: Vec<String>,
    /// The open property keys' ids, and `properties-root`.
    pub expanded_properties: Vec<String>,
    /// The root tags as a person ordered them (`rootTagOrder`).
    pub tag_order: Vec<String>,
    /// The property keys as a person ordered them (`rootPropertyOrder`).
    pub property_order: Vec<String>,
    /// The search box's text, kept across runs (`searchQuery`), and
    /// whether the box is open.
    pub search: String,
    pub search_open: bool,
    /// The vault profiles, and the one in use (`vaultProfiles`,
    /// `vaultProfile`).
    pub profiles: Profiles,
    /// Hidden items shown (`showHiddenItems`).
    pub show_hidden: bool,
    /// The shortcuts and the recent notes folded (`shortcutsExpanded`,
    /// `recentNotesExpanded`).
    pub shortcuts_folded: bool,
    pub recent_folded: bool,
    /// The folders' templates and the "create note" commands
    /// (`folderTemplates`, `templateCommands`).
    #[facet(default)]
    pub templates: Templates,
    /// Folders', tags', properties' and notes' icons (`folderIcons`,
    /// `tagIcons`, `propertyIcons`, `fileIcons`), by target.
    #[facet(default)]
    pub icons: Vec<IconAt>,
    /// Their rows' backgrounds (`…BackgroundColors`).
    #[facet(default)]
    pub backgrounds: Vec<Colour>,
    /// A person's own keys (`keyboardShortcuts`), over the defaults.
    #[facet(default)]
    pub keys: Vec<KeyBinding>,
}

/// An icon for a target (`folder:<path>`, `tag:<tag>`,
/// `property:<key>[=<value>]`, `file:<path>`): its id
/// ([`crate::Icon::id`]).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct IconAt {
    pub target: String,
    pub icon: String,
}

/// What is dragged onto a folder, a tag, a property or the "Tags" row:
/// notes, a tag (Notebook Navigator's `TAG_DRAG_MIME`), or a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drag {
    Notes(Vec<String>),
    Tag(String),
    Folder(String),
    /// A shortcut, by its place from 0, dragged to another place.
    Shortcut(usize),
}

/// A pinned note: pinned in folders, in tags.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Pin {
    pub path: String,
    pub folder: bool,
    pub tag: bool,
}

/// A colour for a folder (`folder:<path>`), a tag (`tag:<tag>`), a
/// property (`property:<key>`) or one value of it
/// (`property:<key>=<value>`), or a note (`file:<path>`): a CSS colour.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Colour {
    pub target: String,
    pub colour: String,
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
    /// The tree's options, kept with the settings and the order.
    tree: TreeOptions,
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
    pins: BTreeMap<String, PinIn>,
    collapsed: BTreeSet<String>,
    appearances: BTreeMap<String, Appearance>,
    colours: BTreeMap<String, String>,
    /// The settings with the selected place's own over them.
    effective: NavigatorSettings,
    /// The periodic notes' patterns, read from the settings.
    periodic: PeriodicNotes,
    /// The calendar is on show (the folders' header toggles it).
    pub calendar_open: bool,
    /// The day the calendar is around; today when none.
    calendar_cursor: Option<NaiveDate>,
    /// The tag tree and the property browser, read from the vault (or the
    /// selection's notes, scoped).
    tag_tree: TagTree,
    property_tree: PropertyTree,
    expanded_tags: BTreeSet<String>,
    expanded_props: BTreeSet<String>,
    tag_order: Vec<String>,
    property_order: Vec<String>,
    /// The search box's search (⌘/Ctrl-clicks write to it too); the list
    /// shows what it matches.
    search: FilterSearch,
    /// The search box is open.
    pub search_open: bool,
    /// With the full-text provider: the notes whose text holds the
    /// search's words, as the source found them.
    full_text: Option<BTreeSet<String>>,
    /// The words the full-text matches are for.
    full_text_for: Option<String>,
    /// The vault as the source has it; `index` is what of it shows.
    all: VaultIndex,
    profiles: Profiles,
    show_hidden: bool,
    /// The profile's hiding, compiled.
    hidden: Hidden,
    shortcuts_folded: bool,
    recent_folded: bool,
    templates: Templates,
    icons: BTreeMap<String, String>,
    backgrounds: BTreeMap<String, String>,
    /// A person's own keys, and the map they make with the settings'.
    key_bindings: Vec<KeyBinding>,
    keymap: KeyMap,
    /// Reads a file's text (a template, notes merged); the source's.
    reader: Reader,
}

impl NavigatorState {
    /// The navigator over `index`, the root selected and open.
    #[must_use]
    pub fn new(index: VaultIndex, settings: NavigatorSettings, clock: Clock) -> Self {
        let single = match settings.start_pane {
            crate::settings::StartPane::Navigation => Pane::Navigation,
            crate::settings::StartPane::Files => Pane::Files,
        };
        let profiles = Profiles::default();
        let hidden = Hidden::new(&profiles.active().rules(), false);
        let mut s = Self {
            index: VaultIndex::default(),
            all: index,
            profiles,
            hidden,
            show_hidden: false,
            search: FilterSearch::default(),
            search_open: false,
            full_text: None,
            full_text_for: None,
            shortcuts_folded: false,
            recent_folded: false,
            templates: Templates::default(),
            icons: BTreeMap::new(),
            backgrounds: BTreeMap::new(),
            key_bindings: Vec::new(),
            keymap: KeyMap::new(settings.vim_keys, &[]),
            reader: Reader::none(),
            periodic: PeriodicNotes::from_settings(&settings),
            calendar_open: false,
            calendar_cursor: None,
            effective: settings.clone(),
            settings,
            expanded: std::iter::once(String::new()).collect(),
            child_order: BTreeMap::new(),
            tree: TreeOptions::default(),
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
            pins: BTreeMap::new(),
            collapsed: BTreeSet::new(),
            appearances: BTreeMap::new(),
            colours: BTreeMap::new(),
            tag_tree: TagTree::default(),
            property_tree: PropertyTree::default(),
            // The "Tags" and "Properties" rows open.
            expanded_tags: std::iter::once(TAGGED.to_owned()).collect(),
            expanded_props: std::iter::once(ROOT.to_owned()).collect(),
            tag_order: Vec::new(),
            property_order: Vec::new(),
        };
        s.index = s.visible();
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

    /// The settings the list is drawn with: the selected folder's or
    /// tag's own over the settings.
    #[must_use]
    pub const fn effective(&self) -> &NavigatorSettings {
        &self.effective
    }

    /// The folder or tag selected (the root when nothing is).
    #[must_use]
    pub fn target(&self) -> NavTarget {
        self.selection
            .target()
            .cloned()
            .unwrap_or_else(|| NavTarget::Folder(String::new()))
    }

    /// A place's own list, if it has one.
    #[must_use]
    pub fn appearance(&self, target: &NavTarget) -> Option<&Appearance> {
        self.appearances.get(&target_key(target))
    }

    /// Where `path` is pinned.
    #[must_use]
    pub fn pinned(&self, path: &str) -> PinIn {
        self.pins.get(path).copied().unwrap_or_default()
    }

    /// Whether `path` is pinned in the kind of place selected.
    #[must_use]
    pub fn is_pinned_here(&self, path: &str) -> bool {
        let at = self.pinned(path);
        match self.target() {
            NavTarget::Folder(_) => at.folder,
            _ => at.tag,
        }
    }

    /// A tag's or a property's colour (`tag:<tag>`, `property:<key>`,
    /// `property:<key>=<value>`, any case): a tag without one takes the
    /// nearest tag above it's (`inheritTagColors`).
    #[must_use]
    pub fn colour(&self, target: &str) -> Option<&str> {
        let key = colour_key(target);
        if let Some(c) = self.frontmatter_colour(&key) {
            return Some(c);
        }
        self.inherited(&self.colours, &key)
    }

    /// The tag tree on show.
    #[must_use]
    pub const fn tag_tree(&self) -> &TagTree {
        &self.tag_tree
    }

    /// The property browser on show.
    #[must_use]
    pub const fn property_tree(&self) -> &PropertyTree {
        &self.property_tree
    }

    /// The search as tags and properties, as ⌘/Ctrl-clicks build it;
    /// empty when the search is more than that.
    #[must_use]
    pub fn query(&self) -> Query {
        Query::parse(self.search.text()).unwrap_or_default()
    }

    /// The search box's search.
    #[must_use]
    pub const fn search(&self) -> &FilterSearch {
        &self.search
    }

    /// The vault profiles.
    #[must_use]
    pub const fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    /// The profile in use.
    #[must_use]
    pub fn profile(&self) -> &VaultProfile {
        self.profiles.active()
    }

    /// Whether hidden items show.
    #[must_use]
    pub const fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// The vault as the source has it, hidden items too.
    #[must_use]
    pub const fn all(&self) -> &VaultIndex {
        &self.all
    }

    /// `folder`'s note, when folder notes are on and it has one.
    #[must_use]
    pub fn folder_note(&self, folder: &str) -> Option<&str> {
        list::folder_note(&self.index, &self.settings, folder)
    }

    /// The list's context: what [`list::rows`] reads past the vault.
    #[must_use]
    pub const fn list_context<'a>(&'a self, target: &'a NavTarget) -> ListContext<'a> {
        ListContext {
            target,
            settings: &self.effective,
            clock: self.clock,
            pins: &self.pins,
            collapsed: &self.collapsed,
            tree: self.tree_cache(),
            search: &self.search,
            full_text: self.full_text.as_ref(),
            hidden: &self.hidden,
        }
    }

    const fn tree_cache(&self) -> &TreeOptions {
        &self.tree
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

    /// The navigation row `y` points down from the pane's top (a drop's
    /// target), counting the scroll.
    #[must_use]
    pub fn nav_row_at(&self, y: f64) -> Option<&NavRow> {
        if y < 0.0 || self.nav_offsets.is_empty() {
            return None;
        }
        let at = y + self.nav_scroll.offset;
        if at > self.nav_offsets.total() {
            return None;
        }
        self.nav_rows.get(self.nav_offsets.at(at))
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
        self.tree = TreeOptions {
            show_root: self.settings.show_root,
            child_order: self.child_order.clone(),
        };
        self.rebuild_trees();
        self.rebuild_nav();
        self.rebuild_list();
    }

    /// Whether the tags or the properties show the selection's alone.
    const fn scoped(&self) -> bool {
        self.settings.scope_tags || self.settings.scope_properties
    }

    /// The tag tree and the property browser read again: from the vault,
    /// or from the selection's notes when scoped
    /// (`scopeTagsToCurrentContext`: a folder or a property selected;
    /// `scopePropertiesToCurrentContext`: a folder or a tag).
    fn rebuild_trees(&mut self) {
        let target = self.target();
        let effective = self.settings_for(&target);
        let selected = |on: bool| -> Option<Vec<&NoteFile>> {
            on.then(|| list::files_of(&self.index, &target, &effective, &self.pins, &self.hidden))
        };
        let s = &self.settings;
        self.tag_tree = if !s.tag_tree {
            TagTree::default()
        } else if let Some(files) = selected(s.scope_tags && !matches!(target, NavTarget::Tag(_))) {
            TagTree::build(files)
        } else {
            TagTree::build(self.index.files())
        };
        let keys = NavigatorSettings::list(&s.property_keys);
        self.property_tree = if !s.property_tree || keys.is_empty() {
            PropertyTree::default()
        } else if let Some(files) =
            selected(s.scope_properties && !matches!(target, NavTarget::Property { .. }))
        {
            PropertyTree::build(files, &keys)
        } else {
            PropertyTree::build(self.index.files(), &keys)
        };
    }

    fn rebuild_nav(&mut self) {
        let mut expanded = self.expanded.clone();
        if !self.settings.show_root {
            // A hidden root is always open.
            expanded.insert(String::new());
        }
        self.nav_rows = tree::flatten(&self.index, &expanded, &self.tree_options());
        let top = self.top_rows();
        if !top.is_empty() {
            let at = usize::from(!self.nav_rows.is_empty());
            self.nav_rows.splice(at..at, top);
        }
        // The tags and the properties, under the folders.
        let bottom = self.nav_rows.pop();
        let below = self.settings.include_descendants;
        if self.settings.tag_tree {
            self.nav_rows.extend(tree::tag_rows(
                &self.tag_tree,
                &self.expanded_tags,
                &TagRowOptions {
                    all_row: self.settings.all_tags_row,
                    untagged: self.settings.show_untagged,
                    sort: self.settings.tag_sort,
                    below,
                    order: &self.tag_order,
                },
            ));
        }
        if self.settings.property_tree {
            self.nav_rows.extend(tree::property_rows(
                &self.property_tree,
                &self.expanded_props,
                self.settings.property_sort,
                below,
                &self.property_order,
            ));
        }
        self.nav_rows.extend(bottom);
        let h = self.nav_item_height();
        self.nav_offsets = Offsets::new(self.nav_rows.iter().map(|r| match r.kind {
            NavRowKind::TopSpacer => tree::TOP_SPACER,
            NavRowKind::BottomSpacer => tree::BOTTOM_SPACER,
            NavRowKind::Banner { .. } => tree::BANNER_HEIGHT,
            _ => h,
        }));
        self.clamp_scroll(Pane::Navigation);
        self.settle(Pane::Navigation);
    }

    fn rebuild_list(&mut self) {
        let target = self.target();
        self.effective = self.settings_for(&target);
        self.list_rows = list::rows(&self.index, &self.list_context(&target));
        let (settings, metrics) = (&self.effective, self.metrics);
        self.list_offsets = Offsets::new(
            self.list_rows
                .iter()
                .map(|r| list::height(r, settings, metrics)),
        );
        self.clamp_scroll(Pane::Files);
        self.settle(Pane::Files);
        // A new selection scopes the tags and properties anew.
        if self.scoped() {
            self.rebuild_trees();
            self.rebuild_nav();
        }
    }

    /// The settings `target`'s list is drawn with.
    #[must_use]
    pub fn settings_for(&self, target: &NavTarget) -> NavigatorSettings {
        self.appearances
            .get(&target_key(target))
            .map_or_else(|| self.settings.clone(), |a| a.over(&self.settings))
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
            self.periodic = PeriodicNotes::from_settings(&settings);
            if settings.search_provider != SearchProvider::FullText {
                self.full_text = None;
                self.full_text_for = None;
            }
            self.keymap = KeyMap::new(settings.vim_keys, &self.key_bindings);
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
        let moved = self.all.apply(event);
        match self.visible_event(event) {
            Some(e) => {
                self.index.apply(&e);
            }
            None => self.index = self.visible(),
        }
        match event {
            SourceEvent::FileDeleted(p) | SourceEvent::FolderDeleted(p) => {
                self.profiles.follow(p, None);
                self.templates.follow(p, None);
                self.follow_looks(p, None);
            }
            SourceEvent::FolderRenamed { from, to } => {
                self.profiles.follow(from, Some(to));
                self.templates.follow(from, Some(to));
                self.follow_looks(from, Some(to));
            }
            SourceEvent::FileRenamed { from, file } => {
                self.profiles.follow(from, Some(&file.path));
                self.templates.follow(from, Some(&file.path));
                self.follow_looks(from, Some(&file.path));
            }
            _ => {}
        }
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
        for (from, to) in &moved {
            if let Some(at) = self.pins.remove(from) {
                self.pins.insert(to.clone(), at);
            }
            if let Some(mut a) = self.appearances.remove(&format!("folder:{from}")) {
                a.target = format!("folder:{to}");
                self.appearances.insert(a.target.clone(), a);
            }
        }
        self.rebuild();
    }

    /// Everything read again from a fresh index (a [`SourceEvent::Reset`]).
    pub fn replace_index(&mut self, index: VaultIndex) {
        self.all = index;
        self.apply(&SourceEvent::Reset);
    }

    /// Selects `folder` in the navigation pane, recorded in the history.
    pub fn select_folder(&mut self, folder: &str) {
        if !self.index.has_folder(folder) {
            return;
        }
        // The note open in the workspace stays selected when it is in the
        // new folder's list (Notebook Navigator's auto-selected file).
        let below = self
            .settings_for(&NavTarget::Folder(folder.to_owned()))
            .include_descendants;
        let keep = self.active.clone().filter(|a| {
            let p = parent(a);
            p == folder || (below && is_within(p, folder))
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
        // `recordFileOpen`.
        if let Some(p) = path
            && self.all.file(p).is_some()
        {
            let limit = usize::try_from(self.settings.recent_notes_count).unwrap_or(5);
            if self.profiles.active_mut().record(p, limit) {
                self.rebuild_nav();
            }
        }
        // The calendar goes to a day's, a week's or a month's note as it
        // opens (`shouldAutoRevealCalendarNoteKind`): to its days, and to
        // a month's own month.
        if let Some(period) = path.and_then(|p| self.periodic.period_of(p)) {
            let go = match period.kind {
                PeriodKind::Day | PeriodKind::Week => !self.calendar_shows(period.start),
                PeriodKind::Month => {
                    Period::containing(
                        PeriodKind::Month,
                        self.calendar_cursor(),
                        self.periodic.week_rules(),
                    ) != period
                }
                PeriodKind::Quarter | PeriodKind::Year => false,
            };
            if go {
                self.calendar_cursor = Some(period.start);
            }
        }
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
        let folder = if !manual
            && self.effective.include_descendants
            && self.settings.reveal_shortest_path
        {
            self.selection
                .folder()
                .filter(|c| is_within(&home, c))
                .map_or_else(|| self.nearest_open(&home), str::to_owned)
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

    /// The highest folder of `home`'s chain whose parents are all open:
    /// the nearest one in view.
    fn nearest_open(&self, home: &str) -> String {
        let chain: Vec<&str> = std::iter::once(home).chain(ancestors(home)).collect();
        chain
            .iter()
            .rev()
            .find(|f| {
                ancestors(f).all(|a| {
                    self.expanded.contains(a) || (a.is_empty() && !self.settings.show_root)
                }) && (!f.is_empty() || self.settings.show_root)
            })
            .map_or_else(|| home.to_owned(), |f| (*f).to_owned())
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
        self.rebuild_list();
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

    /// A click on a folder; its note opens with it when folder notes are
    /// on (`enableFolderNoteLinks`).
    pub fn click_folder(&mut self, folder: &str) -> Option<Outcome> {
        self.focus = Pane::Navigation;
        self.select_folder(folder);
        if self.single == Pane::Navigation {
            self.single = Pane::Files;
        }
        self.open_folder_note(folder, false)
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
        self.rebuild();
    }

    /// Forgets `parent`'s manual order (A to Z again).
    pub fn reset_order(&mut self, parent: &str) {
        if self.child_order.remove(parent).is_some() {
            self.rebuild();
        }
    }

    // ── Tags ───────────────────────────────────────────────────────────

    /// Selects `tag` (a tag on a row clicked, `navigateToTag`): its notes,
    /// and those of the tags below it, recorded in the history.
    pub fn select_tag(&mut self, tag: &str) {
        if self.pick_tag(tag) {
            self.focus = Pane::Files;
            self.single = Pane::Files;
        }
    }

    /// Selects `tag` (or the "Tags" or "Untagged" row), recorded in the
    /// history: whether there was one to select.
    fn pick_tag(&mut self, tag: &str) -> bool {
        let tag = tag.trim_start_matches('#').trim_matches('/');
        if tag.is_empty() {
            return false;
        }
        // As the tree writes it (its first spelling), any case asked.
        let tag = if tags::is_virtual(tag) {
            tag.to_owned()
        } else {
            self.tag_tree
                .get(tag)
                .map_or_else(|| tag.to_owned(), |n| n.display.clone())
        };
        let target = NavTarget::Tag(tag.clone());
        self.pick(target, &tag_row_key(&tag));
        true
    }

    /// Selects `target`, the note open kept selected when it is listed
    /// there; the navigation pane scrolled to `row`.
    fn pick(&mut self, target: NavTarget, row: &str) {
        let s = self.settings_for(&target);
        let keep = self.active.clone().filter(|a| {
            list::files_of(&self.index, &target, &s, &self.pins, &self.hidden)
                .iter()
                .any(|f| f.path == *a)
        });
        self.selection.select_target(target, keep.clone());
        self.rebuild_list();
        match keep {
            Some(file) => self.ask_scroll(Pane::Files, file, Align::Auto, RANK_NAVIGATION),
            None => self.ask_scroll_top(),
        }
        self.ask_scroll(
            Pane::Navigation,
            row.to_owned(),
            Align::Auto,
            RANK_NAVIGATION,
        );
    }

    /// A click on a tag row: selects it; with ⌘/Ctrl, adds it to the
    /// search (AND), with ⇧ too (OR), or takes it out (Notebook
    /// Navigator's `handleTagClick`; not the "Tags" or "Untagged" rows, not
    /// on a phone).
    pub fn click_tag(&mut self, tag: &str, mods: Mods) {
        if mods.command && !self.touch && !tags::is_virtual(tag) {
            let join = if mods.shift { Join::Or } else { Join::And };
            self.toggle_query(Operand::tag(tag), join);
            return;
        }
        self.focus = Pane::Navigation;
        if self.pick_tag(tag) && self.single == Pane::Navigation {
            self.single = Pane::Files;
        }
    }

    /// Selects a property's `key` (and `value`), or the "Properties" row
    /// ([`ROOT`]).
    pub fn select_property(&mut self, key: &str, value: Option<&str>) {
        let key = if key == ROOT {
            ROOT.to_owned()
        } else {
            properties::fold(key)
        };
        if key.is_empty() {
            return;
        }
        let value = value.map(properties::value_path).filter(|v| !v.is_empty());
        let row = property_row_key(&key, value.as_deref());
        self.pick(NavTarget::Property { key, value }, &row);
    }

    /// A click on a property row: selects it; with ⌘/Ctrl, adds it to the
    /// search (AND), with ⇧ too (OR), or takes it out
    /// (`handlePropertyClick`; not the "Properties" row).
    pub fn click_property(&mut self, key: &str, value: Option<&str>, mods: Mods) {
        let modified = mods.command && !self.touch;
        if modified && key != ROOT {
            let join = if mods.shift { Join::Or } else { Join::And };
            self.toggle_query(Operand::property(key, value), join);
            return;
        }
        self.focus = Pane::Navigation;
        self.select_property(key, value);
        if self.single == Pane::Navigation {
            self.single = Pane::Files;
        }
    }

    /// Selects what a navigation row selects.
    pub fn select_nav(&mut self, target: &NavTarget) {
        match target {
            NavTarget::Folder(f) => self.select_folder(f),
            NavTarget::Tag(t) => {
                self.pick_tag(t);
            }
            NavTarget::Property { key, value } => self.select_property(key, value.as_deref()),
        }
    }

    /// Opens or closes a folder, a tag, a property key, or the "Tags" or
    /// "Properties" row.
    pub fn toggle_nav(&mut self, target: &NavTarget) {
        let open = !self.nav_expanded(target);
        self.set_nav_expanded(target, open);
    }

    fn nav_expanded(&self, target: &NavTarget) -> bool {
        match target {
            NavTarget::Folder(f) => self.expanded.contains(f),
            NavTarget::Tag(t) => self.expanded_tags.contains(&expansion_key(t)),
            NavTarget::Property { key, .. } => self.expanded_props.contains(&prop_expansion(key)),
        }
    }

    pub fn set_nav_expanded(&mut self, target: &NavTarget, open: bool) {
        let (set, key) = match target {
            NavTarget::Folder(f) => return self.set_expanded(f, open),
            NavTarget::Tag(t) => (&mut self.expanded_tags, expansion_key(t)),
            NavTarget::Property { key, .. } => (&mut self.expanded_props, prop_expansion(key)),
        };
        let changed = if open {
            set.insert(key)
        } else {
            set.remove(&key)
        };
        if changed {
            self.rebuild_nav();
        }
    }

    // ── The search ─────────────────────────────────────────────────────

    /// Adds `operand` to the search joined by `join`, or takes it out
    /// (`updateFilterQueryWithTag`, `updateFilterQueryWithProperty`): in
    /// the expression when the search is tags and properties alone, else
    /// as a term of its own; the search box opens with it.
    pub fn toggle_query(&mut self, operand: Operand, join: Join) {
        let text = match Query::parse(self.search.text()) {
            Some(mut q) => {
                q.toggle(operand, join);
                q.text()
            }
            None => toggle_term(self.search.text(), &operand.text()),
        };
        if !text.is_empty() {
            self.search_open = true;
        }
        self.set_search(&text);
    }

    /// The search as tags and properties.
    pub fn set_query(&mut self, query: &Query) {
        self.set_search(&query.text());
    }

    /// The search box's text, read again; the list shows what it matches.
    /// With the full-text provider, the matches wait for
    /// [`set_full_text`](Self::set_full_text).
    pub fn set_search(&mut self, text: &str) {
        if text == self.search.text() {
            return;
        }
        self.search = FilterSearch::parse(text, self.today(), DayMonthOrder::default());
        self.full_text = None;
        self.full_text_for = None;
        self.rebuild_list();
        self.ask_scroll_top();
    }

    /// The notes whose text holds the search's words (the source's full
    /// text search), or `None` (a source that cannot) to match the words
    /// against names.
    pub fn set_full_text(&mut self, hits: Option<BTreeSet<String>>) {
        self.full_text_for = Some(self.full_text_words());
        if hits != self.full_text {
            self.full_text = hits;
            self.rebuild_list();
        }
    }

    /// The search's words, as the full text is asked for them.
    #[must_use]
    pub fn full_text_words(&self) -> String {
        self.search.words().join(" ")
    }

    /// Whether the full text must be asked for the search's words: the
    /// provider is the full text, there are words, and the matches are
    /// for others (or none yet).
    #[must_use]
    pub fn full_text_due(&self) -> bool {
        self.settings.search_provider == SearchProvider::FullText
            && !self.search.words().is_empty()
            && self.full_text_for.as_deref() != Some(self.full_text_words().as_str())
    }

    /// Opens the search box, or closes and clears it (`toggleSearch`).
    pub fn toggle_search(&mut self) {
        self.search_open = !self.search_open;
        if !self.search_open {
            self.set_search("");
        }
    }

    /// Selects the vault's root, lists the notes of every folder, and
    /// opens the search box (`search-vault`, `searchWithDescendants`).
    pub fn search_vault(&mut self) {
        if !self.settings.include_descendants {
            let mut s = self.settings.clone();
            s.include_descendants = true;
            self.set_settings(s);
        }
        self.select_folder("");
        self.search_open = true;
    }

    // ── Profiles, hiding, shortcuts and recent notes ───────────────────

    /// What of the vault shows, by the profile's hiding.
    fn visible(&self) -> VaultIndex {
        if self.hidden.is_empty() {
            return self.all.clone();
        }
        let h = &self.hidden;
        let folders: Vec<String> = self
            .all
            .folder_paths()
            .filter(|f| !h.hides_folder(f))
            .map(str::to_owned)
            .collect();
        VaultIndex::from_parts(
            self.all.name().to_owned(),
            folders,
            self.all
                .files()
                .filter(|f| !h.hides_file(f))
                .map(|f| h.shown(f)),
        )
    }

    /// A change in the source, as the shown vault takes it; `None` to
    /// read the shown vault again.
    fn visible_event(&self, e: &SourceEvent) -> Option<SourceEvent> {
        let h = &self.hidden;
        if matches!(e, SourceEvent::Reset) {
            return None;
        }
        if h.is_empty() {
            return Some(e.clone());
        }
        Some(match e {
            SourceEvent::FolderCreated(p) if h.hides_folder(p) => SourceEvent::Reset,
            SourceEvent::FileCreated(f) | SourceEvent::FileModified(f) => {
                if h.hides_file(f) {
                    SourceEvent::FileDeleted(f.path.clone())
                } else {
                    SourceEvent::FileModified(h.shown(f))
                }
            }
            SourceEvent::FileRenamed { from, file } => {
                if h.hides_file(file) {
                    SourceEvent::FileDeleted(from.clone())
                } else {
                    SourceEvent::FileRenamed {
                        from: from.clone(),
                        file: h.shown(file),
                    }
                }
            }
            SourceEvent::FolderRenamed { .. } | SourceEvent::Reset => return None,
            other => other.clone(),
        })
    }

    /// The hiding compiled again and the vault shown anew: a profile
    /// switched or edited, hidden items shown or not.
    fn rehide(&mut self) {
        self.hidden = Hidden::new(&self.profiles.active().rules(), self.show_hidden);
        self.index = self.visible();
        let index = &self.index;
        self.expanded.retain(|p| index.has_folder(p));
        self.expanded.insert(String::new());
        if let NavTarget::Folder(f) = self.target()
            && !self.index.has_folder(&f)
        {
            self.selection
                .select_target(NavTarget::Folder(String::new()), None);
        }
        self.rebuild();
    }

    /// Shows hidden items, or hides them again (`toggle-hidden`).
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.rehide();
    }

    /// Uses the profile `which` names: its id, its name, or its number
    /// (`select-profile`, `select-profile-1`…).
    pub fn select_profile(&mut self, which: &str) -> Result<bool, String> {
        let changed = self.profiles.select(which)?;
        if changed {
            self.rehide();
        }
        Ok(changed)
    }

    /// Adds a profile, as the one in use (`copy`) or empty, and uses it.
    pub fn add_profile(&mut self, name: &str, copy: bool) -> Result<String, String> {
        let id = self.profiles.add(name, copy)?;
        self.select_profile(&id)?;
        Ok(id)
    }

    /// Takes out a profile (not the default).
    pub fn remove_profile(&mut self, which: &str) -> Result<(), String> {
        self.profiles.remove(which)?;
        self.rehide();
        Ok(())
    }

    /// Renames a profile.
    pub fn rename_profile(&mut self, which: &str, name: &str) -> Result<(), String> {
        self.profiles.rename(which, name)?;
        self.rebuild_nav();
        Ok(())
    }

    /// Changes the profile in use: what it hides, its banner.
    pub fn edit_profile(&mut self, f: impl FnOnce(&mut VaultProfile)) {
        let before = self.profiles.active().clone();
        f(self.profiles.active_mut());
        if *self.profiles.active() != before {
            self.rehide();
        }
    }

    /// Hides a folder (`/path`), a tag, or a note (`/path`) in the profile
    /// in use, or shows it again (`unhide`).
    pub fn hide(&mut self, what: &HideTarget, unhide: bool) {
        let (list, pattern): (fn(&mut VaultProfile) -> &mut Vec<String>, String) = match what {
            HideTarget::Folder(p) => (|v| &mut v.hidden_folders, format!("/{p}")),
            HideTarget::Tag(t) => (|v| &mut v.hidden_tags, tag_key(t)),
            HideTarget::Note(p) => (|v| &mut v.hidden_file_names, format!("/{p}")),
        };
        self.edit_profile(|v| {
            let l = list(v);
            if unhide {
                l.retain(|p| !p.eq_ignore_ascii_case(&pattern));
            } else if !l.iter().any(|p| p.eq_ignore_ascii_case(&pattern)) {
                l.push(pattern);
            }
        });
    }

    /// Adds a shortcut to the profile in use (at `at`, else last); `false`
    /// when it has it.
    pub fn add_shortcut(&mut self, shortcut: Shortcut, at: Option<usize>) -> bool {
        let added = self.profiles.active_mut().add_shortcut(shortcut, at);
        if added {
            self.rebuild_nav();
        }
        added
    }

    /// Takes out a shortcut.
    pub fn remove_shortcut(&mut self, shortcut: &Shortcut) -> bool {
        let removed = self.profiles.active_mut().remove_shortcut(shortcut);
        if removed {
            self.rebuild_nav();
        }
        removed
    }

    /// Moves the shortcut at `from` to `to`.
    pub fn move_shortcut(&mut self, from: usize, to: usize) -> bool {
        let moved = self.profiles.active_mut().move_shortcut(from, to);
        if moved {
            self.rebuild_nav();
        }
        moved
    }

    /// Names the shortcut at `at` (`""`: its target's name again).
    pub fn rename_shortcut(&mut self, at: usize, name: &str) -> bool {
        let Some(s) = self.profiles.active_mut().shortcuts.get_mut(at) else {
            return false;
        };
        name.trim().clone_into(&mut s.name);
        self.rebuild_nav();
        true
    }

    /// Opens shortcut `n` (from 1, `open-shortcut-1`…`9`): a note opens, a
    /// folder, tag or property is selected, a saved search is run.
    ///
    /// # Errors
    ///
    /// No shortcut `n`, or what it opens is gone.
    pub fn open_shortcut(&mut self, n: usize) -> Result<Option<Outcome>, String> {
        let s = n
            .checked_sub(1)
            .and_then(|i| self.profile().shortcuts.get(i))
            .cloned()
            .ok_or_else(|| format!("No shortcut {n}"))?;
        self.open_shortcut_entry(&s)
    }

    /// Opens a shortcut.
    ///
    /// # Errors
    ///
    /// What it opens is gone.
    pub fn open_shortcut_entry(&mut self, s: &Shortcut) -> Result<Option<Outcome>, String> {
        match s.kind {
            ShortcutKind::Note => {
                if self.index.file(&s.target).is_none() {
                    return Err(format!("{} is not in the vault", s.target));
                }
                self.reveal(&s.target, true);
                Ok(Some(Outcome::Open {
                    path: s.target.clone(),
                    how: OpenIn::Here,
                    settle: false,
                }))
            }
            ShortcutKind::Search => {
                self.search_open = true;
                self.set_search(&s.target);
                Ok(None)
            }
            ShortcutKind::Folder if !self.index.has_folder(&s.target) => {
                Err(format!("No folder {}", s.target))
            }
            _ => {
                if let Some(t) = s.nav_target() {
                    self.select_nav(&t);
                    self.focus = Pane::Navigation;
                }
                Ok(None)
            }
        }
    }

    /// Opens a recent note.
    #[must_use]
    pub fn open_recent(&self, path: &str) -> Option<Outcome> {
        self.index.file(path).map(|_| Outcome::Open {
            path: path.to_owned(),
            how: OpenIn::Here,
            settle: false,
        })
    }

    /// Folds or unfolds the shortcuts ([`SHORTCUTS`]) or the recent notes
    /// ([`RECENT`]).
    pub fn toggle_section(&mut self, id: &str) {
        match id {
            SHORTCUTS => self.shortcuts_folded = !self.shortcuts_folded,
            RECENT => self.recent_folded = !self.recent_folded,
            _ => return,
        }
        self.rebuild_nav();
    }

    /// The banner, shortcut and recent rows over the folders.
    fn top_rows(&self) -> Vec<NavRow> {
        let p = self.profile();
        let mut rows = Vec::new();
        if let Some(src) = p.banner.as_deref().filter(|b| !b.trim().is_empty()) {
            rows.push(NavRow {
                key: "banner".to_owned(),
                level: 0,
                kind: NavRowKind::Banner {
                    src: src.trim().to_owned(),
                },
            });
        }
        if self.settings.show_shortcuts && !p.shortcuts.is_empty() {
            rows.push(section(
                SHORTCUTS,
                "Shortcuts",
                !self.shortcuts_folded,
                p.shortcuts.len(),
            ));
            if !self.shortcuts_folded {
                for (at, sc) in p.shortcuts.iter().enumerate() {
                    let missing = match sc.kind {
                        ShortcutKind::Note => self.index.file(&sc.target).is_none(),
                        ShortcutKind::Folder => !self.index.has_folder(&sc.target),
                        _ => false,
                    };
                    let name = match (sc.kind, self.index.file(&sc.target)) {
                        (ShortcutKind::Note, Some(f)) if sc.name.is_empty() => {
                            f.title(&self.settings.name_fields()).to_owned()
                        }
                        _ => sc.label(),
                    };
                    rows.push(NavRow {
                        key: format!("shortcut:{at}"),
                        level: 0,
                        kind: NavRowKind::Shortcut {
                            at,
                            shortcut: sc.clone(),
                            name,
                            missing,
                        },
                    });
                }
            }
        }
        if self.settings.show_recent_notes {
            let limit = usize::try_from(self.settings.recent_notes_count).unwrap_or(5);
            let recent: Vec<&NoteFile> = p
                .recent
                .iter()
                .filter_map(|r| self.index.file(r))
                .take(limit)
                .collect();
            if !recent.is_empty() {
                rows.push(section(
                    RECENT,
                    "Recent notes",
                    !self.recent_folded,
                    recent.len(),
                ));
                if !self.recent_folded {
                    let fields = self.settings.name_fields();
                    for f in recent {
                        rows.push(NavRow {
                            key: format!("recent:{}", f.path),
                            level: 0,
                            kind: NavRowKind::Recent {
                                path: f.path.clone(),
                                name: f.title(&fields).to_owned(),
                            },
                        });
                    }
                }
            }
        }
        rows
    }

    // ── Tag operations ─────────────────────────────────────────────────

    /// Adds `tag` to the notes at `paths` (Notebook Navigator's
    /// `addTagToFiles`): a note with it or a tag above it is left, the tags
    /// below it go.
    ///
    /// # Errors
    ///
    /// No notes, one that is not a Markdown note, or a name that cannot be
    /// a tag.
    pub fn add_tag(&self, paths: &[String], tag: &str) -> Result<Outcome, String> {
        self.notes_only(paths)?;
        let tag = tag.trim().trim_start_matches('#').trim_matches('/');
        if !tags::valid(tag) || tags::is_virtual(tag) {
            return Err(format!("{tag:?} cannot be a tag"));
        }
        Ok(write(FileOp::AddTag {
            paths: paths.to_vec(),
            tag: tag.to_owned(),
        }))
    }

    /// Takes `tag` itself (not the tags below it) off the notes at `paths`.
    ///
    /// # Errors
    ///
    /// No notes, or one that is not a Markdown note.
    pub fn remove_tag(&self, paths: &[String], tag: &str) -> Result<Outcome, String> {
        self.notes_only(paths)?;
        if tags::is_virtual(tag) {
            return Err("Pick a tag to remove".to_owned());
        }
        Ok(write(FileOp::RemoveTag {
            paths: paths.to_vec(),
            tag: tag.trim().trim_start_matches('#').to_owned(),
        }))
    }

    /// Takes every tag off the notes at `paths`.
    ///
    /// # Errors
    ///
    /// No notes, or one that is not a Markdown note.
    pub fn clear_tags(&self, paths: &[String]) -> Result<Outcome, String> {
        self.notes_only(paths)?;
        Ok(write(FileOp::ClearTags {
            paths: paths.to_vec(),
        }))
    }

    /// Renames `from` to `to` in every note, and the tags below it
    /// (`TagRenameWorkflow`). The colours, open tags, order and the
    /// selection follow once the vault has done it ([`Self::after_write`]).
    ///
    /// # Errors
    ///
    /// The "Tags" or "Untagged" row, a name that cannot be a tag, the same
    /// name, or a tag moved inside itself.
    pub fn rename_tag(&self, from: &str, to: &str) -> Result<Outcome, String> {
        let from = from.trim().trim_start_matches('#').trim_matches('/');
        let to = to.trim().trim_start_matches('#').trim_matches('/');
        if tags::is_virtual(from) || from.is_empty() {
            return Err("Pick a tag to rename".to_owned());
        }
        if !tags::valid(to) {
            return Err(format!("{to:?} cannot be a tag"));
        }
        if from == to {
            return Err(format!("#{from} is called that already"));
        }
        let (fk, tk) = (tag_key(from), tag_key(to));
        if fk != tk && is_or_below(&tk, &fk) {
            return Err(format!("#{from} cannot move inside itself"));
        }
        Ok(write(FileOp::RenameTag {
            from: from.to_owned(),
            to: to.to_owned(),
        }))
    }

    /// Takes `tag` and the tags below it off every note
    /// (`TagDeleteWorkflow`).
    ///
    /// # Errors
    ///
    /// The "Tags" or "Untagged" row.
    pub fn delete_tag(&self, tag: &str) -> Result<Outcome, String> {
        let tag = tag.trim().trim_start_matches('#').trim_matches('/');
        if tags::is_virtual(tag) || tag.is_empty() {
            return Err("Pick a tag to delete".to_owned());
        }
        Ok(write(FileOp::DeleteTag(tag.to_owned())))
    }

    /// A new note tagged `tag`, beside the selected note (or the one open,
    /// or at the root), then opened (`createNewFileForTag`).
    ///
    /// # Errors
    ///
    /// The "Tags" or "Untagged" row.
    pub fn new_note_in_tag(&self, tag: &str) -> Result<Outcome, String> {
        let tag = tag.trim().trim_start_matches('#').trim_matches('/');
        if tags::is_virtual(tag) || tag.is_empty() {
            return Err("Pick a tag for the new note".to_owned());
        }
        let shown = self
            .tag_tree
            .get(tag)
            .map_or_else(|| tag.to_owned(), |n| n.display.clone());
        let folder = self
            .selection
            .cursor()
            .or(self.active.as_deref())
            .and_then(|p| self.index.file(p))
            .map_or(String::new(), |f| f.folder().to_owned());
        let path = self.untitled(&folder);
        Ok(Outcome::Write {
            ops: vec![FileOp::CreateFile {
                path: path.clone(),
                text: format!("---\ntags:\n- {shown}\n---\n"),
            }],
            open: Some((path, OpenIn::Tab)),
        })
    }

    /// A path for a new note in `folder` that no file has: `Untitled.md`,
    /// then `Untitled 1.md`, …
    #[must_use]
    pub fn untitled(&self, folder: &str) -> String {
        (0..=u32::MAX)
            .map(|n| {
                let name = if n == 0 {
                    "Untitled.md".to_owned()
                } else {
                    format!("Untitled {n}.md")
                };
                join(folder, &name)
            })
            .find(|p| self.index.file(p).is_none())
            .unwrap_or_default()
    }

    /// Sets property `key` on the notes at `paths`: to `value` (a value
    /// row's, as first written: its link markup kept), or empty for the key
    /// itself (`applyPropertyNodeToFiles`). A value replaces what the note
    /// had (a list becomes that one value); notes that have it already are
    /// left.
    ///
    /// # Errors
    ///
    /// No notes, one that is not a Markdown note, no key, or every note has
    /// it already.
    pub fn set_property(
        &self,
        paths: &[String],
        key: &str,
        value: Option<&str>,
    ) -> Result<Outcome, String> {
        self.notes_only(paths)?;
        if key.trim().is_empty() || key == ROOT {
            return Err("Pick a property to set".to_owned());
        }
        // The key as named in the settings, else as the note has it.
        let name = self
            .property_tree
            .key(key)
            .map_or_else(|| key.trim().to_owned(), |k| k.name.clone());
        let written = value.map(|v| {
            self.property_tree
                .value(key, v)
                .map_or_else(|| v.trim().to_owned(), |n| n.written.clone())
        });
        let mut ops = Vec::new();
        for p in paths {
            let Some(f) = self.index.file(p) else {
                continue;
            };
            let had = properties::values_of(f, key);
            let same = match (&written, &had) {
                (None, Some((_, empty, _))) => *empty,
                (Some(w), Some((parts, _, v))) => {
                    !matches!(v, PropertyValue::List(l) if l.len() > 1)
                        && parts.len() == 1
                        && parts.first().map(|p| properties::value_path(p))
                            == Some(properties::value_path(w))
                }
                _ => false,
            };
            if same {
                continue;
            }
            // The key as the note writes it, if it has one in another case.
            let key = f
                .properties
                .iter()
                .find(|(k, _)| properties::fold(k) == properties::fold(&name))
                .map_or_else(|| name.clone(), |(k, _)| k.clone());
            ops.push(FileOp::SetProperty {
                path: p.clone(),
                key,
                value: PropertyValue::Text(written.clone().unwrap_or_default()),
            });
        }
        if ops.is_empty() {
            return Err("Every note has it already".to_owned());
        }
        Ok(Outcome::Write { ops, open: None })
    }

    /// What dropping `drag` on the row of `on` does (`useDragAndDrop`):
    ///
    /// - notes on a tag tag them; on "Untagged", clear their tags; on a
    ///   property, set it;
    /// - a tag on a tag moves it under it (`a/b` on `c` is `c/b`); on the
    ///   "Tags" row, to the top (`promoteTagToRoot`);
    /// - notes on a folder move into it, or are copied with `copy` (the
    ///   copy key held: ⌥ on a Mac, Ctrl elsewhere); a folder on a folder
    ///   moves into it.
    ///
    /// # Errors
    ///
    /// What cannot go there: a tag on itself or inside itself, on
    /// "Untagged", on a property or a folder; notes on the "Tags" row; a
    /// folder into itself or anywhere but a folder; a name taken; tags and
    /// properties on anything but Markdown notes.
    pub fn drop_on(&self, drag: &Drag, on: &NavTarget, copy: bool) -> Result<Outcome, String> {
        match (drag, on) {
            (_, NavTarget::Folder(f)) => self.drop_on_folder(drag, f, copy),
            (Drag::Folder(src), _) => Err(format!("{src} goes on a folder")),
            (Drag::Notes(paths), NavTarget::Tag(t)) if t == UNTAGGED => self.clear_tags(paths),
            (Drag::Notes(_), NavTarget::Tag(t)) if t == TAGGED => {
                Err("Drop notes on a tag to tag them".to_owned())
            }
            (Drag::Notes(paths), NavTarget::Tag(t)) => self.add_tag(paths, t),
            (Drag::Notes(paths), NavTarget::Property { key, value }) => {
                self.set_property(paths, key, value.as_deref())
            }
            (Drag::Tag(src), NavTarget::Tag(t)) if t == UNTAGGED => {
                Err(format!("#{src} cannot go on Untagged"))
            }
            (Drag::Tag(src), NavTarget::Tag(t)) if t == TAGGED => {
                if !src.contains('/') {
                    return Err(format!("#{src} is at the top already"));
                }
                self.rename_tag(src, tags::leaf(src))
            }
            (Drag::Tag(src), NavTarget::Tag(t)) => {
                let (sk, tk) = (tag_key(src), tag_key(t));
                if sk == tk || is_or_below(&tk, &sk) {
                    return Err(format!("#{src} cannot move inside itself"));
                }
                let shown = self
                    .tag_tree
                    .get(t)
                    .map_or_else(|| t.clone(), |n| n.display.clone());
                let to = format!("{shown}/{}", tags::leaf(src));
                if tag_key(&to) == sk {
                    return Err(format!("#{src} is there already"));
                }
                self.rename_tag(src, &to)
            }
            (Drag::Tag(src), _) => Err(format!("#{src} goes on a tag")),
            (Drag::Shortcut(_), _) => Err("A shortcut goes among the shortcuts".to_owned()),
        }
    }

    /// What dropping `drag` among the shortcuts, at place `at` from 0,
    /// does (the shortcuts' sortable list and `handleShortcutDrop`): a
    /// shortcut moves there; notes, a folder or a tag are added there.
    /// Whether anything changed.
    pub fn drop_on_shortcuts(&mut self, drag: &Drag, at: usize) -> bool {
        match drag {
            Drag::Shortcut(from) => *from != at && self.move_shortcut(*from, at),
            Drag::Notes(paths) => {
                let mut added = false;
                for (i, p) in paths.iter().enumerate() {
                    added |=
                        self.add_shortcut(Shortcut::note(p.clone()), Some(at.saturating_add(i)));
                }
                added
            }
            Drag::Folder(f) => {
                self.add_shortcut(Shortcut::to(&NavTarget::Folder(f.clone())), Some(at))
            }
            Drag::Tag(t) => self.add_shortcut(Shortcut::to(&NavTarget::Tag(t.clone())), Some(at)),
        }
    }

    /// Every path is a Markdown note's.
    fn notes_only(&self, paths: &[String]) -> Result<(), String> {
        if paths.is_empty() {
            return Err("Select some notes first".to_owned());
        }
        paths
            .iter()
            .find(|p| !self.index.file(p).is_some_and(NoteFile::is_markdown))
            .map_or(Ok(()), |p| {
                Err(format!(
                    "{p} is not a note: only notes have tags and properties"
                ))
            })
    }

    /// What follows a write the vault did: a tag renamed or deleted takes
    /// its colours, its own list, its place in the order, whether it is
    /// open, and the selection and search with it (`handleTagRename`,
    /// `handleTagDelete`, only once the notes are done).
    pub fn after_write(&mut self, ops: &[FileOp]) {
        let mut changed = false;
        for op in ops {
            let (from, to) = match op {
                FileOp::RenameTag { from, to } => (tag_key(from), Some(to.as_str())),
                FileOp::DeleteTag(tag) => (tag_key(tag), None),
                _ => continue,
            };
            changed = true;
            let moved = |key: &str| -> Option<Option<String>> {
                is_or_below(key, &from).then(|| {
                    to.map(|to| {
                        let tail: String = key.chars().skip(from.chars().count()).collect();
                        tag_key(&format!("{to}{tail}"))
                    })
                })
            };
            let retag = |map: BTreeMap<String, String>| -> BTreeMap<String, String> {
                map.into_iter()
                    .filter_map(|(k, c)| match k.strip_prefix("tag:").map(moved) {
                        Some(Some(Some(new))) => Some((format!("tag:{new}"), c)),
                        Some(Some(None)) => None,
                        _ => Some((k, c)),
                    })
                    .collect()
            };
            self.colours = retag(std::mem::take(&mut self.colours));
            self.icons = retag(std::mem::take(&mut self.icons));
            self.backgrounds = retag(std::mem::take(&mut self.backgrounds));
            self.appearances = std::mem::take(&mut self.appearances)
                .into_iter()
                .filter_map(
                    |(k, mut a)| match k.strip_prefix("tag:").map(|t| moved(&tag_key(t))) {
                        Some(Some(Some(new))) => {
                            a.target = format!("tag:{new}");
                            Some((a.target.clone(), a))
                        }
                        Some(Some(None)) => None,
                        _ => Some((k, a)),
                    },
                )
                .collect();
            self.expanded_tags = std::mem::take(&mut self.expanded_tags)
                .into_iter()
                .filter_map(|k| moved(&k).unwrap_or(Some(k)))
                .collect();
            self.tag_order = std::mem::take(&mut self.tag_order)
                .into_iter()
                .filter_map(|k| moved(&k).unwrap_or(Some(k)))
                .collect();
            if let Some(mut q) = Query::parse(self.search.text()) {
                q.terms.retain_mut(|(_, o)| match o {
                    Operand::Tag(t) => match moved(t) {
                        Some(Some(new)) => {
                            *t = new;
                            true
                        }
                        Some(None) => false,
                        None => true,
                    },
                    Operand::Property { .. } => true,
                });
                self.search =
                    FilterSearch::parse(&q.text(), self.today(), DayMonthOrder::default());
            }
            if let NavTarget::Tag(t) = self.target()
                && let Some(new) = moved(&tag_key(&t))
            {
                let next = match (new, to) {
                    (Some(_), Some(to)) => {
                        let tail: String = t.chars().skip(from.chars().count()).collect();
                        NavTarget::Tag(format!("{to}{tail}"))
                    }
                    _ => NavTarget::Folder(String::new()),
                };
                self.selection.select_target(next, None);
            }
        }
        if changed {
            self.rebuild();
        }
    }

    /// Moves root tag `key` (or [`UNTAGGED`]) `by` places in the manual
    /// order (`rootTagOrder`).
    pub fn move_tag_root(&mut self, key: &str, by: isize) {
        let untagged = self.settings.show_untagged && self.tag_tree.untagged > 0;
        let key = if key == UNTAGGED {
            key.to_owned()
        } else {
            tag_key(key)
        };
        self.tag_order = self.tag_tree.reorder(
            &self.tag_order,
            self.settings.tag_sort,
            self.settings.include_descendants,
            untagged,
            &key,
            by,
        );
        self.rebuild_nav();
    }

    /// Moves property `key` `by` places in the manual order
    /// (`rootPropertyOrder`).
    pub fn move_property_key(&mut self, key: &str, by: isize) {
        self.property_order = self.property_tree.reorder(
            &self.property_order,
            self.settings.property_sort,
            self.settings.include_descendants,
            key,
            by,
        );
        self.rebuild_nav();
    }

    /// Forgets the manual order of the tags (A to Z, or the sort, again).
    pub fn reset_tag_order(&mut self) {
        self.tag_order.clear();
        self.rebuild_nav();
    }

    // ── Pins ───────────────────────────────────────────────────────────

    /// Pins `paths` where the selected place is a kind of (in folders, in
    /// tags), or unpins them there when the first is pinned there.
    pub fn toggle_pin(&mut self, paths: &[String]) {
        let Some(first) = paths.first() else {
            return;
        };
        let on = !self.is_pinned_here(first);
        let tag = !matches!(self.target(), NavTarget::Folder(_));
        for p in paths {
            self.set_pin(p, tag, on);
        }
        self.rebuild_list();
    }

    fn set_pin(&mut self, path: &str, tag: bool, on: bool) {
        let at = self.pins.entry(path.to_owned()).or_default();
        if tag {
            at.tag = on;
        } else {
            at.folder = on;
        }
        if !at.folder && !at.tag {
            self.pins.remove(path);
        }
    }

    /// Pins (`on`) or unpins `path` in folders.
    pub fn pin_in_folders(&mut self, path: &str, on: bool) {
        self.set_pin(path, false, on);
        self.rebuild_list();
    }

    /// Pins every folder note the list holds that is not pinned ("Pin all
    /// folder notes"): how many.
    pub fn pin_folder_notes(&mut self) -> usize {
        let target = self.target();
        let s = self.effective.clone();
        let notes: Vec<String> = match &target {
            NavTarget::Folder(f) => self.index.files_in(f, s.include_descendants),
            other => list::files_of(&self.index, other, &s, &self.pins, &self.hidden),
        }
        .into_iter()
        .filter(|n| list::is_folder_note(&self.index, &s, &n.path))
        .filter(|n| !self.pinned(&n.path).folder)
        .map(|n| n.path.clone())
        .collect();
        for n in &notes {
            self.set_pin(n, false, true);
        }
        self.rebuild_list();
        notes.len()
    }

    // ── Groups and overrides ───────────────────────────────────────────

    /// Folds or unfolds the group `key` (a heading's [`ListRowKind::Header`]
    /// `group`).
    pub fn toggle_group(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_owned());
        }
        self.rebuild_list();
    }

    /// Gives `target` its own list (an empty one lets it go); fields equal
    /// to the settings' are let go.
    pub fn set_appearance(&mut self, target: &NavTarget, appearance: Appearance) {
        let key = target_key(target);
        let mut a = appearance;
        a.target.clone_from(&key);
        let a = a.pruned(&self.settings);
        if a.is_empty() {
            self.appearances.remove(&key);
        } else {
            self.appearances.insert(key, a);
        }
        self.rebuild_list();
    }

    /// Changes the selected place's own list with `f`.
    pub fn edit_appearance(&mut self, f: impl FnOnce(&mut Appearance)) {
        let target = self.target();
        let mut a = self
            .appearance(&target)
            .cloned()
            .unwrap_or_else(|| Appearance::new(target_key(&target)));
        f(&mut a);
        self.set_appearance(&target, a);
    }

    /// Sets a tag's or a property's colour (`None` lets it go).
    pub fn set_colour(&mut self, target: &str, colour: Option<&str>) {
        let target = colour_key(target);
        match colour.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => self.colours.insert(target, c.to_owned()),
            None => self.colours.remove(&target),
        };
        self.rebuild_nav();
        self.rebuild_list();
    }

    // ── Folder notes ───────────────────────────────────────────────────

    /// Makes `path` the note of a new folder beside it ("Convert to folder
    /// note", `convertFileToFolderNote`): a folder named for it, the note
    /// moved in and named as its folder's note, then opened.
    ///
    /// # Errors
    ///
    /// Folder notes are off, there is no such note, or the folder is there.
    pub fn convert_to_folder_note(&mut self, path: &str) -> Result<Outcome, String> {
        let s = &self.settings;
        if !s.folder_notes {
            return Err("folder notes are off (Settings → Navigator)".to_owned());
        }
        let f = self
            .index
            .file(path)
            .ok_or_else(|| format!("no note {path}"))?;
        if list::is_folder_note(&self.index, s, path) {
            return Err(format!("{path} is its folder's note already"));
        }
        let up = f.folder();
        let folder = join(up, f.name());
        if self.index.has_folder(&folder) {
            return Err(format!("there is a folder {folder} already"));
        }
        let ext = f.extension();
        let to = join(
            &folder,
            &format!("{}.{ext}", list::folder_note_name(s, &folder)),
        );
        if s.pin_created_folder_note {
            self.set_pin(&to, false, true);
        }
        Ok(Outcome::Write {
            ops: vec![
                FileOp::CreateFolder(folder),
                FileOp::Move {
                    from: path.to_owned(),
                    to: to.clone(),
                },
            ],
            open: Some((to, OpenIn::Here)),
        })
    }

    /// Makes `path` its folder's note ("Set as folder note",
    /// `setFileAsFolderNote`): renamed in place to the folder note's name.
    ///
    /// # Errors
    ///
    /// Folder notes are off, it is at the root or already the note, the
    /// folder has a note, or the name is taken.
    pub fn set_as_folder_note(&mut self, path: &str) -> Result<Outcome, String> {
        let s = &self.settings;
        if !s.folder_notes {
            return Err("folder notes are off (Settings → Navigator)".to_owned());
        }
        let f = self
            .index
            .file(path)
            .ok_or_else(|| format!("no note {path}"))?;
        let folder = f.folder();
        if folder.is_empty() {
            return Err("the vault's root has no folder note".to_owned());
        }
        if list::is_folder_note(&self.index, s, path) {
            return Err(format!("{path} is its folder's note already"));
        }
        if let Some(n) = list::folder_note(&self.index, s, folder) {
            return Err(format!("{folder} has a note already: {n}"));
        }
        let to = join(
            folder,
            &format!("{}.{}", list::folder_note_name(s, folder), f.extension()),
        );
        if self.index.file(&to).is_some() {
            return Err(format!("{to} is there already"));
        }
        Ok(Outcome::Write {
            ops: vec![FileOp::Move {
                from: path.to_owned(),
                to,
            }],
            open: None,
        })
    }

    /// Lets `folder`'s note go ("Detach folder note"): renamed to `name`
    /// (Notebook Navigator asks for a name; left out, `<name> (note)`), so
    /// it is a note of the folder's like any other.
    ///
    /// # Errors
    ///
    /// The folder has no note, or the name is taken.
    pub fn detach_folder_note(
        &mut self,
        folder: &str,
        name: Option<&str>,
    ) -> Result<Outcome, String> {
        let note = self
            .folder_note(folder)
            .ok_or_else(|| format!("{folder} has no folder note"))?
            .to_owned();
        let f = self
            .index
            .file(&note)
            .ok_or_else(|| format!("no note {note}"))?;
        let ext = f.extension();
        let base = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(
            || format!("{} (note)", f.name()),
            |n| n.strip_suffix(&format!(".{ext}")).unwrap_or(n).to_owned(),
        );
        let to = join(folder, &format!("{base}.{ext}"));
        if self.index.file(&to).is_some() {
            return Err(format!("{to} is there already"));
        }
        Ok(Outcome::Write {
            ops: vec![FileOp::Move { from: note, to }],
            open: None,
        })
    }

    /// Opens `folder`'s note, in a new tab when `tab`, else where the
    /// settings say (`folderNoteOpenLocation`).
    #[must_use]
    pub fn open_folder_note(&self, folder: &str, tab: bool) -> Option<Outcome> {
        let note = self.folder_note(folder)?;
        let how = if tab || self.settings.folder_note_open == FolderNoteOpen::NewTab {
            OpenIn::Tab
        } else {
            OpenIn::Here
        };
        Some(Outcome::Open {
            path: note.to_owned(),
            how,
            settle: false,
        })
    }

    // ── The manual order ───────────────────────────────────────────────

    /// Moves the selected notes `by` places in the manual order (⌘↑, ⌘↓;
    /// `moveManualSortSelectionByDirection`): every note of the list gets
    /// its rank, `(place + 1) × 1000`, in its manual-sort property; only
    /// the ranks that change are written. `None` unless sorted by hand.
    #[must_use]
    pub fn move_in_order(&self, by: isize) -> Option<Outcome> {
        let s = &self.effective;
        if s.sort != SortOption::Manual || by == 0 {
            return None;
        }
        let mut order: Vec<String> = self
            .list_order()
            .into_iter()
            .filter(|p| self.index.file(p).is_some_and(NoteFile::is_markdown))
            .map(str::to_owned)
            .collect();
        let selected = self.selection.files();
        let picked: Vec<usize> = (0..order.len())
            .filter(|i| order.get(*i).is_some_and(|p| selected.contains(p)))
            .collect();
        if picked.is_empty() {
            return None;
        }
        // Up: from the top; down: from the bottom, so a block moves whole.
        let mut seq = picked;
        if by > 0 {
            seq.reverse();
        }
        let last = order.len().saturating_sub(1);
        for _ in 0..by.unsigned_abs() {
            for i in &mut seq {
                let to = if by > 0 {
                    i.saturating_add(1).min(last)
                } else {
                    i.saturating_sub(1)
                };
                let blocked = order.get(to).is_some_and(|p| selected.contains(p)) && to != *i;
                if to != *i && !blocked {
                    order.swap(*i, to);
                    *i = to;
                }
            }
        }
        let key = s.manual_sort_property.clone();
        let mut rank = 0.0_f64;
        let ops: Vec<FileOp> = order
            .iter()
            .filter_map(|p| {
                rank += list::RANK_STEP;
                let f = self.index.file(p)?;
                (list::rank(f, &key) != Some(rank)).then(|| FileOp::SetProperty {
                    path: p.clone(),
                    key: key.clone(),
                    value: PropertyValue::Number(rank),
                })
            })
            .collect();
        (!ops.is_empty()).then_some(Outcome::Write { ops, open: None })
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

    /// The keyboard map: the settings' (`vim_keys`) with a person's own
    /// over them.
    #[must_use]
    pub const fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// A person's own keys.
    #[must_use]
    pub fn key_bindings(&self) -> &[KeyBinding] {
        &self.key_bindings
    }

    /// Binds `action` to `keys` (hotkeys as text, `Mod+Shift+P`; none
    /// turns it off), or with `None` back to its default.
    pub fn bind_keys(&mut self, action: KeyAction, keys: Option<Vec<String>>) {
        self.key_bindings.retain(|b| b.action != action.id());
        if let Some(keys) = keys {
            self.key_bindings.push(KeyBinding {
                action: action.id().to_owned(),
                keys,
            });
        }
        self.keymap = KeyMap::new(self.settings.vim_keys, &self.key_bindings);
    }

    /// Does key action `action` (what the keyboard map read a key as):
    /// in the pane with the keyboard for `pane:` actions, else in its own
    /// pane. The ones the view does (rename, delete, the search box's)
    /// do nothing here.
    pub fn run_key(&mut self, action: KeyAction) -> Option<Outcome> {
        use KeyAction as A;
        let (key, mods) = match action {
            A::PaneMoveUp => (Key::Up, Mods::NONE),
            A::PaneMoveDown => (Key::Down, Mods::NONE),
            A::PanePageUp => (Key::PageUp, Mods::NONE),
            A::PanePageDown => (Key::PageDown, Mods::NONE),
            A::PaneHome => (Key::Home, Mods::NONE),
            A::PaneEnd => (Key::End, Mods::NONE),
            A::NavCollapseOrParent => {
                self.nav_key(Key::Left, Mods::NONE);
                return None;
            }
            A::NavExpandOrFocusList => {
                self.nav_key(Key::Right, Mods::NONE);
                return None;
            }
            A::NavFocusList | A::SearchFocusList => {
                self.focus_pane(Pane::Files);
                return None;
            }
            A::ListFocusNavigation | A::SearchFocusNavigation => {
                self.focus_pane(Pane::Navigation);
                return None;
            }
            A::ListFocusEditor => return Some(Outcome::FocusEditor),
            A::ListSelectAll => return self.list_key(Key::A, Mods::COMMAND),
            A::ListExtendSelectionUp => return self.list_key(Key::Up, Mods::SHIFT),
            A::ListExtendSelectionDown => return self.list_key(Key::Down, Mods::SHIFT),
            A::ListRangeToStart => return self.list_key(Key::Home, Mods::SHIFT),
            A::ListRangeToEnd => return self.list_key(Key::End, Mods::SHIFT),
            A::ListManualSortUp => return self.move_in_order(-1),
            A::ListManualSortDown => return self.move_in_order(1),
            A::ListOpen => return self.list_key(Key::Enter, Mods::NONE),
            A::ListOpenTab => return self.list_key(Key::Enter, Mods::SHIFT),
            A::ListOpenSplit => return self.list_key(Key::Enter, Mods::COMMAND),
            A::ListPin => return self.list_key(Key::Pin, Mods::COMMAND),
            A::PaneRename | A::DeleteSelected | A::SearchClose => return None,
        };
        self.key(key, mods)
    }

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
        let target = self.selection.target().cloned();
        let current = target.as_ref().and_then(|t| {
            self.nav_rows
                .iter()
                .position(|r| r.target().as_ref() == Some(t))
        });
        let at = current.and_then(|c| selectable.iter().position(|i| *i == c));
        let go = |s: &mut Self, to: Option<usize>| {
            if let Some(row) = to.and_then(|t| selectable.get(t)).copied()
                && let Some(t) = s.nav_rows.get(row).and_then(NavRow::target)
            {
                s.select_nav(&t);
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
                match (
                    row.as_ref().and_then(NavRow::expansion),
                    row.and_then(|r| r.target()),
                ) {
                    (Some(false), Some(t)) => self.set_nav_expanded(&t, true),
                    _ => self.focus_pane(Pane::Files),
                }
            }
            Key::Left => {
                let row = current.and_then(|c| self.nav_rows.get(c)).cloned();
                if let Some(r) = &row
                    && !matches!(r.kind, NavRowKind::Folder { .. })
                {
                    if r.expansion() == Some(true)
                        && let Some(t) = r.target()
                    {
                        self.set_nav_expanded(&t, false);
                    } else if let Some(c) = current {
                        // To the row it is under.
                        let level = r.level;
                        let up = self.nav_rows.get(..c).and_then(|rows| {
                            rows.iter()
                                .rev()
                                .find(|p| p.level < level && p.selectable())
                        });
                        if let Some(t) = up.and_then(NavRow::target) {
                            self.select_nav(&t);
                        }
                    }
                } else if let Some(NavRow {
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
        if mods.command
            && !mods.shift
            && matches!(key, Key::Up | Key::Down)
            && self.effective.sort == SortOption::Manual
        {
            return self.move_in_order(if key == Key::Down { 1 } else { -1 });
        }
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
            Key::Pin => {
                let picked: Vec<String> = self.selection.files().iter().cloned().collect();
                self.toggle_pin(&picked);
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

    // ── The calendar and periodic notes ────────────────────────────────

    /// The periodic notes' paths, as the settings write them.
    #[must_use]
    pub const fn periodic(&self) -> &PeriodicNotes {
        &self.periodic
    }

    /// Today, in the person's time zone.
    #[must_use]
    pub fn today(&self) -> NaiveDate {
        day_of(self.clock.now, self.clock.offset)
    }

    /// The day the calendar is around.
    #[must_use]
    pub fn calendar_cursor(&self) -> NaiveDate {
        self.calendar_cursor.unwrap_or_else(|| self.today())
    }

    /// What the calendar draws.
    #[must_use]
    pub fn calendar(&self) -> CalendarGrid {
        calendar::grid(
            CalendarContext {
                index: &self.index,
                periodic: &self.periodic,
                today: self.today(),
                active: self.active.as_deref(),
            },
            &self.settings,
            self.calendar_cursor(),
        )
    }

    /// Whether `date` is among the days the calendar shows.
    fn calendar_shows(&self, date: NaiveDate) -> bool {
        let (start, rows) = calendar::week_window(
            self.calendar_cursor(),
            self.settings.calendar_weeks,
            self.periodic.week_rules(),
        );
        let end = crate::period::add_days(start, i64::from(rows).saturating_mul(7));
        start <= date && date < end
    }

    /// Shows or hides the calendar (`toggle-calendar`); it is not shown
    /// with the calendar off in the settings.
    pub const fn toggle_calendar(&mut self) {
        self.calendar_open = !self.calendar_open;
    }

    /// Whether the calendar is drawn: on in the settings, and shown.
    #[must_use]
    pub const fn calendar_visible(&self) -> bool {
        self.settings.calendar && self.calendar_open
    }

    /// The calendar `delta` pages on (back, when negative): a month at a
    /// time on a whole month, else as many weeks as show.
    pub fn calendar_move(&mut self, delta: i32) {
        self.calendar_cursor = Some(calendar::step(
            self.calendar_cursor(),
            delta,
            self.settings.calendar_weeks,
        ));
    }

    /// The calendar around `date`.
    pub const fn calendar_go(&mut self, date: NaiveDate) {
        self.calendar_cursor = Some(date);
    }

    /// The calendar back on today (its Today button); today's note opens
    /// if there is one (`handleToday`), and none is made.
    pub fn calendar_today(&mut self) -> Option<Outcome> {
        let today = self.today();
        self.calendar_cursor = Some(today);
        let period = self.periodic.period(PeriodKind::Day, today);
        let path = self.periodic.path(period)?;
        self.index.file(&path).is_some().then_some(Outcome::Open {
            path,
            how: OpenIn::Here,
            settle: false,
        })
    }

    /// Opens `period`'s note, or makes it (`openOrCreateCustomCalendarNote`):
    /// an [`Outcome::Open`] when the vault has it, else an
    /// [`Outcome::Write`] that makes it ([`FileOp::CreateFile`], a heading
    /// until templates come) and opens it.
    ///
    /// # Errors
    ///
    /// Notes of its kind are off, or their pattern makes no path.
    pub fn open_period(&self, period: Period, how: OpenIn) -> Result<Outcome, String> {
        let path = self
            .periodic
            .path(period)
            .ok_or_else(|| self.periodic.why_not(period.kind).unwrap_or_default())?;
        if self.index.file(&path).is_some() {
            return Ok(Outcome::Open {
                path,
                how,
                settle: false,
            });
        }
        if self.index.folder(&path).is_some() {
            return Err(format!("{path} is a folder"));
        }
        let text = self.periodic_text(period.kind, &path, period.start);
        Ok(Outcome::Write {
            ops: vec![FileOp::CreateFile {
                path: path.clone(),
                text,
            }],
            open: Some((path, how)),
        })
    }

    /// Opens, or makes, the note of `kind` holding `date`
    /// (`open-daily-note`, `open-weekly-note`, …, for today).
    ///
    /// # Errors
    ///
    /// As [`open_period`](Self::open_period).
    pub fn open_periodic(
        &self,
        kind: PeriodKind,
        date: NaiveDate,
        how: OpenIn,
    ) -> Result<Outcome, String> {
        self.open_period(self.periodic.period(kind, date), how)
    }

    /// The note open, as a periodic note: its period.
    #[must_use]
    pub fn active_period(&self) -> Option<Period> {
        self.active
            .as_deref()
            .and_then(|p| self.periodic.period_of(p))
    }

    /// Opens, or makes, the note `by` periods on from the one open
    /// (`open-next-periodic-note`, `open-previous-periodic-note`).
    ///
    /// # Errors
    ///
    /// The note open is no periodic note.
    pub fn open_adjacent_period(&self, by: i32, how: OpenIn) -> Result<Outcome, String> {
        let p = self.active_period().ok_or(NOT_PERIODIC)?;
        self.open_period(p.shift(by), how)
    }

    /// Opens, or makes, the note of the period the open one is in: a
    /// day's week, a week's month (the month of its first day), …,
    /// skipping kinds that are off (`open-parent-periodic-note`).
    ///
    /// # Errors
    ///
    /// The note open is no periodic note, or a year's.
    pub fn open_parent_period(&self, how: OpenIn) -> Result<Outcome, String> {
        let p = self.active_period().ok_or(NOT_PERIODIC)?;
        let parent = self
            .periodic
            .parent(p)
            .ok_or("A yearly note has no parent period")?;
        self.open_period(parent, how)
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
            pinned: self
                .pins
                .iter()
                .map(|(path, at)| Pin {
                    path: path.clone(),
                    folder: at.folder,
                    tag: at.tag,
                })
                .collect(),
            collapsed: self.collapsed.iter().cloned().collect(),
            appearances: self.appearances.values().cloned().collect(),
            colours: self
                .colours
                .iter()
                .map(|(target, colour)| Colour {
                    target: target.clone(),
                    colour: colour.clone(),
                })
                .collect(),
            calendar: self.calendar_open,
            expanded_tags: self.expanded_tags.iter().cloned().collect(),
            expanded_properties: self.expanded_props.iter().cloned().collect(),
            tag_order: self.tag_order.clone(),
            property_order: self.property_order.clone(),
            search: self.search.text().to_owned(),
            search_open: self.search_open,
            profiles: self.profiles.clone(),
            show_hidden: self.show_hidden,
            shortcuts_folded: self.shortcuts_folded,
            recent_folded: self.recent_folded,
            templates: self.templates.clone(),
            icons: self
                .icons
                .iter()
                .map(|(target, icon)| IconAt {
                    target: target.clone(),
                    icon: icon.clone(),
                })
                .collect(),
            backgrounds: self
                .backgrounds
                .iter()
                .map(|(target, colour)| Colour {
                    target: target.clone(),
                    colour: colour.clone(),
                })
                .collect(),
            keys: self.key_bindings.clone(),
        }
    }

    /// Puts back what [`view_state`](Self::view_state) kept; folders that
    /// are gone are let go.
    pub fn restore(&mut self, state: &ViewState) {
        self.profiles = state.profiles.clone();
        self.show_hidden = state.show_hidden;
        self.shortcuts_folded = state.shortcuts_folded;
        self.recent_folded = state.recent_folded;
        self.templates = state.templates.clone();
        self.icons = state
            .icons
            .iter()
            .map(|i| (i.target.clone(), i.icon.clone()))
            .collect();
        self.backgrounds = state
            .backgrounds
            .iter()
            .map(|c| (c.target.clone(), c.colour.clone()))
            .collect();
        self.key_bindings.clone_from(&state.keys);
        self.keymap = KeyMap::new(self.settings.vim_keys, &self.key_bindings);
        self.search_open = state.search_open || !state.search.is_empty();
        self.search = FilterSearch::parse(&state.search, self.today(), DayMonthOrder::default());
        self.hidden = Hidden::new(&self.profiles.active().rules(), self.show_hidden);
        self.index = self.visible();
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
        self.pins = state
            .pinned
            .iter()
            .map(|p| {
                (
                    p.path.clone(),
                    PinIn {
                        folder: p.folder,
                        tag: p.tag,
                    },
                )
            })
            .collect();
        self.collapsed = state.collapsed.iter().cloned().collect();
        self.appearances = state
            .appearances
            .iter()
            .map(|a| (a.target.clone(), a.clone()))
            .collect();
        self.colours = state
            .colours
            .iter()
            .map(|c| (c.target.clone(), c.colour.clone()))
            .collect();
        self.calendar_open = state.calendar;
        // Kept from before there were tags: the rows open as at first.
        if !state.expanded_tags.is_empty() {
            self.expanded_tags = state.expanded_tags.iter().cloned().collect();
        }
        if !state.expanded_properties.is_empty() {
            self.expanded_props = state.expanded_properties.iter().cloned().collect();
        }
        self.tag_order.clone_from(&state.tag_order);
        self.property_order.clone_from(&state.property_order);
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

/// What [`NavigatorState::hide`] hides: a folder, a tag, a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HideTarget {
    Folder(String),
    Tag(String),
    Note(String),
}

/// A section's header row.
fn section(id: &str, name: &str, expanded: bool, count: usize) -> NavRow {
    NavRow {
        key: format!("section:{id}"),
        level: 0,
        kind: NavRowKind::Section {
            id: id.to_owned(),
            name: name.to_owned(),
            expanded,
            count,
        },
    }
}

/// Why there is no period to move from.
const NOT_PERIODIC: &str = "The note open is not a periodic note";

/// `name` in `folder` (the root: `name` alone).
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A folder row's key.
#[must_use]
pub fn folder_key(path: &str) -> String {
    format!("folder:{path}")
}

/// A tag row's key (the "Tags" and "Untagged" rows' too).
#[must_use]
pub fn tag_row_key(tag: &str) -> String {
    format!("tag:{}", expansion_key(tag))
}

/// A property row's key.
#[must_use]
pub fn property_row_key(key: &str, value: Option<&str>) -> String {
    match value {
        _ if key == ROOT => format!("property:{ROOT}"),
        Some(v) => format!("property:{}", properties::value_id(key, v)),
        None => format!("property:{}", properties::key_id(key)),
    }
}

/// What the open tags hold for `tag`: its key, or the row's id.
fn expansion_key(tag: &str) -> String {
    if tags::is_virtual(tag) {
        tag.to_owned()
    } else {
        tag_key(tag)
    }
}

/// What the open properties hold for `key`: its id, or [`ROOT`].
fn prop_expansion(key: &str) -> String {
    if key == ROOT {
        ROOT.to_owned()
    } else {
        properties::key_id(key)
    }
}

/// A colour's target as kept: `tag:<key>`, `property:<key>[=<value>]`
/// folded (Notebook Navigator keys colours by the lower-case path).
#[must_use]
pub fn colour_key(target: &str) -> String {
    if let Some(t) = target.strip_prefix("tag:") {
        return format!("tag:{}", tag_key(t));
    }
    if let Some(p) = target.strip_prefix("property:") {
        let p = p.strip_prefix("key:").unwrap_or(p);
        return match p.split_once('=') {
            Some((k, v)) => format!(
                "property:{}={}",
                properties::fold(k),
                properties::value_path(v)
            ),
            None => format!("property:{}", properties::fold(p)),
        };
    }
    target.to_owned()
}

/// One write, nothing opened after.
fn write(op: FileOp) -> Outcome {
    Outcome::Write {
        ops: vec![op],
        open: None,
    }
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
