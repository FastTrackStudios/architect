//! An extension's embed pages, drawing its blocks at build time.
//!
//! In the app, a fence an extension claims (`[[embeds]] page =
//! "pages/kf.tsx"`, `claims = { fence = ["kf"] }`) is drawn by that page
//! in a sub-document, its input the fence (`useEmbed()`). A site draws it
//! the same way, once: the page is loaded in a runtime of its own with the
//! block as its input, renders, and the element tree it drew is written as
//! HTML. Its styles arrive as CSS on the theme's variables
//! (`$border` is `var(--border)`), so the site's theme styles it.
//!
//! What a page cannot do here is anything that needs the app: it has no
//! actions, no shared state and no clock past its first render. A page
//! that waits on the app draws its first, empty, frame.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use architect::action::ActionList;
use architect_extensions::contrib::{Claim, EmbedDecl, EmbedInput};
use architect_extensions::manifest::Manifest;
use architect_pages::files::{Origin, PageFile, PageKind};
use architect_pages::host::AppHost;
use architect_pages::runner::{Load, Log, PageLimits, Services};
use architect_scripting::vm::Cancel;

use crate::blocks::{Block, BlockRenderer, Drawn};

/// An extension's embed pages, as a [`BlockRenderer`] for the fences they
/// claim.
pub struct PageBlocks {
    id: String,
    /// Each embed drawn by a page, with that page's file.
    embeds: Vec<(EmbedDecl, PathBuf)>,
    js: Load,
}

impl std::fmt::Debug for PageBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageBlocks")
            .field("id", &self.id)
            .field(
                "embeds",
                &self.embeds.iter().map(|(e, _)| &e.id).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl PageBlocks {
    /// The extension whose package is `dir` (its `extension.toml`): every
    /// `[[embeds]]` drawn by a page that claims a fence.
    ///
    /// # Errors
    ///
    /// Its manifest is missing or does not read.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("extension.toml");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let manifest = Manifest::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let embeds = manifest
            .contributes
            .embeds
            .into_iter()
            .filter(|e| !e.claims.fence.is_empty())
            .filter_map(|e| {
                let page = dir.join(e.page.as_deref()?);
                Some((e, page))
            })
            .collect();
        Ok(Self {
            id: manifest.id,
            embeds,
            js: architect_pages::js::loader(),
        })
    }

    /// The extension's id (`lab.board`).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The fences it draws.
    #[must_use]
    pub fn fences(&self) -> Vec<String> {
        self.embeds
            .iter()
            .flat_map(|(e, _)| e.claims.fence.iter().cloned())
            .collect()
    }
}

impl BlockRenderer for PageBlocks {
    fn name(&self) -> &str {
        &self.id
    }

    fn claims(&self, claim: &Claim) -> bool {
        self.embeds.iter().any(|(e, _)| e.claims(claim))
    }

    fn render(&self, block: &Block<'_>) -> Result<Drawn, String> {
        let (embed, path) = self
            .embeds
            .iter()
            .find(|(e, _)| e.claims(block.claim))
            .ok_or_else(|| format!("nothing of {} claims {}", self.id, block.claim))?;
        let file = PageFile::new(path, Origin::Extension)
            .ok_or_else(|| format!("{} is not a page file", path.display()))?;
        if !matches!(
            file.kind,
            PageKind::Tsx | PageKind::Ts | PageKind::Jsx | PageKind::Js
        ) {
            return Err(format!(
                "{}: only TSX and JS pages draw in a site yet",
                path.display()
            ));
        }
        let input = EmbedInput {
            claim: block.claim.to_string(),
            source: block.source.to_owned(),
            props: serde_json::json!({ "info": block.info, "page": block.page }),
        };
        // Answers to what the page starts never come: nothing here runs
        // past its first render. The receiver lives until it has.
        let (inbox, _answers) = mpsc::channel();
        let services = Services {
            host: Arc::new(AppHost::new(ActionList::new()).with_extension(self.id.clone())),
            inbox,
            log: Log::default(),
            stop: Cancel::default(),
            limits: PageLimits::default(),
            extension: self.id.clone(),
            embed: Some(Arc::new(Mutex::new(input))),
        };
        let mut engine = (self.js)(&file, &services)
            .map_err(|why| format!("{} ({}): {why}", embed.id, path.display()))?;
        let elements = engine
            .render()
            .map_err(|why| format!("{} ({}): {why}", embed.id, path.display()))?;
        Ok(Drawn::Elements(elements))
    }
}
