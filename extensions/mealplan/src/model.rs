//! The notes Mealplan keeps, and how they read and write. Pure.
//!
//! - **A week's plan**, `Meal plans/2026-W47.md`: frontmatter `type:
//!   meal-plan`, `week`, and `meals`, a list of `{date, meal, recipe,
//!   servings, note, event}` (`recipe` a `.cook` file's vault path).
//! - **A pantry item**, `Pantry/Flour.md`: `type: pantry`, `qty`,
//!   `unit`, `aisle`, `staple`, `min`, `expires`, `aliases`.
//! - **The substitutions**, `Pantry/Substitutions.md`: `type:
//!   substitutions` and `substitutions`, a list of `{from, to, ratio,
//!   always, note}`.
//!
//! A write keeps the frontmatter keys it does not own and the body as
//! they were.

use std::fmt::Write as _;

use chrono::{Datelike as _, Duration, NaiveDate, Weekday};
use serde_json::{Map, Value};
use serde_yaml::Mapping;

use crate::service::{MealplanError, PantryItem, PlannedMeal, Range, Substitution};

/// A meal of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Meal {
    Breakfast,
    Lunch,
    Snack,
    Dinner,
}

impl Meal {
    pub const ALL: [Self; 4] = [Self::Breakfast, Self::Lunch, Self::Snack, Self::Dinner];

    /// Its name, as notes and actions write it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Breakfast => "breakfast",
            Self::Lunch => "lunch",
            Self::Snack => "snack",
            Self::Dinner => "dinner",
        }
    }

    /// As a person writes it (`Supper`, `brunch`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "breakfast" | "brunch" => Some(Self::Breakfast),
            "lunch" => Some(Self::Lunch),
            "dinner" | "supper" | "tea" => Some(Self::Dinner),
            "snack" | "snacks" => Some(Self::Snack),
            _ => None,
        }
    }

    /// `Breakfast`.
    #[must_use]
    pub fn title(self) -> String {
        let s = self.as_str();
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
            .unwrap_or_default()
    }

    /// When it is eaten unless a day says: HH:MM.
    #[must_use]
    pub const fn time(self) -> &'static str {
        match self {
            Self::Breakfast => "08:00",
            Self::Lunch => "12:30",
            Self::Snack => "15:00",
            Self::Dinner => "18:30",
        }
    }
}

/// `s` as a meal, or why not.
///
/// # Errors
///
/// Not a meal.
pub fn meal(s: &str) -> Result<Meal, MealplanError> {
    Meal::parse(s).ok_or_else(|| {
        MealplanError::Invalid(format!(
            "{s:?} is not a meal: breakfast, lunch, dinner or snack"
        ))
    })
}

/// A date as written: YYYY-MM-DD, `today`, `tomorrow`, `yesterday`.
///
/// # Errors
///
/// Not a date.
pub fn date(s: &str, today: NaiveDate) -> Result<NaiveDate, MealplanError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "today" => Ok(today),
        "tomorrow" => Ok(today + Duration::days(1)),
        "yesterday" => Ok(today - Duration::days(1)),
        t => NaiveDate::parse_from_str(t, "%Y-%m-%d")
            .map_err(|_| MealplanError::Invalid(format!("{s:?} is not a date (YYYY-MM-DD)"))),
    }
}

