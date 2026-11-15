//! The automatic grocery list, pure.
//!
//! What the planned meals need (the Cookbook's `ingredients`, scaled
//! and nested recipes cooked out), added up with units converted, less
//! the pantry, substitutions applied, staples under their min added,
//! grouped by aisle; and the checklist note it is written as,
//! regenerated with ticked items and lines written by hand kept.
//!
//! Task's fulfilment and shopping (`task:features/mealplan/mealplan/src/fulfillment.rs`,
//! `task:features/mealplan/mealplan/src/shopping.rs`), over notes.

use std::collections::{BTreeMap, BTreeSet};

use std::fmt::Write as _;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml::Mapping;

use crate::model::{expired, fold, is_item, number_of, properties, scalar, split, stem, text_of};
use crate::service::{Aisle, Groceries, GroceryItem, PantryItem, Substitution};
use crate::units::{convert, number, tidy};

/// One ingredient a meal needs, as `architect.cookbook` `ingredients`
/// answers it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Need {
    pub name: String,
    #[serde(default)]
    pub qty: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub recipe: String,
    #[serde(default)]
    pub optional: bool,
}

/// Less than this is nothing.
const EPS: f64 = 1e-6;

/// The aisle every item without one goes under.
pub const OTHER: &str = "Other";

/// The aisles in the order a shop is walked; others between the last
/// and [`OTHER`].
const AISLES: [&str; 8] = [
    "Produce",
    "Bakery",
    "Meat & fish",
    "Dairy & eggs",
    "Baking",
    "Pantry",
    "Spices",
    "Frozen",
];

/// The aisle a food is usually in, by a word of its name.
#[must_use]
pub fn guess_aisle(name: &str) -> Option<&'static str> {
    const WORDS: [(&str, &[&str]); 8] = [
        (
            "Produce",
            &[
                "bell pepper",
                "chilli pepper",
                "spring onion",
                "onion",
                "garlic",
                "tomato",
                "potato",
                "carrot",
                "lettuce",
                "spinach",
                "apple",
                "banana",
                "lemon",
                "lime",
                "orange",
                "basil",
                "parsley",
                "cilantro",
                "coriander leaves",
                "ginger",
                "mushroom",
                "berry",
                "berries",
                "avocado",
                "cucumber",
                "celery",
                "zucchini",
                "courgette",
                "leek",
                "kale",
                "cabbage",
                "broccoli",
                "herb",
            ],
        ),
        (
            "Spices",
            &[
                "salt", "pepper", "cumin", "paprika", "cinnamon", "oregano", "thyme", "rosemary",
                "chilli", "chili", "nutmeg", "turmeric", "spice", "bay lea",
            ],
        ),
        (
            "Dairy & eggs",
            &[
                "milk",
                "butter",
                "cheese",
                "cream",
                "yogurt",
                "yoghurt",
                "egg",
                "parmesan",
                "mozzarella",
            ],
        ),
        (
            "Meat & fish",
            &[
                "chicken", "beef", "pork", "bacon", "sausage", "fish", "salmon", "tuna", "shrimp",
                "prawn", "lamb", "turkey", "mince", "ham",
            ],
        ),
        (
            "Bakery",
            &["bread", "bun", "tortilla", "bagel", "pita", "brioche"],
        ),
        (
            "Baking",
            &[
                "flour",
                "sugar",
                "yeast",
                "baking",
                "cocoa",
                "vanilla",
                "chocolate",
                "cornstarch",
            ],
        ),
        ("Frozen", &["frozen", "ice cream"]),
        (
            "Pantry",
            &[
                "rice",
                "pasta",
                "spaghetti",
                "noodle",
                "oil",
                "vinegar",
                "honey",
                "oat",
                "bean",
                "lentil",
                "stock",
                "broth",
                "sauce",
                "syrup",
                "jam",
                "peanut",
                "nut",
                "tin",
                "can of",
            ],
        ),
    ];
    let n = fold(name);
    WORDS
        .iter()
        .find(|(_, words)| words.iter().any(|w| n.contains(w)))
        .map(|(a, _)| *a)
}

