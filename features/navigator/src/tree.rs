//! The navigation pane's rows: the folder tree flattened, depth first,
//! into the list a virtual scroller draws.
//!
//! Notebook Navigator's `flattenFolderTree` (`src/utils/treeFlattener.ts`):
//! pre-order, a folder's children only when it is expanded; with the root
//! shown it is level 0 and its folders level 1, else the top folders are
//! level 0. Siblings in a parent's manual order first, the rest A to Z
//! (`sortFolderSiblings`, natural order). Spacers at the top and bottom
//! (`TOP_SPACER` 8, `BOTTOM_SPACER` 20).
//!
//! Under the folders, the tags ([`tag_rows`], `flattenTagTree`): the
//! "Tags" row over them (its tags one level in) or the tags at the top,
//! Untagged among the roots; then the property browser
//! ([`property_rows`]): the "Properties" row, its keys, each key's values.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::index::{NoteCount, VaultIndex};
use crate::profiles::Shortcut;
use crate::properties::{PropertyTree, ROOT, key_id, value_id};
use crate::selection::NavTarget;
use crate::settings::TreeSort;
use crate::source::file_name;
use crate::tags::{TAGGED, TagTree, UNTAGGED};

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
    /// A tag, or the "Tags" ([`crate::tags::TAGGED`]) or "Untagged"
    /// ([`crate::tags::UNTAGGED`]) row.
    Tag {
        /// Its path as written (what selecting it lists), or the row's id.
        tag: String,
        /// Its key ([`crate::tags::tag_key`]), or the row's id.
        key: String,
        name: String,
        has_children: bool,
        expanded: bool,
        count: NoteCount,
    },
    /// A property's key or one of its values, or the "Properties" row
    /// (key [`crate::properties::ROOT`]).
    Property {
        /// Folded.
        key: String,
        /// Its path, for a value.
        value: Option<String>,
        name: String,
        has_children: bool,
        expanded: bool,
        count: NoteCount,
    },
    /// The profile's banner over the rest: an image, a vault path or a
    /// URL (`navigationBanner`).
    Banner {
        src: String,
    },
    /// The "Shortcuts" or "Recent notes" header ([`SHORTCUTS`],
    /// [`RECENT`]), folding the rows under it.
    Section {
        id: String,
        name: String,
        expanded: bool,
        count: usize,
    },
    /// A shortcut, `at` from 0 (its command is `at + 1`); `missing` when
    /// what it opens is gone.
    Shortcut {
        at: usize,
        shortcut: Shortcut,
        name: String,
        missing: bool,
    },
    /// A recent note.
    Recent {
        path: String,
        name: String,
    },
    TopSpacer,
    BottomSpacer,
}

/// The shortcuts' section.
pub const SHORTCUTS: &str = "shortcuts";
/// The recent notes' section.
pub const RECENT: &str = "recent";
/// The banner's height.
pub const BANNER_HEIGHT: f64 = 96.0;

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
        matches!(
            self.kind,
            NavRowKind::Folder { .. } | NavRowKind::Tag { .. } | NavRowKind::Property { .. }
        )
    }

    /// What selecting it selects.
    #[must_use]
    pub fn target(&self) -> Option<NavTarget> {
        match &self.kind {
            NavRowKind::Folder { path, .. } => Some(NavTarget::Folder(path.clone())),
            NavRowKind::Tag { tag, .. } => Some(NavTarget::Tag(tag.clone())),
            NavRowKind::Property { key, value, .. } => Some(NavTarget::Property {
                key: key.clone(),
                value: value.clone(),
            }),
            _ => None,
        }
    }

    /// Whether it has rows below it, and they show.
    #[must_use]
    pub const fn expansion(&self) -> Option<bool> {
        match &self.kind {
            NavRowKind::Folder {
                has_children: true,
                expanded,
                ..
            }
            | NavRowKind::Tag {
                has_children: true,
                expanded,
                ..
            }
            | NavRowKind::Property {
                has_children: true,
                expanded,
                ..
            } => Some(*expanded),
            _ => None,
        }
    }
}

