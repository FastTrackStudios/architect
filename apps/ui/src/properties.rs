//! A note's Properties (#442): its YAML frontmatter as Obsidian's
//! Properties panel, above the note's editor, folded by default.
//!
//! The editor (`crate::note_editor`) holds the note's body; this holds its
//! frontmatter block, read and written with the vault's own frontmatter
//! code (`vault::obsidian::obsidian_parse` reads it,
//! `vault::obsidian::frontmatter_write` writes an edit in place, keeping
//! each untouched key's lines byte for byte). A property's type is what
//! the vault declares (`.obsidian/types.json`), else the built-in schema
//! for the note's `kind:` (`vault::property_schema`), else what its value
//! looks like.
//!
//! The edits are text to text ([`set`], [`rename`], [`remove`],
//! [`move_to`], [`add`]): the note on show is the typing, not the file, so
//! the panel edits what the editor's note will save, through the same save.
//! (The navigator's `NAVIGATOR_PROPERTY_*` write files on disk and across
//! notes: a key renamed everywhere, not one note's.)

use std::collections::HashMap;
use std::sync::Arc;

use architect_shell::use_shell;
use dioxus::prelude::*;
use lucide_dioxus::{
    Calendar, CalendarClock, ChevronDown, ChevronRight, ChevronUp, Hash, Link, List, Plus,
    SquareCheck, Tags, Type, X,
};
use serde_json::Value;
use vault::obsidian::frontmatter_write::write_frontmatter;
use vault::obsidian::obsidian_parse::{FrontmatterEntry, parse_frontmatter, serialize_frontmatter};
use vault::property_schema::{FALLBACK_KIND, KIND_KEY, PropertySchemaRegistry, PropertyType};

/// What a property holds, and so how its row edits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    List,
    Number,
    Checkbox,
    Date,
    Datetime,
    Tags,
    /// One `[[wikilink]]`.
    Link,
    /// A list of `[[wikilinks]]`.
    Links,
}

impl Kind {
    /// Its name, as `.obsidian/types.json` and the row's menu say it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::List => "list",
            Self::Number => "number",
            Self::Checkbox => "checkbox",
            Self::Date => "date",
            Self::Datetime => "datetime",
            Self::Tags => "tags",
            Self::Link => "link",
            Self::Links => "links",
        }
    }

    const fn is_list(self) -> bool {
        matches!(self, Self::List | Self::Tags | Self::Links)
    }
}

/// A note's text cut into its frontmatter block (fences and the newline
/// after them; empty for none) and its body. A block that is not a YAML
/// mapping stays in the body, as source to fix by hand.
#[must_use]
pub fn split(text: &str) -> (&str, &str) {
    let (entries, offset) = parse_frontmatter(text);
    let Some(head) = text.get(..offset) else {
        return ("", text);
    };
    if offset == 0 || (entries.is_empty() && has_yaml(head)) {
        return ("", text);
    }
    (head, text.get(offset..).unwrap_or_default())
}

/// Whether a fenced block holds anything but fences, blanks and comments.
fn has_yaml(block: &str) -> bool {
    block.lines().any(|l| {
        let t = l.trim();
        !t.is_empty() && t != "---" && !t.starts_with('#')
    })
}

/// The block's properties, in order.
#[must_use]
pub fn entries(block: &str) -> Vec<FrontmatterEntry> {
    parse_frontmatter(block).0
}

/// `block` written again as `after`, in place where it can be.
fn rewrite(block: &str, before: &[FrontmatterEntry], after: &[FrontmatterEntry]) -> String {
    if before.is_empty() {
        return serialize_frontmatter(after);
    }
    let out = write_frontmatter(block, block.len(), before, after);
    // In place keeps the block's order: a new order is written afresh.
    let keys = |e: &[FrontmatterEntry]| e.iter().map(|e| e.key.clone()).collect::<Vec<_>>();
    if keys(&entries(&out)) == keys(after) {
        out
    } else {
        serialize_frontmatter(after)
    }
}

/// `key` set to `value` (added at the end when the note lacks it).
#[must_use]
pub fn set(block: &str, key: &str, value: Value) -> String {
    let before = entries(block);
    let mut after = before.clone();
    match after.iter_mut().find(|e| e.key == key) {
        Some(e) => e.value = value,
        None => after.push(FrontmatterEntry {
            key: key.to_owned(),
            value,
        }),
    }
    rewrite(block, &before, &after)
}

/// A new, empty property `key`; `None` when the name is no good or taken.
#[must_use]
pub fn add(block: &str, key: &str) -> Option<String> {
    let key = key.trim();
    if !valid_key(key) || entries(block).iter().any(|e| e.key == key) {
        return None;
    }
    Some(set(block, key, Value::Null))
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && !key.contains(['\n', ':', '#']) && !key.starts_with(['-', ' '])
}