/// The ISO week `date` is in: `2026-W47`.
#[must_use]
pub fn week_id(date: NaiveDate) -> String {
    let w = date.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

/// The Monday of ISO week `s` (`2026-W47`, `2026w47`).
#[must_use]
pub fn week_monday(s: &str) -> Option<NaiveDate> {
    let s = s.trim().to_ascii_uppercase();
    let (y, w) = s.split_once('W')?;
    let y: i32 = y.trim_end_matches('-').parse().ok()?;
    let w: u32 = w.parse().ok()?;
    NaiveDate::from_isoywd_opt(y, w, Weekday::Mon)
}

/// The Monday of the week `date` is in.
#[must_use]
pub fn monday(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

/// The most days a range may hold.
pub const MAX_DAYS: i64 = 62;

/// A range's dates, `from`..`to`, and its week when it is one.
///
/// # Errors
///
/// A date or week it cannot read, a range backwards or too long.
pub fn range(
    r: &Range,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate, Option<String>), MealplanError> {
    let text = |o: &Option<String>| {
        o.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    if let Some(w) = text(&r.week) {
        let m = week_monday(&w)
            .ok_or_else(|| MealplanError::Invalid(format!("{w:?} is not a week (2026-W47)")))?;
        return Ok((m, m + Duration::days(7), Some(week_id(m))));
    }
    if let Some(from) = text(&r.from) {
        let from = date(&from, today)?;
        let to = match text(&r.to) {
            Some(t) => date(&t, today)?,
            None => from + Duration::days(7),
        };
        if to <= from || (to - from).num_days() > MAX_DAYS {
            return Err(MealplanError::Invalid(format!(
                "{from}..{to}: `to` comes after `from`, at most {MAX_DAYS} days on"
            )));
        }
        let week = (from.weekday() == Weekday::Mon && to == from + Duration::days(7))
            .then(|| week_id(from));
        return Ok((from, to, week));
    }
    let d = match text(&r.date) {
        Some(d) => date(&d, today)?,
        None => today,
    };
    let m = monday(d);
    Ok((m, m + Duration::days(7), Some(week_id(m))))
}

/// A name folded for matching: lower case, single spaces, a plural's
/// `s` off (`Eggs` and `egg`, `tomatoes` and `tomato`).
#[must_use]
pub fn fold(name: &str) -> String {
    let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
    let mut s = words.join(" ");
    if s.chars().count() > 3 {
        if s.ends_with("oes") || s.ends_with("ches") || s.ends_with("shes") {
            s.truncate(s.len().saturating_sub(2));
        } else if s.ends_with('s') && !s.ends_with("ss") {
            s.pop();
        }
    }
    s
}

/// A file's name without its folder and extension.
#[must_use]
pub fn stem(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(s, _)| s).to_owned()
}

/// A name as a file name: what a path cannot hold taken out.
#[must_use]
pub fn file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|#^[]".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        "Untitled".to_owned()
    } else {
        s
    }
}

// ── Frontmatter ─────────────────────────────────────────────────────

/// A note's frontmatter and its body; none and the whole text when it
/// has none (or it does not parse).
#[must_use]
pub fn split(text: &str) -> (Mapping, &str) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (Mapping::new(), text);
    };
    let (yaml, body) = if let Some((y, b)) = rest.split_once("\n---\n") {
        (y, b)
    } else if let Some(y) = rest.strip_suffix("\n---") {
        (y, "")
    } else if let Some(b) = rest.strip_prefix("---\n") {
        ("", b)
    } else {
        return (Mapping::new(), text);
    };
    match serde_yaml::from_str::<Mapping>(yaml) {
        Ok(m) => (m, body),
        Err(_) if yaml.trim().is_empty() => (Mapping::new(), body),
        Err(_) => (Mapping::new(), text),
    }
}

/// A note's frontmatter as JSON values, by key.
#[must_use]
pub fn properties(text: &str) -> Map<String, Value> {
    let (m, _) = split(text);
    serde_json::to_value(m)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// `owned` (YAML lines it writes itself) then the keys of `rest` it
/// does not own, as frontmatter, then `body`.
fn join(owned: &str, rest: &Mapping, owns: &[&str], body: &str) -> String {
    let mut kept = rest.clone();
    for k in owns {
        kept.remove(*k);
    }
    let mut out = String::from("---\n");
    out.push_str(owned);
    if !kept.is_empty()
        && let Ok(y) = serde_yaml::to_string(&kept)
    {
        out.push_str(&y);
    }
    out.push_str("---\n");
    out.push_str(body);
    out
}

/// `s` as a YAML scalar: plain when that reads back as the same text,
/// quoted otherwise.
#[must_use]
pub fn scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.trim() == s
        && s.chars()
            .all(|c| c.is_alphanumeric() || " /._-()',&+".contains(c))
        && !s.starts_with(['-', '\'', '&'])
        && serde_yaml::from_str::<serde_yaml::Value>(s).is_ok_and(|v| v.as_str() == Some(s));
    if plain {
        s.to_owned()
    } else {
        serde_json::to_string(s).unwrap_or_default()
    }
}

/// A JSON value as the text it reads as (`4`, `"dinner"`).
#[must_use]
pub fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A JSON value as a number (`4`, `"1.5"`).
#[must_use]
pub fn number_of(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn bool_of(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(s.trim(), "true" | "yes"),
        _ => false,
    }
}

