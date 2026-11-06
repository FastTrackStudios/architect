//! Fuzzy matching: the palette's engine (`architect-palette`), the one
//! the ⌘K Command Palette ranks with, over each item's search fields.

use crate::provider::Item;

/// Score and annotate a list of items against a query string.
///
/// Items are scored across all their `search_fields` with the engine's
/// [`architect_palette::fuzzy`], earlier fields weighted higher (matching
/// on `label` scores more than matching on a secondary field): field 0
/// full, each later one 20% less, down to 20%.
///
/// Returns items with `score` and `match_positions` populated.
/// Items that don't match at all are filtered out.
#[must_use]
pub fn score_items(items: Vec<Item>, query: &str) -> Vec<Item> {
    if query.is_empty() {
        // No query = return all items with base score 0
        return items;
    }

    let mut scored: Vec<Item> = items
        .into_iter()
        .filter_map(|mut item| {
            let mut best: Option<(f64, Vec<usize>)> = None;
            for (field_idx, field) in item.search_fields.iter().enumerate() {
                let Some(m) = architect_palette::fuzzy(query, field) else {
                    continue;
                };
                // Field position penalty: later fields score lower.
                let penalty = 1.0 - (idx_as_f64(field_idx) * 0.2).min(0.8);
                let adjusted = score_as_f64(m.score) * penalty;
                if best.as_ref().is_none_or(|(s, _)| adjusted > *s) {
                    best = Some((adjusted, m.positions));
                }
            }
            let (score, positions) = best?;
            item.score = score;
            item.match_positions = positions
                .into_iter()
                .map(|p| u32::try_from(p).unwrap_or(u32::MAX))
                .collect();
            Some(item)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

/// Exact substring matching (for when the user toggles exact mode).
#[must_use]
pub fn score_items_exact(items: Vec<Item>, query: &str) -> Vec<Item> {
    if query.is_empty() {
        return items;
    }

    let query_lower = query.to_lowercase();

    let mut scored: Vec<Item> = items
        .into_iter()
        .filter_map(|mut item| {
            for field in &item.search_fields {
                let field_lower = field.to_lowercase();
                if let Some(pos) = field_lower.find(&query_lower) {
                    // Score exact matches by position (earlier = better) and length ratio.
                    let position_score = 100.0 - idx_as_f64(pos).min(50.0);
                    let length_ratio = idx_as_f64(query.len()) / idx_as_f64(field.len());
                    item.score = length_ratio.mul_add(50.0, position_score);
                    let start = u32::try_from(pos).unwrap_or(u32::MAX);
                    let end = u32::try_from(pos.saturating_add(query.len())).unwrap_or(u32::MAX);
                    item.match_positions = (start..end).collect();
                    return Some(item);
                }
            }
            None
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

/// A slice index / length as an `f64` score input.
// Scores are heuristics over collection indices; above 2^53 the
// precision loss is unobservable and the value is nonsense anyway.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
const fn idx_as_f64(n: usize) -> f64 {
    n as f64
}

/// An engine score as an `f64` score input.
// Scores are small (a few hundred); i64 -> f64 is exact well past them.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
const fn score_as_f64(n: i64) -> f64 {
    n as f64
}
