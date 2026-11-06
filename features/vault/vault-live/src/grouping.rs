//! Grouping rows by a value: what a base's `groupBy` does
//! ([`crate::bases::execute_view`]), and what the navigator's list pane
//! (`features/navigator`) groups notes by a property with. One
//! implementation for both.
//!
//! A group's label is its value as text ([`label`]): a string as it is,
//! a number or a boolean written out, `null` as `""`, a list or an object
//! as its JSON. Groups keep the order their first item came in
//! ([`Order::FirstSeen`], a base's), or are sorted by label
//! ([`compare_labels`]: numbers by value and before text, text in natural
//! order, `Item 2` before `Item 10`).

use std::cmp::Ordering;

/// One group: its label and its items, in the order they came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<T> {
    /// `""` for items with no value.
    pub label: String,
    pub items: Vec<T>,
}

/// The order groups come in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// As their first item came (a base's `groupBy`).
    #[default]
    FirstSeen,
    /// By label, numbers by value; the empty group last.
    Asc,
    /// By label, the other way; the empty group last.
    Desc,
}

/// A value as a group's label.
#[must_use]
pub fn label(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `items` grouped by the label `key` gives each, in `order`.
pub fn group_by<T>(
    items: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> String,
    order: Order,
) -> Vec<Group<T>> {
    let mut groups: indexmap::IndexMap<String, Vec<T>> = indexmap::IndexMap::new();
    for item in items {
        groups.entry(key(&item)).or_default().push(item);
    }
    let mut out: Vec<Group<T>> = groups
        .into_iter()
        .map(|(label, items)| Group { label, items })
        .collect();
    if order != Order::FirstSeen {
        out.sort_by(|a, b| {
            match (a.label.is_empty(), b.label.is_empty()) {
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => {}
            }
            let by = compare_labels(&a.label, &b.label);
            if order == Order::Desc {
                by.reverse()
            } else {
                by
            }
        });
    }
    out
}

/// Two labels compared: as numbers when both are, a number before text,
/// else as text in natural order (any case, runs of digits by value); the
/// same text in another case by its bytes, so the order is total.
#[must_use]
pub fn compare_labels(a: &str, b: &str) -> Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => natural(a, b),
    }
    .then_with(|| a.cmp(b))
}

/// `a` and `b` as text a person reads: letters in any case, each run of
/// digits as the number it writes.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) else {
            return a.is_empty().cmp(&b.is_empty()).reverse();
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (na, ra) = split_digits(a);
            let (nb, rb) = split_digits(b);
            let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
            let by = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if by != Ordering::Equal {
                return by;
            }
            (a, b) = (ra, rb);
        } else {
            let by = x.to_lowercase().cmp(y.to_lowercase());
            if by != Ordering::Equal {
                return by;
            }
            (a, b) = (
                a.get(x.len_utf8()..).unwrap_or_default(),
                b.get(y.len_utf8()..).unwrap_or_default(),
            );
        }
    }
}

/// The digits `s` starts with, and the rest.
fn split_digits(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.split_at(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn groups_keep_first_seen_order_or_sort_with_the_empty_group_last() {
        let rows = [
            ("a", json!("todo")),
            ("b", json!(null)),
            ("c", json!("done")),
            ("d", json!("todo")),
        ];
        let first = group_by(rows.iter(), |r| label(&r.1), Order::FirstSeen);
        let labels: Vec<_> = first.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["todo", "", "done"]);
        assert_eq!(first[0].items.len(), 2);
        let asc = group_by(rows.iter(), |r| label(&r.1), Order::Asc);
        let labels: Vec<_> = asc.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["done", "todo", ""]);
        let desc = group_by(rows.iter(), |r| label(&r.1), Order::Desc);
        let labels: Vec<_> = desc.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["todo", "done", ""]);
    }

    #[test]
    fn text_sorts_naturally_after_the_numbers() {
        let g = group_by(
            ["Item 10", "item 2", "3", "Alpha", "Item 2"],
            |s| (*s).to_owned(),
            Order::Asc,
        );
        let labels: Vec<_> = g.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["3", "Alpha", "Item 2", "item 2", "Item 10"]);
    }

    #[test]
    fn numbers_sort_by_value() {
        let g = group_by(["10", "9", "100"], |s| (*s).to_owned(), Order::Asc);
        let labels: Vec<_> = g.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["9", "10", "100"]);
    }
}