// ── A week's plan ───────────────────────────────────────────────────

/// The meals of a plan note, in order.
#[must_use]
pub fn read_meals(text: &str) -> Vec<PlannedMeal> {
    let props = properties(text);
    let mut out: Vec<PlannedMeal> = props
        .get("meals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let get = |k: &str| m.get(k).and_then(text_of);
            let recipe = get("recipe")?;
            let meal = get("meal")
                .and_then(|s| Meal::parse(&s))
                .unwrap_or(Meal::Dinner);
            Some(PlannedMeal {
                date: get("date")?,
                meal: meal.as_str().to_owned(),
                name: stem(&recipe),
                recipe,
                servings: m.get("servings").and_then(number_of),
                note: get("note"),
                event: get("event"),
            })
        })
        .collect();
    sort_meals(&mut out);
    out
}

/// Meals by date, then breakfast to dinner (the order they were planned
/// in kept within a meal).
pub fn sort_meals(meals: &mut [PlannedMeal]) {
    meals.sort_by_key(|m| (m.date.clone(), Meal::parse(&m.meal)));
}

/// A plan note's text: `existing`'s other keys and body kept.
#[must_use]
pub fn write_plan(existing: Option<&str>, week: &str, meals: &[PlannedMeal]) -> String {
    let (rest, body) = existing.map_or_else(|| (Mapping::new(), ""), split);
    let mut owned = format!("type: meal-plan\nweek: {}\n", scalar(week));
    if meals.is_empty() {
        owned.push_str("meals: []\n");
    } else {
        owned.push_str("meals:\n");
        for m in meals {
            let _ = writeln!(owned, "  - date: {}", scalar(&m.date));
            let _ = writeln!(owned, "    meal: {}", scalar(&m.meal));
            let _ = writeln!(owned, "    recipe: {}", scalar(&m.recipe));
            if let Some(s) = m.servings {
                let _ = writeln!(owned, "    servings: {}", crate::units::number(s));
            }
            if let Some(n) = m.note.as_deref().filter(|n| !n.trim().is_empty()) {
                let _ = writeln!(owned, "    note: {}", scalar(n));
            }
            if let Some(e) = m.event.as_deref().filter(|n| !n.trim().is_empty()) {
                let _ = writeln!(owned, "    event: {}", scalar(e));
            }
        }
    }
    let body = if existing.is_none() {
        format!("# Meals, {week}\n")
    } else {
        body.to_owned()
    };
    join(&owned, &rest, &["type", "week", "meals"], &body)
}

// ── The pantry ──────────────────────────────────────────────────────