fn aisle_rank(aisle: &str) -> (usize, String) {
    let i = AISLES
        .iter()
        .position(|a| a.eq_ignore_ascii_case(aisle))
        .unwrap_or_else(|| AISLES.len() + usize::from(aisle == OTHER));
    (i, aisle.to_lowercase())
}

/// One row being added up.
#[derive(Debug, Clone)]
struct Row {
    key: String,
    name: String,
    qty: Option<f64>,
    unit: String,
    display: Vec<String>,
    recipes: BTreeSet<String>,
    note: Option<String>,
}

/// What a set of needs comes to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Built {
    /// What to buy, each with its aisle (not ticked: see [`render`]).
    pub items: Vec<(String, GroceryItem)>,
    /// What the pantry covers.
    pub have: Vec<String>,
    /// `butter → olive oil`.
    pub substituted: Vec<String>,
}

/// The line a person reads: `600 g flour`, `2 eggs`, `salt (a pinch)`.
#[must_use]
pub fn line(item: &GroceryItem) -> String {
    let mut s = match item.qty {
        Some(q) if item.unit.is_empty() => format!("{} {}", number(q), item.name),
        Some(q) => format!("{} {} {}", number(q), item.unit, item.name),
        None => item.name.clone(),
    };
    let mut notes: Vec<&str> = Vec::new();
    if item.qty.is_none()
        && let Some(d) = item.display.as_deref()
    {
        notes.push(d);
    }
    if let Some(n) = item.note.as_deref() {
        notes.push(n);
    }
    if !notes.is_empty() {
        let _ = write!(s, " ({})", notes.join("; "));
    }
    s
}

