//! Fuzzy matching engine using nucleo (same algorithm Walker uses).
//!
//! Ported from Elephant's fzf-based scoring, but using nucleo which is
//! a pure-Rust implementation of fzf's V2 algorithm.

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::provider::Item;

/// Score and annotate a list of items against a query string.
///
/// Items are scored across all their `search_fields`, with earlier fields
/// weighted higher (matching on `label` scores more than matching on a
/// secondary field). This mirrors Elephant's field-position penalty.
///
/// Returns items with `score` and `match_positions` populated.
/// Items that don't match at all are filtered out.
#[must_use]
pub fn score_items(items: Vec<Item>, query: &str) -> Vec<Item> {
    if query.is_empty() {
        // No query = return all items with base score 0
        return items;
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let atom = Atom::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );

    let mut scored: Vec<Item> = items
        .into_iter()
        .filter_map(|mut item| {
            let mut best_score: Option<u32> = None;
            let mut best_positions = Vec::new();

            for (field_idx, field) in item.search_fields.iter().enumerate() {
                let mut buf = Vec::new();
                let haystack = Utf32Str::new(field, &mut buf);

                let mut indices = Vec::new();
                if let Some(score) = atom.indices(haystack, &mut matcher, &mut indices) {
                    // Field position penalty: later fields score lower.
                    // Field 0 = full score, field 1 = -20%, field 2 = -40%, etc.
                    let penalty = 1.0 - (idx_as_f64(field_idx) * 0.2).min(0.8);
                    let adjusted = clamp_to_u32(f64::from(score) * penalty);

                    if best_score.is_none_or(|s| adjusted > s) {
                        best_score = Some(adjusted);
                        best_positions.clone_from(&indices);
                    }
                }
            }

            if let Some(score) = best_score {
                item.score = f64::from(score);
                item.match_positions = best_positions;
                Some(item)
            } else {
                None
            }
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

/// A computed score clamped into `u32`.
// f64 -> u32 has no total conversion in std; the guards make the cast exact.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn clamp_to_u32(score: f64) -> u32 {
    if score.is_finite() && score > 0.0 {
        score.min(f64::from(u32::MAX)) as u32
    } else {
        0
    }
}
