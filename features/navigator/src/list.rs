//! The list pane's rows: the selected folder's notes, sorted, grouped by
//! date, and each row's height.
//!
//! Notebook Navigator's list items (`src/hooks/listPaneData/listItems.ts`),
//! `sortFiles` (`src/utils/sortUtils.ts`), date groups
//! (`DateUtils.getDateGroupInfo`, `src/utils/dateUtils.ts`) and row heights
//! (`src/utils/listPaneMeasurements.ts`).

use std::cmp::Ordering;

use crate::index::VaultIndex;
use crate::settings::{Grouping, ListMode, NavigatorSettings, SortOption};
use crate::source::{Millis, NoteFile};
use crate::tree::natural_cmp;

/// What a list row is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListRowKind {
    /// A group's heading (`Today`, `March`, `2024`).
    Header {
        label: String,
    },
    /// A note.
    File {
        path: String,
        /// Its folder, shown under it when it is not the selected one.
        parent: Option<String>,
        /// Whether its preview has text (a row without is shorter).
        has_preview: bool,
    },
    TopSpacer,
    BottomSpacer,
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

/// The notes of `folder` (and the folders below it when the settings say
/// so), ordered and grouped.
#[must_use]
pub fn rows(
    index: &VaultIndex,
    folder: &str,
    settings: &NavigatorSettings,
    clock: Clock,
) -> Vec<ListRow> {
    let mut files = index.files_in(folder, settings.include_descendants);
    sort(&mut files, settings.sort, settings.title_from_frontmatter);
    let mut out = vec![ListRow {
        key: "top-spacer".to_owned(),
        kind: ListRowKind::TopSpacer,
    }];
    let grouped = settings.grouping == Grouping::Date && settings.sort.by_date();
    let mut group: Option<String> = None;
    for f in files {
        if grouped {
            let at = date_of(f, settings.sort);
            let (key, label) = date_group(at, clock);
            if group.as_deref() != Some(key.as_str()) {
                out.push(ListRow {
                    key: format!("header:{key}"),
                    kind: ListRowKind::Header { label },
                });
                group = Some(key);
            }
        }
        let parent = f.folder();
        let shown_parent = (settings.include_descendants && parent != folder && !parent.is_empty())
            .then(|| parent.to_owned());
        out.push(ListRow {
            key: f.path.clone(),
            kind: ListRowKind::File {
                path: f.path.clone(),
                parent: shown_parent,
                has_preview: f.preview.as_deref().is_some_and(|p| !p.trim().is_empty()),
            },
        });
    }
    out.push(ListRow {
        key: "bottom-spacer".to_owned(),
        kind: ListRowKind::BottomSpacer,
    });
    out
}

/// The date a row shows and groups by: the sort's, the edited one for a
/// sort by name (Notebook Navigator's `alphabeticalDateMode`).
#[must_use]
pub const fn date_of(f: &NoteFile, sort: SortOption) -> Millis {
    if sort.shows_created() {
        f.created
    } else {
        f.modified
    }
}

/// Orders `files` by `sort`; ties by path.
pub fn sort(files: &mut [&NoteFile], sort: SortOption, frontmatter: bool) {
    files.sort_by(|a, b| compare(a, b, sort, frontmatter).then_with(|| a.path.cmp(&b.path)));
}

fn compare(a: &NoteFile, b: &NoteFile, sort: SortOption, frontmatter: bool) -> Ordering {
    match sort {
        SortOption::ModifiedDesc => b.modified.cmp(&a.modified),
        SortOption::ModifiedAsc => a.modified.cmp(&b.modified),
        SortOption::CreatedDesc => b.created.cmp(&a.created),
        SortOption::CreatedAsc => a.created.cmp(&b.created),
        SortOption::TitleAsc => natural_cmp(a.title(frontmatter), b.title(frontmatter)),
        SortOption::TitleDesc => natural_cmp(b.title(frontmatter), a.title(frontmatter)),
        SortOption::FilenameAsc => natural_cmp(a.file_name(), b.file_name()),
        SortOption::FilenameDesc => natural_cmp(b.file_name(), a.file_name()),
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

/// How tall a row is drawn.
#[must_use]
pub fn height(row: &ListRow, settings: &NavigatorSettings, m: ListMetrics) -> f64 {
    let title_rows = f64::from(settings.title_rows.clamp(1, 3));
    match &row.kind {
        ListRowKind::TopSpacer => m.top_spacer,
        ListRowKind::BottomSpacer => m.bottom_spacer,
        ListRowKind::Header { .. } => m.group_header,
        ListRowKind::File {
            parent,
            has_preview,
            ..
        } => {
            if settings.mode == ListMode::Compact {
                return title_rows.mul_add(
                    m.title_line,
                    f64::from(settings.compact_height) - m.title_line,
                );
            }
            let mut h = title_rows.mul_add(m.title_line, m.base_padding);
            if settings.show_preview && *has_preview {
                h = f64::from(settings.preview_rows.clamp(1, 5)).mul_add(m.multi_line, h);
            }
            if settings.show_date || (settings.show_parent_folder && parent.is_some()) {
                h += m.single_line;
            }
            h
        }
    }
}
