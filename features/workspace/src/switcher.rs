//! A phone's tab switcher, as data: which tabs it shows (one [`Place`]'s,
//! in order), where its cards go ([`CardGrid`]), and where a card dragged
//! to another card's place is moved ([`reorder_dest`]).
//!
//! The host draws it (feature `ui`, `host/switcher.rs`); this is the
//! model's side, tested without a renderer.

use crate::model::{Docks, Group, GroupId, Node, PaneId, Rect, Side, Workspace};
use crate::ops::Dest;

/// Where a set of tabs lives, as the switcher groups them: the main
/// window's tabs (its center and its floats), one of its docks, or a
/// pop-out window (its center, docks and floats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Main,
    Dock(Side),
    /// The pop-out at this index in the workspace's `windows` (1 and up).
    Window(usize),
}

impl Place {
    /// Its name in the switcher's menu.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Main => "Tabs".to_owned(),
            Self::Dock(Side::Left) => "Left sidebar".to_owned(),
            Self::Dock(Side::Right) => "Right sidebar".to_owned(),
            Self::Dock(_) => "Bottom panel".to_owned(),
            Self::Window(n) => format!("Window {}", n.saturating_add(1)),
        }
    }
}

/// One card: a tab, and where it is in its group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub pane: PaneId,
    pub group: GroupId,
    pub index: usize,
}

impl Workspace {
    /// Every place that has tabs, the main window's first (even with none:
    /// the switcher always has it), then its docks (left, right, bottom),
    /// then the pop-outs.
    #[must_use]
    pub fn places(&self) -> Vec<Place> {
        let mut out = vec![Place::Main];
        for side in Docks::SIDES {
            if !self.cards(Place::Dock(side)).is_empty() {
                out.push(Place::Dock(side));
            }
        }
        for n in 1..self.windows.len() {
            if !self.cards(Place::Window(n)).is_empty() {
                out.push(Place::Window(n));
            }
        }
        out
    }

    /// The tabs `place` holds, as cards, group by group in order.
    #[must_use]
    pub fn cards(&self, place: Place) -> Vec<Card> {
        let mut groups: Vec<&Group> = Vec::new();
        match place {
            Place::Main => {
                if let Some(main) = self.windows.first() {
                    tree(main.center.as_ref(), &mut groups);
                    groups.extend(main.floats.iter().map(|f| &f.group));
                }
            }
            Place::Dock(side) => {
                let root = self
                    .windows
                    .first()
                    .and_then(|w| w.docks.get(side))
                    .and_then(|d| d.root.as_ref());
                tree(root, &mut groups);
            }
            Place::Window(n) => {
                if let Some(window) = self.windows.get(n).filter(|_| n > 0) {
                    tree(window.center.as_ref(), &mut groups);
                    for side in Docks::SIDES {
                        tree(
                            window.docks.get(side).and_then(|d| d.root.as_ref()),
                            &mut groups,
                        );
                    }
                    groups.extend(window.floats.iter().map(|f| &f.group));
                }
            }
        }
        groups
            .into_iter()
            .flat_map(|g| {
                g.tabs.iter().enumerate().map(|(index, pane)| Card {
                    pane: pane.clone(),
                    group: g.id,
                    index,
                })
            })
            .collect()
    }

    /// The place `pane` is in.
    #[must_use]
    pub fn place_of(&self, pane: &PaneId) -> Option<Place> {
        let loc = self.locate(pane)?;
        Some(match (loc.window, loc.area) {
            (0, crate::model::Area::Dock(side)) => Place::Dock(side),
            (0, _) => Place::Main,
            (n, _) => Place::Window(n),
        })
    }

    /// How many tabs the main window holds: the count on a phone's tabs
    /// button, and "N tabs" in the switcher.
    #[must_use]
    pub fn tab_count(&self) -> usize {
        self.cards(Place::Main).len()
    }
}

/// Every group under `node`, onto `out`.
fn tree<'a>(node: Option<&'a Node>, out: &mut Vec<&'a Group>) {
    if let Some(n) = node {
        n.groups(out);
    }
}

/// Where the card at `from` goes to end up at `to` (both indices into
/// `cards`, `to` counted as if `from` were already taken out): before the
/// card that is there now, or after the last. `None` when it does not
/// move.
#[must_use]
pub fn reorder_dest(cards: &[Card], from: usize, to: usize) -> Option<Dest> {
    if from == to || from >= cards.len() {
        return None;
    }
    let rest: Vec<&Card> = cards
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .map(|(_, c)| c)
        .collect();
    // Indices as the group has them now: `move_pane` counts the removal
    // within one group itself.
    let (group, index) = if let Some(anchor) = rest.get(to) {
        (anchor.group, anchor.index)
    } else {
        let last = rest.last()?;
        (last.group, last.index.saturating_add(1))
    };
    Some(Dest::Group {
        group,
        index: Some(index),
    })
}