/// The property without `key`; the block gone with its last one.
#[must_use]
pub fn remove(block: &str, key: &str) -> String {
    let before = entries(block);
    let after: Vec<_> = before.iter().filter(|e| e.key != key).cloned().collect();
    write_frontmatter(block, block.len(), &before, &after)
}

/// Property `from` called `to`, in its place, its value's lines as they
/// were; `None` when `to` is no good or another property's.
#[must_use]
pub fn rename(block: &str, from: &str, to: &str) -> Option<String> {
    let to = to.trim();
    let before = entries(block);
    if to == from {
        return Some(block.to_owned());
    }
    if !valid_key(to) || before.iter().any(|e| e.key == to) {
        return None;
    }
    let after: Vec<_> = before
        .iter()
        .map(|e| FrontmatterEntry {
            key: if e.key == from {
                to.to_owned()
            } else {
                e.key.clone()
            },
            value: e.value.clone(),
        })
        .collect();
    // The key's own line, as written: the rest of the block untouched.
    let mut out = String::with_capacity(block.len());
    let mut done = false;
    for line in block.split_inclusive('\n') {
        if !done && let Some(rest) = line.strip_prefix(from).filter(|r| r.starts_with(':')) {
            out.push_str(to);
            out.push_str(rest);
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if done && entries(&out) == after {
        Some(out)
    } else {
        Some(serialize_frontmatter(&after))
    }
}

/// The property at `from` moved to `to`.
#[must_use]
pub fn move_to(block: &str, from: usize, to: usize) -> String {
    let before = entries(block);
    let mut after = before.clone();
    if from >= after.len() || to >= after.len() || from == to {
        return block.to_owned();
    }
    let e = after.remove(from);
    after.insert(to, e);
    rewrite(block, &before, &after)
}

/// What a vault says its properties are: `.obsidian/types.json`'s `types`.
#[must_use]
pub fn declared_types(root: &std::path::Path) -> HashMap<String, String> {
    let path = root.join(".obsidian").join("types.json");
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v.get("types").cloned())
        .and_then(|t| serde_json::from_value(t).ok())
        .unwrap_or_default()
}

/// The types a note's properties are read as.
#[derive(Clone, Default)]
pub struct Types {
    pub declared: Arc<HashMap<String, String>>,
}

impl PartialEq for Types {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.declared, &other.declared)
    }
}

/// The type of property `key` holding `value`, in a note of `kind`.
#[must_use]
pub fn kind_of(types: &Types, kind: &str, key: &str, value: &Value) -> Kind {
    let declared = types
        .declared
        .get(key)
        .and_then(|t| from_obsidian(t))
        .or_else(|| {
            static SCHEMA: std::sync::LazyLock<PropertySchemaRegistry> =
                std::sync::LazyLock::new(PropertySchemaRegistry::with_builtins);
            SCHEMA.lookup(kind, key).and_then(|d| from_schema(&d.ty))
        });
    let seen = by_value(key, value);
    match declared {
        // A declared type the value cannot be is the value's.
        Some(k) if fits(k, value) => k,
        _ => seen,
    }
}

fn from_obsidian(t: &str) -> Option<Kind> {
    Some(match t {
        "text" => Kind::Text,
        "multitext" | "aliases" => Kind::List,
        "number" => Kind::Number,
        "checkbox" => Kind::Checkbox,
        "date" => Kind::Date,
        "datetime" => Kind::Datetime,
        "tags" => Kind::Tags,
        _ => return None,
    })
}

const fn from_schema(t: &PropertyType) -> Option<Kind> {
    Some(match t {
        PropertyType::Text | PropertyType::Multitext | PropertyType::EnumWithMetadata { .. } => {
            Kind::Text
        }
        PropertyType::Number => Kind::Number,
        PropertyType::Checkbox => Kind::Checkbox,
        PropertyType::Date => Kind::Date,
        PropertyType::Datetime => Kind::Datetime,
        PropertyType::Tags => Kind::Tags,
        PropertyType::Aliases => Kind::List,
        PropertyType::Link => Kind::Link,
        PropertyType::LinkList => Kind::Links,
        _ => return None,
    })
}

fn fits(k: Kind, v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(_) => k == Kind::Checkbox,
        Value::Number(_) => matches!(k, Kind::Number | Kind::Text),
        Value::String(s) => match k {
            Kind::Date => is_date(s),
            Kind::Datetime => is_datetime(s) || is_date(s),
            Kind::Number => s.parse::<f64>().is_ok(),
            Kind::Checkbox => false,
            _ => true,
        },
        Value::Array(_) => k.is_list(),
        Value::Object(_) => false,
    }
}

