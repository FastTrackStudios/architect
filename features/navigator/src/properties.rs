//! The property browser: the frontmatter keys a person names
//! ([`crate::NavigatorSettings::property_keys`]), each with its values, and
//! how many notes have each.
//!
//! Notebook Navigator's property tree (`src/utils/propertyTree.ts`
//! `registerPropertyTreeEntry`, `buildPropertyKeyNodeId`,
//! `buildPropertyValueNodeId`; `src/utils/propertyUtils.ts`
//! `extractFrontmatterPropertyValues`, `normalizePropertyTreeValuePath`),
//! its counts (`useNavigationNoteCounts.ts`) and the list a node selects
//! (`src/utils/fileFinder.ts`):
//!
//! - Only the keys named show (any case), each as first written there; a
//!   key no note has still shows, empty.
//! - A value's parts are its values: text trimmed, a number or a yes/no
//!   written out, a list's items each. An empty value (`key:` alone) is
//!   the key's own: no value row.
//! - A value is matched by its path: `[[Target|Alias]]` is the alias (or
//!   the target), any case; it shows with its links unwrapped.
//! - A node's id is `key:<key>`, a value's `key:<key>=<value>` (`%` and
//!   `=` escaped as `%25` and `%3D`), keys and values case folded.
//! - A key counts its notes with an empty value as its own and the rest as
//!   below; a value counts its notes.
//! - A key selected lists every note with it when notes from below are on,
//!   else those with it empty; a value, the notes with it; the "Properties"
//!   row, every note with a named key when notes from below are on, else
//!   none.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::index::NoteCount;
use crate::list::value_parts;
use crate::settings::TreeSort;
use crate::source::{NoteFile, PropertyValue};
use crate::tree::natural_cmp;

/// The "Properties" row's id (`properties-root`).
pub const ROOT: &str = "properties-root";

/// A key or a value as it is matched: trimmed, lower case.
#[must_use]
pub fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

/// A value's path: a `[[link]]` as its alias or target (without a `#`),
/// any case.
#[must_use]
pub fn value_path(part: &str) -> String {
    match wiki(part) {
        Some((target, alias)) => fold(alias.unwrap_or(target).trim_start_matches('#')),
        None => fold(part),
    }
}

/// A value as it shows: a `[[Target|Alias]]` as its alias or the target's
/// name, a `[text](url)` as its text.
#[must_use]
pub fn value_display(part: &str) -> String {
    if let Some((target, alias)) = wiki(part) {
        return alias
            .unwrap_or_else(|| target.rsplit_once('/').map_or(target, |(_, n)| n))
            .to_owned();
    }
    if let Some(rest) = part.trim().strip_prefix('[')
        && let Some((text, url)) = rest.split_once("](")
        && url.ends_with(')')
    {
        return text.to_owned();
    }
    part.trim().to_owned()
}

/// `[[target|alias]]`'s target and alias.
fn wiki(part: &str) -> Option<(&str, Option<&str>)> {
    let inner = part.trim().strip_prefix("[[")?.strip_suffix("]]")?;
    Some(match inner.split_once('|') {
        Some((t, a)) => (t.trim(), Some(a.trim())),
        None => (inner.trim(), None),
    })
}

fn escape(s: &str) -> String {
    s.replace('%', "%25").replace('=', "%3D")
}

fn unescape(s: &str) -> String {
    s.replace("%3D", "=").replace("%25", "%")
}

/// A key's id: `key:<key>`.
#[must_use]
pub fn key_id(key: &str) -> String {
    format!("key:{}", escape(&fold(key)))
}

/// A value's id: `key:<key>=<value>`.
#[must_use]
pub fn value_id(key: &str, value: &str) -> String {
    format!("key:{}={}", escape(&fold(key)), escape(&value_path(value)))
}

/// The key and value (none for a key) an id names.
#[must_use]
pub fn parse_id(id: &str) -> Option<(String, Option<String>)> {
    let rest = id.strip_prefix("key:")?;
    Some(match rest.split_once('=') {
        Some((k, v)) => (unescape(k), Some(unescape(v))),
        None => (unescape(rest), None),
    })
}

/// A note's values for `key` (any case): the parts of each, and whether
/// it has the key with no value. `None` when it does not have it.
#[must_use]
pub fn values_of<'a>(f: &'a NoteFile, key: &str) -> Option<(Vec<String>, bool, &'a PropertyValue)> {
    let want = fold(key);
    let (_, v) = f.properties.iter().find(|(k, _)| fold(k) == want)?;
    let parts = value_parts(v);
    let empty = matches!(v, PropertyValue::Text(t) if t.trim().is_empty());
    Some((parts, empty, v))
}

/// Whether `f` is in the list of property `key` (value `value`, or the key
/// itself), notes from below on or off.
#[must_use]
pub fn lists(f: &NoteFile, key: &str, value: Option<&str>, below: bool) -> bool {
    let Some((parts, empty, _)) = values_of(f, key) else {
        return false;
    };
    value.map_or(below || empty, |v| {
        let want = value_path(v);
        parts.iter().any(|p| value_path(p) == want)
    })
}

