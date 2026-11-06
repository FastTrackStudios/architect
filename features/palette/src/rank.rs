//! The order rows come in: what [`Engine`](crate::Engine) ranks with, for
//! every source.

use crate::fuzzy::{Match, fuzzy};
use crate::item::Item;
use crate::recent::Recents;

/// A row in order, with what lit up in its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub item: Item,
    /// Matched characters of the label (empty when it matched elsewhere,
    /// or the query is empty).
    pub positions: Vec<usize>,
    pub score: i64,
}

/// Bonus for having been run recently, most recent the most.
const RECENT: i64 = 40;

/// The commands matching `text`, best first.
///
/// Enabled before disabled, then by match (the label counts most, the
/// group and the id less), then by how recently each ran. An empty `text`
/// lists the recent ones first, then the rest by group and label.
#[must_use]
pub fn rank(items: &[Item], text: &str, recents: &Recents) -> Vec<Ranked> {
    let mut rows: Vec<Ranked> = items
        .iter()
        .filter_map(|c| {
            let m = score_command(text, c)?;
            let recent = recents.rank(&c.id).map_or(0, |r| {
                RECENT.saturating_sub(i64::try_from(r).unwrap_or(RECENT).saturating_mul(4))
            });
            Some(Ranked {
                item: c.clone(),
                positions: m.positions,
                score: m.score.saturating_add(recent.max(0)),
            })
        })
        .collect();
    if text.trim().is_empty() {
        rows.sort_by(|a, b| {
            let ra = recents.rank(&a.item.id);
            let rb = recents.rank(&b.item.id);
            b.item
                .enabled
                .cmp(&a.item.enabled)
                .then(match (ra, rb) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| a.item.group.cmp(&b.item.group))
                .then_with(|| a.item.label.cmp(&b.item.label))
        });
    } else {
        rows.sort_by(|a, b| {
            b.item
                .enabled
                .cmp(&a.item.enabled)
                .then(b.score.cmp(&a.score))
                .then_with(|| a.item.label.cmp(&b.item.label))
        });
    }
    rows
}

/// How `text` matches a command: its label counts most; its group and
/// label together, its detail and its id half, lighting nothing.
#[must_use]
pub fn score_command(text: &str, item: &Item) -> Option<Match> {
    fuzzy(text, &item.label).or_else(|| {
        fuzzy(text, &format!("{} {}", item.group, item.label))
            .or_else(|| fuzzy(text, &item.detail))
            .or_else(|| fuzzy(text, &item.id))
            .map(|m| Match {
                score: m.score / 2,
                positions: Vec::new(),
            })
    })
}

/// How `text` matches something to open: its title (`label`), else its
/// detail at half, lighting nothing.
#[must_use]
pub fn score_open(text: &str, title: &str, detail: &str) -> Option<Match> {
    fuzzy(text, title).or_else(|| {
        fuzzy(text, detail).map(|mut m| {
            m.positions.clear();
            m.score /= 2;
            m
        })
    })
}

/// Items to open matching `text`, best first: by match ([`score_open`]),
/// then the more recent, then by label.
#[must_use]
pub fn rank_open(items: Vec<Item>, text: &str) -> Vec<Ranked> {
    let mut rows: Vec<Ranked> = items
        .into_iter()
        .filter_map(|item| {
            let m = score_open(text, &item.label, &item.detail)?;
            Some(Ranked {
                item,
                positions: m.positions,
                score: m.score,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.item.recency.cmp(&a.item.recency))
            .then_with(|| a.item.label.cmp(&b.item.label))
    });
    rows
}