/// How the tags are drawn.
#[derive(Debug, Clone, Copy)]
pub struct TagRowOptions<'a> {
    /// The "Tags" row over them (`showAllTagsFolder`).
    pub all_row: bool,
    /// The "Untagged" row (`showUntagged`), when a note has no tag.
    pub untagged: bool,
    pub sort: TreeSort,
    /// Notes from below count (`includeDescendantNotes`).
    pub below: bool,
    /// The roots as a person ordered them (`rootTagOrder`).
    pub order: &'a [String],
    /// Nested tags as a person ordered them, by parent (`tagChildOrder`).
    pub child_order: &'a BTreeMap<String, Vec<String>>,
}

/// `list` with those `order` names first, in its order, then the rest
/// as they were (a child new since is appended, `useChildOrderAppend`).
#[must_use]
pub fn hand_ordered(list: Vec<String>, order: Option<&Vec<String>>) -> Vec<String> {
    let Some(order) = order else { return list };
    let mut out: Vec<String> = order.iter().filter(|o| list.contains(o)).cloned().collect();
    out.extend(list.into_iter().filter(|x| !order.contains(x)));
    out
}

/// `list` with `item` moved `by` places; `None` when it is not there or
/// cannot move.
#[must_use]
pub fn moved_in(list: &[String], item: &str, by: isize) -> Option<Vec<String>> {
    let at = list.iter().position(|x| x == item)?;
    let to = at
        .saturating_add_signed(by)
        .min(list.len().saturating_sub(1));
    if to == at {
        return None;
    }
    let mut out = list.to_vec();
    let x = out.remove(at);
    out.insert(to, x);
    Some(out)
}

/// The tag rows: the "Tags" row and the tags below it when it is open (or
/// the tags at the top), each open one's children; `expanded` holds the
/// open tags' keys (and [`TAGGED`]).
#[must_use]
pub fn tag_rows(tree: &TagTree, expanded: &BTreeSet<String>, o: &TagRowOptions<'_>) -> Vec<NavRow> {
    let mut rows = Vec::new();
    let untagged = o.untagged && tree.untagged > 0;
    if tree.is_empty() && !untagged {
        return rows;
    }
    let level = if o.all_row {
        let open = expanded.contains(TAGGED);
        rows.push(NavRow {
            key: format!("tag:{TAGGED}"),
            level: 0,
            kind: NavRowKind::Tag {
                tag: TAGGED.to_owned(),
                key: TAGGED.to_owned(),
                name: "Tags".to_owned(),
                has_children: true,
                expanded: open,
                count: NoteCount {
                    current: if o.below { tree.tagged } else { 0 },
                    descendants: 0,
                },
            },
        });
        if !open {
            return rows;
        }
        1
    } else {
        0
    };
    for root in tree.roots(o.order, o.sort, o.below, untagged) {
        if root == UNTAGGED {
            rows.push(NavRow {
                key: format!("tag:{UNTAGGED}"),
                level,
                kind: NavRowKind::Tag {
                    tag: UNTAGGED.to_owned(),
                    key: UNTAGGED.to_owned(),
                    name: "Untagged".to_owned(),
                    has_children: false,
                    expanded: false,
                    count: NoteCount {
                        current: tree.untagged,
                        descendants: 0,
                    },
                },
            });
        } else {
            walk_tag(tree, &root, level, expanded, o, &mut rows);
        }
    }
    rows
}

