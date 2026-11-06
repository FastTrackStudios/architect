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
//! `defaultListMode` standard, `compactItemHeight` 28; the list pane in
//! full: `propertyGroupKey` empty, `showGroupHeaderItemCounts` false,
//! `showFolderGroupPaths` true, `filterPinnedByFolder` false,
//! `showFeatureImage` true, `featureImageSize` 64, `forceSquareFeatureImage`
//! true, `hideDrawingPreviewImages` true, `showFileTags` true,
//! `showFileTagAncestors` false, `colorFileTags` true,
//! `showFileTagsInCompactMode` false, `showFileProperties` true,
//! `showFileTaskProgress` true, `hideFileTaskProgressWhenComplete` false,
//! `textCountDisplay` none, `textCountPlacement` title,
//! `useFrontmatterMetadata` false, `manualSortPropertyKey` `sort_index`,
//! `enableFolderNotes` false, `folderNoteNamePattern` `{{folder}}`,
//! `hideFolderNoteInList` true, `pinCreatedFolderNote` false,
//! `folderNoteOpenLocation` current tab; the calendar and the periodic
//! notes: `calendarEnabled` true, `calendarConfirmBeforeCreate` true,
//! `calendarWeeksToShow` 1, `calendarLeftPlacement` navigation,
//! `calendarWeekendDays` sat-sun, `calendarMonthHeadingFormat` full,
//! `calendarHighlightToday` true, `calendarShowOutsideMonthDays` true,
//! `calendarShowFeatureImage` true, `calendarShowTasks` true,
//! `calendarShowWeekNumber` and `calendarShowQuarter` false,
//! `periodicNotesFolder` empty, and the patterns `YYYY/YYYYMMDD`,
//! `gggg/[W]ww`, `YYYY/YYYYMM`, `YYYY/[Q]Q` and `YYYY`. Its
//! `calendarLocale` (system default, Obsidian's `en`: weeks from Sunday)
//! is [`WeekStart`]; month and day names are English.
//! The search, shortcuts and recent notes: `searchProvider` internal (the
//! filter search), `showShortcuts` true, `showRecentNotes` true,
//! `recentNotesCount` 5.
//! Files and templates: `confirmBeforeDelete` true, `springLoadedFolders`
//! true with `springLoadedFoldersInitialDelay` and
//! `springLoadedFoldersSubsequentDelay` 0.5 s, `dateFormat` `MMM D, YYYY`,
//! `timeFormat` `h:mm a`, `calendarTemplateFolder` empty, and no folder
//! note or periodic note templates (`folderNoteTemplate`,
//! `calendarCustomFileTemplate` and the week's to the year's).
//! Icons and colours: `showFolderIcons`, `inheritFolderColors`,
//! `showTagIcons`, `inheritTagColors`, `showPropertyIcons`,
//! `inheritPropertyColors` and `showFileIcons` true; `colorIconOnly`,
//! `showFilenameMatchIcons` and `showCategoryIcons` false with empty maps;
//! `frontmatterIconField` `icon`, `frontmatterColorField` `color`,
//! `frontmatterBackgroundField` `background`.
//! The tags and the property browser: `showTags` true, `showAllTagsFolder`
//! true, `showUntagged` true, `scopeTagsToCurrentContext` false,
//! `tagSortOrder` alpha-asc; `showProperties` true, the profile's
//! `propertyKeys` none (so no keys show until some are named),
//! `scopePropertiesToCurrentContext` false, `propertySortOrder`
//! alpha-asc.
//!
//! What a person sets for one folder or tag (its sort, grouping, rows,
//! mode, notes from below) is not here but in [`crate::Appearance`],
//! kept with the view's state.

// The derived section is one long list of settings, built at once.
#![allow(clippy::large_stack_arrays)]

use architect_settings::Settings;
use facet::Facet;