/// `needs` (optional ones left out) added up, less `pantry` as of `day`
/// (expired stock is none), with `subs` applied; with `staples`, every
/// staple under its min too.
#[must_use]
pub fn build(
    needs: &[Need],
    pantry: &[PantryItem],
    subs: &[Substitution],
    day: NaiveDate,
    staples: bool,
) -> Built {
    let mut substituted: Vec<String> = Vec::new();
    let mut note_sub = |s: &Substitution| {
        let t = format!("{} → {}", s.from, s.to);
        if !substituted.contains(&t) {
            substituted.push(t);
        }
    };

    // Added up: one row per food and kind of unit.
    let mut rows: Vec<Row> = Vec::new();
    for n in needs.iter().filter(|n| !n.optional) {
        let (name, qty) = subs
            .iter()
            .find(|s| s.always && fold(&s.from) == fold(&n.name))
            .map_or_else(
                || (n.name.trim().to_owned(), n.qty),
                |s| {
                    note_sub(s);
                    (s.to.clone(), n.qty.map(|q| q * s.ratio))
                },
            );
        let key = fold(&name);
        let recipe = stem(&n.recipe);
        let unit = n.unit.trim().to_owned();
        if let Some(q) = qty {
            if let Some((r, add)) = rows.iter_mut().find_map(|r| {
                let add =
                    (r.key == key && r.qty.is_some()).then(|| convert(q, &unit, &r.unit))??;
                Some((r, add))
            }) {
                r.qty = Some(r.qty.unwrap_or(0.0) + add);
                r.recipes.insert(recipe);
                continue;
            }
            rows.push(Row {
                key,
                name,
                qty: Some(q),
                unit,
                display: Vec::new(),
                recipes: BTreeSet::from([recipe]),
                note: None,
            });
        } else {
            let display = n.display.clone().filter(|d| !d.trim().is_empty());
            if let Some(r) = rows.iter_mut().find(|r| r.key == key) {
                if let Some(d) = display
                    && !r.display.contains(&d)
                {
                    r.display.push(d);
                }
                r.recipes.insert(recipe);
                continue;
            }
            rows.push(Row {
                key,
                name,
                qty: None,
                unit,
                display: display.into_iter().collect(),
                recipes: BTreeSet::from([recipe]),
                note: None,
            });
        }
    }

    // Less the pantry: what is left of each item, in its own unit.
    let mut left: Vec<Option<f64>> = pantry
        .iter()
        .map(|i| if expired(i, day) { Some(0.0) } else { i.qty })
        .collect();
    let there = |i: usize, left: &[Option<f64>]| -> bool {
        left.get(i).copied().flatten().map_or_else(
            || pantry.get(i).is_some_and(|p| p.staple && !expired(p, day)),
            |q| q > EPS,
        )
    };
    let mut have: Vec<String> = Vec::new();
    let mut kept: Vec<Row> = Vec::new();
    for mut r in rows {
        let Some(i) = pantry.iter().position(|p| is_item(p, &r.name)) else {
            kept.push(r);
            continue;
        };
        let item = &pantry[i];
        match (r.qty, left[i]) {
            // A staple with no count, or any of it for no amount: there.
            (_, None) | (None, _) if there(i, &left) => {
                have.push(r.name);
                continue;
            }
            (Some(q), Some(stock)) => match convert(stock, &item.unit, &r.unit) {
                Some(h) => {
                    let used = h.min(q);
                    left[i] = Some(stock - convert(used, &r.unit, &item.unit).unwrap_or(stock));
                    if q - used <= EPS {
                        have.push(r.name);
                        continue;
                    }
                    r.qty = Some(q - used);
                }
                None if stock > EPS => {
                    r.note = Some(format!(
                        "have {}",
                        line(&GroceryItem {
                            name: item.name.to_lowercase(),
                            qty: Some(stock),
                            unit: item.unit.clone(),
                            ..GroceryItem::default()
                        })
                    ));
                }
                None => {}
            },
            _ => {}
        }
        kept.push(r);
    }

    // Substitutions the pantry can stand in for.
    let mut rows: Vec<Row> = Vec::new();
    'row: for r in kept {
        for s in subs.iter().filter(|s| !s.always && fold(&s.from) == r.key) {
            let Some(j) = pantry.iter().position(|p| is_item(p, &s.to)) else {
                continue;
            };
            let sub = &pantry[j];
            match (r.qty, left[j]) {
                (_, None) | (None, _) if there(j, &left) => {
                    note_sub(s);
                    have.push(format!("{} (as {})", r.name, sub.name.to_lowercase()));
                    continue 'row;
                }
                (Some(q), Some(stock)) => {
                    let want = q * s.ratio;
                    if let Some(h) = convert(stock, &sub.unit, &r.unit)
                        && h + EPS >= want
                    {
                        left[j] = Some(stock - convert(want, &r.unit, &sub.unit).unwrap_or(0.0));
                        note_sub(s);
                        have.push(format!("{} (as {})", r.name, sub.name.to_lowercase()));
                        continue 'row;
                    }
                }
                _ => {}
            }
        }
        rows.push(r);
    }

    // Staples under their min.
    if staples {
        for (i, p) in pantry.iter().enumerate() {
            let Some(min) = p.min.filter(|_| p.staple) else {
                continue;
            };
            let now = left[i].unwrap_or(0.0);
            let had = if expired(p, day) {
                0.0
            } else {
                p.qty.unwrap_or(0.0)
            };
            if had >= min || rows.iter().any(|r| is_item(p, &r.name)) {
                continue;
            }
            rows.push(Row {
                key: fold(&p.name),
                name: p.name.to_lowercase(),
                qty: Some(min - now.min(had)),
                unit: p.unit.clone(),
                display: Vec::new(),
                recipes: BTreeSet::new(),
                note: Some("restock".to_owned()),
            });
        }
    }

    // A row with no amount beside one with: one row.
    let with_qty: BTreeSet<String> = rows
        .iter()
        .filter(|r| r.qty.is_some())
        .map(|r| r.key.clone())
        .collect();
    rows.retain(|r| r.qty.is_some() || !with_qty.contains(&r.key));

    let mut items: Vec<(String, GroceryItem)> = rows
        .into_iter()
        .map(|r| {
            let aisle = pantry
                .iter()
                .find(|p| is_item(p, &r.name))
                .and_then(|p| p.aisle.clone())
                .or_else(|| guess_aisle(&r.name).map(str::to_owned))
                .unwrap_or_else(|| OTHER.to_owned());
            let (qty, unit) = match r.qty {
                Some(q) => {
                    let (q, u) = tidy(q, &r.unit);
                    (Some(q), u)
                }
                None => (None, r.unit),
            };
            let mut item = GroceryItem {
                name: r.name,
                qty,
                unit,
                display: (!r.display.is_empty()).then(|| r.display.join(", ")),
                note: r.note,
                recipes: r.recipes.into_iter().filter(|s| !s.is_empty()).collect(),
                text: String::new(),
                checked: false,
                own: false,
            };
            item.text = line(&item);
            (aisle, item)
        })
        .collect();
    items.sort_by(|(a, x), (b, y)| {
        aisle_rank(a)
            .cmp(&aisle_rank(b))
            .then_with(|| fold(&x.name).cmp(&fold(&y.name)))
    });
    Built {
        items,
        have,
        substituted,
    }
}