fn by_value(key: &str, v: &Value) -> Kind {
    let k = key.to_ascii_lowercase();
    if k == "tags" || k == "tag" {
        return Kind::Tags;
    }
    if k == "aliases" {
        return Kind::List;
    }
    match v {
        Value::Bool(_) => Kind::Checkbox,
        Value::Number(_) => Kind::Number,
        Value::String(s) if is_datetime(s) => Kind::Datetime,
        Value::String(s) if is_date(s) => Kind::Date,
        Value::String(s) if is_link(s) => Kind::Link,
        Value::Array(items)
            if !items.is_empty() && items.iter().all(|i| i.as_str().is_some_and(is_link)) =>
        {
            Kind::Links
        }
        Value::Array(_) => Kind::List,
        _ => Kind::Text,
    }
}

fn is_link(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("[[") && s.ends_with("]]")
}

/// `YYYY-MM-DD`.
#[must_use]
pub fn is_date(s: &str) -> bool {
    parse_date(s).is_some()
}

/// `YYYY-MM-DDTHH:MM[:SS…]`.
#[must_use]
pub fn is_datetime(s: &str) -> bool {
    s.split_once('T')
        .is_some_and(|(d, t)| is_date(d) && parse_time(t).is_some())
}

/// `(year, month, day)` of `YYYY-MM-DD`.
#[must_use]
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.trim().splitn(3, '-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && d >= 1 && d <= days_in(y, m)).then_some((y, m, d))
}

/// `(hour, minute)` of `HH:MM[:SS…]`.
fn parse_time(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split(':');
    let h: u32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.get(..2)?.parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

const fn leap(y: i32) -> bool {
    (y.rem_euclid(4) == 0 && y.rem_euclid(100) != 0) || y.rem_euclid(400) == 0
}

/// How many days month `m` of year `y` has.
#[must_use]
pub const fn days_in(y: i32, m: u32) -> u32 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The weekday of `y-m-d`, Monday 0 … Sunday 6 (Sakamoto's method).
#[must_use]
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y.saturating_sub(1) } else { y };
    let t = usize::try_from(m.saturating_sub(1))
        .ok()
        .and_then(|i| T.get(i).copied())
        .unwrap_or(0);
    let d = i32::try_from(d).unwrap_or(1);
    let sum = y
        .saturating_add(y.div_euclid(4))
        .saturating_sub(y.div_euclid(100))
        .saturating_add(y.div_euclid(400))
        .saturating_add(t)
        .saturating_add(d);
    // Sunday 0 → Monday 0.
    u32::try_from(sum.saturating_add(6).rem_euclid(7)).unwrap_or(0)
}

fn fmt_date((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

const MONTHS: [&str; 12] = [
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

/// A value's text, for a text row.
fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A value's items, for a list row.
fn items_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(text_of).filter(|s| !s.is_empty()).collect(),
        Value::Null => Vec::new(),
        other => {
            let s = text_of(other);
            if s.is_empty() { Vec::new() } else { vec![s] }
        }
    }
}

/// The note a `[[link]]` names: `[[a/b|c]]` → `a/b`.
#[must_use]
pub fn link_target(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|t| t.strip_suffix("]]"))
        .unwrap_or(t);
    t.split(['|', '#']).next().unwrap_or(t).trim().to_owned()
}

fn link_label(s: &str) -> String {
    let t = s.trim();
    let t = t
        .strip_prefix("[[")
        .and_then(|t| t.strip_suffix("]]"))
        .unwrap_or(t);
    t.rsplit_once('|').map_or_else(
        || t.rsplit('/').next().unwrap_or(t).to_owned(),
        |(_, alias)| alias.to_owned(),
    )
}

const STYLE: &str = "
.note-props { font-size: 13px; }
.note-props .np-head { display: flex; align-items: center; gap: 6px; cursor: pointer; padding: 4px 6px;
  margin-left: -6px; border-radius: 6px; user-select: none; }
