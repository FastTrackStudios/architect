//! The list pane's rows, and each row's lines and height.
//!
//! The selected folder's or tag's notes, filtered (folder notes and
//! drawings' previews hidden), sorted, the pinned ones first, grouped (by
//! date, folder or a property's value).
//!
//! Notebook Navigator's list items (`src/hooks/listPaneData/listItems.ts`
//! `buildListItemsInternal`), `sortFiles` (`src/utils/sortUtils.ts`), date
//! groups (`DateUtils.getDateGroupInfo`, `src/utils/dateUtils.ts`), group
//! collapse keys (`src/utils/listGroupCollapse.ts`), feature images
//! (`src/utils/drawingFeatureImages.ts`, `shouldShowFeatureImageArea`), the
//! manual sort (`src/utils/manualSort.ts`) and row heights
//! (`src/utils/listPaneMeasurements.ts`
//! `calculateNormalListFileRowHeightEstimate`).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::hidden::Hidden;
use crate::index::VaultIndex;
use crate::properties;
use crate::search::{FilterSearch, SearchContext};
use crate::selection::NavTarget;
use crate::settings::{
    CountPlacement, Grouping, ListMode, NavigatorSettings, SortOption, TextCount,
};
use crate::source::{Millis, NoteFile, PropertyValue, file_name, parent};
use crate::tags::{self, TAGGED, UNTAGGED};
use crate::tree::{TreeOptions, natural_cmp, ordered_children};
use vault_proto::grouping::{Order, group_by};

/// What a list row is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListRowKind {
    /// A group's heading (`Pinned`, `Today`, `March`, `Projects/Alpha`,
    /// a property's value). A click folds it.
    Header {
        label: String,
        /// Its collapse key ([`collapse_key`]).
        group: String,
        collapsed: bool,
        /// How many notes it holds.
        count: usize,
        /// The pinned section's.
        pinned: bool,
    },
    /// A note.
    File {
        path: String,
        /// Its folder, shown under it when it is not the selected one.
        parent: Option<String>,
        /// Whether its preview has text (a row without is shorter).
        has_preview: bool,
        /// In the pinned section.
        pinned: bool,
        /// What its row draws, and so how tall it is.
        lines: Lines,
        /// Its image, a cover or a kind's badge.
        thumb: Option<Thumb>,
    },
    TopSpacer,
    BottomSpacer,
}

/// What a note's row draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lines {
    /// Lines of its name (1 to 3).
    pub title: u32,
    /// Lines of preview (0 for none).
    pub preview: u32,
    /// The line of its date, folder and task progress.
    pub meta: bool,
    /// Whether its date shows on it.
    pub date: bool,
    /// Whether its task progress shows on it.
    pub tasks: bool,
    /// The row of its tags.
    pub tags: bool,
    /// The row of its properties (and its count, placed there).
    pub properties: bool,
    /// Its count beside its name.
    pub count_in_title: bool,
}

impl Lines {
    /// Rows of pills: tags, properties.
    #[must_use]
    pub fn pills(self) -> u32 {
        u32::from(self.tags).saturating_add(u32::from(self.properties))
    }
}

/// A note's image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thumb {
    /// A file of the vault (the source gives its URL).
    File(String),
    /// A URL, as a property or the source gave it.
    Url(String),
    /// No picture: its kind's badge (`PDF`, `CANVAS`, `EXCALIDRAW`).
    Badge(String),
}

/// One row of the list pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub key: String,
    pub kind: ListRowKind,
}