/// Where a switcher's cards go.
///
/// A grid of cards the shape of the pane they preview (no taller than
/// [`CardGrid::TALLEST`]), each with its title under it. Two columns on a
/// phone, more on a wider screen (an iPad in compact width).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardGrid {
    pub columns: usize,
    /// A card's preview, points.
    pub card_w: f32,
    pub card_h: f32,
    /// Around the grid and between its cards.
    pub pad: f32,
    pub gap: f32,
    /// Under each card: its title.
    pub title_h: f32,
}

impl CardGrid {
    /// A card's least height for its width.
    pub const SQUAT: f32 = 1.0;
    /// A card's most height for its width (a tall pane is cut at its foot).
    pub const TALLEST: f32 = 1.42;
    /// Around the grid.
    pub const PAD: f32 = 16.0;
    /// Between the cards, across and down.
    pub const GAP: f32 = 18.0;
    /// A title's line under its card, and the space above it.
    pub const TITLE: f32 = 30.0;

    /// The grid for a switcher `width` points wide previewing panes of
    /// `pane_w` by `pane_h`.
    #[must_use]
    pub fn new(width: f32, pane_w: f32, pane_h: f32) -> Self {
        let columns: usize = if width < 600.0 {
            2
        } else if width < 900.0 {
            3
        } else {
            4
        };
        let across = count_f32(columns);
        let card_w =
            across.mul_add(-Self::GAP, 2.0f32.mul_add(-Self::PAD, width) + Self::GAP) / across;
        let card_w = card_w.max(40.0);
        let shape = if pane_w > 0.0 {
            (pane_h / pane_w).clamp(Self::SQUAT, Self::TALLEST)
        } else {
            Self::TALLEST
        };
        Self {
            columns,
            card_w,
            card_h: card_w * shape,
            pad: Self::PAD,
            gap: Self::GAP,
            title_h: Self::TITLE,
        }
    }

    /// From one row's top to the next's.
    #[must_use]
    pub fn row_h(&self) -> f32 {
        self.card_h + self.title_h + self.gap
    }

    /// The preview of card `i`, from the grid's top left.
    #[must_use]
    pub fn rect(&self, i: usize) -> Rect {
        let across = self.columns.max(1);
        let col = count_f32(i.checked_rem(across).unwrap_or(0));
        let row = count_f32(i.checked_div(across).unwrap_or(0));
        Rect::new(
            col.mul_add(self.card_w + self.gap, self.pad),
            row.mul_add(self.row_h(), self.pad),
            self.card_w,
            self.card_h,
        )
    }

    /// How tall `n` cards are, the padding under them included.
    #[must_use]
    pub fn height(&self, n: usize) -> f32 {
        let rows = count_f32(n.div_ceil(self.columns));
        if n == 0 {
            return 2.0 * self.pad;
        }
        rows.mul_add(self.row_h(), 2.0f32.mul_add(self.pad, -self.gap))
    }

    /// The card a point is nearest (grid coordinates), of `n`: where a
    /// dragged card would go.
    #[must_use]
    pub fn index_at(&self, x: f32, y: f32, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let col = ((x - self.pad + self.gap / 2.0) / (self.card_w + self.gap)).floor();
        let row = ((y - self.pad + self.gap / 2.0) / self.row_h()).floor();
        let col = index_of(col).min(self.columns.saturating_sub(1));
        let row = index_of(row);
        (row.saturating_mul(self.columns).saturating_add(col)).min(n.saturating_sub(1))
    }
}

/// A count as points (no grid holds more cards than an `f32` counts).
#[allow(clippy::cast_precision_loss, clippy::as_conversions)]
const fn count_f32(n: usize) -> f32 {
    n as f32
}