/// What the list's search box searches with (`searchProvider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum SearchProvider {
    /// The filter search: names, tags, properties, dates, folders
    #[default]
    Filter,
    /// The filter search, its words in the notes' text
    FullText,
}

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
    /// By hand: the notes' `sort_index` (the manual-sort property)
    Manual,
}

impl SortOption {
    /// Every option, in the menu's order.
    pub const ALL: [Self; 9] = [
        Self::ModifiedDesc,
        Self::ModifiedAsc,
        Self::CreatedDesc,
        Self::CreatedAsc,
        Self::TitleAsc,
        Self::TitleDesc,
        Self::FilenameAsc,
        Self::FilenameDesc,
        Self::Manual,
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
            Self::Manual => "Manual (⌘↑ ⌘↓ to move)",
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
            Self::Manual => "manual",
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

/// How the list is grouped (Notebook Navigator's `noteGrouping`; its
/// `custom`, the manual sort's header notes, is not ported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum Grouping {
    /// No groups
    None,
    /// By date (Today, Yesterday, …), when sorted by a date
    #[default]
    Date,
    /// By folder, with notes from folders below
    Folder,
    /// By a property's value (the "Group by property" key)
    Property,
}

impl Grouping {
    pub const ALL: [Self; 4] = [Self::None, Self::Date, Self::Folder, Self::Property];

    /// Its id: `none`, `date`, `folder`, `property`.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Date => "date",
            Self::Folder => "folder",
            Self::Property => "property",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }

    /// What the menu calls it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No groups",
            Self::Date => "Date",
            Self::Folder => "Folder",
            Self::Property => "Property",
        }
    }

    /// The grouping a list shows under `sort` (`resolveEffectiveListGroupingForSort`):
    /// none under a manual sort; by date only when sorted by a date.
    #[must_use]
    pub const fn under(self, sort: SortOption) -> Self {
        match (self, sort) {
            (_, SortOption::Manual) => Self::None,
            (Self::Date, s) if !s.by_date() => Self::None,
            (g, _) => g,
        }
    }
}

/// What a note's row counts in its text (`textCountDisplay`; characters
/// wait on a source that counts them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum TextCount {
    /// Nothing
    #[default]
    None,
    /// Words
    Words,
}

/// Where a note's count shows (`textCountPlacement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum CountPlacement {
    /// Beside its name
    #[default]
    Title,
    /// With its properties
    Property,
}

/// Where a folder's note opens (`folderNoteOpenLocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum FolderNoteOpen {
    /// The tab on show
    #[default]
    CurrentTab,
    /// A new tab
    NewTab,
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

/// The day a week starts on, and so how weeks are numbered (Notebook
/// Navigator's `calendarLocale`, as Moment's locales number weeks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum WeekStart {
    /// Sunday (week 1 holds January 1, as in the United States)
    #[default]
    Sunday,
    /// Monday (ISO 8601: week 1 holds January 4)
    Monday,
    /// Saturday (week 1 holds January 1)
    Saturday,
}

/// The days the calendar shades as the weekend (`calendarWeekendDays`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum WeekendDays {
    /// None
    None,
    /// Saturday and Sunday
    #[default]
    SatSun,
    /// Friday and Saturday
    FriSat,
    /// Thursday and Friday
    ThuFri,
}

/// How the calendar names its month (`calendarMonthHeadingFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum MonthFormat {
    /// January
    #[default]
    Full,
    /// Jan
    Short,
}

/// Where the calendar goes when one pane shows at a time
/// (`calendarLeftPlacement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum CalendarPlacement {
    /// Under the folders
    #[default]
    Navigation,
    /// Under both panes
    Below,
}

/// How tags or property keys are ordered (`tagSortOrder`,
/// `propertySortOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Facet)]
#[repr(u8)]
pub enum TreeSort {
    /// A to Z
    #[default]
    AlphaAsc,
    /// Z to A
    AlphaDesc,
    /// Fewest notes first
    FrequencyAsc,
    /// Most notes first
    FrequencyDesc,
}

