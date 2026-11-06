//! What was run or opened lately, most recent first, kept to a length.

use facet::Facet;

/// A bounded most-recent-first list of ids.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Recents {
    pub ids: Vec<String>,
    pub keep: usize,
}

impl Default for Recents {
    fn default() -> Self {
        Self::new(30)
    }
}

impl Recents {
    #[must_use]
    pub const fn new(keep: usize) -> Self {
        Self {
            ids: Vec::new(),
            keep,
        }
    }

    /// `id` was just used: it goes to the front.
    pub fn touch(&mut self, id: &str) {
        self.ids.retain(|i| i != id);
        self.ids.insert(0, id.to_owned());
        self.ids.truncate(self.keep);
    }

    /// How recently `id` was used: 0 for the last, `None` when not lately.
    #[must_use]
    pub fn rank(&self, id: &str) -> Option<usize> {
        self.ids.iter().position(|i| i == id)
    }
}
