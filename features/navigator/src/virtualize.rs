//! The virtual window: of a long list of rows of known heights, only those
//! in view (and a few either side) are drawn.
//!
//! Notebook Navigator uses `TanStack` Virtual with `overscan` 10 for both
//! panes, and scrolls to an index with an alignment (`auto` keeps a row
//! that is already in view where it is, `center` centres it):
//! `docs/scroll-orchestration.md`. Here the offsets are a prefix sum, so
//! finding the rows in view is a binary search however long the list is
//! (100,000 notes: `tests/scale.rs`).

/// Rows drawn either side of the view.
pub const OVERSCAN: usize = 10;

/// Where a row should land when scrolled to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Only as far as it takes to bring it into view.
    #[default]
    Auto,
    Start,
    Center,
    End,
}

/// Rows' offsets, from their heights.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Offsets {
    /// `starts[i]` is row `i`'s top; one more entry, the total.
    starts: Vec<f64>,
}

impl Offsets {
    #[must_use]
    pub fn new(heights: impl IntoIterator<Item = f64>) -> Self {
        let mut starts = vec![0.0];
        let mut at = 0.0;
        for h in heights {
            at += h.max(0.0);
            starts.push(at);
        }
        Self { starts }
    }

    /// How many rows.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every row's height together.
    #[must_use]
    pub fn total(&self) -> f64 {
        self.starts.last().copied().unwrap_or(0.0)
    }

    /// Row `i`'s top.
    #[must_use]
    pub fn start(&self, i: usize) -> f64 {
        self.starts.get(i).copied().unwrap_or_else(|| self.total())
    }

    /// Row `i`'s height.
    #[must_use]
    pub fn height(&self, i: usize) -> f64 {
        self.start(i.saturating_add(1)) - self.start(i)
    }

    /// The row at `y` (the last when past the end).
    #[must_use]
    pub fn at(&self, y: f64) -> usize {
        // The first start past `y`, less one.
        self.starts
            .partition_point(|s| *s <= y)
            .saturating_sub(1)
            .min(self.len().saturating_sub(1))
    }

    /// The furthest a view `view` tall can scroll.
    #[must_use]
    pub fn max_scroll(&self, view: f64) -> f64 {
        (self.total() - view).max(0.0)
    }

    /// The rows to draw for a view `view` tall scrolled to `scroll`: those
    /// in view, and `overscan` either side.
    #[must_use]
    pub fn window(&self, scroll: f64, view: f64, overscan: usize) -> std::ops::Range<usize> {
        if self.is_empty() {
            return 0..0;
        }
        let first = self.at(scroll.max(0.0));
        let last = self.at((scroll + view.max(0.0)).max(0.0));
        let start = first.saturating_sub(overscan);
        let end = last
            .saturating_add(overscan)
            .saturating_add(1)
            .min(self.len());
        start..end
    }

    /// Where to scroll for row `i` to be in a view `view` tall now at
    /// `scroll`, aligned by `align`.
    #[must_use]
    pub fn scroll_to(&self, i: usize, align: Align, scroll: f64, view: f64) -> f64 {
        let top = self.start(i);
        let bottom = top + self.height(i);
        let to = match align {
            Align::Start => top,
            Align::End => bottom - view,
            Align::Center => top - (view - (bottom - top)) / 2.0,
            Align::Auto => {
                if top < scroll {
                    top
                } else if bottom > scroll + view {
                    bottom - view
                } else {
                    scroll
                }
            }
        };
        to.clamp(0.0, self.max_scroll(view))
    }
}