impl TreeSort {
    pub const ALL: [Self; 4] = [
        Self::AlphaAsc,
        Self::AlphaDesc,
        Self::FrequencyAsc,
        Self::FrequencyDesc,
    ];

    /// Notebook Navigator's name for it: `alpha-asc`, …
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::AlphaAsc => "alpha-asc",
            Self::AlphaDesc => "alpha-desc",
            Self::FrequencyAsc => "frequency-asc",
            Self::FrequencyDesc => "frequency-desc",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
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
    pub grouping: Grouping,
    /// Group by property
    /// The frontmatter property "Group by: Property" groups notes by; notes
    /// without it go last, under "None".
    #[setting(placeholder = "status")]
    pub group_property: String,
    /// Count in group headings
    /// How many notes each group holds, beside its heading.
    pub show_group_counts: bool,
    /// Folder groups show paths
    /// A folder group's heading is its path from the folder selected.
    pub show_folder_group_paths: bool,
    /// Only pin notes in their folder
    /// A pinned note shows pinned in its own folder, not in folders above.
    pub filter_pinned_by_folder: bool,
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
    /// Compact row height
    #[setting(control = slider(20..=40, step = 1), unit = " pt")]
    pub compact_height: u32,
    /// Feature images
    /// A note's image beside it; a PDF's cover, an SVG, a drawing's preview.
    pub show_feature_image: bool,
    /// Image properties
    /// Frontmatter properties that name a note's image, first set wins.
    #[setting(placeholder = "cover, image")]
    pub feature_image_properties: String,
    /// Image size
    #[setting(control = slider(64..=128, step = 32), unit = " pt")]
    pub feature_image_size: u32,
    /// Square images
    pub force_square_image: bool,
    /// Hide drawing previews
    /// A drawing's PNG preview is not listed beside the drawing.
    pub hide_drawing_previews: bool,
    /// Tags
    /// A note's tags under it; a click goes to the tag.
    pub show_tags: bool,
    /// Tags' parents
    /// `projects/alpha`, not `alpha`.
    pub show_tag_ancestors: bool,
    /// Coloured tags
    /// Tags with a colour show in it, and first.
    pub color_tags: bool,
    /// Tags in compact rows
    pub show_tags_compact: bool,
    /// Properties
    /// The values of the properties listed below, under a note.
    pub show_properties: bool,
    /// Properties shown
    /// Frontmatter keys whose values show on a note's row.
    #[setting(placeholder = "status, priority")]
    pub list_properties: String,
    /// Task progress
    /// Done of all of a note's tasks, with a bar.
    pub show_task_progress: bool,
    /// Hide when all done
    pub hide_task_progress_complete: bool,
    /// Count
    /// A note's words, beside its name or with its properties.
    pub text_count: TextCount,
    /// Count shows
    #[setting(control = segmented)]
    pub text_count_placement: CountPlacement,
    /// Names and dates from frontmatter
    /// A note's name, created and modified times from its properties.
    pub use_frontmatter: bool,
    /// Name property
    /// Properties that hold a note's name, first set wins.
    #[setting(placeholder = "title, name")]
    pub frontmatter_name_field: String,
    /// Created property
    #[setting(placeholder = "created")]
    pub frontmatter_created_field: String,
    /// Modified property
    #[setting(placeholder = "modified")]
    pub frontmatter_modified_field: String,
    /// Manual sort property
    /// Where a note's place in a manual order is kept, in its frontmatter.
    pub manual_sort_property: String,
    /// Folder notes
    /// A note named for its folder is the folder's: a click on the folder opens it.
    pub folder_notes: bool,
    /// Folder note name
    /// `{{folder}}` is the folder's name; a name without it is every folder's (`index`).
    pub folder_note_name: String,
    /// Hide folder notes in the list
    pub hide_folder_note: bool,
    /// Pin new folder notes
    /// A note converted to its folder's is pinned there.
    pub pin_created_folder_note: bool,
    /// Folder notes open in
    #[setting(control = segmented)]
    pub folder_note_open: FolderNoteOpen,
    /// Calendar
    /// Days under the folders, the calendar button in their header shows
    /// them: a click opens a day's note, or makes it.
    pub calendar: bool,
    /// Confirm before creating a note
    /// Ask before a click makes a periodic note that is not there yet.
    pub calendar_confirm_create: bool,
    /// Weeks to show
    /// 6 is the whole month.
    #[setting(control = stepper(1..=6))]
    pub calendar_weeks: u32,
    /// Calendar in one pane
    /// Where the calendar goes when one pane shows at a time.
    #[setting(control = segmented)]
    pub calendar_placement: CalendarPlacement,
    /// Week starts on
    /// And so how weeks are numbered.
    pub week_start: WeekStart,
    /// Weekend days
    /// Shaded in the calendar.
    pub weekend_days: WeekendDays,
    /// Month name
    #[setting(control = segmented)]
    pub month_format: MonthFormat,
    /// Highlight today
    pub calendar_highlight_today: bool,
    /// Days from other months
    /// With the whole month on show, the days before and after it.
    pub calendar_outside_days: bool,
    /// Feature images on days
    /// A daily note's image fills its day.
    pub calendar_feature_images: bool,
    /// Tasks on days
    /// A ring on days, weeks and months whose notes have tasks left.
    pub calendar_tasks: bool,
    /// Week numbers
    /// A column of them; a click opens the week's note.
    pub calendar_week_numbers: bool,
    /// Quarter
    /// The quarter beside the month; a click opens its note.
    pub calendar_quarter: bool,
    /// Periodic notes folder
    /// Where periodic notes go; their patterns can add folders under it.
    #[setting(placeholder = "Journal")]
    pub periodic_folder: String,
    /// Daily notes
    /// A Moment date pattern: `YYYY/YYYYMMDD`. Wrap words in brackets,
    /// `[Daily]/YYYY-MM-DD`.
    #[setting(placeholder = "YYYY/YYYYMMDD")]
    pub daily_pattern: String,
    /// Weekly notes
    /// `gggg/[W]ww` (the locale's weeks), `GGGG-[W]WW` (ISO); empty, none.
    #[setting(placeholder = "gggg/[W]ww")]
    pub weekly_pattern: String,
    /// Monthly notes
    #[setting(placeholder = "YYYY/YYYYMM")]
    pub monthly_pattern: String,
    /// Quarterly notes
    #[setting(placeholder = "YYYY/[Q]Q")]
    pub quarterly_pattern: String,
    /// Yearly notes
    #[setting(placeholder = "YYYY")]
    pub yearly_pattern: String,
    /// Tags
    /// The vault's tags under the folders, nested by `/`.
    pub tag_tree: bool,
    /// "Tags" row
    /// The tags under one row that lists every tagged note.
    pub all_tags_row: bool,
    /// Untagged
    /// A row for the notes with no tag.
    pub show_untagged: bool,
    /// Only the selection's tags
    /// With a folder or a property selected, the tags its notes have.
    pub scope_tags: bool,
    /// Sort tags
    pub tag_sort: TreeSort,
    /// Property browser
    /// The keys named below under the tags, each with its values.
    pub property_tree: bool,
    /// Property keys
    /// Frontmatter keys the property browser shows.
    #[setting(placeholder = "status, type")]
    pub property_keys: String,
    /// Only the selection's properties
    /// With a folder or a tag selected, the values its notes have.
    pub scope_properties: bool,
    /// Sort properties
    pub property_sort: TreeSort,
    /// Search with
    /// The filter search reads words in names; the full text, in the
    /// notes' text (architect's own index).
    #[setting(control = segmented)]
    pub search_provider: SearchProvider,
    /// Shortcuts
    /// Notes, folders, tags, properties and saved searches over the
    /// folders.
    pub show_shortcuts: bool,
    /// Recent notes
    /// The notes opened last, over the folders.
    pub show_recent_notes: bool,
    /// Recent notes to show
    #[setting(control = stepper(1..=20))]
    pub recent_notes_count: u32,
    /// Ask before deleting
    /// Ask before notes and folders go to the trash.
    pub confirm_delete: bool,
    /// Open folders while dragging
    /// A folder held under a drag opens (spring-loaded folders).
    pub spring_loaded_folders: bool,
    /// First folder opens after
    #[setting(control = slider(100..=3000, step = 100), unit = " ms")]
    pub spring_loaded_delay_ms: u32,
    /// Next folders open after
    #[setting(control = slider(100..=3000, step = 100), unit = " ms")]
    pub spring_loaded_next_delay_ms: u32,
    /// Date format
    /// A Moment format: `{{date}}` and `{{today}}` in templates.
    #[setting(placeholder = "MMM D, YYYY")]
    pub date_format: String,
    /// Time format
    /// `{{time}}` in templates.
    #[setting(placeholder = "h:mm a")]
    pub time_format: String,
    /// Template folder
    /// Where the template picker looks; empty, the whole vault.
    #[setting(placeholder = "Templates")]
    pub template_folder: String,
    /// Folder note template
    /// The template a new folder note is made from (a vault path).
    #[setting(placeholder = "Templates/Folder.md")]
    pub folder_note_template: String,
    /// Daily note template
    #[setting(placeholder = "Templates/Daily.md")]
    pub daily_template: String,
    /// Weekly note template
    #[setting(placeholder = "Templates/Weekly.md")]
    pub weekly_template: String,
    /// Monthly note template
    #[setting(placeholder = "Templates/Monthly.md")]
    pub monthly_template: String,
    /// Quarterly note template
    #[setting(placeholder = "Templates/Quarterly.md")]
    pub quarterly_template: String,
    /// Yearly note template
    #[setting(placeholder = "Templates/Yearly.md")]
    pub yearly_template: String,
    /// Folder icons
    /// A folder's own icon in place of the folder.
    pub show_folder_icons: bool,
    /// Folders take the colour above
    /// A folder with no colour of its own takes the nearest above's.
    pub inherit_folder_colors: bool,
    /// Tag icons
    pub show_tag_icons: bool,
    /// Tags take the colour above
    pub inherit_tag_colors: bool,
    /// Property icons
    pub show_property_icons: bool,
    /// Values take their key's colour
    pub inherit_property_colors: bool,
    /// Colour the icon only
    /// A colour paints the icon, not the name.
    pub color_icon_only: bool,
    /// File icons
    /// An icon beside a note's name: its own, its name's or its type's.
    pub show_file_icons: bool,
    /// Icons by name
    /// A note whose name holds a word below takes its icon.
    pub file_name_icons: bool,
    /// Name icons
    #[setting(placeholder = "meeting=users, todo=list-todo")]
    pub file_name_icon_map: String,
    /// Icons by type
    /// A file takes its type's icon (`md`, `pdf`, `canvas`, images, …).
    pub file_type_icons: bool,
    /// Type icons
    #[setting(placeholder = "pdf=book, csv=sheet")]
    pub file_type_icon_map: String,
    /// Icon property
    /// The frontmatter key a note's icon is read from and written to
    /// (with names and dates from frontmatter).
    #[setting(placeholder = "icon")]
    pub frontmatter_icon_field: String,
    /// Colour property
    #[setting(placeholder = "color")]
    pub frontmatter_color_field: String,
    /// Background property
    #[setting(placeholder = "background")]
    pub frontmatter_background_field: String,
    /// Vim keys
    /// `h` `j` `k` `l` move through the panes too, `g` and ⇧G to the top
    /// and the end.
    pub vim_keys: bool,
}