fn walk_tag(
    tree: &TagTree,
    key: &str,
    level: u16,
    expanded: &BTreeSet<String>,
    o: &TagRowOptions<'_>,
    rows: &mut Vec<NavRow>,
) {
    let Some(node) = tree.get(key) else {
        return;
    };
    let open = expanded.contains(key);
    let has_children = !node.children.is_empty();
    let mut count = node.count();
    if !o.below {
        count.descendants = 0;
    }
    rows.push(NavRow {
        key: format!("tag:{key}"),
        level,
        kind: NavRowKind::Tag {
            tag: node.display.clone(),
            key: key.to_owned(),
            name: node.name.clone(),
            has_children,
            expanded: open && has_children,
            count,
        },
    });
    if open {
        for c in hand_ordered(tree.children(key, o.sort, o.below), o.child_order.get(key)) {
            walk_tag(tree, &c, level.saturating_add(1), expanded, o, rows);
        }
    }
}

/// The property browser's rows: the "Properties" row, its keys when it is
/// open, an open key's values; `expanded` holds [`ROOT`] and the open
/// keys' ids ([`crate::properties::key_id`]).
#[must_use]
pub fn property_rows(
    tree: &PropertyTree,
    expanded: &BTreeSet<String>,
    sort: TreeSort,
    below: bool,
    order: &[String],
    value_order: &BTreeMap<String, Vec<String>>,
    with_row: bool,
) -> Vec<NavRow> {
    let mut rows = Vec::new();
    if tree.is_empty() {
        return rows;
    }
    // Without its row (`showAllPropertiesFolder` off) the keys are the top
    // level, always on show.
    let open = !with_row || expanded.contains(ROOT);
    let top = u16::from(with_row);
    if with_row {
        rows.push(NavRow {
            key: format!("property:{ROOT}"),
            level: 0,
            kind: NavRowKind::Property {
                key: ROOT.to_owned(),
                value: None,
                name: "Properties".to_owned(),
                has_children: true,
                expanded: open,
                count: NoteCount {
                    current: if below { tree.notes.len() } else { 0 },
                    descendants: 0,
                },
            },
        });
    }
    if !open {
        return rows;
    }
    for key in tree.roots(order, sort, below) {
        let Some(k) = tree.key(&key) else { continue };
        let id = key_id(&key);
        let values = hand_ordered(tree.values(&key, sort), value_order.get(&key));
        let open = expanded.contains(&id) && !values.is_empty();
        let mut count = k.count();
        if !below {
            count.descendants = 0;
        }
        rows.push(NavRow {
            key: format!("property:{id}"),
            level: top,
            kind: NavRowKind::Property {
                key: key.clone(),
                value: None,
                name: k.name.clone(),
                has_children: !values.is_empty(),
                expanded: open,
                count,
            },
        });
        if !open {
            continue;
        }
        for v in values {
            let Some(node) = k.values.get(&v) else {
                continue;
            };
            rows.push(NavRow {
                key: format!("property:{}", value_id(&key, &v)),
                level: top + 1,
                kind: NavRowKind::Property {
                    key: key.clone(),
                    value: Some(v.clone()),
                    name: node.name.clone(),
                    has_children: false,
                    expanded: false,
                    count: NoteCount {
                        current: node.notes.len(),
                        descendants: 0,
                    },
                },
            });
        }
    }
    rows
}

/// What the tree is drawn with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeOptions {
    /// A row for the vault's root over its folders.
    pub show_root: bool,
    /// Each parent's children by name, in the order a person gave them
    /// (Notebook Navigator's `folderChildOrder`, the root's under `""`).
    pub child_order: BTreeMap<String, Vec<String>>,
    /// Folders Z to A (`folderSortOrder` `alpha-desc`).
    pub descending: bool,
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
    // By hand first, else as the source orders them, else by name.
    let manual = opts
        .child_order
        .get(parent)
        .or_else(|| index.source_order(parent));
    let rank =
        |path: &str| manual.and_then(|order| order.iter().position(|n| n == file_name(path)));
    let mut children: Vec<String> = folder.folders.iter().cloned().collect();
    children.sort_by(|a, b| match (rank(a), rank(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => {
            let by_name = natural_cmp(file_name(a), file_name(b)).then_with(|| a.cmp(b));
            if opts.descending {
                by_name.reverse()
            } else {
                by_name
            }
        }
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