impl ListRow {
    /// The note it is, if it is one.
    #[must_use]
    pub fn file(&self) -> Option<&str> {
        match &self.kind {
            ListRowKind::File { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// Now, and the person's time zone: what date groups are made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Clock {
    pub now: Millis,
    /// The time zone's offset from UTC, in milliseconds.
    pub offset: Millis,
}

const DAY: Millis = 86_400_000;

impl Clock {
    /// The local day number (days since 1970-01-01) of `at`.
    #[must_use]
    pub const fn day(self, at: Millis) -> i64 {
        at.saturating_add(self.offset).div_euclid(DAY)
    }
}

/// Where a note is pinned: in folders, in tags (`PinnedNotes`, one switch
/// per kind of place).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PinIn {
    pub folder: bool,
    pub tag: bool,
}

/// What the list is made from, past the vault.
#[derive(Debug, Clone, Copy)]
pub struct ListContext<'a> {
    /// The folder or tag selected.
    pub target: &'a NavTarget,
    /// The settings, with the target's own over them ([`crate::Appearance`]).
    pub settings: &'a NavigatorSettings,
    pub clock: Clock,
    pub pins: &'a BTreeMap<String, PinIn>,
    /// The groups folded, by [`collapse_key`].
    pub collapsed: &'a BTreeSet<String>,
    /// The tree's order, for folder groups.
    pub tree: &'a TreeOptions,
    /// The search box's search: the notes it matches.
    pub search: &'a FilterSearch,
    /// With the full-text provider, the notes whose text holds the
    /// search's words (the source's index); `None` matches the words
    /// against names.
    pub full_text: Option<&'a BTreeSet<String>>,
    /// The profile's hiding: the folders whose notes stay out of the lists
    /// above them.
    pub hidden: &'a Hidden,
    /// The order property groups come in, when the place gives one (a
    /// project's Songs, Notes, Sub-projects, Tasks); the rest after, by
    /// name.
    pub group_order: Option<&'a [String]>,
}

/// The notes of a folder, a tag or a property, as `rows` lists them:
/// what is hidden left out, unsorted.
///
/// A folder's take the folders below it when the settings say so, but
/// for those `hidden` keeps out of the lists above them.
#[must_use]
pub fn files_of<'a>(
    index: &'a VaultIndex,
    target: &NavTarget,
    settings: &NavigatorSettings,
    pins: &BTreeMap<String, PinIn>,
    hidden: &Hidden,
) -> Vec<&'a NoteFile> {
    let files = match target {
        NavTarget::Folder(folder) => {
            let mut files = index.files_in(folder, settings.include_descendants);
            if settings.include_descendants {
                files.retain(|f| !hidden.bounded_between(f.folder(), folder));
            }
            files
        }
        NavTarget::Tag(tag) if tag == TAGGED => index
            .files()
            .filter(|f| f.is_markdown() && !f.tags.is_empty())
            .collect(),
        NavTarget::Tag(tag) if tag == UNTAGGED => index
            .files()
            .filter(|f| f.is_markdown() && f.tags.is_empty())
            .collect(),
        NavTarget::Tag(tag) => index.files().filter(|f| has_tag(f, tag)).collect(),
        NavTarget::Property { key, value } => {
            let below = settings.include_descendants;
            let keys = NavigatorSettings::list(&settings.property_keys);
            index
                .files()
                .filter(|f| f.is_markdown())
                .filter(|f| {
                    if key == properties::ROOT {
                        below && keys.iter().any(|k| properties::lists(f, k, None, true))
                    } else {
                        properties::lists(f, key, value.as_deref(), below)
                    }
                })
                .collect()
        }
    };
    // A tag or a property lists vaults' notes: a provider's items (a task
    // as a row of the Queue) are the same notes again, or none (#443).
    let files: Vec<&NoteFile> = if matches!(target, NavTarget::Folder(_)) {
        files
    } else {
        files
            .into_iter()
            .filter(|f| index.in_vault(&f.path))
            .collect()
    };
    let pinned = |p: &str| pins.get(p).is_some_and(|w| w.folder || w.tag);
    files
        .into_iter()
        .filter(|f| {
            !(settings.folder_notes
                && settings.hide_folder_note
                && is_folder_note(index, settings, &f.path)
                && !pinned(&f.path))
        })
        .filter(|f| !(settings.hide_drawing_previews && is_drawing_preview(index, &f.path)))
        .collect()
}

/// Whether `f` has `tag` or a tag below it (`tag/…`), case folded.
#[must_use]
pub fn has_tag(f: &NoteFile, tag: &str) -> bool {
    let tag = tag.trim_start_matches('#');
    f.tags.iter().any(|t| {
        let t = t.trim_start_matches('#');
        t.eq_ignore_ascii_case(tag)
            || (t.len() > tag.len()
                && t.as_bytes().get(tag.len()) == Some(&b'/')
                && t.get(..tag.len())
                    .is_some_and(|h| h.eq_ignore_ascii_case(tag)))
    })
}

/// The scope a collapse key starts with: `folder:<path>`, `tag:<tag>`,
/// `property:key:<key>[=<value>]`.
#[must_use]
pub fn scope_of(target: &NavTarget) -> String {
    match target {
        NavTarget::Folder(f) => format!("folder:{f}"),
        NavTarget::Tag(t) => format!("tag:{t}"),
        NavTarget::Property { key, value: None } if key == properties::ROOT => {
            format!("property:{}", properties::ROOT)
        }
        NavTarget::Property { key, value: None } => {
            format!("property:{}", properties::key_id(key))
        }
        NavTarget::Property {
            key,
            value: Some(v),
        } => format!("property:{}", properties::value_id(key, v)),
    }
}

/// A group's collapse key (`buildListGroupCollapseKey`):
/// `scope=<scope>;group=<mode>;id=<group id>`.
#[must_use]
pub fn collapse_key(scope: &str, mode: &str, id: &str) -> String {
    format!("scope={scope};group={mode};id={id}")
}