/// A floored, non-negative coordinate as an index (a grid has fewer rows
/// than a `usize` counts).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions
)]
fn index_of(v: f32) -> usize {
    if v.is_finite() && v > 0.0 {
        v as usize
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pane;
    use crate::ops::Target;

    fn ids(cards: &[Card]) -> Vec<&str> {
        cards.iter().map(|c| c.pane.0.as_str()).collect()
    }

    fn lab() -> Workspace {
        let mut w = Workspace::new();
        let _ = w.open(Pane::new("a", "note", "A"), Target::Tab);
        let _ = w.open(Pane::new("b", "note", "B"), Target::Tab);
        let _ = w.open(Pane::new("c", "note", "C"), Target::Tab);
        let _ = w.open(Pane::new("g", "graph", "G"), Target::Split(Side::Right));
        let _ = w.open(
            Pane::new("files", "files", "Files"),
            Target::Dock(Side::Left),
        );
        w
    }

    #[test]
    fn the_main_place_holds_every_center_tab_group_by_group() {
        let w = lab();
        assert_eq!(ids(&w.cards(Place::Main)), ["a", "b", "c", "g"]);
        assert_eq!(ids(&w.cards(Place::Dock(Side::Left))), ["files"]);
        assert_eq!(w.tab_count(), 4);
        assert_eq!(w.places(), [Place::Main, Place::Dock(Side::Left)]);
        assert_eq!(
            w.place_of(&PaneId::new("files")),
            Some(Place::Dock(Side::Left))
        );
        assert_eq!(w.place_of(&PaneId::new("g")), Some(Place::Main));
    }

    #[test]
    fn a_pop_out_is_a_place_of_its_own() {
        let mut w = lab();
        let _ = w.move_pane(&PaneId::new("b"), Dest::Window(None));
        assert_eq!(ids(&w.cards(Place::Main)), ["a", "c", "g"]);
        assert_eq!(ids(&w.cards(Place::Window(1))), ["b"]);
        assert!(w.places().contains(&Place::Window(1)));
        assert_eq!(w.place_of(&PaneId::new("b")), Some(Place::Window(1)));
        assert_eq!(Place::Window(1).name(), "Window 2");
    }

    /// Moves the card at `from` to `to` as the switcher does.
    fn reorder(w: &mut Workspace, from: usize, to: usize) {
        let cards = w.cards(Place::Main);
        let pane = cards[from].pane.clone();
        if let Some(dest) = reorder_dest(&cards, from, to) {
            w.move_pane(&pane, dest).unwrap_or_else(|e| panic!("{e:?}"));
        }
    }

    #[test]
    fn a_card_dragged_along_its_group_lands_where_it_was_let_go() {
        let mut w = lab();
        reorder(&mut w, 0, 2);
        assert_eq!(ids(&w.cards(Place::Main)), ["b", "c", "a", "g"]);
        reorder(&mut w, 2, 0);
        assert_eq!(ids(&w.cards(Place::Main)), ["a", "b", "c", "g"]);
        reorder(&mut w, 1, 1);
        assert_eq!(ids(&w.cards(Place::Main)), ["a", "b", "c", "g"]);
    }

    #[test]
    fn a_card_dragged_past_its_group_joins_the_next() {
        let mut w = lab();
        // Past the graph, the last card: after it, in its group.
        reorder(&mut w, 0, 3);
        assert_eq!(ids(&w.cards(Place::Main)), ["b", "c", "g", "a"]);
        // The graph to the front: before `b`, in the first group.
        reorder(&mut w, 2, 0);
        assert_eq!(ids(&w.cards(Place::Main)), ["g", "b", "c", "a"]);
    }

    #[test]
    fn a_pinned_card_stays_at_its_groups_front() {
        let mut w = lab();
        let _ = w.set_pinned(&PaneId::new("c"), true);
        reorder(&mut w, 0, 2);
        let cards = w.cards(Place::Main);
        assert_eq!(cards.len(), 4);
        assert!(ids(&cards).contains(&"a"));
    }

    #[test]
    fn a_phone_gets_two_columns_and_a_wider_screen_more() {
        let phone = CardGrid::new(402.0, 402.0, 630.0);
        assert_eq!(phone.columns, 2);
        assert!((phone.card_w - 176.0).abs() < 0.01);
        // A tall pane is cut at the foot.
        assert!(phone.card_w.mul_add(-CardGrid::TALLEST, phone.card_h).abs() < 0.01);
        assert_eq!(CardGrid::new(694.0, 694.0, 900.0).columns, 3);
        assert_eq!(CardGrid::new(1024.0, 1024.0, 700.0).columns, 4);
    }

    #[test]
    fn cards_fill_rows_left_to_right() {
        let g = CardGrid::new(402.0, 402.0, 402.0);
        let (a, b, c) = (g.rect(0), g.rect(1), g.rect(2));
        assert!((a.x - CardGrid::PAD).abs() < 0.01);
        assert!((b.x - (a.x + g.card_w + g.gap)).abs() < 0.01);
        assert!((b.y - a.y).abs() < 0.01);
        assert!((c.x - a.x).abs() < 0.01);
        assert!((c.y - (a.y + g.row_h())).abs() < 0.01);
        // Three cards: two rows, padded.
        let tall = 2.0f32.mul_add(g.pad, 2.0 * g.row_h()) - g.gap;
        assert!((g.height(3) - tall).abs() < 0.01);
    }

    #[test]
    fn a_point_finds_the_card_under_it_and_the_last_past_the_end() {
        let g = CardGrid::new(402.0, 402.0, 402.0);
        for i in 0..5 {
            let r = g.rect(i);
            assert_eq!(g.index_at(r.x + r.w / 2.0, r.y + r.h / 2.0, 5), i);
        }
        assert_eq!(g.index_at(-50.0, -50.0, 5), 0);
        assert_eq!(g.index_at(390.0, 2000.0, 5), 4);
        assert_eq!(g.index_at(10.0, 10.0, 0), 0);
    }
}
