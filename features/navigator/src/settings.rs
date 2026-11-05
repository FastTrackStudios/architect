//! The navigator's settings: one section (`navigator`) on architect's
//! settings page, declared as a struct.
//!
//! Names and defaults are Notebook Navigator's (`src/settings/types.ts`,
//! `src/settings/defaultSettings.ts`): `dualPane` true, `dualPaneOrientation`
//! horizontal, `narrowSidebarLayout` single pane, `startView` files,
//! `paneTransitionDuration` 150 ms, `showRootFolder` true, `showNoteCount`
//! and `separateNoteCounts` true, `autoRevealActiveFile` and
//! `autoRevealShortestPath` true, `smartCollapse` true, `navItemHeight` 28,
//! `navIndent` 16, `defaultFolderSort` modified, newest first,
//! `noteGrouping` date, `showFilePreview` true, `previewRows` 2,
//! `fileNameRows` 1, `showFileDate` and `showParentFolder` true,
//! `defaultListMode` standard, `compactItemHeight` 28.

use architect_settings::Settings;
use facet::Facet;

/// How the two panes sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum Orientation {
    /// Side by side
    #[default]
    Horizontal,
    /// One above the other
    Vertical,
}

/// What a narrow sidebar does with two side-by-side panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum NarrowLayout {
    /// Keep both
    Keep,
    /// One pane at a time
    #[default]
    SinglePane,
    /// One above the other
    Vertical,
}

/// The pane a single-pane navigator opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum StartPane {
    /// Folders
    Navigation,
    /// Notes
    #[default]
    Files,
}

/// How the list is ordered (Notebook Navigator's `SortOption`; by property
/// comes with properties).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum SortOption {
    /// Edited, newest first
    #[default]
    ModifiedDesc,
    /// Edited, oldest first
    ModifiedAsc,
    /// Created, newest first
    CreatedDesc,
    /// Created, oldest first
    CreatedAsc,
    /// Title, A to Z
    TitleAsc,
    /// Title, Z to A
    TitleDesc,
    /// File name, A to Z
    FilenameAsc,
    /// File name, Z to A
    FilenameDesc,
}

impl SortOption {
    /// Every option, in the menu's order.
    pub const ALL: [Self; 8] = [
        Self::ModifiedDesc,
        Self::ModifiedAsc,
        Self::CreatedDesc,
        Self::CreatedAsc,
        Self::TitleAsc,
        Self::TitleDesc,
        Self::FilenameAsc,
        Self::FilenameDesc,
    ];

    /// What the menu calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ModifiedDesc => "Date edited (newest on top)",
            Self::ModifiedAsc => "Date edited (oldest on top)",
            Self::CreatedDesc => "Date created (newest on top)",
            Self::CreatedAsc => "Date created (oldest on top)",
            Self::TitleAsc => "Title (A on top)",
            Self::TitleDesc => "Title (Z on top)",
            Self::FilenameAsc => "File name (A on top)",
            Self::FilenameDesc => "File name (Z on top)",
        }
    }

    /// Its id, as Notebook Navigator writes it: `modified-desc`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::ModifiedDesc => "modified-desc",
            Self::ModifiedAsc => "modified-asc",
            Self::CreatedDesc => "created-desc",
            Self::CreatedAsc => "created-asc",
            Self::TitleAsc => "title-asc",
            Self::TitleDesc => "title-desc",
            Self::FilenameAsc => "filename-asc",
            Self::FilenameDesc => "filename-desc",
        }
    }

    /// The option of an id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Whether it orders by a date (and so may group by one).
    #[must_use]
    pub const fn by_date(self) -> bool {
        matches!(
            self,
            Self::ModifiedDesc | Self::ModifiedAsc | Self::CreatedDesc | Self::CreatedAsc
        )
    }

    /// Whether the date a row shows is the created one.
    #[must_use]
    pub const fn shows_created(self) -> bool {
        matches!(self, Self::CreatedDesc | Self::CreatedAsc)
    }
}