/// The notes of the target, ordered and grouped.
#[must_use]
pub fn rows(index: &VaultIndex, cx: &ListContext<'_>) -> Vec<ListRow> {
    let s = cx.settings;
    let mut files = files_of(index, cx.target, s, cx.pins, cx.hidden);
    if cx.search.is_active() || cx.full_text.is_some() {
        let fields = s.name_fields();
        let by_created = matches!(s.sort, SortOption::CreatedAsc | SortOption::CreatedDesc);
        let search_cx = SearchContext {
            name_fields: &fields,
            by_created,
            offset: cx.clock.offset,
            dates: Some((s, cx.clock)),
        };
        match cx.full_text {
            Some(hits) => {
                let rest = cx.search.without_words();
                files.retain(|f| hits.contains(&f.path) && rest.matches(f, &search_cx));
            }
            None => files.retain(|f| cx.search.matches(f, &search_cx)),
        }
    }
    sort(&mut files, s, cx.clock);
    let scope = scope_of(cx.target);
    let folder = cx.target.folder();
    let (pinned, rest): (Vec<&NoteFile>, Vec<&NoteFile>) =
        files.into_iter().partition(|f| is_pinned(f, cx));
    let mut out = Rows {
        index,
        cx,
        out: vec![ListRow {
            key: "top-spacer".to_owned(),
            kind: ListRowKind::TopSpacer,
        }],
    };
    if !pinned.is_empty() {
        let key = collapse_key(&scope, "pinned", "pinned");
        out.group(&key, "pinned", "Pinned", true, &pinned);
    }
    let grouping = s.grouping.under(s.sort);
    match grouping {
        Grouping::None if s.sort == SortOption::Manual => {
            let key = &s.manual_sort_property;
            let (md, other): (Vec<&NoteFile>, Vec<&NoteFile>) =
                rest.into_iter().partition(|f| f.is_markdown());
            let (ranked, unsorted): (Vec<&NoteFile>, Vec<&NoteFile>) =
                md.into_iter().partition(|f| rank(f, key).is_some());
            if !pinned.is_empty() && !ranked.is_empty() {
                out.header(
                    &collapse_key(&scope, "manual", "section:notes"),
                    "Notes",
                    ranked.len(),
                    false,
                );
            }
            out.files(&ranked, false);
            if !unsorted.is_empty() {
                let id = collapse_key(&scope, "manual", "section:unsorted");
                out.group(&id, "unsorted", "Unsorted", false, &unsorted);
            }
            out.files(&other, false);
        }
        Grouping::None => {
            if !pinned.is_empty() && !rest.is_empty() {
                let key = collapse_key(&scope, "none", "section:notes");
                out.group(&key, "notes", "Notes", false, &rest);
            } else {
                out.files(&rest, false);
            }
        }
        Grouping::Date => {
            let field = if s.sort.shows_created() {
                "ctime"
            } else {
                "mtime"
            };
            let mut groups: Vec<(String, String, Vec<&NoteFile>)> = Vec::new();
            for f in rest {
                let (key, label) = date_group(date_of(f, s, cx.clock), cx.clock);
                match groups.last_mut() {
                    Some(g) if g.0 == key => g.2.push(f),
                    _ => groups.push((key, label, vec![f])),
                }
            }
            for (key, label, files) in groups {
                let id = collapse_key(&scope, "date", &format!("date:{field}:{key}"));
                out.group(&id, &key, &label, false, &files);
            }
        }
        Grouping::Folder => {
            for (id, label, files) in folder_groups(index, folder, rest, cx) {
                let key = collapse_key(&scope, "folder", &id);
                match label {
                    // The folder's own notes: no heading, unless pins or
                    // other groups stand before them.
                    None if pinned.is_empty() => out.files(&files, false),
                    None => out.group(&key, &id, "Notes", false, &files),
                    Some(label) => out.group(&key, &id, &label, false, &files),
                }
            }
        }
        Grouping::Property => {
            let property = s.group_property.trim();
            let mode = format!("property:{property}");
            let mut groups = property_groups(rest, |f| f.get(property));
            if let Some(order) = cx.group_order {
                let at = |label: &str| {
                    order
                        .iter()
                        .position(|o| o.eq_ignore_ascii_case(label))
                        .unwrap_or(usize::MAX)
                };
                groups.sort_by_key(|g| at(&g.label));
                // A place in its provider's order keeps each group's hand
                // order too: the ranked first (an album's songs by
                // `sort_index`), the rest as sorted.
                let key = &s.manual_sort_property;
                for g in &mut groups {
                    g.items.sort_by(|a, b| match (rank(a, key), rank(b, key)) {
                        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => Ordering::Equal,
                    });
                }
            }
            for g in groups {
                let key = collapse_key(&scope, &mode, &g.id);
                out.group(&key, &g.id, &g.label, false, &g.items);
            }
        }
    }
    out.out.push(ListRow {
        key: "bottom-spacer".to_owned(),
        kind: ListRowKind::BottomSpacer,
    });
    out.out
}

struct Rows<'a, 'c> {
    index: &'a VaultIndex,
    cx: &'a ListContext<'c>,
    out: Vec<ListRow>,
}

impl Rows<'_, '_> {
    fn header(&mut self, group: &str, label: &str, count: usize, pinned: bool) -> bool {
        let collapsed = self.cx.collapsed.contains(group);
        self.out.push(ListRow {
            key: format!("header:{group}"),
            kind: ListRowKind::Header {
                label: label.to_owned(),
                group: group.to_owned(),
                collapsed,
                count,
                pinned,
            },
        });
        collapsed
    }

    /// A heading and, unless it is folded, its notes.
    fn group(&mut self, key: &str, _id: &str, label: &str, pinned: bool, files: &[&NoteFile]) {
        if !self.header(key, label, files.len(), pinned) {
            self.files(files, pinned);
        }
    }

    fn files(&mut self, files: &[&NoteFile], pinned: bool) {
        let cx = self.cx;
        for f in files {
            // Its folder, as a row says it; none under a folder's own
            // heading (grouped by folder, the heading names it).
            let parent_shown = shown_parent(f, cx.target, cx.settings)
                .filter(|_| cx.settings.grouping.under(cx.settings.sort) != Grouping::Folder)
                .map(|p| self.index.display_path(&p));
            let has_preview = f.preview.as_deref().is_some_and(|p| !p.trim().is_empty());
            let thumb = thumb(self.index, f, cx.settings);
            let lines = lines(f, cx, pinned, parent_shown.is_some(), has_preview);
            self.out.push(ListRow {
                key: f.path.clone(),
                kind: ListRowKind::File {
                    path: f.path.clone(),
                    parent: parent_shown,
                    has_preview,
                    pinned,
                    lines,
                    thumb,
                },
            });
        }
    }
}