// ── The note ────────────────────────────────────────────────────────

/// A checklist line's state and text: `- [x] 2 eggs`.
fn checkbox(l: &str) -> Option<(bool, &str)> {
    let t = l.trim_start();
    let rest = t.strip_prefix("- [").or_else(|| t.strip_prefix("* ["))?;
    let mut c = rest.chars();
    let mark = c.next()?;
    let rest = c.as_str().strip_prefix("] ")?;
    match mark {
        ' ' => Some((false, rest.trim_end())),
        'x' | 'X' => Some((true, rest.trim_end())),
        _ => None,
    }
}

/// The items a list's frontmatter says it built: by line, their name,
/// amount and unit.
fn built_items(props: &serde_json::Map<String, Value>) -> BTreeMap<String, GroceryItem> {
    props
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let line = v.get("line").and_then(text_of)?;
            Some((
                line.clone(),
                GroceryItem {
                    name: v
                        .get("name")
                        .and_then(text_of)
                        .unwrap_or_else(|| line.clone()),
                    qty: v.get("qty").and_then(number_of),
                    unit: v.get("unit").and_then(text_of).unwrap_or_default(),
                    text: line,
                    ..GroceryItem::default()
                },
            ))
        })
        .collect()
}

/// A grocery note read back: its aisles (its `## ` headings) and their
/// lines, the built ones known by the frontmatter's `items`.
#[must_use]
pub fn read_list(path: &str, text: &str) -> Groceries {
    let props = properties(text);
    let built = built_items(&props);
    let (_, body) = split(text);
    let mut aisles: Vec<Aisle> = Vec::new();
    for l in body.lines() {
        if let Some(h) = l.strip_prefix("## ") {
            aisles.push(Aisle {
                aisle: h.trim().to_owned(),
                items: Vec::new(),
            });
            continue;
        }
        let Some((checked, t)) = checkbox(l) else {
            continue;
        };
        let item = built.get(t).map_or_else(
            || GroceryItem {
                name: t.to_owned(),
                text: t.to_owned(),
                checked,
                own: true,
                ..GroceryItem::default()
            },
            |b| GroceryItem {
                checked,
                ..b.clone()
            },
        );
        if aisles.is_empty() {
            aisles.push(Aisle {
                aisle: OTHER.to_owned(),
                items: Vec::new(),
            });
        }
        if let Some(a) = aisles.last_mut() {
            a.items.push(item);
        }
    }
    aisles.retain(|a| !a.items.is_empty());
    let get = |k: &str| props.get(k).and_then(text_of);
    Groceries {
        path: path.to_owned(),
        week: get("week"),
        from: get("from").unwrap_or_default(),
        to: get("to").unwrap_or_default(),
        aisles,
        substituted: props
            .get("substituted")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(text_of)
            .collect(),
        text: text.to_owned(),
    }
}

/// What a list is for, in its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct For {
    pub week: Option<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl For {
    #[must_use]
    pub fn title(&self) -> String {
        self.week
            .clone()
            .unwrap_or_else(|| format!("{} to {}", self.from, self.to - chrono::Duration::days(1)))
    }
}