.note-props .np-head:hover { background: var(--np-hover); }
.note-props .np-row { display: flex; align-items: flex-start; gap: 8px; min-height: 28px; padding: 2px 0; }
.note-props .np-row:hover { background: var(--np-hover); border-radius: 6px; }
.note-props .np-key { display: flex; align-items: center; gap: 6px; width: 30%; min-width: 84px; max-width: 160px; flex-shrink: 0; height: 28px; }
.note-props .np-key input { width: 100%; background: transparent; border: none; outline: none; font-size: 13px; padding: 0; }
.note-props .np-val { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 4px; min-height: 28px; }
.note-props .np-val input { flex: 1; min-width: 48px; background: transparent; border: none; outline: none; font-size: 13px; padding: 4px 0; }
.note-props .np-pill { display: inline-flex; align-items: center; gap: 4px; padding: 1px 8px; border-radius: 999px; font-size: 12px; line-height: 20px; }
.note-props .np-pill .np-x { cursor: pointer; opacity: 0.6; }
.note-props .np-tools { display: flex; gap: 2px; opacity: 0; height: 28px; align-items: center; }
.note-props .np-row:hover .np-tools { opacity: 1; }
.note-props .np-tool { cursor: pointer; padding: 3px; border-radius: 4px; display: flex; }
.note-props .np-tool:hover { background: var(--np-hover); }
.note-props .np-cal { display: grid; grid-template-columns: repeat(7, 28px); gap: 2px; }
.note-props .np-day { height: 26px; display: flex; align-items: center; justify-content: center; border-radius: 6px; cursor: pointer; font-size: 12px; }
.note-props .np-day:hover { background: var(--np-hover); }
";

