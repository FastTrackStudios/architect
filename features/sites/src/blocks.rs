//! Blocks an extension draws: the seam that makes a site architect's.
//!
//! A note's fenced block is *claimed* by its info string's first word,
//! the same [`Claim`] an extension's `[[embeds]]` makes in the app
//! (`claims = { fence = ["kf"] }`). At build time the site asks its
//! [`Blocks`] who claims each fence; the first [`BlockRenderer`] that does
//! draws it, as an element tree ([`Drawn::Elements`], what a page draws)
//! or finished HTML/SVG ([`Drawn::Html`]). Nobody claiming it leaves an
//! ordinary code block.
//!
//! - [`crate::PageBlocks`] (feature `pages`): an extension's own embed
//!   page, run once per block (the Board's ```` ```kf ```` chart).
//! - [`FnBlock`]: a Rust function, for a renderer that lives in a crate
//!   (a diagram to SVG, a base view's table over the vault).

use std::sync::Arc;

use architect_extensions::contrib::Claim;
use architect_pages::Element;

use crate::element_html::elements_to_html;

/// One block to draw.
#[derive(Debug, Clone, Copy)]
pub struct Block<'a> {
    /// What claims it (`fence:kf`).
    pub claim: &'a Claim,
    /// The fence's whole info string (```` ```kf title="Intro" ````).
    pub info: &'a str,
    /// The block's text: the fence's body.
    pub source: &'a str,
    /// The note it is in, by its vault path.
    pub page: &'a str,
}

/// What a renderer drew.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drawn {
    /// An element tree, as a page draws one (`architect_pages::Element`).
    Elements(Vec<Element>),
    /// Finished markup (HTML, or inline SVG), written as is.
    Html(String),
}

impl Drawn {
    /// As HTML.
    #[must_use]
    pub fn into_html(self) -> String {
        match self {
            Self::Elements(elements) => elements_to_html(&elements),
            Self::Html(html) => html,
        }
    }
}

/// Draws the blocks it claims, at build time.
pub trait BlockRenderer: Send + Sync {
    /// Who draws (`lab.board`): what the drawn block is marked with.
    fn name(&self) -> &str;

    /// Whether it draws `claim`.
    fn claims(&self, claim: &Claim) -> bool;

    /// Draw `block`.
    ///
    /// # Errors
    ///
    /// Why it could not: the build reports it against the note and shows
    /// the block's source in its place.
    fn render(&self, block: &Block<'_>) -> Result<Drawn, String>;
}

/// The renderers a site draws blocks with, asked in the order added.
#[derive(Clone, Default)]
pub struct Blocks {
    renderers: Vec<Arc<dyn BlockRenderer>>,
}

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.renderers.iter().map(|r| r.name().to_owned()))
            .finish()
    }
}

impl Blocks {
    /// None: every fence is a code block.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// With `renderer` asked after those already added.
    #[must_use]
    pub fn with(mut self, renderer: impl BlockRenderer + 'static) -> Self {
        self.add(Arc::new(renderer));
        self
    }

    /// Add `renderer`, asked after those already added.
    pub fn add(&mut self, renderer: Arc<dyn BlockRenderer>) {
        self.renderers.push(renderer);
    }

    /// Whether none are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.renderers.is_empty()
    }

    /// The first renderer claiming `block`'s claim: its name and what it
    /// drew. `None` when nobody claims it.
    #[must_use]
    pub fn draw(&self, block: &Block<'_>) -> Option<(String, Result<String, String>)> {
        let renderer = self.renderers.iter().find(|r| r.claims(block.claim))?;
        let drawn = renderer.render(block).map(Drawn::into_html);
        Some((renderer.name().to_owned(), drawn))
    }
}

/// A renderer that is a Rust function, for the fences it names.
///
/// ```
/// use architect_sites::{Blocks, Drawn, FnBlock};
/// let blocks = Blocks::new().with(FnBlock::new("demo", &["shout"], |block| {
///     Ok(Drawn::Html(format!("<p>{}</p>", block.source.to_uppercase())))
/// }));
/// assert!(!blocks.is_empty());
/// ```
pub struct FnBlock<F> {
    name: String,
    fences: Vec<String>,
    draw: F,
}

impl<F> FnBlock<F>
where
    F: Fn(&Block<'_>) -> Result<Drawn, String> + Send + Sync,
{
    /// `draw` for every fence in `fences`, as `name`.
    pub fn new(name: impl Into<String>, fences: &[&str], draw: F) -> Self {
        Self {
            name: name.into(),
            fences: fences.iter().map(|f| (*f).to_owned()).collect(),
            draw,
        }
    }
}

impl<F> BlockRenderer for FnBlock<F>
where
    F: Fn(&Block<'_>) -> Result<Drawn, String> + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn claims(&self, claim: &Claim) -> bool {
        matches!(claim, Claim::Fence(lang) if self.fences.iter().any(|f| f.eq_ignore_ascii_case(lang)))
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        (self.draw)(block)
    }
}