impl NavigatorSettings {
    /// A comma list's items, trimmed, empty ones left out.
    #[must_use]
    pub fn list(text: &str) -> Vec<&str> {
        text.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// The properties a note's name is read from: none unless names come
    /// from frontmatter; `title` when the field is left empty.
    #[must_use]
    pub fn name_fields(&self) -> Vec<&str> {
        if !self.use_frontmatter {
            return Vec::new();
        }
        let fields = Self::list(&self.frontmatter_name_field);
        if fields.is_empty() {
            vec!["title"]
        } else {
            fields
        }
    }
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
            compact_height: 28,
            group_property: String::new(),
            show_group_counts: false,
            show_folder_group_paths: true,
            filter_pinned_by_folder: false,
            show_feature_image: true,
            feature_image_properties: String::new(),
            feature_image_size: 64,
            force_square_image: true,
            hide_drawing_previews: true,
            show_tags: true,
            show_tag_ancestors: false,
            color_tags: true,
            show_tags_compact: false,
            show_properties: true,
            list_properties: String::new(),
            show_task_progress: true,
            hide_task_progress_complete: false,
            text_count: TextCount::None,
            text_count_placement: CountPlacement::Title,
            use_frontmatter: false,
            frontmatter_name_field: String::new(),
            frontmatter_created_field: String::new(),
            frontmatter_modified_field: String::new(),
            manual_sort_property: "sort_index".to_owned(),
            folder_notes: false,
            folder_note_name: "{{folder}}".to_owned(),
            hide_folder_note: true,
            pin_created_folder_note: false,
            folder_note_open: FolderNoteOpen::CurrentTab,
            calendar: true,
            calendar_confirm_create: true,
            calendar_weeks: 1,
            calendar_placement: CalendarPlacement::Navigation,
            week_start: WeekStart::Sunday,
            weekend_days: WeekendDays::SatSun,
            month_format: MonthFormat::Full,
            calendar_highlight_today: true,
            calendar_outside_days: true,
            calendar_feature_images: true,
            calendar_tasks: true,
            calendar_week_numbers: false,
            calendar_quarter: false,
            periodic_folder: String::new(),
            daily_pattern: crate::calendar::DEFAULT_DAILY.to_owned(),
            weekly_pattern: crate::calendar::DEFAULT_WEEKLY.to_owned(),
            monthly_pattern: crate::calendar::DEFAULT_MONTHLY.to_owned(),
            quarterly_pattern: crate::calendar::DEFAULT_QUARTERLY.to_owned(),
            yearly_pattern: crate::calendar::DEFAULT_YEARLY.to_owned(),
            tag_tree: true,
            all_tags_row: true,
            show_untagged: true,
            scope_tags: false,
            tag_sort: TreeSort::AlphaAsc,
            property_tree: true,
            property_keys: String::new(),
            scope_properties: false,
            property_sort: TreeSort::AlphaAsc,
            search_provider: SearchProvider::Filter,
            show_shortcuts: true,
            show_recent_notes: true,
            recent_notes_count: 5,
            confirm_delete: true,
            spring_loaded_folders: true,
            spring_loaded_delay_ms: 500,
            spring_loaded_next_delay_ms: 500,
            date_format: "MMM D, YYYY".to_owned(),
            time_format: "h:mm a".to_owned(),
            template_folder: String::new(),
            folder_note_template: String::new(),
            daily_template: String::new(),
            weekly_template: String::new(),
            monthly_template: String::new(),
            quarterly_template: String::new(),
            yearly_template: String::new(),
            show_folder_icons: true,
            inherit_folder_colors: true,
            show_tag_icons: true,
            inherit_tag_colors: true,
            show_property_icons: true,
            inherit_property_colors: true,
            color_icon_only: false,
            show_file_icons: true,
            file_name_icons: false,
            file_name_icon_map: String::new(),
            file_type_icons: false,
            file_type_icon_map: String::new(),
            frontmatter_icon_field: "icon".to_owned(),
            frontmatter_color_field: "color".to_owned(),
            frontmatter_background_field: "background".to_owned(),
            vim_keys: false,
        }
    }
}