/// A note's Properties: `block` its frontmatter (empty for none), each
/// edit given to `on_change` as the block written again; a link's note
/// given to `on_link`.
#[component]
pub fn PropertiesBar(
    block: String,
    types: Types,
    on_change: Callback<String>,
    #[props(default)] on_link: Option<Callback<String>>,
    /// Keys not shown as rows (a collection's list draws them, #668); the
    /// block keeps them.
    #[props(default)]
    hidden: Vec<String>,
) -> Element {
    let t = use_shell().theme;
    let mut open = use_signal(|| false);
    let mut adding = use_signal(|| None::<String>);
    let list = entries(&block);
    let kind = list
        .iter()
        .find(|e| e.key == KIND_KEY)
        .and_then(|e| e.value.as_str())
        .unwrap_or(FALLBACK_KIND)
        .to_owned();
    let count = list.len();
    let last = count.saturating_sub(1);
    let shown = list.iter().filter(|e| !hidden.contains(&e.key)).count();
    let vars = format!("--np-hover: {};", t.pill);
    let rows = list
        .into_iter()
        .enumerate()
        .filter(|(_, e)| !hidden.contains(&e.key))
        .map(|(i, e)| {
            let k = kind_of(&types, &kind, &e.key, &e.value);
            let on_value = {
                let (block, key) = (block.clone(), e.key.clone());
                move |v: Value| on_change.call(set(&block, &key, v))
            };
            let on_rename = {
                let (block, key) = (block.clone(), e.key.clone());
                move |to: String| {
                    if let Some(b) = rename(&block, &key, &to) {
                        on_change.call(b);
                    }
                }
            };
            let on_remove = {
                let (block, key) = (block.clone(), e.key.clone());
                move |()| on_change.call(remove(&block, &key))
            };
            let on_move = {
                let block = block.clone();
                move |up: bool| {
                    let to = if up {
                        i.saturating_sub(1)
                    } else {
                        i.saturating_add(1)
                    };
                    on_change.call(move_to(&block, i, to));
                }
            };
            rsx! {
                PropertyRow {
                    key: "{e.key}",
                    name: e.key.clone(),
                    value: e.value,
                    kind: k,
                    first: i == 0,
                    last: i == last,
                    on_value,
                    on_rename,
                    on_remove,
                    on_move,
                    on_link,
                }
            }
        });
    rsx! {
        document::Style { {STYLE} }
        div {
            class: "note-props",
            "data-testid": "note-properties",
            "data-open": "{open()}",
            style: "{vars} color: {t.ink}; border-bottom: 1px solid {t.rule}; padding-bottom: 6px;",
            div {
                class: "np-head",
                "data-testid": "note-properties-toggle",
                onclick: move |_| open.toggle(),
                if open() {
                    ChevronDown { size: 14, color: t.ink_3.to_string() }
                } else {
                    ChevronRight { size: 14, color: t.ink_3.to_string() }
                }
                span { style: "font-weight: 600; color: {t.ink_2};", "Properties" }
                span {
                    style: "font-size: 11px; color: {t.ink_3}; background: {t.pill}; border-radius: 999px; padding: 0 7px; line-height: 18px;",
                    "{shown}"
                }
            }
            if open() {
                div { style: "display: flex; flex-direction: column; padding: 4px 0 2px 0;",
                    {rows}
                    if let Some(name) = adding() {
                        div { class: "np-row",
                            div { class: "np-key",
                                Plus { size: 14, color: t.ink_3.to_string() }
                                input {
                                    "data-testid": "note-property-new",
                                    "aria-label": "New property name",
                                    placeholder: "Property name",
                                    style: "color: {t.ink};",
                                    autofocus: "true",
                                    value: "{name}",
                                    oninput: move |e| adding.set(Some(e.value())),
                                    onkeydown: {
                                        let block = block.clone();
                                        move |e: KeyboardEvent| match e.key().to_string().as_str() {
                                            "Enter" => {
                                                let name = adding().unwrap_or_default();
                                                if let Some(b) = add(&block, &name) {
                                                    on_change.call(b);
                                                }
                                                adding.set(None);
                                            }
                                            "Escape" => adding.set(None),
                                            _ => {}
                                        }
                                    },
                                    onblur: move |_| adding.set(None),
                                }
                            }
                        }
                    } else {
                        div {
                            class: "np-head",
                            "data-testid": "note-property-add",
                            style: "color: {t.ink_3}; margin-top: 2px;",
                            onclick: move |_| adding.set(Some(String::new())),
                            Plus { size: 14, color: t.ink_3.to_string() }
                            span { "Add property" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn KindIcon(kind: Kind, color: String) -> Element {
    let size = 14;
    match kind {
        Kind::Text => rsx! { Type { size, color } },
        Kind::List => rsx! { List { size, color } },
        Kind::Number => rsx! { Hash { size, color } },
        Kind::Checkbox => rsx! { SquareCheck { size, color } },
        Kind::Date => rsx! { Calendar { size, color } },
        Kind::Datetime => rsx! { CalendarClock { size, color } },
        Kind::Tags => rsx! { Tags { size, color } },
        Kind::Link | Kind::Links => rsx! { Link { size, color } },
    }
}

#[component]
fn PropertyRow(
    name: String,
    value: Value,
    kind: Kind,
    first: bool,
    last: bool,
    on_value: Callback<Value>,
    on_rename: Callback<String>,
    on_remove: Callback<()>,
    on_move: Callback<bool>,
    on_link: Option<Callback<String>>,
) -> Element {
    let t = use_shell().theme;
    let mut key_draft = use_signal(|| None::<String>);
    let shown_key = key_draft().unwrap_or_else(|| name.clone());
    let commit_key = move || {
        let mut key_draft = key_draft;
        if let Some(to) = key_draft.take() {
            on_rename.call(to);
        }
    };
    rsx! {
        div { class: "np-row", "data-property": "{name}", "data-kind": "{kind.name()}",
            div { class: "np-key", title: "{kind.name()}",
                KindIcon { kind, color: t.ink_3.to_string() }
                input {
                    "aria-label": "Property name",
                    style: "color: {t.ink_2};",
                    value: "{shown_key}",
                    oninput: move |e| key_draft.set(Some(e.value())),
                    onkeydown: move |e: KeyboardEvent| if e.key().to_string() == "Enter" { commit_key() },
                    onblur: move |_| commit_key(),
                }
            }
            div { class: "np-val",
                ValueEditor { value, kind, on_value, on_link }
            }
            div { class: "np-tools",
                if !first {
                    span { class: "np-tool", title: "Move up", onclick: move |_| on_move.call(true),
                        ChevronUp { size: 14, color: t.ink_3.to_string() }
                    }
                }
                if !last {
                    span { class: "np-tool", title: "Move down", onclick: move |_| on_move.call(false),
                        ChevronDown { size: 14, color: t.ink_3.to_string() }
                    }
                }
                span { class: "np-tool", "data-testid": "note-property-remove", title: "Remove property",
                    onclick: move |_| on_remove.call(()),
                    X { size: 14, color: t.ink_3.to_string() }
                }
            }
        }
    }
}

#[component]
fn ValueEditor(
    value: Value,
    kind: Kind,
    on_value: Callback<Value>,
    on_link: Option<Callback<String>>,
) -> Element {
    match kind {
        Kind::Checkbox => rsx! { CheckboxValue { value, on_value } },
        Kind::Date | Kind::Datetime => {
            rsx! { DateValue { value, with_time: kind == Kind::Datetime, on_value } }
        }
        Kind::List | Kind::Tags | Kind::Links => {
            rsx! { ListValue { value, kind, on_value, on_link } }
        }
        Kind::Link => rsx! { LinkValue { value, on_value, on_link } },
        Kind::Text | Kind::Number => {
            rsx! { TextValue { value, number: kind == Kind::Number, on_value } }
        }
    }
}

#[component]
fn TextValue(value: Value, number: bool, on_value: Callback<Value>) -> Element {
    let t = use_shell().theme;
    let text = text_of(&value);
    let mut draft = use_signal(|| None::<String>);
    let shown = draft().unwrap_or_else(|| text.clone());
    let commit = move || {
        let Some(s) = draft.take() else { return };
        if s == text {
            return;
        }
        let v = if s.trim().is_empty() {
            Value::Null
        } else if number {
            s.trim()
                .parse::<i64>()
                .map(Value::from)
                .or_else(|_| s.trim().parse::<f64>().map(Value::from))
                .unwrap_or(Value::String(s))
        } else {
            Value::String(s)
        };
        on_value.call(v);
    };
    let mut commit_enter = commit.clone();
    let mut commit_blur = commit;
    rsx! {
        input {
            "aria-label": "Property value",
            style: "color: {t.ink};",
            placeholder: "Empty",
            inputmode: if number { "decimal" } else { "text" },
            value: "{shown}",
            oninput: move |e| draft.set(Some(e.value())),
            onkeydown: move |e: KeyboardEvent| if e.key().to_string() == "Enter" { commit_enter() },
            onblur: move |_| commit_blur(),
        }
    }
}

#[component]
fn CheckboxValue(value: Value, on_value: Callback<Value>) -> Element {
    let t = use_shell().theme;
    let on = value.as_bool().unwrap_or(false);
    let (bg, border) = if on {
        (t.accent.clone(), t.accent.clone())
    } else {
        ("transparent".into(), t.ink_3.clone())
    };
    rsx! {
        span {
            role: "checkbox",
            "aria-checked": "{on}",
            "data-testid": "note-property-checkbox",
            style: "width: 16px; height: 16px; border-radius: 4px; border: 1.5px solid {border}; background: {bg}; \
                    display: inline-flex; align-items: center; justify-content: center; cursor: pointer; color: {t.accent_ink}; font-size: 11px;",
            onclick: move |_| on_value.call(Value::Bool(!on)),
            if on { "✓" }
        }
    }
}

#[component]
fn LinkValue(
    value: Value,
    on_value: Callback<Value>,
    on_link: Option<Callback<String>>,
) -> Element {
    let s = text_of(&value);
    if s.is_empty() {
        return rsx! { TextValue { value, number: false, on_value } };
    }
    rsx! {
        Pill {
            label: link_label(&s),
            link: true,
            on_open: move |()| if let Some(go) = on_link { go.call(link_target(&s)) },
            on_remove: move |()| on_value.call(Value::Null),
        }
    }
}

#[component]
fn Pill(
    label: String,
    #[props(default)] link: bool,
    #[props(default)] on_open: Option<Callback<()>>,
    on_remove: Callback<()>,
) -> Element {
    let t = use_shell().theme;
    let (bg, ink) = if link {
        (t.accent_soft, t.accent)
    } else {
        (t.pill, t.ink)
    };
    let cursor = if on_open.is_some() {
        "pointer"
    } else {
        "default"
    };
    rsx! {
        span { class: "np-pill", style: "background: {bg}; color: {ink};",
            span { style: "cursor: {cursor};",
                onclick: move |_| if let Some(open) = on_open { open.call(()) },
                "{label}"
            }
            span { class: "np-x", title: "Remove", onclick: move |_| on_remove.call(()),
                X { size: 11, color: ink.to_string() }
            }
        }
    }
}

#[component]
fn ListValue(
    value: Value,
    kind: Kind,
    on_value: Callback<Value>,
    on_link: Option<Callback<String>>,
) -> Element {
    let t = use_shell().theme;
    let items = items_of(&value);
    let mut draft = use_signal(String::new);
    let with = move |next: Vec<String>| {
        on_value.call(Value::Array(next.into_iter().map(Value::String).collect()));
    };
    let pills = items.iter().enumerate().map(|(i, item)| {
        let (label, link) = match kind {
            Kind::Tags => (format!("#{}", item.trim_start_matches('#')), false),
            Kind::Links if is_link(item) => (link_label(item), true),
            _ => (item.clone(), is_link(item)),
        };
        let target = link_target(item);
        let rest: Vec<String> = items
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, s)| s.clone())
            .collect();
        let with = with.clone();
        rsx! {
            Pill {
                key: "{i}-{item}",
                label,
                link,
                on_open: if link { on_link.map(|go| Callback::new(move |()| go.call(target.clone()))) } else { None },
                on_remove: move |()| with(rest.clone()),
            }
        }
    });
    let placeholder = if items.is_empty() { "Empty" } else { "" };
    rsx! {
        {pills}
        input {
            "aria-label": "Add a value",
            style: "color: {t.ink};",
            placeholder,
            value: "{draft}",
            oninput: move |e| draft.set(e.value()),
            onkeydown: move |e: KeyboardEvent| {
                if e.key().to_string() == "Enter" {
                    let mut s = draft().trim().to_owned();
                    if kind == Kind::Tags {
                        let hashes = s.len().saturating_sub(s.trim_start_matches('#').len());
                        s.drain(..hashes);
                    }
                    if kind == Kind::Links && !s.is_empty() && !is_link(&s) {
                        s = format!("[[{s}]]");
                    }
                    if !s.is_empty() {
                        let mut next = items.clone();
                        next.push(s);
                        with(next);
                    }
                    draft.set(String::new());
                }
            },
        }
    }
}

#[component]
fn DateValue(value: Value, with_time: bool, on_value: Callback<Value>) -> Element {
    let t = use_shell().theme;
    let raw = text_of(&value);
    let (date_part, time_part) = raw
        .split_once('T')
        .map_or((raw.as_str(), ""), |(d, t)| (d, t));
    let date = parse_date(date_part);
    let time = parse_time(time_part).unwrap_or((9, 0));
    let mut open = use_signal(|| false);
    let month = use_signal(|| date.map_or((2026, 1), |(y, m, _)| (y, m)));
    let label = date.map_or_else(
        || "Empty".to_owned(),
        |(y, m, d)| {
            let name = usize::try_from(m.saturating_sub(1))
                .ok()
                .and_then(|i| MONTHS.get(i))
                .copied()
                .unwrap_or("");
            let day = format!("{d} {name} {y}");
            if with_time {
                format!("{day}, {:02}:{:02}", time.0, time.1)
            } else {
                day
            }
        },
    );
    let write = move |d: (i32, u32, u32), (h, mi): (u32, u32)| {
        let s = if with_time {
            format!("{}T{h:02}:{mi:02}:00", fmt_date(d))
        } else {
            fmt_date(d)
        };
        on_value.call(Value::String(s));
    };
    let (y, m) = month();
    let lead = weekday(y, m, 1);
    let days = days_in(y, m);
    let month_name = usize::try_from(m.saturating_sub(1))
        .ok()
        .and_then(|i| MONTHS.get(i))
        .copied()
        .unwrap_or("");
    let step = move |back: bool| {
        let mut month = month;
        let (y, m) = month();
        month.set(match (back, m) {
            (true, 1) => (y.saturating_sub(1), 12),
            (true, m) => (y, m.saturating_sub(1)),
            (false, 12) => (y.saturating_add(1), 1),
            (false, m) => (y, m.saturating_add(1)),
        });
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 6px;",
            span {
                "data-testid": "note-property-date",
                style: "cursor: pointer; padding: 4px 0; color: {t.ink};",
                onclick: move |_| open.toggle(),
                "{label}"
            }
            if open() {
                div {
                    "data-testid": "note-property-calendar",
                    style: "padding: 8px; border: 1px solid {t.rule}; border-radius: 8px; background: {t.sheet}; box-shadow: {t.shadow_float}; align-self: flex-start;",
                    div { style: "display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px;",
                        span { class: "np-tool", onclick: move |_| step(true), ChevronUp { size: 14, color: t.ink_3.to_string() } }
                        span { style: "font-weight: 600; color: {t.ink};", "{month_name} {y}" }
                        span { class: "np-tool", onclick: move |_| step(false), ChevronDown { size: 14, color: t.ink_3.to_string() } }
                    }
                    div { class: "np-cal",
                        for w in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                            span { class: "np-day", style: "color: {t.ink_3}; cursor: default;", "{w}" }
                        }
                        for i in 0..lead {
                            span { key: "pad-{i}" }
                        }
                        for d in 1..=days {
                            span {
                                key: "{d}",
                                class: "np-day",
                                style: if date == Some((y, m, d)) { format!("background: {}; color: {};", t.accent, t.accent_ink) } else { format!("color: {};", t.ink) },
                                onclick: move |_| {
                                    write((y, m, d), time);
                                    if !with_time {
                                        open.set(false);
                                    }
                                },
                                "{d}"
                            }
                        }
                    }
                    if with_time {
                        input {
                            "aria-label": "Time",
                            style: "margin-top: 6px; width: 70px; color: {t.ink}; background: {t.pill}; border: none; border-radius: 6px; padding: 3px 6px;",
                            value: "{time.0:02}:{time.1:02}",
                            onchange: move |e| {
                                if let (Some(d), Some(tm)) = (date, parse_time(&e.value())) {
                                    write(d, tm);
                                }
                            },
                        }
                    }
                    div { style: "display: flex; gap: 8px; margin-top: 6px; font-size: 12px;",
                        span { class: "np-tool", style: "color: {t.ink_3};", onclick: move |_| { on_value.call(Value::Null); open.set(false); }, "Clear" }
                        span { class: "np-tool", style: "color: {t.ink_3};", onclick: move |_| open.set(false), "Done" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: Welcome\ntags: [lab/demo, start]\nstatus: active\nfeature: \"[[Attachments/label-lab.png]]\"\ncreated: 2026-09-01T08:00:00\ndone: false\n---\n# Welcome\n";

    #[test]
    fn a_note_splits_into_its_block_and_its_body() {
        let (block, body) = split(NOTE);
        assert!(block.starts_with("---\n") && block.ends_with("---\n"));
        assert_eq!(body, "# Welcome\n");
        assert_eq!(split("# Just a body\n"), ("", "# Just a body\n"));
        // Not a mapping: left in the body to fix by hand.
        let bad = "---\n: : [\n---\nbody\n";
        assert_eq!(split(bad), ("", bad));
    }

    #[test]
    fn each_property_reads_as_its_type() {
        let (block, _) = split(NOTE);
        let types = Types::default();
        let kinds: Vec<_> = entries(block)
            .iter()
            .map(|e| {
                (
                    e.key.clone(),
                    kind_of(&types, FALLBACK_KIND, &e.key, &e.value),
                )
            })
            .collect();
        let get = |k: &str| kinds.iter().find(|(x, _)| x == k).map(|(_, v)| *v);
        assert_eq!(get("title"), Some(Kind::Text));
        assert_eq!(get("tags"), Some(Kind::Tags));
        assert_eq!(get("feature"), Some(Kind::Link));
        assert_eq!(get("created"), Some(Kind::Datetime));
        assert_eq!(get("done"), Some(Kind::Checkbox));
        let declared = Types {
            declared: Arc::new([("status".to_owned(), "multitext".to_owned())].into()),
        };
        // Declared a list, a scalar reads as a list of one (as Obsidian
        // reads it); a declared type the value cannot be is not used.
        assert_eq!(
            kind_of(&declared, FALLBACK_KIND, "status", &Value::from("active")),
            Kind::List
        );
        let dated = Types {
            declared: Arc::new([("due".to_owned(), "date".to_owned())].into()),
        };
        assert_eq!(
            kind_of(&dated, FALLBACK_KIND, "due", &Value::from("soon")),
            Kind::Text
        );
    }

    #[test]
    fn a_set_rewrites_only_its_key() {
        let (block, _) = split(NOTE);
        let out = set(block, "status", Value::from("done"));
        assert!(out.contains("status: done\n"));
        assert!(out.contains("tags: [lab/demo, start]\n"), "{out}");
        assert!(out.contains("created: 2026-09-01T08:00:00\n"), "{out}");
        let added = add(&out, "rating").expect("a new key");
        assert!(
            added.trim_end().ends_with("rating:\n---") || added.contains("rating:"),
            "{added}"
        );
        assert!(add(&added, "rating").is_none(), "taken");
        assert!(add(&added, "a: b").is_none(), "no good");
    }

    #[test]
    fn a_rename_keeps_its_place_and_its_lines() {
        let (block, _) = split(NOTE);
        let out = rename(block, "status", "state").expect("renamed");
        let keys: Vec<_> = entries(&out).into_iter().map(|e| e.key).collect();
        assert_eq!(
            keys,
            ["title", "tags", "state", "feature", "created", "done"]
        );
        assert!(out.contains("tags: [lab/demo, start]\n"));
        assert!(rename(block, "status", "title").is_none());
    }

    #[test]
    fn a_property_moves_and_goes() {
        let (block, _) = split(NOTE);
        let moved = move_to(block, 0, 2);
        let keys: Vec<_> = entries(&moved).into_iter().map(|e| e.key).collect();
        assert_eq!(
            keys.get(..3),
            Some(&["tags".to_owned(), "status".to_owned(), "title".to_owned()][..])
        );
        let gone = remove(block, "status");
        assert!(!gone.contains("status"));
        assert!(gone.contains("tags: [lab/demo, start]\n"));
        let one = "---\na: 1\n---\n";
        assert_eq!(remove(one, "a"), "");
    }

    #[test]
    fn dates_and_weekdays() {
        assert_eq!(parse_date("2026-11-07"), Some((2026, 11, 7)));
        assert!(parse_date("2026-02-30").is_none());
        assert!(is_datetime("2026-09-01T08:00:00"));
        assert_eq!(weekday(2026, 11, 7), 5, "a Saturday");
        assert_eq!(weekday(2024, 2, 29), 3, "a Thursday");
        assert_eq!(days_in(2024, 2), 29);
        assert_eq!(
            link_target("[[Projects/Album/project|Album]]"),
            "Projects/Album/project"
        );
        assert_eq!(link_label("[[Attachments/label-lab.png]]"), "label-lab.png");
    }
}
