//! The navigation pane's rows: the folder tree flattened, depth first,
//! into the list a virtual scroller draws.
//!
//! Notebook Navigator's `flattenFolderTree` (`src/utils/treeFlattener.ts`):
//! pre-order, a folder's children only when it is expanded; with the root
//! shown it is level 0 and its folders level 1, else the top folders are
//! level 0. Siblings in a parent's manual order first, the rest A to Z
//! (`sortFolderSiblings`, natural order). Spacers at the top and bottom
//! (`TOP_SPACER` 8, `BOTTOM_SPACER` 20). Later layers add their own rows
//! (tags, properties, shortcuts, recent notes) as more [`NavRowKind`]s.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::index::{NoteCount, VaultIndex};
use crate::source::file_name;

/// Notebook Navigator's `NAVPANE_MEASUREMENTS`: the spacers' heights.
pub const TOP_SPACER: f64 = 8.0;
pub const BOTTOM_SPACER: f64 = 20.0;

/// What a navigation row is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NavRowKind {
    /// A folder (`""` the vault's root).
    Folder {
        path: String,
        name: String,
        /// It holds folders: it has a chevron.
        has_children: bool,
        expanded: bool,
        count: NoteCount,
    },
    TopSpacer,
    BottomSpacer,
}

/// One row of the navigation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavRow {
    /// Unique among the rows: what the view keys it by.
    pub key: String,
    /// How far in it is drawn.
    pub level: u16,
    pub kind: NavRowKind,
}

impl NavRow {
    /// The folder it is, if it is one.
    #[must_use]
    pub fn folder(&self) -> Option<&str> {
        match &self.kind {
            NavRowKind::Folder { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Whether the keyboard and a click can select it.
    #[must_use]
    pub const fn selectable(&self) -> bool {
        matches!(self.kind, NavRowKind::Folder { .. })
    }
}

/// What the tree is drawn with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeOptions {
    /// A row for the vault's root over its folders.
    pub show_root: bool,
    /// Each parent's children by name, in the order a person gave them
    /// (Notebook Navigator's `folderChildOrder`, the root's under `""`).
    pub child_order: BTreeMap<String, Vec<String>>,
}

/// The rows of the tree, with `expanded` folders open.
#[must_use]
pub fn flatten(index: &VaultIndex, expanded: &BTreeSet<String>, opts: &TreeOptions) -> Vec<NavRow> {
    let mut rows = vec![NavRow {
        key: "top-spacer".to_owned(),
        level: 0,
        kind: NavRowKind::TopSpacer,
    }];
    if opts.show_root {
        walk(index, "", 0, expanded, opts, &mut rows);
    } else {
        for child in ordered_children(index, "", opts) {
            walk(index, &child, 0, expanded, opts, &mut rows);
        }
    }
    rows.push(NavRow {
        key: "bottom-spacer".to_owned(),
        level: 0,
        kind: NavRowKind::BottomSpacer,
    });
    rows
}

fn walk(
    index: &VaultIndex,
    path: &str,
    level: u16,
    expanded: &BTreeSet<String>,
    opts: &TreeOptions,
    rows: &mut Vec<NavRow>,
) {
    let Some(folder) = index.folder(path) else {
        return;
    };
    let has_children = !folder.folders.is_empty();
    // The root, when hidden, is always open (Notebook Navigator's
    // "expansion locked" root).
    let open = expanded.contains(path);
    rows.push(NavRow {
        key: format!("folder:{path}"),
        level,
        kind: NavRowKind::Folder {
            path: path.to_owned(),
            name: index.folder_name(path).to_owned(),
            has_children,
            expanded: open && has_children,
            count: folder.count,
        },
    });
    if open {
        for child in ordered_children(index, path, opts) {
            walk(index, &child, level.saturating_add(1), expanded, opts, rows);
        }
    }
}

/// `parent`'s folders in the order they show.
#[must_use]
pub fn ordered_children(index: &VaultIndex, parent: &str, opts: &TreeOptions) -> Vec<String> {
    let Some(folder) = index.folder(parent) else {
        return Vec::new();
    };
    let manual = opts.child_order.get(parent);
    let rank =
        |path: &str| manual.and_then(|order| order.iter().position(|n| n == file_name(path)));
    let mut children: Vec<String> = folder.folders.iter().cloned().collect();
    children.sort_by(|a, b| match (rank(a), rank(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => natural_cmp(file_name(a), file_name(b)).then_with(|| a.cmp(b)),
    });
    children
}

/// `parent`'s children by name with `name` moved `by` places (up when
/// negative), as the new manual order.
///
/// Every child is listed, so the order holds as folders come and go
/// (Notebook Navigator's `mergeDisplayedOrder` keeps the rest after).
#[must_use]
pub fn reorder(
    index: &VaultIndex,
    parent: &str,
    opts: &TreeOptions,
    name: &str,
    by: isize,
) -> Vec<String> {
    let mut names: Vec<String> = ordered_children(index, parent, opts)
        .iter()
        .map(|p| file_name(p).to_owned())
        .collect();
    if let Some(at) = names.iter().position(|n| n == name) {
        let to = at
            .saturating_add_signed(by)
            .min(names.len().saturating_sub(1));
        let moved = names.remove(at);
        names.insert(to, moved);
    }
    names
}

/// Natural order, case folded: `Note 2` before `Note 10`
/// (`Intl.Collator {numeric: true, sensitivity: 'base'}`).
#[must_use]
pub fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut lhs = left.chars().peekable();
    let mut rhs = right.chars().peekable();
    loop {
        match (lhs.peek().copied(), rhs.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) if l.is_ascii_digit() && r.is_ascii_digit() => {
                let ln = digits(&mut lhs);
                let rn = digits(&mut rhs);
                let (ln, rn) = (ln.trim_start_matches('0'), rn.trim_start_matches('0'));
                let ord = ln.len().cmp(&rn.len()).then_with(|| ln.cmp(rn));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(l), Some(r)) => {
                let ord = l.to_lowercase().cmp(r.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                lhs.next();
                rhs.next();
            }
        }
    }
}

fn digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}