/// The folder shown under a note: in a tag's list, or a note from a
/// folder below the one selected (`shouldShowFileItemParentFolderLine`).
fn shown_parent(f: &NoteFile, target: &NavTarget, s: &NavigatorSettings) -> Option<String> {
    let p = f.folder();
    match target {
        NavTarget::Folder(folder) => {
            (s.include_descendants && p != folder && !p.is_empty()).then(|| p.to_owned())
        }
        _ => (!p.is_empty()).then(|| p.to_owned()),
    }
}

/// Whether `f` shows in the pinned section of this list.
fn is_pinned(f: &NoteFile, cx: &ListContext<'_>) -> bool {
    let Some(at) = cx.pins.get(&f.path) else {
        return false;
    };
    match cx.target {
        NavTarget::Folder(folder) => {
            at.folder && (!cx.settings.filter_pinned_by_folder || f.folder() == folder)
        }
        _ => at.tag,
    }
}

/// The notes in folder groups: the selected folder's own first (no
/// label), then each folder below in the tree's order, labelled with its
/// path from the selected one (or its name). In a tag's list, every
/// folder from the root, the root's notes under the vault's name.
fn folder_groups<'a>(
    index: &VaultIndex,
    selected: Option<&str>,
    files: Vec<&'a NoteFile>,
    cx: &ListContext<'_>,
) -> Vec<(String, Option<String>, Vec<&'a NoteFile>)> {
    let position = tree_positions(index, cx.tree);
    let mut groups: BTreeMap<(usize, String), Vec<&'a NoteFile>> = BTreeMap::new();
    for f in files {
        let p = f.folder().to_owned();
        let at = position.get(&p).copied().unwrap_or(usize::MAX);
        groups.entry((at, p)).or_default().push(f);
    }
    let mut out = Vec::new();
    for ((_, folder), files) in groups {
        let (id, label) = selected.map_or_else(
            || {
                let label = if folder.is_empty() {
                    index.name().to_owned()
                } else if cx.settings.show_folder_group_paths {
                    folder.clone()
                } else {
                    file_name(&folder).to_owned()
                };
                (format!("folder:/{folder}"), Some(label))
            },
            |sel| {
                let id = format!("folder:{folder}");
                if folder == sel {
                    return (id, None);
                }
                let rel = folder
                    .strip_prefix(sel)
                    .map_or(folder.as_str(), |r| r.trim_start_matches('/'));
                let label = if cx.settings.show_folder_group_paths {
                    rel.to_owned()
                } else {
                    file_name(&folder).to_owned()
                };
                (id, Some(label))
            },
        );
        out.push((id, label, files));
    }
    // The selected folder's own notes first.
    out.sort_by_key(|(_, label, _)| label.is_some());
    out
}

/// Each folder's place in the tree, every folder open (a parent before
/// its folders, siblings in the tree's order).
fn tree_positions(index: &VaultIndex, tree: &TreeOptions) -> HashMap<String, usize> {
    fn walk(index: &VaultIndex, path: &str, tree: &TreeOptions, out: &mut HashMap<String, usize>) {
        for child in ordered_children(index, path, tree) {
            let n = out.len();
            out.insert(child.clone(), n);
            walk(index, &child, tree, out);
        }
    }
    let mut out = HashMap::new();
    out.insert(String::new(), 0);
    walk(index, "", tree, &mut out);
    out
}

/// What `f`'s row draws (`resolveListPaneAppearance`): compact rows only
/// their name (and tags, when asked); pinned ones a line of preview and no
/// date or folder.
fn lines(
    f: &NoteFile,
    cx: &ListContext<'_>,
    pinned: bool,
    parent: bool,
    has_preview: bool,
) -> Lines {
    let s = cx.settings;
    let compact = s.mode == ListMode::Compact;
    let tasks = !compact
        && s.show_task_progress
        && f.tasks
            .is_some_and(|t| t.total > 0 && !(s.hide_task_progress_complete && t.unfinished == 0));
    let date = !compact && !pinned && s.show_date;
    let meta = date || (!compact && !pinned && s.show_parent_folder && parent) || tasks;
    let preview = if compact || !s.show_preview || !has_preview || s.preview_rows == 0 {
        0
    } else if pinned {
        1
    } else {
        s.preview_rows.clamp(1, 5)
    };
    let tags = s.show_tags && (!compact || s.show_tags_compact) && !shown_tags(f, cx).is_empty();
    let words = s.text_count == TextCount::Words && f.words.is_some();
    let properties = !compact
        && ((s.show_properties && !shown_properties(f, s).is_empty())
            || (words && s.text_count_placement == CountPlacement::Property));
    Lines {
        title: s.title_rows.clamp(1, 3),
        preview,
        meta,
        date,
        tasks,
        tags,
        properties,
        count_in_title: words && s.text_count_placement == CountPlacement::Title,
    }
}

/// The tags a row shows: the selected tag left out, the last segment
/// unless ancestors show (`showFileTagAncestors`).
#[must_use]
pub fn shown_tags<'a>(f: &'a NoteFile, cx: &ListContext<'_>) -> Vec<(&'a str, &'a str)> {
    let selected = match cx.target {
        NavTarget::Tag(t) if !tags::is_virtual(t) => Some(t.trim_start_matches('#')),
        _ => None,
    };
    f.tags
        .iter()
        .map(|t| t.trim_start_matches('#'))
        .filter(|t| selected.is_none_or(|sel| !t.eq_ignore_ascii_case(sel)))
        .map(|t| {
            let shown = if cx.settings.show_tag_ancestors {
                t
            } else {
                t.rsplit_once('/').map_or(t, |(_, last)| last)
            };
            (t, shown)
        })
        .collect()
}

