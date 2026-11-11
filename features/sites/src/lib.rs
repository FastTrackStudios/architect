//! `architect-sites`: a vault published as a static site (#364).
//!
//! Any vault (a folder of markdown notes, Obsidian's or a docs folder
//! written for Zola) becomes a site of finished HTML: what a reader gets
//! needs no markdown parser, no vault and no renderer in the browser.
//! architect's own docs (`docs/content`) are the first site
//! (`just docs`), served by the architect server
//! (`architect::host::EngineHost::site`).
//!
//! Moved here from Task's `task:features/ssg` (`task:features/ssg/spec/ssg.md`)
//! and rebuilt on architect's vault:
//!
//! - **The vault reads the folder.** Its walker finds the notes and
//!   attachments (`vault::obsidian::Vault::open`), its link index
//!   ([`vault::obsidian::LinkIndex`]) resolves every `[[wikilink]]`,
//!   markdown link and Zola `@/page.md` link, and gives the backlinks;
//!   its parser gives the tags. Nothing here scans or resolves on its own.
//! - **A link resolves or is reported.** A link into the vault becomes
//!   the page's URL; one that names nothing is a [`Problem`], drawn as
//!   marked text, and fails a [`Options::strict`] build.
//! - **Extensions draw their blocks.** A fenced block an extension claims
//!   (```` ```kf ````) is drawn at build time by a [`BlockRenderer`] into
//!   static HTML. [`PageBlocks`] (feature `pages`) runs the extension's
//!   own embed page once per block and writes the element tree it draws,
//!   the same tree a page draws in a pane; a base view or a diagram plugs
//!   in the same way.
//! - **The layout is pure components** ([`ui`]): each a function of the
//!   built [`Site`] alone, with no hooks, state or I/O, rendered to a
//!   string per page. That is what lets a route be pre-rendered and later
//!   hydrated (the island path of #325) without the two renders
//!   disagreeing. What a reader does in the browser (search, the theme,
//!   the phone menu) is one small script over the finished page.
//! - **The theme is architect's** (`architect-theme`): a light and a dark
//!   theme's tokens become CSS variables, picked by the reader's system
//!   or a toggle.
//!
//! [`build`] does all of it: [`Site::read`] then [`write()`].

pub mod archive;
mod assets;
mod bases;
mod blocks;
#[cfg(feature = "wasi")]
mod component_blocks;
mod element_html;
mod frontmatter;
mod installed;
pub mod listing;
pub mod lyrics;
mod markdown;
pub mod media;
#[cfg(feature = "pages")]
mod page_blocks;
pub mod sanitize;
mod scope;
mod search;
mod site;
pub mod ui;
mod url;
mod write;

pub use architect_extensions::contrib::Claim;
pub use architect_extensions::share_layers::ShareLayersDecl;
pub use bases::BaseBlocks;
pub use blocks::{Block, BlockRenderer, Blocks, Drawn, FnBlock};
#[cfg(feature = "wasi")]
pub use component_blocks::{ComponentBlocks, DRAW_TIMEOUT};
pub use element_html::elements_to_html;
pub use frontmatter::Meta;
pub use installed::{InstalledBlocks, InstalledLayers};
pub use lyrics::LyricsBlocks;
pub use markdown::Heading;
#[cfg(feature = "pages")]
pub use page_blocks::PageBlocks;
pub use sanitize::{BLOCK_BOX_STYLE, sanitize, sanitize_block};
pub use scope::Scope;
pub use search::SearchEntry;
pub use site::{Crumb, NavNode, Options, Page, Problem, Site, SiteError};
pub use url::{encode_path, fragment_id, slug_segment};
pub use write::{Report, WriteError, write};

/// Read the vault in `opts.vault`, draw it with `blocks`, and write the
/// site to `out`.
///
/// # Errors
///
/// The vault is empty or unreadable, a [`Options::strict`] build has a
/// [`Problem`], or the site could not be written.
pub fn build(opts: &Options, blocks: &Blocks, out: &std::path::Path) -> Result<Report, BuildError> {
    let site = Site::read(opts, blocks)?;
    Ok(write(&site, out)?)
}

/// Why [`build`] failed.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Read(#[from] SiteError),
    #[error(transparent)]
    Write(#[from] WriteError),
}
