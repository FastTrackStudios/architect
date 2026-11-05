//! Grouping by a property's value, as Obsidian's Bases groups rows and
//! Notebook Navigator's list groups notes (`getPropertyGroupingValueFromRecord`,
//! `src/utils/sortUtils.ts`; `buildListItemsInternal`,
//! `src/hooks/listPaneData/listItems.ts`).
//!
//! It knows nothing of notes: [`by_value`] takes any items and a way to
//! read each one's value, so a Bases view groups its rows with the same
//! function the list groups its notes with.
//!
//! - A value's parts: text trimmed, a number written out, `true` or
//!   `false`, a list's items each. An item with no parts has no value.
//! - A group is one whole value (a list is one group, not one per item):
//!   its key the parts joined with `\u{0}`, its label the parts joined with
//!   `", "`, a `[[link]]` shown as its name.
//! - Numbers come before text; numbers in order, text in natural order
//!   (`Item 2` before `Item 10`), the key breaking ties; reversed when
//!   `descending`.
//! - Items without a value go last, in a group labelled "None".
//! - Items keep their order inside a group.

use std::cmp::Ordering;

use crate::source::PropertyValue;
use crate::tree::natural_cmp;

/// The trailing group's id and label (`property-none`,
/// `strings.listPane.propertyGroupNoValue`).
pub const NONE_ID: &str = "property-none";
pub const NONE_LABEL: &str = "None";

/// One group of items.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueGroup<T> {
    /// `property-value:<parts>` or [`NONE_ID`].
    pub id: String,
    pub label: String,
    pub items: Vec<T>,
}

/// A value's parts.
#[must_use]
pub fn parts(value: &PropertyValue) -> Vec<String> {
    match value {
        PropertyValue::Text(t) => {
            let t = t.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![t.to_owned()]
            }
        }
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
pub fn display(part: &str) -> &str {
    let Some(inner) = part.strip_prefix("[[").and_then(|p| p.strip_suffix("]]")) else {
        return part;
    };
    match inner.split_once('|') {
        Some((_, shown)) => shown,
        None => inner.rsplit_once('/').map_or(inner, |(_, n)| n),
    }
}

/// `items` grouped by the value `value` reads from each.
pub fn by_value<T>(
    items: impl IntoIterator<Item = T>,
    value: impl Fn(&T) -> Option<&PropertyValue>,
    descending: bool,
) -> Vec<ValueGroup<T>> {
    // (key, label, numeric, items), in first-seen order.
    let mut groups: Vec<(String, String, Option<f64>, Vec<T>)> = Vec::new();
    let mut none = Vec::new();
    for item in items {
        let v = value(&item);
        let p = v.map(parts).unwrap_or_default();
        if p.is_empty() {
            none.push(item);
            continue;
        }
        let key = p.join("\u{0}");
        if let Some(g) = groups.iter_mut().find(|g| g.0 == key) {
            g.3.push(item);
        } else {
            let label = p.iter().map(|s| display(s)).collect::<Vec<_>>().join(", ");
            let numeric = match v {
                Some(PropertyValue::Number(n)) => Some(*n),
                _ => None,
            };
            groups.push((key, label, numeric, vec![item]));
        }
    }
    groups.sort_by(|a, b| {
        let o = match (a.2, b.2) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => natural_cmp(&a.1, &b.1),
        }
        .then_with(|| a.0.cmp(&b.0));
        if descending { o.reverse() } else { o }
    });
    let mut out: Vec<ValueGroup<T>> = groups
        .into_iter()
        .map(|(key, label, _, items)| ValueGroup {
            id: format!("property-value:{key}"),
            label,
            items,
        })
        .collect();
    if !none.is_empty() {
        out.push(ValueGroup {
            id: NONE_ID.to_owned(),
            label: NONE_LABEL.to_owned(),
            items: none,
        });
    }
    out
}