/// The properties a row shows, as `(key, value)`: the listed keys it has.
#[must_use]
pub fn shown_properties(f: &NoteFile, s: &NavigatorSettings) -> Vec<(String, String)> {
    NavigatorSettings::list(&s.list_properties)
        .into_iter()
        .filter_map(|k| {
            let v = f.get(k)?;
            let shown = value_label(v);
            (!shown.is_empty()).then(|| (k.to_owned(), shown))
        })
        .collect()
}

// ── Property values ────────────────────────────────────────────────────

/// The group of notes with no value (`property-none`,
/// `strings.listPane.propertyGroupNoValue`).
pub const NONE_GROUP: &str = "property-none";
pub const NONE_LABEL: &str = "None";

/// One group of the list grouped by a property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyGroup<T> {
    /// `property-value:<label>`, or [`NONE_GROUP`].
    pub id: String,
    pub label: String,
    pub items: Vec<T>,
}

/// `items` grouped by the property `value` reads from each, as Bases
/// groups rows.
///
/// Through `vault_proto::grouping::group_by`, the one grouping in
/// architect: a group per whole value, shown as [`value_label`]; numbers
/// first, then text in natural order; those with none last, under "None"
/// (`getPropertyGroupingValueFromRecord`, `src/utils/sortUtils.ts`).
pub fn property_groups<T>(
    items: impl IntoIterator<Item = T>,
    value: impl Fn(&T) -> Option<&PropertyValue>,
) -> Vec<PropertyGroup<T>> {
    group_by(
        items,
        |i| value(i).map(value_label).unwrap_or_default(),
        Order::Asc,
    )
    .into_iter()
    .map(|g| {
        if g.label.is_empty() {
            PropertyGroup {
                id: NONE_GROUP.to_owned(),
                label: NONE_LABEL.to_owned(),
                items: g.items,
            }
        } else {
            PropertyGroup {
                id: format!("property-value:{}", g.label),
                label: g.label,
                items: g.items,
            }
        }
    })
    .collect()
}

