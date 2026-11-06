//! Where rows come from: a [`Source`] each, searched together by the
//! [`Engine`].

use crate::item::Item;
use crate::open::Opener;
use crate::rank::{Ranked, rank, rank_open};
use crate::recent::Recents;

/// How a source's rows are put in order against the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ranking {
    /// As commands: enabled first, the label counting most (group, detail
    /// and id at half), boosted by having been run lately; an empty text
    /// lists the recent ones first, then by group and label ([`rank`]).
    Commands,
    /// As things to open: the title, else the detail at half, then the
    /// more recent ([`rank_open`]).
    #[default]
    Open,
}

/// One place rows come from: the action list, the vault's notes, the
/// themes, an extension's contributed source, the apps on the system.
///
/// A source is asked each time the text changes, with its own prefix
/// (`theme:`) already taken off; it keeps its rows short (the recent and
/// the matching, not a whole library) and leaves the matching and the
/// order to the [`Engine`].
pub trait Source {
    /// Its id, which is also its prefix (`note`: `note:` narrows to it).
    fn id(&self) -> &str;
    /// Its name, shown over its rows (`Notes`).
    fn title(&self) -> &str;
    /// How its rows are ranked.
    fn ranking(&self) -> Ranking {
        Ranking::Open
    }
    /// Its rows for `text`; each [`Item::source`] is set to [`Self::id`]
    /// by the engine when left empty.
    fn items(&self, text: &str) -> Vec<Item>;
}

/// A [`Source`] from a closure.
pub struct FnSource {
    id: String,
    title: String,
    ranking: Ranking,
    items: Box<dyn Fn(&str) -> Vec<Item>>,
}

impl FnSource {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        items: impl Fn(&str) -> Vec<Item> + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            ranking: Ranking::Open,
            items: Box::new(items),
        }
    }

    /// Ranked as `ranking` ([`Ranking::Open`] unless it says).
    #[must_use]
    pub fn ranking(mut self, ranking: Ranking) -> Self {
        self.ranking = ranking;
        self
    }
}

impl Source for FnSource {
    fn id(&self) -> &str {
        &self.id
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn ranking(&self) -> Ranking {
        self.ranking
    }
    fn items(&self, text: &str) -> Vec<Item> {
        (self.items)(text)
    }
}

impl Source for Opener {
    fn id(&self) -> &str {
        &self.kind
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn items(&self, text: &str) -> Vec<Item> {
        (self.items)(text).into_iter().map(Item::from).collect()
    }
}

/// Every source, searched together: the one engine the palette, ⌘O and
/// the launcher rank through.
#[derive(Default)]
pub struct Engine {
    sources: Vec<Box<dyn Source>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("sources", &self.ids())
            .finish()
    }
}

impl Engine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a source; one with the same id is replaced, in its place.
    pub fn register(&mut self, source: impl Source + 'static) {
        self.register_boxed(Box::new(source));
    }

    /// [`Self::register`], boxed already.
    pub fn register_boxed(&mut self, source: Box<dyn Source>) {
        if let Some(at) = self.sources.iter().position(|s| s.id() == source.id()) {
            if let Some(slot) = self.sources.get_mut(at) {
                *slot = source;
            }
        } else {
            self.sources.push(source);
        }
    }

    /// Source `id`, gone. False when there was none.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.id() != id);
        self.sources.len() != before
    }

    /// The sources' ids, in the order they were added.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.sources.iter().map(|s| s.id()).collect()
    }

    /// Source `id`'s title.
    #[must_use]
    pub fn title(&self, id: &str) -> Option<&str> {
        self.sources
            .iter()
            .find(|s| s.id() == id)
            .map(|s| s.title())
    }

    /// The source `input` narrows to (`theme: dark` → `theme`) when its
    /// prefix is a source's id, and the text after it.
    #[must_use]
    pub fn narrow<'a>(&'a self, input: &'a str) -> (Option<&'a str>, &'a str) {
        match input.split_once(':') {
            Some((k, rest)) => self
                .sources
                .iter()
                .map(|s| s.id())
                .find(|id| *id == k.trim())
                .map_or((None, input.trim()), |id| (Some(id), rest.trim())),
            None => (None, input.trim()),
        }
    }

    /// What `input` finds, best first.
    ///
    /// An `id:` prefix asks one source; otherwise every source is asked.
    /// Each source's rows are ranked by its [`Ranking`]; with text typed,
    /// the lists are merged by score (enabled first; a tie keeps the
    /// sources' order); with none, each source's order stands, the sources
    /// one after another.
    #[must_use]
    pub fn search(&self, input: &str, recents: &Recents) -> Vec<Ranked> {
        let (only, text) = self.narrow(input);
        let mut rows: Vec<Ranked> = self
            .sources
            .iter()
            .filter(|s| only.is_none_or(|id| id == s.id()))
            .flat_map(|s| {
                let items: Vec<Item> = s
                    .items(text)
                    .into_iter()
                    .map(|mut i| {
                        if i.source.is_empty() {
                            s.id().clone_into(&mut i.source);
                        }
                        i
                    })
                    .collect();
                match s.ranking() {
                    Ranking::Commands => rank(&items, text, recents),
                    Ranking::Open => rank_open(items, text),
                }
            })
            .collect();
        if !text.is_empty() {
            rows.sort_by(|a, b| {
                b.item
                    .enabled
                    .cmp(&a.item.enabled)
                    .then(b.score.cmp(&a.score))
            });
        }
        rows
    }
}
