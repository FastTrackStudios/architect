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
                    let penalty = 1.0 - (field_idx as f64 * 0.2).min(0.8);
                    let adjusted = (score as f64 * penalty) as u32;

                    if best_score.is_none_or(|s| adjusted > s) {
                        best_score = Some(adjusted);
                        best_positions = indices.to_vec();
                    }
                }
            }

            if let Some(score) = best_score {
                item.score = score as f64;
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
                    let position_score = 100.0 - (pos as f64).min(50.0);
                    let length_ratio = query.len() as f64 / field.len() as f64;
                    item.score = position_score + (length_ratio * 50.0);
                    item.match_positions = (pos as u32..(pos + query.len()) as u32).collect();
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