/// The list's note: `built` under its aisles.
///
/// An item is ticked when the list before had it ticked; the lines
/// before that were ticked and are no longer built, and the lines
/// written by hand, are kept under their headings; the text above the
/// first heading is kept.
#[must_use]
pub fn render(span: &For, built: &Built, existing: Option<&str>) -> String {
    let before = existing.map(|t| read_list("", t));
    let (rest, body) = existing.map_or_else(|| (Mapping::new(), ""), split);
    let ticked: BTreeSet<String> = before
        .iter()
        .flat_map(|g| g.aisles.iter().flat_map(|a| a.items.iter()))
        .filter(|i| i.checked && !i.own)
        .map(|i| fold(&i.name))
        .collect();

    let mut aisles: Vec<Aisle> = Vec::new();
    let push = |aisles: &mut Vec<Aisle>, aisle: &str, item: GroceryItem| {
        if let Some(a) = aisles.iter_mut().find(|a| a.aisle == aisle) {
            a.items.push(item);
        } else {
            aisles.push(Aisle {
                aisle: aisle.to_owned(),
                items: vec![item],
            });
        }
    };
    let names: BTreeSet<String> = built.items.iter().map(|(_, i)| fold(&i.name)).collect();
    for (aisle, item) in &built.items {
        let mut item = item.clone();
        item.checked = ticked.contains(&fold(&item.name));
        push(&mut aisles, aisle, item);
    }
    for a in before.iter().flat_map(|g| g.aisles.iter()) {
        for i in &a.items {
            if i.own || (i.checked && !names.contains(&fold(&i.name))) {
                push(&mut aisles, &a.aisle, i.clone());
            }
        }
    }
    aisles.sort_by_key(|a| aisle_rank(&a.aisle));

    let mut owned = String::from("type: groceries\n");
    if let Some(w) = &span.week {
        let _ = writeln!(owned, "week: {}", scalar(w));
    }
    let _ = writeln!(owned, "from: {}\nto: {}", span.from, span.to);
    let listed: Vec<&GroceryItem> = aisles
        .iter()
        .flat_map(|a| a.items.iter())
        .filter(|i| !i.own)
        .collect();
    if listed.is_empty() {
        owned.push_str("items: []\n");
    } else {
        owned.push_str("items:\n");
        for i in listed {
            let _ = writeln!(owned, "  - name: {}", scalar(&i.name));
            if let Some(q) = i.qty {
                let _ = writeln!(owned, "    qty: {}", number(q));
            }
            if !i.unit.is_empty() {
                let _ = writeln!(owned, "    unit: {}", scalar(&i.unit));
            }
            let _ = writeln!(owned, "    line: {}", scalar(&i.text));
        }
    }
    if !built.substituted.is_empty() {
        owned.push_str("substituted:\n");
        for s in &built.substituted {
            let _ = writeln!(owned, "  - {}", scalar(s));
        }
    }

    let head = match body.find("\n## ") {
        _ if existing.is_none() => format!("# Groceries, {}\n", span.title()),
        Some(i) => body.get(..=i).unwrap_or_default().to_owned(),
        None if body.starts_with("## ") => String::new(),
        None => {
            body.lines()
                .filter(|l| checkbox(l).is_none())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        }
    };
    let mut text = head.trim_end().to_owned();
    text.push('\n');
    if aisles.is_empty() {
        text.push_str("\nNothing to buy.\n");
    }
    for a in &aisles {
        let _ = writeln!(text, "\n## {}", a.aisle);
        for i in &a.items {
            let _ = writeln!(text, "- [{}] {}", if i.checked { "x" } else { " " }, i.text);
        }
    }

    let mut kept = rest;
    for k in ["type", "week", "from", "to", "items", "substituted"] {
        kept.remove(k);
    }
    let mut out = String::from("---\n");
    out.push_str(&owned);
    if !kept.is_empty()
        && let Ok(y) = serde_yaml::to_string(&kept)
    {
        out.push_str(&y);
    }
    out.push_str("---\n");
    out.push_str(&text);
    out
}

