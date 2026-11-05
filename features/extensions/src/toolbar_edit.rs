//! Editing a toolbar by item (`docs/content/architecture/toolbars.md`).
//!
//! What the visual editor (`crate::toolbar_ui`) and the `TOOLBAR_*`
//! actions both do to a [`ToolbarDecl`], so a person, a script or a model
//! edits it the same way, one item at a time.
//!
//! An item is named by its place ([`At`]): `3` is the fourth item, `2.1`
//! the second button of the group that is the third. A place to put one
//! is the same: `3` before the fourth item (`len` at the end), `2.1`
//! before the group's second button. Pure, wasm-clean.

use serde_json::Value;

use crate::toolbar::{ToolItem, ToolKind, ToolbarDecl};

/// An item's place in a toolbar, or a place to put one: `3`, or `2.1`
/// (inside the group at `2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct At {
    pub index: usize,
    /// Its place inside the group at `index`.
    pub inner: Option<usize>,
}

impl At {
    /// The top-level place `index`.
    #[must_use]
    pub const fn top(index: usize) -> Self {
        Self { index, inner: None }
    }

    /// Place `inner` inside the group at `index`.
    #[must_use]
    pub const fn within(index: usize, inner: usize) -> Self {
        Self {
            index,
            inner: Some(inner),
        }
    }

    /// `3` or `2.1`.
    ///
    /// # Errors
    ///
    /// It is not one.
    pub fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("{text:?} is not a place (`3`, or `2.1` inside a group)");
        let mut parts = text.trim().split('.');
        let index = parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad)?;
        let inner = match parts.next() {
            None => None,
            Some(p) => Some(p.parse().map_err(|_| bad())?),
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(Self { index, inner })
    }
}

impl std::fmt::Display for At {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner {
            None => write!(f, "{}", self.index),
            Some(i) => write!(f, "{}.{i}", self.index),
        }
    }
}

impl std::str::FromStr for At {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Self::parse(s)
    }
}

/// The item at `at`.
#[must_use]
pub fn get(t: &ToolbarDecl, at: At) -> Option<&ToolItem> {
    let top = t.items.get(at.index)?;
    at.inner.map_or(Some(top), |i| top.items.get(i))
}

fn get_mut(t: &mut ToolbarDecl, at: At) -> Option<&mut ToolItem> {
    let top = t.items.get_mut(at.index)?;
    match at.inner {
        None => Some(top),
        Some(i) => top.items.get_mut(i),
    }
}

fn no_item(at: At) -> String {
    format!("no item at {at} (TOOLBAR_GET lists them)")
}

/// Puts `item` at `at` (the end when `None`): its place.
///
/// # Errors
///
/// No such place; a group's place for anything but a button.
pub fn insert(t: &mut ToolbarDecl, at: Option<At>, item: ToolItem) -> Result<At, String> {
    let at = at.unwrap_or_else(|| At::top(t.items.len()));
    match at.inner {
        None => {
            if at.index > t.items.len() {
                return Err(format!("{at} is past the end ({})", t.items.len()));
            }
            t.items.insert(at.index, item);
        }
        Some(i) => {
            if item.kind != ToolKind::Button {
                return Err("a group holds buttons".to_owned());
            }
            let group = t
                .items
                .get_mut(at.index)
                .filter(|g| g.kind == ToolKind::Group)
                .ok_or_else(|| format!("no group at {}", at.index))?;
            if i > group.items.len() {
                return Err(format!(
                    "{at} is past the group's end ({})",
                    group.items.len()
                ));
            }
            group.items.insert(i, item);
        }
    }
    Ok(at)
}

/// Takes away the item at `at` (a group left empty goes too): it.
///
/// # Errors
///
/// No item there.
pub fn remove(t: &mut ToolbarDecl, at: At) -> Result<ToolItem, String> {
    match at.inner {
        None => {
            if at.index >= t.items.len() {
                return Err(no_item(at));
            }
            Ok(t.items.remove(at.index))
        }
        Some(i) => {
            let group = t
                .items
                .get_mut(at.index)
                .filter(|g| g.kind == ToolKind::Group && i < g.items.len())
                .ok_or_else(|| no_item(at))?;
            let item = group.items.remove(i);
            if group.items.is_empty() {
                t.items.remove(at.index);
            }
            Ok(item)
        }
    }
}

