//! A recipe made for more or fewer people.
//!
//! Cooklang says which quantities scale: every one unless pinned
//! (`@salt{=1%tsp}`); timers never do. A recipe whose servings are not a
//! number (`1 loaf`) scales by the factor asked of one batch.

use crate::model::{Ingredient, Recipe};

/// How many times the recipe `servings` is wanted: `None` when it
/// already is, or when either is not a positive number.
#[must_use]
pub fn factor(recipe_servings: Option<u32>, wanted: Option<f64>) -> Option<f64> {
    let wanted = wanted.filter(|w| w.is_finite() && *w > 0.0)?;
    let base = f64::from(recipe_servings.unwrap_or(1).max(1));
    let f = wanted / base;
    ((f - 1.0).abs() > f64::EPSILON).then_some(f)
}

/// `recipe` made for `wanted` servings: its scalable quantities times
/// the factor, their display rewritten; its servings the ones wanted.
#[must_use]
pub fn scaled(mut recipe: Recipe, wanted: Option<f64>) -> Recipe {
    let Some(f) = factor(recipe.servings, wanted) else {
        return recipe;
    };
    for i in &mut recipe.ingredients.0 {
        scale_one(i, f);
    }
    if let Some(w) = wanted {
        // Servings are whole people; a half is rounded up.
        recipe.servings = Some(round_up(w));
    }
    recipe
}

fn round_up(w: f64) -> u32 {
    let mut n: u32 = 0;
    while f64::from(n) < w && n < 10_000 {
        n = n.saturating_add(1);
    }
    n
}

/// One ingredient times `f`, unless Cooklang pinned it.
pub fn scale_one(i: &mut Ingredient, f: f64) {
    if !i.scalable {
        return;
    }
    if let Some(q) = i.qty.as_mut() {
        *q *= f;
        i.qty_display = Some(number(*q));
    }
    if let Some(q) = i.qty_max.as_mut() {
        *q *= f;
    }
    if let (Some(lo), Some(hi)) = (i.qty, i.qty_max) {
        i.qty_display = Some(format!("{}-{}", number(lo), number(hi)));
    }
}

/// A quantity as a cook reads it: whole numbers bare, else at most two
/// decimals, trailing zeros dropped (`1.5`, `0.33`).
#[must_use]
pub fn number(q: f64) -> String {
    let s = format!("{q:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]

    use super::*;
    use crate::parse::parse_cook;

    #[test]
    fn doubles_what_scales_and_keeps_what_is_pinned() {
        let r = parse_cook(
            "Cookbook/Pancakes.cook",
            "---\nservings: 2\n---\nMix @flour{200%g}, @salt{=1%pinch} and @milk{300%ml}.",
        )
        .unwrap();
        let r = scaled(r, Some(4.0));
        assert_eq!(r.servings, Some(4));
        let by = |n: &str| r.ingredients.iter().find(|i| i.name == n).unwrap().clone();
        assert_eq!(by("flour").qty, Some(400.0));
        assert_eq!(by("milk").qty_display.as_deref(), Some("600"));
        assert_eq!(by("salt").qty, Some(1.0));
    }

    #[test]
    fn numbers_read_as_a_cook_writes_them() {
        assert_eq!(number(2.0), "2");
        assert_eq!(number(1.5), "1.5");
        assert_eq!(number(1.0 / 3.0), "0.33");
    }

    #[test]
    fn the_same_servings_is_no_change() {
        assert_eq!(factor(Some(4), Some(4.0)), None);
        assert_eq!(factor(Some(4), None), None);
        assert_eq!(factor(None, Some(3.0)), Some(3.0));
    }
}