/// How the list is grouped (Notebook Navigator's `noteGrouping`; by folder
/// and by property come with the full list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum Grouping {
    /// No groups
    None,
    /// By date (Today, Yesterday, …), when sorted by a date
    #[default]
    Date,
}

/// How tall a note's row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum ListMode {
    /// Standard: name, preview, date and folder
    #[default]
    Standard,
    /// Compact: the name alone
    Compact,
}

/// Navigator
/// The sidebar's folders and notes.
#[derive(Debug, Clone, PartialEq, Eq, Facet, Settings)]
#[settings(id = "navigator", icon = "folder-tree")]
pub struct NavigatorSettings {
    /// Two panes
    /// Folders and notes side by side (or stacked); off, one at a time.
    pub dual_pane: bool,
    /// Orientation
    #[setting(control = segmented)]
    pub orientation: Orientation,
    /// In a narrow sidebar
    /// When the sidebar is narrower than the folders plus 150 points.
    pub narrow_layout: NarrowLayout,
    /// Start on
    /// The pane one-at-a-time shows first.
    #[setting(control = segmented)]
    pub start_pane: StartPane,
    /// Pane transition
    /// How long one pane takes to slide over the other.
    #[setting(control = slider(50..=350, step = 10), unit = " ms")]
    pub transition_ms: u32,
    /// Show the vault's root
    /// A row for the vault itself, over its folders.
    pub show_root: bool,
    /// Note counts
    pub show_note_count: bool,
    /// Separate counts
    /// A folder's own notes, then those in its folders below ("3 • 12").
    pub separate_note_counts: bool,
    /// Reveal the open note
    /// Open its folders and scroll to it as it opens.
    pub auto_reveal: bool,
    /// Reveal by the shortest path
    /// With notes from folders below, stop at the nearest open folder.
    pub reveal_shortest_path: bool,
    /// Smart collapse
    /// Collapse all keeps the selected folder in view.
    pub smart_collapse: bool,
    /// Folder row height
    #[setting(control = slider(20..=40, step = 1), unit = " pt")]
    pub item_height: u32,
    /// Indent
    #[setting(control = slider(10..=24, step = 1), unit = " pt")]
    pub indent: u32,
    /// Sort by
    pub sort: SortOption,
    /// Group by
    #[setting(control = segmented)]
    pub grouping: Grouping,
    /// Notes from folders below
    /// A folder lists the notes of every folder inside it too.
    pub include_descendants: bool,
    /// Note row
    #[setting(control = segmented)]
    pub mode: ListMode,
    /// Title lines
    #[setting(control = stepper(1..=3))]
    pub title_rows: u32,
    /// Preview
    /// The start of each note under its name.
    pub show_preview: bool,
    /// Preview lines
    #[setting(control = stepper(1..=5))]
    pub preview_rows: u32,
    /// Date
    pub show_date: bool,
    /// Folder
    /// The note's folder, when it is not the one selected.
    pub show_parent_folder: bool,
    /// Title from frontmatter
    /// A note's `title` property over its file name.
    pub title_from_frontmatter: bool,
    /// Compact row height
    #[setting(control = slider(20..=40, step = 1), unit = " pt")]
    pub compact_height: u32,
}

impl Default for NavigatorSettings {
    fn default() -> Self {
        Self {
            dual_pane: true,
            orientation: Orientation::Horizontal,
            narrow_layout: NarrowLayout::SinglePane,
            start_pane: StartPane::Files,
            transition_ms: 150,
            show_root: true,
            show_note_count: true,
            separate_note_counts: true,
            auto_reveal: true,
            reveal_shortest_path: true,
            smart_collapse: true,
            item_height: 28,
            indent: 16,
            sort: SortOption::ModifiedDesc,
            grouping: Grouping::Date,
            include_descendants: false,
            mode: ListMode::Standard,
            title_rows: 1,
            show_preview: true,
            preview_rows: 2,
            show_date: true,
            show_parent_folder: true,
            title_from_frontmatter: false,
            compact_height: 28,
        }
    }
}