/// A value's parts: text trimmed, a number written out, `true` or `false`,
/// a list's items each; none when it is blank.
#[must_use]
pub fn value_parts(value: &PropertyValue) -> Vec<String> {
    match value {
        PropertyValue::Text(t) => Some(t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .into_iter()
            .collect(),
        PropertyValue::Number(n) => vec![number(*n)],
        PropertyValue::Bool(b) => vec![b.to_string()],
        PropertyValue::List(items) => items
            .iter()
            .map(|i| i.trim())
            .filter(|i| !i.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

/// A value as a row and a group heading show it: its parts joined with
/// `", "`, a `[[link]]` as its name; `""` when it is blank.
#[must_use]
pub fn value_label(value: &PropertyValue) -> String {
    value_parts(value)
        .iter()
        .map(|p| shown_part(p))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A number as text: whole numbers without a point.
#[must_use]
pub fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// A part as it shows: `[[Note]]` and `[[Note|Shown]]` as their names.
#[must_use]
pub fn shown_part(part: &str) -> &str {
    let Some(inner) = part.strip_prefix("[[").and_then(|p| p.strip_suffix("]]")) else {
        return part;
    };
    match inner.split_once('|') {
        Some((_, shown)) => shown,
        None => inner.rsplit_once('/').map_or(inner, |(_, n)| n),
    }
}

// ── Images ─────────────────────────────────────────────────────────────

/// Raster images a row can show as they are.
const RASTER: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "avif", "heic", "heif", "bmp",
];

/// A note's image (`shouldShowFeatureImageArea`); none in compact rows.
///
/// A property naming one, the source's, the file itself (an image, an
/// SVG), a drawing's preview, or a kind's badge (a PDF without a cover, a
/// canvas, a base, a drawing without a preview).
#[must_use]
pub fn thumb(index: &VaultIndex, f: &NoteFile, s: &NavigatorSettings) -> Option<Thumb> {
    if !s.show_feature_image || s.mode == ListMode::Compact {
        return None;
    }
    for key in NavigatorSettings::list(&s.feature_image_properties) {
        if let Some(PropertyValue::Text(t)) = f.get(key) {
            if let Some(t) = image_ref(t) {
                return Some(t);
            }
        }
    }
    if let Some(t) = f.feature_image.as_deref().and_then(image_ref) {
        return Some(t);
    }
    let ext = f.extension().to_ascii_lowercase();
    if RASTER.contains(&ext.as_str()) || ext == "svg" {
        return Some(Thumb::File(f.path.clone()));
    }
    if let Some(kind) = drawing_kind(f) {
        return Some(
            drawing_previews(&f.path)
                .into_iter()
                .find(|p| index.file(p).is_some())
                .map_or_else(|| Thumb::Badge(kind.to_owned()), Thumb::File),
        );
    }
    match ext.as_str() {
        "pdf" | "canvas" | "base" => Some(Thumb::Badge(ext.to_ascii_uppercase())),
        _ => None,
    }
}

/// An image a property or the source names: a URL (`https` only, as
/// Notebook Navigator, or a source's `file://`) or a path in the vault
/// (`[[cover.png]]` too).
fn image_ref(text: &str) -> Option<Thumb> {
    let t = text.trim();
    let t = t
        .strip_prefix("![[")
        .or_else(|| t.strip_prefix("[["))
        .and_then(|r| r.strip_suffix("]]"))
        .map_or(t, |inner| inner.split_once('|').map_or(inner, |(p, _)| p));
    if t.is_empty() || t.starts_with("http://") {
        return None;
    }
    if t.starts_with("https://") || t.starts_with("data:") || t.starts_with("file://") {
        Some(Thumb::Url(t.to_owned()))
    } else {
        Some(Thumb::File(t.to_owned()))
    }
}

/// The drawing a file is: Excalidraw (`.excalidraw.md`, `.excalidraw`, a
/// note with `excalidraw-plugin`) or tldraw (`.tldr`, a note with
/// `tldraw-file: true`).
#[must_use]
pub fn drawing_kind(f: &NoteFile) -> Option<&'static str> {
    let path = f.path.to_ascii_lowercase();
    if path.ends_with(".excalidraw.md")
        || path.ends_with(".excalidraw")
        || (f.is_markdown() && f.get("excalidraw-plugin").is_some())
    {
        return Some("EXCALIDRAW");
    }
    if f.extension().eq_ignore_ascii_case("tldr")
        || (f.is_markdown() && f.get("tldraw-file") == Some(&PropertyValue::Bool(true)))
    {
        return Some("TLDRAW");
    }
    None
}

/// Where a drawing's previews would be: `<base>.dark.png`,
/// `<base>.light.png`, `<base>.png`.
#[must_use]
pub fn drawing_previews(path: &str) -> [String; 3] {
    let base = path
        .strip_suffix(".md")
        .or_else(|| path.strip_suffix(".tldr"))
        .unwrap_or(path);
    [
        format!("{base}.dark.png"),
        format!("{base}.light.png"),
        format!("{base}.png"),
    ]
}

/// Whether `path` is a drawing's preview (`hideDrawingPreviewImages`).
#[must_use]
pub fn is_drawing_preview(index: &VaultIndex, path: &str) -> bool {
    let Some(base) = path
        .strip_suffix(".dark.png")
        .or_else(|| path.strip_suffix(".light.png"))
        .or_else(|| path.strip_suffix(".png"))
    else {
        return false;
    };
    let drawing = |p: &str| index.file(p).is_some_and(|f| drawing_kind(f).is_some());
    drawing(&format!("{base}.md")) || drawing(base) || drawing(&format!("{base}.tldr"))
}

// ── Folder notes ───────────────────────────────────────────────────────

/// The kinds a folder note may be, in the order they are looked for
/// (`folderNoteType`'s priority).
pub const FOLDER_NOTE_KINDS: [&str; 3] = ["md", "canvas", "base"];

/// The name `folder`'s note has (`folderNoteNamePattern`): `{{folder}}`
/// its name; left empty, the folder's name.
#[must_use]
pub fn folder_note_name(s: &NavigatorSettings, folder: &str) -> String {
    let name = file_name(folder);
    let pattern = s.folder_note_name.trim();
    if pattern.is_empty() {
        name.to_owned()
    } else {
        pattern.replace("{{folder}}", name)
    }
}

/// `folder`'s note, if folder notes are on and it has one
/// (`getFolderNote`): `<folder>/<name>.md`, `.canvas`, `.base`, then
/// `<name>.excalidraw.md`. The vault's root has none.
#[must_use]
pub fn folder_note<'a>(
    index: &'a VaultIndex,
    s: &NavigatorSettings,
    folder: &str,
) -> Option<&'a str> {
    if !s.folder_notes || folder.is_empty() {
        return None;
    }
    let name = folder_note_name(s, folder);
    FOLDER_NOTE_KINDS
        .iter()
        .map(|ext| format!("{folder}/{name}.{ext}"))
        .chain(std::iter::once(format!("{folder}/{name}.excalidraw.md")))
        .find_map(|p| index.file(&p).map(|f| f.path.as_str()))
}

/// Whether `path` is its folder's note.
#[must_use]
pub fn is_folder_note(index: &VaultIndex, s: &NavigatorSettings, path: &str) -> bool {
    folder_note(index, s, parent(path)) == Some(path)
}

// ── Dates and order ────────────────────────────────────────────────────

/// When `f` was made: its frontmatter's, when names and dates come from
/// there and it has one (`frontmatterCreatedField`).
#[must_use]
pub fn created(f: &NoteFile, s: &NavigatorSettings, clock: Clock) -> Millis {
    frontmatter_date(f, s, &s.frontmatter_created_field, clock).unwrap_or(f.created)
}

/// When `f` last changed (`frontmatterModifiedField`).
#[must_use]
pub fn modified(f: &NoteFile, s: &NavigatorSettings, clock: Clock) -> Millis {
    frontmatter_date(f, s, &s.frontmatter_modified_field, clock).unwrap_or(f.modified)
}

fn frontmatter_date(
    f: &NoteFile,
    s: &NavigatorSettings,
    field: &str,
    clock: Clock,
) -> Option<Millis> {
    if !s.use_frontmatter {
        return None;
    }
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    match f.get(field)? {
        PropertyValue::Number(n) => Some(epoch(*n)),
        PropertyValue::Text(t) => parse_date(t, clock.offset),
        _ => None,
    }
}

/// A number as a time: seconds below 32503680000 (the year 3000), else
/// milliseconds (Notebook Navigator's rule).
#[allow(clippy::as_conversions, clippy::cast_possible_truncation)]
fn epoch(n: f64) -> Millis {
    if n.abs() < 32_503_680_000.0 {
        (n * 1000.0) as Millis
    } else {
        n as Millis
    }
}

/// An ISO 8601 date or time: `2024-03-05`, `2024-03-05T09:30`,
/// `2024-03-05 09:30:15`, with `Z` or `±HH:MM`; without a zone, local
/// (`offset` from UTC).
#[must_use]
pub fn parse_date(text: &str, offset: Millis) -> Option<Millis> {
    let text = text.trim();
    let num = |digits: &str| -> Option<i64> {
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| digits.parse().ok())?
    };
    let (date, rest) = text.split_at(text.len().min(10));
    let mut parts = date.split('-');
    let (year, month, day) = (
        num(parts.next()?)?,
        num(parts.next()?)?,
        num(parts.next()?)?,
    );
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut ms = days_from_civil(year, month, day).saturating_mul(DAY);
    let rest = rest.strip_prefix(['T', ' ']).unwrap_or(rest);
    let (time, zone) = rest
        .find(['Z', '+', '-'])
        .map_or((rest, ""), |at| rest.split_at(at));
    if !time.is_empty() {
        let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
        let mut parts = hms.split(':');
        let hour = num(parts.next()?)?;
        let min = num(parts.next().unwrap_or("0"))?;
        let sec = num(parts.next().unwrap_or("0"))?;
        let frac = frac.get(..frac.len().min(3)).unwrap_or("");
        let milli = if frac.is_empty() {
            0
        } else {
            let pad = 3_u32.saturating_sub(u32::try_from(frac.len()).unwrap_or(3));
            num(frac)?.saturating_mul(10_i64.saturating_pow(pad))
        };
        ms = ms
            .saturating_add(hour.saturating_mul(3_600_000))
            .saturating_add(min.saturating_mul(60_000))
            .saturating_add(sec.saturating_mul(1000))
            .saturating_add(milli);
    }
    let zone_offset = match zone {
        "" => offset,
        "Z" => 0,
        z => {
            let z = z.get(1..)?.replace(':', "");
            let hours = num(z.get(..2)?)?;
            let mins = num(z.get(2..).unwrap_or("0")).unwrap_or(0);
            let at = hours
                .saturating_mul(3_600_000)
                .saturating_add(mins.saturating_mul(60_000));
            if zone.starts_with('-') {
                at.saturating_neg()
            } else {
                at
            }
        }
    };
    Some(ms.saturating_sub(zone_offset))
}