/// `text` with item `item` (its name or its line) ticked or not: the
/// new text and the item; `None` when there is no such line.
#[must_use]
pub fn tick(text: &str, item: &str, checked: bool) -> Option<(String, GroceryItem)> {
    let list = read_list("", text);
    let want = fold(item);
    let found = list
        .aisles
        .iter()
        .flat_map(|a| a.items.iter())
        .find(|i| fold(&i.name) == want || fold(&i.text) == want)?
        .clone();
    let mut out = String::with_capacity(text.len());
    let mut done = false;
    for l in text.split_inclusive('\n') {
        if !done
            && let Some((_, t)) = checkbox(l)
            && t == found.text
        {
            let indent = l.strip_suffix(l.trim_start()).unwrap_or_default();
            let end = if l.ends_with('\n') { "\n" } else { "" };
            let _ = write!(
                out,
                "{indent}- [{}] {}{end}",
                if checked { "x" } else { " " },
                found.text
            );
            done = true;
        } else {
            out.push_str(l);
        }
    }
    Some((out, GroceryItem { checked, ..found }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()
    }

    fn need(name: &str, qty: Option<f64>, unit: &str, recipe: &str) -> Need {
        Need {
            name: name.into(),
            qty,
            unit: unit.into(),
            recipe: format!("Cookbook/{recipe}.cook"),
            ..Need::default()
        }
    }

    fn item(name: &str, qty: Option<f64>, unit: &str) -> PantryItem {
        PantryItem {
            path: format!("Pantry/{name}.md"),
            name: name.into(),
            qty,
            unit: unit.into(),
            ..PantryItem::default()
        }
    }

    fn texts(b: &Built) -> Vec<String> {
        b.items.iter().map(|(_, i)| i.text.clone()).collect()
    }

    // Task's units_and_scaling: 500 g and 0.5 kg are one row.
    #[test]
    fn the_same_food_in_different_units_is_one_row() {
        let b = build(
            &[
                need("flour", Some(500.0), "g", "Dough"),
                need("Flour", Some(0.5), "kg", "Pie"),
            ],
            &[],
            &[],
            day(),
            true,
        );
        assert_eq!(texts(&b), ["1 kg flour"]);
        assert_eq!(b.items[0].1.recipes, ["Dough", "Pie"]);
        assert_eq!(b.items[0].0, "Baking");
    }

    // Task's units_and_scaling: grams and cloves are not added.
    #[test]
    fn units_that_do_not_reconcile_stay_apart() {
        let b = build(
            &[
                need("garlic", Some(2.0), "clove", "X"),
                need("garlic", Some(30.0), "g", "X"),
            ],
            &[],
            &[],
            day(),
            true,
        );
        assert_eq!(b.items.len(), 2, "{:?}", texts(&b));
    }

    #[test]
    fn the_pantry_is_taken_off_in_its_own_unit() {
        let b = build(
            &[
                need("flour", Some(600.0), "g", "Bread"),
                need("milk", Some(300.0), "ml", "Pancakes"),
                need("eggs", Some(2.0), "", "Pancakes"),
                need("salt", None, "", "Bread"),
            ],
            &[
                item("Flour", Some(0.25), "kg"),
                item("Milk", Some(1.0), "l"),
                PantryItem {
                    staple: true,
                    ..item("Salt", None, "")
                },
            ],
            &[],
            day(),
            true,
        );
        assert_eq!(texts(&b), ["2 eggs", "350 g flour"]);
        assert!(b.have.contains(&"milk".to_owned()) && b.have.contains(&"salt".to_owned()));
    }

    #[test]
    fn expired_stock_does_not_count_and_staples_restock() {
        let b = build(
            &[need("milk", Some(300.0), "ml", "Pancakes")],
            &[
                PantryItem {
                    expires: Some("2026-11-10".into()),
                    ..item("Milk", Some(1.0), "l")
                },
                PantryItem {
                    staple: true,
                    min: Some(2.0),
                    aisle: Some("Pantry".into()),
                    ..item("Rice", Some(0.5), "kg")
                },
            ],
            &[],
            day(),
            true,
        );
        assert_eq!(texts(&b), ["300 ml milk", "1.5 kg rice (restock)"]);
    }

    #[test]
    fn substitutions_stand_in_from_the_pantry_or_always() {
        let subs = [
            Substitution {
                from: "butter".into(),
                to: "olive oil".into(),
                ratio: 0.75,
                ..Substitution::default()
            },
            Substitution {
                from: "milk".into(),
                to: "oat milk".into(),
                always: true,
                ..Substitution::default()
            },
        ];
        let b = build(
            &[
                need("butter", Some(100.0), "g", "Cake"),
                need("milk", Some(200.0), "ml", "Cake"),
                need("oat milk", Some(100.0), "ml", "Porridge"),
            ],
            &[item("Olive oil", Some(500.0), "g")],
            &subs,
            day(),
            true,
        );
        assert_eq!(texts(&b), ["300 ml oat milk"]);
        assert_eq!(b.substituted, ["milk → oat milk", "butter → olive oil"]);
        // Not enough of the stand-in: bought as written.
        let b = build(
            &[need("butter", Some(100.0), "g", "Cake")],
            &[item("Olive oil", Some(10.0), "g")],
            &subs,
            day(),
            true,
        );
        assert_eq!(texts(&b), ["100 g butter"]);
    }

    #[test]
    fn optional_ingredients_are_left_off() {
        let b = build(
            &[Need {
                optional: true,
                ..need("chives", Some(1.0), "tbsp", "Soup")
            }],
            &[],
            &[],
            day(),
            true,
        );
        assert!(b.items.is_empty());
    }

    // Task's shopping_run: a tick (the kitchen pass, the shop pass) and
    // a row added by hand survive the list being built again.
    #[test]
    fn a_rebuilt_list_keeps_its_ticks_and_hand_written_lines() {
        let span = For {
            week: Some("2026-W47".into()),
            from: day(),
            to: day() + chrono::Duration::days(7),
        };
        let first = build(
            &[
                need("flour", Some(600.0), "g", "Bread"),
                need("eggs", Some(2.0), "", "Cake"),
                need("oats", Some(200.0), "g", "Porridge"),
            ],
            &[],
            &[],
            day(),
            true,
        );
        let text = render(&span, &first, None);
        assert!(text.contains("# Groceries, 2026-W47\n\n## Dairy & eggs\n- [ ] 2 eggs\n\n## Baking\n- [ ] 600 g flour\n\n## Pantry\n- [ ] 200 g oats\n"), "{text}");
        let (text, eggs) = tick(&text, "eggs", true).unwrap();
        assert_eq!(eggs.text, "2 eggs");
        let (text, _) = tick(&text, "200 g oats", true).unwrap();
        let text = text.replace("## Baking\n", "## Baking\n- [ ] baking paper\n");
        // More flour, no oats: the eggs stay ticked, the oats were bought
        // so they stay too, the paper stays.
        let again = build(
            &[
                need("flour", Some(900.0), "g", "Bread"),
                need("eggs", Some(2.0), "", "Cake"),
            ],
            &[],
            &[],
            day(),
            true,
        );
        let text = render(&span, &again, Some(&text));
        assert!(text.contains("- [x] 2 eggs\n"), "{text}");
        assert!(text.contains("- [ ] 900 g flour\n"), "{text}");
        assert!(text.contains("- [x] 200 g oats\n"), "{text}");
        assert!(text.contains("- [ ] baking paper\n"), "{text}");
        let list = read_list("Groceries/2026-W47.md", &text);
        let paper = list
            .aisles
            .iter()
            .flat_map(|a| &a.items)
            .find(|i| i.text == "baking paper")
            .unwrap();
        assert!(paper.own);
        // Unticked, a line no longer built goes on the next build.
        let (text, _) = tick(&text, "oats", false).unwrap();
        let text = render(&span, &again, Some(&text));
        assert!(!text.contains("oats"), "{text}");
    }
}