/// Moves the item at `from` to place `to` (a place as the toolbar is
/// before the move): its new place.
///
/// # Errors
///
/// No item at `from`; no place `to`; a group into a group.
pub fn move_item(t: &mut ToolbarDecl, from: At, to: At) -> Result<At, String> {
    let item = get(t, from).ok_or_else(|| no_item(from))?.clone();
    if to.inner.is_some() && item.kind != ToolKind::Button {
        return Err("a group holds buttons".to_owned());
    }
    // A place on either side of itself: where it is.
    let beside = |a: usize, b: usize| b == a || b == a.saturating_add(1);
    let stays = match (from.inner, to.inner) {
        (None, None) => beside(from.index, to.index),
        (Some(i), Some(j)) => from.index == to.index && beside(i, j),
        _ => false,
    };
    if stays {
        return Ok(from);
    }
    // Its group goes with it when it is the last in it.
    let group_goes =
        from.inner.is_some() && t.items.get(from.index).is_some_and(|g| g.items.len() == 1);
    remove(t, from)?;
    let mut to = to;
    match from.inner {
        // What was after it moved up one.
        None if to.index > from.index => to.index = to.index.saturating_sub(1),
        Some(_) if group_goes && to.index > from.index => {
            to.index = to.index.saturating_sub(1);
        }
        Some(i) if !group_goes && to.index == from.index => {
            if let Some(j) = to.inner.filter(|j| *j > i) {
                to.inner = Some(j.saturating_sub(1));
            }
        }
        _ => {}
    }
    insert(t, Some(to), item)
}

/// Sets the fields `patch` names on the item at `at` (an object of
/// [`ToolItem`]'s fields; `null` clears one).
///
/// # Errors
///
/// No item there; a patch that is not an object, or not an item's fields.
pub fn set(t: &mut ToolbarDecl, at: At, patch: &Value) -> Result<(), String> {
    let item = get_mut(t, at).ok_or_else(|| no_item(at))?;
    *item = patched(&*item, patch)?;
    Ok(())
}

/// Sets the toolbar's own fields `patch` names (`title`, `where`,
/// `hide_labels`, `style`; not `id` or `items`).
///
/// # Errors
///
/// A patch that is not an object, names `id` or `items`, or is not a
/// toolbar's fields.
pub fn configure(t: &mut ToolbarDecl, patch: &Value) -> Result<(), String> {
    let fields = patch.as_object().ok_or("a patch is an object of fields")?;
    if fields.contains_key("id") || fields.contains_key("items") {
        return Err(
            "its id and items are not set this way (TOOLBAR_RENAME, TOOLBAR_INSERT…)".into(),
        );
    }
    let items = std::mem::take(&mut t.items);
    let mut next: ToolbarDecl = patched(&*t, patch)?;
    next.items = items;
    *t = next;
    Ok(())
}

/// The items `at.index..at.index + count` drawn as one group labelled
/// `label`: its place.
///
/// # Errors
///
/// Fewer than `count` items there, or one that is not a button.
pub fn group(t: &mut ToolbarDecl, at: At, count: usize, label: &str) -> Result<At, String> {
    if at.inner.is_some() {
        return Err("already in a group".to_owned());
    }
    let end = at.index.saturating_add(count.max(1));
    let taken = t
        .items
        .get(at.index..end)
        .ok_or_else(|| format!("fewer than {count} items from {at}"))?;
    if let Some(n) = taken.iter().position(|i| i.kind != ToolKind::Button) {
        return Err(format!(
            "item {} is not a button: a group holds buttons",
            at.index.saturating_add(n)
        ));
    }
    let items: Vec<ToolItem> = t.items.drain(at.index..end).collect();
    t.items.insert(
        at.index,
        ToolItem {
            kind: ToolKind::Group,
            label: label.to_owned(),
            items,
            ..ToolItem::default()
        },
    );
    Ok(at)
}

/// The group at `at` back into its buttons, in its place.
///
/// # Errors
///
/// No group there.
pub fn ungroup(t: &mut ToolbarDecl, at: At) -> Result<(), String> {
    let at = At::top(at.index);
    if get(t, at).is_none_or(|g| g.kind != ToolKind::Group) {
        return Err(format!("no group at {at}"));
    }
    let g = t.items.remove(at.index);
    for (n, b) in g.items.into_iter().enumerate() {
        t.items.insert(at.index.saturating_add(n), b);
    }
    Ok(())
}

/// `value` with `patch`'s fields over it, read back as a `T`.
fn patched<T>(value: &T, patch: &Value) -> Result<T, String>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let fields = patch.as_object().ok_or("a patch is an object of fields")?;
    let mut whole = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let obj = whole.as_object_mut().ok_or("not an object")?;
    for (k, v) in fields {
        if v.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(whole).map_err(|e| format!("not its fields: {e}"))
}

/// An item made from JSON (an action's argument), its kind a button when
/// it says none: `{"action": "A"}`, `{"kind": "separator"}`.
///
/// # Errors
///
/// It is not an item.
pub fn item_from(json: &Value) -> Result<ToolItem, String> {
    serde_json::from_value(json.clone()).map_err(|e| format!("not an item: {e}"))
}