/// The day number of a date (Howard Hinnant's `days_from_civil`).
#[must_use]
pub const fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y.saturating_sub(1) } else { y };
    let era = y.div_euclid(400);
    let yoe = y.saturating_sub(era.saturating_mul(400));
    let mp = if m > 2 {
        m.saturating_sub(3)
    } else {
        m.saturating_add(9)
    };
    let doy = (mp.saturating_mul(153).saturating_add(2) / 5)
        .saturating_add(d)
        .saturating_sub(1);
    let doe = yoe
        .saturating_mul(365)
        .saturating_add(yoe / 4)
        .saturating_sub(yoe / 100)
        .saturating_add(doy);
    era.saturating_mul(146_097)
        .saturating_add(doe)
        .saturating_sub(719_468)
}

/// The date a row shows and groups by: the sort's, the edited one for
/// other sorts (Notebook Navigator's `alphabeticalDateMode`).
#[must_use]
pub fn date_of(f: &NoteFile, s: &NavigatorSettings, clock: Clock) -> Millis {
    if s.sort.shows_created() {
        created(f, s, clock)
    } else {
        modified(f, s, clock)
    }
}

/// A note's place in the manual order: its `key` property, a number.
#[must_use]
pub fn rank(f: &NoteFile, key: &str) -> Option<f64> {
    match f.get(key)? {
        PropertyValue::Number(n) => Some(*n),
        PropertyValue::Text(t) => t.trim().parse().ok(),
        _ => None,
    }
}

/// The step between ranks a manual sort writes (`MANUAL_SORT_RANK_STEP`).
pub const RANK_STEP: f64 = 1000.0;

/// Orders `files` by the settings' sort; ties by path. A manual sort puts
/// ranked notes first, then the rest of the notes, then other files.
pub fn sort(files: &mut [&NoteFile], s: &NavigatorSettings, clock: Clock) {
    let names = s.name_fields();
    files.sort_by(|a, b| compare(a, b, s, &names, clock).then_with(|| a.path.cmp(&b.path)));
}

fn compare(
    a: &NoteFile,
    b: &NoteFile,
    s: &NavigatorSettings,
    names: &[&str],
    clock: Clock,
) -> Ordering {
    match s.sort {
        SortOption::ModifiedDesc => modified(b, s, clock).cmp(&modified(a, s, clock)),
        SortOption::ModifiedAsc => modified(a, s, clock).cmp(&modified(b, s, clock)),
        SortOption::CreatedDesc => created(b, s, clock).cmp(&created(a, s, clock)),
        SortOption::CreatedAsc => created(a, s, clock).cmp(&created(b, s, clock)),
        SortOption::TitleAsc => natural_cmp(a.title(names), b.title(names)),
        SortOption::TitleDesc => natural_cmp(b.title(names), a.title(names)),
        SortOption::FilenameAsc => natural_cmp(a.file_name(), b.file_name()),
        SortOption::FilenameDesc => natural_cmp(b.file_name(), a.file_name()),
        SortOption::Manual => {
            let key = &s.manual_sort_property;
            let class = |f: &NoteFile| match (f.is_markdown(), rank(f, key)) {
                (true, Some(_)) => 0,
                (true, None) => 1,
                (false, _) => 2,
            };
            class(a)
                .cmp(&class(b))
                .then_with(|| match (rank(a, key), rank(b, key)) {
                    (Some(left), Some(right)) => {
                        left.partial_cmp(&right).unwrap_or(Ordering::Equal)
                    }
                    _ => natural_cmp(a.title(names), b.title(names)),
                })
        }
    }
}

