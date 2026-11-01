//! ⌘O: open anything. Each feature registers an [`Opener`] for what it
//! can open (notes, songs, workspace views, screensets); a query asks them
//! all at once, or one by its prefix (`song:`).

use crate::fuzzy::fuzzy;

/// Something that can be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    /// The opener it came from (`note`).
    pub kind: String,
    /// What the opener opens it by.
    pub id: String,
    pub title: String,
    pub detail: String,
    /// Higher is more recent (a timestamp, or an order); 0 when unknown.
    pub recency: u64,
}

impl OpenItem {
    pub fn new(kind: impl Into<String>, id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            title: title.into(),
            detail: String::new(),
            recency: 0,
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    #[must_use]
    pub const fn recency(mut self, recency: u64) -> Self {
        self.recency = recency;
        self
    }
}

/// Where an opened item goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenIn {
    /// In place of the active tab (↵).
    Here,
    /// A new tab (⌘↵).
    Tab,
    /// A split beside the active one (⌘⌥↵).
    Split,
}

/// One kind of thing that can be opened.
pub struct Opener {
    /// Its prefix and its name (`note`, `Notes`).
    pub kind: String,
    pub title: String,
    /// Everything it can open (a feature keeps this short: recent and
    /// matching items, not a whole library).
    pub items: Box<dyn Fn(&str) -> Vec<OpenItem>>,
    pub open: Box<dyn Fn(&OpenItem, OpenIn)>,
}

impl Opener {
    pub fn new(
        kind: impl Into<String>,
        title: impl Into<String>,
        items: impl Fn(&str) -> Vec<OpenItem> + 'static,
        open: impl Fn(&OpenItem, OpenIn) + 'static,
    ) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            items: Box::new(items),
            open: Box::new(open),
        }
    }
}

/// Every opener, searched together.
#[derive(Default)]
pub struct Openers {
    openers: Vec<Opener>,
}

/// An item that matched, with what lit up in its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub item: OpenItem,
    pub positions: Vec<usize>,
    pub score: i64,
}

impl Openers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an opener; one of the same kind is replaced.
    pub fn register(&mut self, opener: Opener) {
        self.openers.retain(|o| o.kind != opener.kind);
        self.openers.push(opener);
    }

    pub fn remove(&mut self, kind: &str) {
        self.openers.retain(|o| o.kind != kind);
    }

    #[must_use]
    pub fn title(&self, kind: &str) -> Option<&str> {
        self.openers
            .iter()
            .find(|o| o.kind == kind)
            .map(|o| o.title.as_str())
    }

    /// What `query` finds: `kind:` narrows to one opener; an empty query
    /// lists the most recent of each; matches best first, then recent.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<Found> {
        let (only, text) = match query.split_once(':') {
            Some((k, rest)) if self.openers.iter().any(|o| o.kind == k.trim()) => {
                (Some(k.trim()), rest.trim())
            }
            _ => (None, query.trim()),
        };
        let mut found: Vec<Found> = self
            .openers
            .iter()
            .filter(|o| only.is_none_or(|k| k == o.kind))
            .flat_map(|o| (o.items)(text))
            .filter_map(|item| {
                let m = fuzzy(text, &item.title).or_else(|| {
                    fuzzy(text, &item.detail).map(|mut m| {
                        m.positions.clear();
                        m.score /= 2;
                        m
                    })
                })?;
                Some(Found {
                    item,
                    positions: m.positions,
                    score: m.score,
                })
            })
            .collect();
        found.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.item.recency.cmp(&a.item.recency))
                .then_with(|| a.item.title.cmp(&b.item.title))
        });
        found
    }

    /// Opens `item` with the opener that found it. False when no opener
    /// has its kind.
    #[must_use = "false when nothing could open it"]
    pub fn open(&self, item: &OpenItem, open_in: OpenIn) -> bool {
        self.openers
            .iter()
            .find(|o| o.kind == item.kind)
            .is_some_and(|o| {
                (o.open)(item, open_in);
                true
            })
    }
}