/// A pantry item from its properties (a note's frontmatter, an index
/// row's values).
#[must_use]
pub fn item_of(path: &str, props: &Map<String, Value>, today: NaiveDate) -> PantryItem {
    let get = |k: &str| props.get(k).and_then(text_of);
    let mut item = PantryItem {
        path: path.to_owned(),
        name: get("name").unwrap_or_else(|| stem(path)),
        qty: props.get("qty").and_then(number_of),
        unit: get("unit").unwrap_or_default(),
        aisle: get("aisle").or_else(|| get("category")),
        staple: bool_of(props.get("staple")),
        min: props.get("min").and_then(number_of),
        expires: get("expires"),
        aliases: match props.get("aliases") {
            Some(Value::Array(a)) => a.iter().filter_map(text_of).collect(),
            Some(v) => text_of(v)
                .map(|s| {
                    s.split(',')
                        .map(|a| a.trim().to_owned())
                        .filter(|a| !a.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        },
        low: false,
        expired: false,
    };
    flags(&mut item, today);
    item
}

/// Sets `low` and `expired` as of `today`.
pub fn flags(item: &mut PantryItem, today: NaiveDate) {
    item.low = item.staple && item.min.is_some_and(|m| item.qty.unwrap_or(0.0) < m);
    item.expired = expired(item, today);
}

/// Whether it is past its date on `day`.
#[must_use]
pub fn expired(item: &PantryItem, day: NaiveDate) -> bool {
    item.expires
        .as_deref()
        .and_then(|e| NaiveDate::parse_from_str(e.trim(), "%Y-%m-%d").ok())
        .is_some_and(|e| e < day)
}

/// Whether `name` (an ingredient's) is this item: its name or an alias.
#[must_use]
pub fn is_item(item: &PantryItem, name: &str) -> bool {
    let n = fold(name);
    fold(&item.name) == n || item.aliases.iter().any(|a| fold(a) == n)
}

/// A pantry note's text: `existing`'s other keys and body kept.
#[must_use]
pub fn write_item(existing: Option<&str>, item: &PantryItem) -> String {
    let (rest, body) = existing.map_or_else(|| (Mapping::new(), ""), split);
    let mut owned = String::from("type: pantry\n");
    if let Some(q) = item.qty {
        let _ = writeln!(owned, "qty: {}", crate::units::number(q));
    }
    if !item.unit.trim().is_empty() {
        let _ = writeln!(owned, "unit: {}", scalar(item.unit.trim()));
    }
    if let Some(a) = item.aisle.as_deref().filter(|a| !a.trim().is_empty()) {
        let _ = writeln!(owned, "aisle: {}", scalar(a.trim()));
    }
    if item.staple {
        owned.push_str("staple: true\n");
    }
    if let Some(m) = item.min {
        let _ = writeln!(owned, "min: {}", crate::units::number(m));
    }
    if let Some(e) = item.expires.as_deref().filter(|e| !e.trim().is_empty()) {
        let _ = writeln!(owned, "expires: {}", scalar(e.trim()));
    }
    if !item.aliases.is_empty() {
        owned.push_str("aliases:\n");
        for a in &item.aliases {
            let _ = writeln!(owned, "  - {}", scalar(a));
        }
    }
    let body = if existing.is_none() {
        format!("# {}\n", item.name)
    } else {
        body.to_owned()
    };
    join(
        &owned,
        &rest,
        &[
            "type", "qty", "unit", "aisle", "category", "staple", "min", "expires", "aliases",
        ],
        &body,
    )
}

// ── Substitutions ───────────────────────────────────────────────────

/// The substitutions of `Substitutions.md`.
#[must_use]
pub fn read_subs(text: &str) -> Vec<Substitution> {
    properties(text)
        .get("substitutions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let get = |k: &str| s.get(k).and_then(text_of);
            Some(Substitution {
                from: get("from")?,
                to: get("to")?,
                ratio: s
                    .get("ratio")
                    .and_then(number_of)
                    .filter(|r| *r > 0.0)
                    .unwrap_or(1.0),
                always: bool_of(s.get("always")),
                note: get("note"),
            })
        })
        .collect()
}

/// `Substitutions.md`'s text: `existing`'s other keys and body kept.
#[must_use]
pub fn write_subs(existing: Option<&str>, subs: &[Substitution]) -> String {
    let (rest, body) = existing.map_or_else(|| (Mapping::new(), ""), split);
    let mut owned = String::from("type: substitutions\n");
    if subs.is_empty() {
        owned.push_str("substitutions: []\n");
    } else {
        owned.push_str("substitutions:\n");
        for s in subs {
            let _ = writeln!(owned, "  - from: {}", scalar(&s.from));
            let _ = writeln!(owned, "    to: {}", scalar(&s.to));
            let _ = writeln!(owned, "    ratio: {}", crate::units::number(s.ratio));
            if s.always {
                owned.push_str("    always: true\n");
            }
            if let Some(n) = s.note.as_deref().filter(|n| !n.trim().is_empty()) {
                let _ = writeln!(owned, "    note: {}", scalar(n));
            }
        }
    }
    let body = if existing.is_none() {
        "# Substitutions\n\nWhat stands in for what: `ratio` of `to` for one of `from`. `always` buys `to` instead; otherwise it is used when the pantry has enough of it.\n".to_owned()
    } else {
        body.to_owned()
    };
    join(&owned, &rest, &["type", "substitutions"], &body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn weeks_and_ranges() {
        assert_eq!(week_id(d("2026-11-18")), "2026-W47");
        assert_eq!(week_monday("2026-W47"), Some(d("2026-11-16")));
        assert_eq!(week_monday("2026w47"), Some(d("2026-11-16")));
        // The first ISO week of 2027 starts in 2027-01-04.
        assert_eq!(week_id(d("2027-01-03")), "2026-W53");
        let today = d("2026-11-18");
        let (f, t, w) = range(&Range::default(), today).unwrap();
        assert_eq!(
            (f, t, w.as_deref()),
            (d("2026-11-16"), d("2026-11-23"), Some("2026-W47"))
        );
        let (f, t, w) = range(
            &Range {
                from: Some("2026-11-18".into()),
                to: Some("2026-11-21".into()),
                ..Range::default()
            },
            today,
        )
        .unwrap();
        assert_eq!((f, t, w), (d("2026-11-18"), d("2026-11-21"), None));
        assert!(
            range(
                &Range {
                    from: Some("2026-11-18".into()),
                    to: Some("2026-11-18".into()),
                    ..Range::default()
                },
                today
            )
            .is_err()
        );
        assert!(
            range(
                &Range {
                    week: Some("nope".into()),
                    ..Range::default()
                },
                today
            )
            .is_err()
        );
    }

    #[test]
    fn names_fold_for_matching() {
        assert_eq!(fold("Eggs"), "egg");
        assert_eq!(fold("tomatoes"), "tomato");
        assert_eq!(fold("Olive  Oil"), "olive oil");
        assert_eq!(fold("glass"), "glass");
        assert_eq!(fold("peas"), "pea");
    }

    #[test]
    fn a_plan_note_round_trips_and_keeps_what_it_does_not_own() {
        let meals = vec![
            PlannedMeal {
                date: "2026-11-17".into(),
                meal: "dinner".into(),
                recipe: "Cookbook/Pancakes.cook".into(),
                servings: Some(4.0),
                note: Some("with: berries".into()),
                ..PlannedMeal::default()
            },
            PlannedMeal {
                date: "2026-11-16".into(),
                meal: "breakfast".into(),
                recipe: "Cookbook/Oats.cook".into(),
                ..PlannedMeal::default()
            },
        ];
        let text = write_plan(None, "2026-W47", &meals);
        assert!(text.starts_with("---\ntype: meal-plan\nweek: 2026-W47\nmeals:\n  - date: 2026-11-17\n    meal: dinner\n    recipe: Cookbook/Pancakes.cook\n    servings: 4\n    note: \"with: berries\"\n"), "{text}");
        let read = read_meals(&text);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].recipe, "Cookbook/Oats.cook", "sorted by date");
        assert_eq!(read[1].name, "Pancakes");
        assert_eq!(read[1].note.as_deref(), Some("with: berries"));

        let edited = text
            .replace(
                "# Meals, 2026-W47\n",
                "# Meals, 2026-W47\n\nShop on Saturday.\n",
            )
            .replacen("---\n", "---\ntags: [food]\n", 1);
        let again = write_plan(Some(&edited), "2026-W47", &read[..1]);
        assert!(again.contains("Shop on Saturday."), "{again}");
        assert!(again.contains("tags:\n- food\n"), "{again}");
        assert_eq!(read_meals(&again).len(), 1);
    }

    #[test]
    fn a_pantry_item_round_trips() {
        let today = d("2026-11-18");
        let text = "---\ntype: pantry\nqty: 400\nunit: g\naisle: Baking\nstaple: true\nmin: 500\nexpires: 2026-11-01\naliases: [plain flour]\nbrand: Acme\n---\n# Flour\n";
        let item = item_of("Pantry/Flour.md", &properties(text), today);
        assert_eq!(item.name, "Flour");
        assert_eq!(item.qty, Some(400.0));
        assert!(item.low && item.expired && item.staple);
        assert!(is_item(&item, "plain flour"));
        assert!(is_item(&item, "flours"));
        let out = write_item(
            Some(text),
            &PantryItem {
                qty: Some(900.0),
                expires: None,
                ..item
            },
        );
        assert!(
            out.contains("qty: 900\n") && !out.contains("expires") && out.contains("brand: Acme"),
            "{out}"
        );
        assert!(out.ends_with("---\n# Flour\n"), "{out}");
    }

    #[test]
    fn substitutions_round_trip() {
        let subs = vec![Substitution {
            from: "butter".into(),
            to: "olive oil".into(),
            ratio: 0.75,
            always: false,
            note: None,
        }];
        let text = write_subs(None, &subs);
        assert_eq!(read_subs(&text), subs);
        assert!(
            text.contains("  - from: butter\n    to: olive oil\n    ratio: 0.75\n"),
            "{text}"
        );
    }
}