/// Which group a date falls in: its key and its heading. Future, Today,
/// Yesterday, Previous 7 days, Previous 30 days, then the month (this
/// year), then the year.
#[must_use]
pub fn date_group(at: Millis, clock: Clock) -> (String, String) {
    let today = clock.day(clock.now);
    let day = clock.day(at);
    let rel = |k: &str, l: &str| (format!("relative:{k}"), l.to_owned());
    if day > today {
        return rel("future", "Future");
    }
    if day == today {
        return rel("today", "Today");
    }
    if day == today.saturating_sub(1) {
        return rel("yesterday", "Yesterday");
    }
    if day > today.saturating_sub(7) {
        return rel("previous-7-days", "Previous 7 days");
    }
    if day > today.saturating_sub(30) {
        return rel("previous-30-days", "Previous 30 days");
    }
    let (y, m, _) = civil(day);
    let (this_year, _, _) = civil(today);
    if y == this_year {
        let name = MONTHS
            .get(usize::try_from(m.saturating_sub(1)).unwrap_or(0))
            .copied()
            .unwrap_or("");
        (format!("month:{y:04}-{m:02}"), name.to_owned())
    } else {
        (format!("year:{y:04}"), y.to_string())
    }
}

/// The months' names.
pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The year, month (1–12) and day of a day number (Howard Hinnant's
/// `civil_from_days`).
#[must_use]
pub const fn civil(days: i64) -> (i64, i64, i64) {
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.saturating_sub(era.saturating_mul(146_097));
    let yoe = (doe
        .saturating_sub(doe / 1460)
        .saturating_add(doe / 36_524)
        .saturating_sub(doe / 146_096))
        / 365;
    let y = yoe.saturating_add(era.saturating_mul(400));
    let doy = doe.saturating_sub(
        yoe.saturating_mul(365)
            .saturating_add(yoe / 4)
            .saturating_sub(yoe / 100),
    );
    let mp = (doy.saturating_mul(5).saturating_add(2)) / 153;
    let d = doy
        .saturating_sub((mp.saturating_mul(153).saturating_add(2)) / 5)
        .saturating_add(1);
    let m = if mp < 10 {
        mp.saturating_add(3)
    } else {
        mp.saturating_sub(9)
    };
    let y = if m <= 2 { y.saturating_add(1) } else { y };
    (y, m, d)
}

/// The list's measures, a desk's or a phone's (Notebook Navigator's
/// `listPaneMeasurements`, desktop / mobile).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMetrics {
    pub base_padding: f64,
    pub title_line: f64,
    pub single_line: f64,
    pub multi_line: f64,
    pub group_header: f64,
    pub top_spacer: f64,
    pub bottom_spacer: f64,
}

impl ListMetrics {
    pub const DESK: Self = Self {
        base_padding: 16.0,
        title_line: 20.0,
        single_line: 19.0,
        multi_line: 18.0,
        group_header: 27.0,
        top_spacer: 8.0,
        bottom_spacer: 20.0,
    };
    pub const PHONE: Self = Self {
        base_padding: 24.0,
        title_line: 21.0,
        single_line: 20.0,
        multi_line: 19.0,
        group_header: 35.0,
        top_spacer: 8.0,
        bottom_spacer: 20.0,
    };
}

/// A row of pills (tags, properties) and an image's least height
/// (`LISTPANE_MEASUREMENTS`).
pub const PILL_ROW: f64 = 26.0;
pub const IMAGE_FLOOR: f64 = 42.0;

/// How tall a row is drawn.
#[must_use]
pub fn height(row: &ListRow, settings: &NavigatorSettings, m: ListMetrics) -> f64 {
    match &row.kind {
        ListRowKind::TopSpacer => m.top_spacer,
        ListRowKind::BottomSpacer => m.bottom_spacer,
        ListRowKind::Header { .. } => m.group_header,
        ListRowKind::File { lines, thumb, .. } => file_height(*lines, thumb.is_some(), settings, m),
    }
}

/// A note's row: its name's lines, its preview, its date line and its
/// pills over the padding, at least an image's height. Compact: the
/// compact height for one line of name, and its pills.
#[must_use]
pub fn file_height(lines: Lines, image: bool, settings: &NavigatorSettings, m: ListMetrics) -> f64 {
    let title = f64::from(lines.title.clamp(1, 3));
    let pills = f64::from(lines.pills()) * PILL_ROW;
    if settings.mode == ListMode::Compact {
        return title.mul_add(
            m.title_line,
            f64::from(settings.compact_height) - m.title_line,
        ) + pills;
    }
    let mut body = title * m.title_line;
    body = f64::from(lines.preview).mul_add(m.multi_line, body);
    if lines.meta {
        body += m.single_line;
    }
    body += pills;
    let floor = if image { IMAGE_FLOOR } else { 0.0 };
    m.base_padding + body.max(floor)
}