/// One value of a key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValueNode {
    /// Its path ([`value_path`]).
    pub value: String,
    /// As it shows ([`value_display`]).
    pub name: String,
    /// As it was first written: what setting it on a note writes
    /// (`assignmentValue`), a link's markup kept.
    pub written: String,
    pub notes: BTreeSet<String>,
}

/// One key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyNode {
    /// Folded.
    pub key: String,
    /// As named in the settings.
    pub name: String,
    /// Its values by path.
    pub values: BTreeMap<String, ValueNode>,
    /// Every note with it.
    pub notes: BTreeSet<String>,
    /// The notes with it empty.
    pub direct: BTreeSet<String>,
}

impl KeyNode {
    #[must_use]
    pub fn count(&self) -> NoteCount {
        NoteCount {
            current: self.direct.len(),
            descendants: self.notes.len().saturating_sub(self.direct.len()),
        }
    }
}

/// The named keys of a set of notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PropertyTree {
    keys: BTreeMap<String, KeyNode>,
    /// The Markdown notes with any of the keys.
    pub notes: BTreeSet<String>,
}

impl PropertyTree {
    /// The tree of `keys` (as named) over `files`' Markdown notes.
    #[must_use]
    pub fn build<'a>(files: impl IntoIterator<Item = &'a NoteFile>, keys: &[&str]) -> Self {
        let mut tree = Self::default();
        for k in keys {
            let key = fold(k);
            if !key.is_empty() {
                tree.keys.entry(key.clone()).or_insert_with(|| KeyNode {
                    key,
                    name: k.trim().to_owned(),
                    ..KeyNode::default()
                });
            }
        }
        let mut files: Vec<&NoteFile> = files.into_iter().filter(|f| f.is_markdown()).collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        for f in files {
            for node in tree.keys.values_mut() {
                let Some((parts, empty, _)) = values_of(f, &node.key) else {
                    continue;
                };
                node.notes.insert(f.path.clone());
                tree.notes.insert(f.path.clone());
                if empty {
                    node.direct.insert(f.path.clone());
                }
                for p in parts {
                    let path = value_path(&p);
                    if path.is_empty() {
                        continue;
                    }
                    let v = node
                        .values
                        .entry(path.clone())
                        .or_insert_with(|| ValueNode {
                            value: path,
                            name: value_display(&p),
                            written: p.clone(),
                            ..ValueNode::default()
                        });
                    // A link's markup is what is written, once one is seen.
                    if p.trim().starts_with("[[") && !v.written.trim().starts_with("[[") {
                        v.written.clone_from(&p);
                    }
                    v.notes.insert(f.path.clone());
                }
            }
        }
        tree
    }

    #[must_use]
    pub fn key(&self, key: &str) -> Option<&KeyNode> {
        self.keys.get(&fold(key))
    }

    #[must_use]
    pub fn value(&self, key: &str, value: &str) -> Option<&ValueNode> {
        self.key(key)?.values.get(&value_path(value))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The keys, in `order` first, then by `sort`.
    #[must_use]
    pub fn roots(&self, order: &[String], sort: TreeSort, below: bool) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for k in order {
            let k = fold(k);
            if self.keys.contains_key(&k) && !out.contains(&k) {
                out.push(k);
            }
        }
        let n = |k: &KeyNode| if below { k.notes.len() } else { k.direct.len() };
        let mut rest: Vec<&KeyNode> = self
            .keys
            .values()
            .filter(|k| !out.contains(&k.key))
            .collect();
        rest.sort_by(|a, b| {
            order_by(sort, n(a).cmp(&n(b)), || {
                natural_cmp(&a.name, &b.name).then_with(|| a.key.cmp(&b.key))
            })
        });
        out.extend(rest.into_iter().map(|k| k.key.clone()));
        out
    }

    /// `key`'s values' paths, by `sort`.
    #[must_use]
    pub fn values(&self, key: &str, sort: TreeSort) -> Vec<String> {
        let Some(k) = self.key(key) else {
            return Vec::new();
        };
        let mut out: Vec<&ValueNode> = k.values.values().collect();
        out.sort_by(|a, b| {
            order_by(sort, a.notes.len().cmp(&b.notes.len()), || {
                natural_cmp(&a.name, &b.name).then_with(|| a.value.cmp(&b.value))
            })
        });
        out.into_iter().map(|v| v.value.clone()).collect()
    }

    /// The keys with `key` moved `by` places, as the new manual order.
    #[must_use]
    pub fn reorder(
        &self,
        order: &[String],
        sort: TreeSort,
        below: bool,
        key: &str,
        by: isize,
    ) -> Vec<String> {
        let mut keys = self.roots(order, sort, below);
        if let Some(at) = keys.iter().position(|k| *k == fold(key)) {
            let to = at
                .saturating_add_signed(by)
                .min(keys.len().saturating_sub(1));
            let moved = keys.remove(at);
            keys.insert(to, moved);
        }
        keys
    }
}

/// `sort`'s order from how two compare by count and A to Z.
fn order_by(sort: TreeSort, by_count: Ordering, alpha: impl Fn() -> Ordering) -> Ordering {
    match sort {
        TreeSort::AlphaAsc => alpha(),
        TreeSort::AlphaDesc => alpha().reverse(),
        TreeSort::FrequencyAsc => by_count.then_with(alpha),
        TreeSort::FrequencyDesc => by_count.then_with(alpha).reverse(),
    }
}