/// A short id from `title`: lower case words joined by `-` (`My tools` →
/// `my-tools`), `toolbar` when nothing is left.
#[must_use]
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "toolbar".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use serde_json::json;

    use super::*;

    fn button(action: &str) -> ToolItem {
        ToolItem {
            action: action.into(),
            ..ToolItem::default()
        }
    }

    /// The toolbar's buttons' actions, a group's in brackets.
    fn shape(t: &ToolbarDecl) -> String {
        t.items
            .iter()
            .map(|i| match i.kind {
                ToolKind::Group => format!(
                    "[{}]",
                    i.items
                        .iter()
                        .map(|b| b.action.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                ToolKind::Separator => "|".to_owned(),
                _ => i.action.clone(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn bar(actions: &[&str]) -> ToolbarDecl {
        ToolbarDecl {
            items: actions.iter().map(|a| button(a)).collect(),
            ..ToolbarDecl::default()
        }
    }

    #[test]
    fn a_place_reads_and_says_itself() {
        assert_eq!(At::parse("3").unwrap(), At::top(3));
        assert_eq!(At::parse("2.1").unwrap(), At::within(2, 1));
        assert_eq!(At::within(2, 1).to_string(), "2.1");
        for bad in ["", "a", "1.", "1.2.3", "-1"] {
            assert!(At::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn items_go_in_come_out_and_move_by_place() {
        let mut t = bar(&["A", "B", "C"]);
        insert(&mut t, None, button("D")).unwrap();
        insert(
            &mut t,
            Some(At::top(0)),
            ToolItem {
                kind: ToolKind::Separator,
                ..ToolItem::default()
            },
        )
        .unwrap();
        assert_eq!(shape(&t), "| A B C D");
        assert!(insert(&mut t, Some(At::top(9)), button("X")).is_err());
        // Forward and back along the top level.
        move_item(&mut t, At::top(1), At::top(4)).unwrap();
        assert_eq!(shape(&t), "| B C A D");
        move_item(&mut t, At::top(3), At::top(0)).unwrap();
        assert_eq!(shape(&t), "A | B C D");
        // Beside itself: nothing moves.
        move_item(&mut t, At::top(2), At::top(3)).unwrap();
        assert_eq!(shape(&t), "A | B C D");
        assert_eq!(
            remove(&mut t, At::top(1)).unwrap().kind,
            ToolKind::Separator
        );
        assert!(remove(&mut t, At::top(9)).is_err());
    }

    #[test]
    fn buttons_regroup_and_a_group_left_empty_goes() {
        let mut t = bar(&["A", "B", "C", "D"]);
        group(&mut t, At::top(1), 2, "Loop").unwrap();
        assert_eq!(shape(&t), "A [B C] D");
        // Into the group, at its end and its start.
        move_item(&mut t, At::top(2), At::within(1, 2)).unwrap();
        assert_eq!(shape(&t), "A [B C D]");
        move_item(&mut t, At::top(0), At::within(1, 0)).unwrap();
        assert_eq!(shape(&t), "[A B C D]");
        // Along it.
        move_item(&mut t, At::within(0, 0), At::within(0, 3)).unwrap();
        assert_eq!(shape(&t), "[B C A D]");
        // Out of it, after it.
        move_item(&mut t, At::within(0, 3), At::top(1)).unwrap();
        assert_eq!(shape(&t), "[B C A] D");
        // A whole group moves as one; into a group it may not.
        move_item(&mut t, At::top(0), At::top(2)).unwrap();
        assert_eq!(shape(&t), "D [B C A]");
        assert!(move_item(&mut t, At::top(1), At::within(1, 0)).is_err());
        ungroup(&mut t, At::top(1)).unwrap();
        assert_eq!(shape(&t), "D B C A");
        group(&mut t, At::top(0), 1, "").unwrap();
        // The last one out: the group goes, what was after it moves up.
        move_item(&mut t, At::within(0, 0), At::top(4)).unwrap();
        assert_eq!(shape(&t), "B C A D");
        assert!(group(&mut t, At::top(3), 2, "").is_err());
    }

    #[test]
    fn a_patch_sets_and_clears_fields() {
        let mut t = bar(&["A"]);
        t.title = "T".into();
        set(
            &mut t,
            At::top(0),
            &json!({"label": "Play", "colors": {"fill": "primary"}, "icon": "play"}),
        )
        .unwrap();
        assert_eq!(t.items[0].label, "Play");
        assert_eq!(t.items[0].colors.fill, "primary");
        set(&mut t, At::top(0), &json!({"icon": null})).unwrap();
        assert!(t.items[0].icon.is_empty());
        assert!(set(&mut t, At::top(0), &json!({"kind": "nope"})).is_err());
        assert!(set(&mut t, At::top(0), &json!("label")).is_err());
        configure(
            &mut t,
            &json!({"title": "Mine", "style": {"rounding": 12}, "hide_labels": true}),
        )
        .unwrap();
        assert_eq!(
            (t.title.as_str(), t.style.rounding, t.hide_labels),
            ("Mine", Some(12), true)
        );
        assert_eq!(t.items.len(), 1, "its items stay");
        assert!(configure(&mut t, &json!({"items": []})).is_err());
        assert_eq!(
            item_from(&json!({"kind": "separator"})).unwrap().kind,
            ToolKind::Separator
        );
        assert_eq!(slug(" My  tools! "), "my-tools");
        assert_eq!(slug("…"), "toolbar");
    }
}
