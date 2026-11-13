//! Inline embeds: an extension's view drawn inside a host's document,
//! sized to its content.
//!
//! An embed draws a block it claims — a fenced block by its language, a
//! note by its `type:`, an embed's target by its type —
//! (`docs/content/architecture/extensions.md`, "Inline embeds").
//!
//! An extension declares `[[embeds]]` (`contrib::EmbedDecl`) in its
//! manifest, or contributes them at run time; they go with it when it
//! unloads. An app asks [`Embeds::resolve`] whether a block is claimed,
//! and mounts [`EmbedView`] (feature `ui`) where it is:
//!
//! ```ignore
//! let claim = Claim::fence("kf").unwrap();
//! rsx! {
//!     EmbedView { claim, source: body, props: json!({ "info": info }),
//!         fallback: rsx! { pre { "{body}" } } }
//! }
//! ```
//!
//! How it is drawn follows the view: a linked-in view is a component in
//! the host's own document (content-sized by layout, its events the
//! document's); a page (TSX/JS on `QuickJS`, Luau) and a process's view
//! are a Blitz sub-document, a `remote::RemoteDocument`, whose content
//! height the host measures and lays out, up to the embed's
//! `max_height` ([`DEFAULT_MAX_HEIGHT`]), past which it scrolls. Either
//! way the view is given an `EmbedInput` (the claim, the block's text,
//! the host's properties) and is drawn again when it changes.

use std::path::PathBuf;

use crate::catalog::Origin;
use crate::contrib::{Claim, EmbedDecl};
use crate::host::Host;
use crate::manifest::Kind;

/// How tall an embed grows when it says no `max_height`, points.
pub const DEFAULT_MAX_HEIGHT: u32 = 480;

/// How short a sub-document embed is before its first frame has a size,
/// points.
pub const MIN_HEIGHT: f32 = 24.0;

/// A claimed block's embed: whose it is, what draws it, how tall it may
/// grow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedSpec {
    /// The extension.
    pub extension: String,
    /// Its declaration.
    pub embed: EmbedDecl,
    /// What was asked.
    pub claim: Claim,
    /// The view name it is drawn and metered as (`<view>`, or
    /// `embed.<id>` for a page of its own).
    pub view: String,
    /// The page file that draws it, in its package: the embed's `page`,
    /// or its view's.
    pub page: Option<String>,
    /// The extension's folder (a linked built-in has none).
    pub dir: Option<PathBuf>,
    pub origin: Origin,
    pub kind: Kind,
    /// The most it grows to, points.
    pub max_height: u32,
}

/// The app's inline embeds, from its host's contributions. Cheap to
/// clone.
#[derive(Clone, Debug)]
pub struct Embeds {
    host: Host,
}

impl Embeds {
    #[must_use]
    pub const fn new(host: Host) -> Self {
        Self { host }
    }

    /// The embed that draws `claim`, when a loaded extension claims it
    /// (the first loaded that does).
    #[must_use]
    pub fn resolve(&self, claim: &Claim) -> Option<EmbedSpec> {
        let by = self.host.contributions().embed_for(claim)?;
        self.spec(by.owner, by.item, claim.clone())
    }

    /// Extension `extension`'s embed `id` (its `[[embeds]]` id, or the
    /// view one draws), whatever it claims: what an editor's widget
    /// decoration mounts (`crate::editor`). Its claim is
    /// `embed_type:widget`.
    #[must_use]
    pub fn of(&self, extension: &str, id: &str) -> Option<EmbedSpec> {
        let embed = self
            .host
            .contributions()
            .embeds()
            .into_iter()
            .filter(|by| by.owner == extension)
            .map(|by| by.item)
            .find(|e| e.id == id || e.view.as_deref() == Some(id))?;
        self.spec(
            extension.to_owned(),
            embed,
            Claim::EmbedType("widget".to_owned()),
        )
    }

    /// The base view type `id` a loaded extension adds (`[[base_views]]`),
    /// as an embed: what Bases mounts for a base's view of that type
    /// ([`BaseViewEmbed`], feature `ui`). Its claim is
    /// `embed_type:base_view`.
    #[must_use]
    pub fn base_view(&self, id: &str) -> Option<EmbedSpec> {
        let by = self.host.contributions().base_view(id)?;
        let decl = by.item;
        let embed = EmbedDecl {
            id: format!("base_view.{}", decl.id),
            view: decl.page.is_none().then(|| decl.view_name()),
            title: decl.name,
            page: decl.page,
            claims: crate::contrib::EmbedClaims {
                embed_type: vec!["base_view".to_owned()],
                ..Default::default()
            },
            max_height: None,
        };
        self.spec(by.owner, embed, Claim::EmbedType("base_view".to_owned()))
    }

    fn spec(&self, owner: String, embed: EmbedDecl, claim: Claim) -> Option<EmbedSpec> {
        let loaded = self.host.get(&owner)?;
        let page = embed.page.clone().or_else(|| {
            let view = embed.view.as_deref()?;
            loaded
                .manifest
                .views
                .iter()
                .find(|v| v.id == view)
                .and_then(|v| v.page.clone())
        });
        Some(EmbedSpec {
            extension: owner,
            view: embed.view_name(),
            page,
            dir: loaded.dir.clone(),
            origin: loaded.origin,
            kind: loaded.manifest.kind,
            max_height: embed.max_height.unwrap_or(DEFAULT_MAX_HEIGHT),
            claim,
            embed,
        })
    }

    /// Every claim a loaded extension makes, with its owner and embed:
    /// what an editor's fence registry is filled from.
    #[must_use]
    pub fn claims(&self) -> Vec<(Claim, String, String)> {
        self.host
            .contributions()
            .embeds()
            .into_iter()
            .flat_map(|by| {
                let (owner, id) = (by.owner, by.item.id.clone());
                by.item
                    .claims
                    .all()
                    .into_iter()
                    .map(move |c| (c, owner.clone(), id.clone()))
            })
            .collect()
    }

    /// The fence languages claimed now, lower case, each once.
    #[must_use]
    pub fn fences(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .claims()
            .into_iter()
            .filter_map(|(c, _, _)| match c {
                Claim::Fence(lang) => Some(lang.to_ascii_lowercase()),
                Claim::NoteType(_) | Claim::EmbedType(_) => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

impl Host {
    /// Its inline embeds.
    #[must_use]
    pub fn embeds(&self) -> Embeds {
        Embeds::new(self.clone())
    }
}

#[cfg(feature = "ui")]
pub use view::{BaseViewEmbed, EmbedAsk, EmbedPages, EmbedView, WidgetView};

#[cfg(feature = "ui")]
mod view {
    use std::rc::Rc;

    use dioxus::prelude::*;
    use serde_json::Value;

    use super::{EmbedSpec, Embeds};
    use crate::contrib::{Claim, EmbedInput};
    use crate::interface::ExtensionUi;
    use crate::linked::EmbedContext;

    /// A page embed, as [`EmbedPages`] is asked for it.
    #[derive(Clone)]
    pub struct EmbedAsk {
        pub spec: EmbedSpec,
        /// What its runs may reach: the extension's capabilities.
        pub gate: architect_scripting::luau::Gate,
        /// What it is given, live.
        pub input: Signal<EmbedInput>,
        /// A guest's content (a share link's note, #629): the page runs
        /// on a host that reaches nothing of the person's, `gate` unused.
        pub guest: bool,
    }

    impl std::fmt::Debug for EmbedAsk {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("EmbedAsk")
                .field("spec", &self.spec)
                .finish_non_exhaustive()
        }
    }

    /// Draws a page embed: `architect-pages` provides it
    /// (`architect_pages::ui::use_embed_pages`), a sub-document sized to
    /// its content. `None` leaves it to the extension's own view.
    #[derive(Clone)]
    pub struct EmbedPages(pub Rc<dyn Fn(&EmbedAsk) -> Option<Element>>);

    /// The block `claim` claims, drawn by the extension that claims it,
    /// inline and content-sized; `fallback` (the plain block) when none
    /// does, or once it unloads.
    ///
    /// `source` is the block's text (a fence's body, a note's), `props`
    /// the host's properties for it (a note's frontmatter, a fence's info
    /// string); the view is drawn again when either changes. The
    /// `ExtensionUi` in context says what is loaded.
    ///
    /// `guest`: the block is a guest's content (a share link's note,
    /// #629), never drawn with the person's reach. Only a page embed
    /// draws it, on a host with no action, no host-service call and no
    /// vault; a linked or process view, which runs with the person's own
    /// session, does not, and `fallback` shows.
    #[component]
    pub fn EmbedView(
        claim: Claim,
        source: String,
        #[props(default)] props: Value,
        #[props(default)] fallback: Option<Element>,
        #[props(default)] guest: bool,
    ) -> Element {
        let now = EmbedInput {
            claim: claim.to_string(),
            source,
            props,
        };
        let mut input = use_signal(|| now.clone());
        use_effect(use_reactive!(|now| {
            if *input.peek() != now {
                input.set(now);
            }
        }));
        let plain = move || fallback.clone().unwrap_or_else(|| rsx! {});
        let Some(ui) = try_use_context::<ExtensionUi>() else {
            return plain();
        };
        let _ = ui.revision();
        let host = ui.host();
        let Some(spec) = Embeds::new(host.clone()).resolve(&claim) else {
            return plain();
        };
        if guest {
            return draw_guest(&host, spec, input, plain);
        }
        draw(&host, spec, input, plain)
    }

    /// `spec` drawn for a guest's content, given `input`: only by a page
    /// (its own or its view's), on a host that reaches nothing of the
    /// person's (`architect_pages::host::AppHost::guest`); else `plain`.
    /// A linked view runs in the app's process with the person's session,
    /// a process view with its own connection's: neither can be kept from
    /// the person's vaults, so neither draws a guest's content.
    fn draw_guest(
        host: &crate::host::Host,
        spec: EmbedSpec,
        input: Signal<EmbedInput>,
        plain: impl Fn() -> Element,
    ) -> Element {
        if host.get(&spec.extension).is_none() {
            return plain();
        }
        let Some(pages) = try_consume_context::<EmbedPages>() else {
            return plain();
        };
        let key = format!("{}/{}", spec.extension, spec.embed.id);
        let ask = EmbedAsk {
            // Unused for a guest: nothing passes.
            gate: std::sync::Arc::new(|_| Err("a guest's content runs nothing".to_owned())),
            input,
            spec,
            guest: true,
        };
        (pages.0)(&ask).map_or_else(plain, |page| {
            rsx! {
                div { class: "embed", "data-embed": "{key}", "data-guest": "true", style: "{BOX}", {page} }
            }
        })
    }

    /// An extension's embed `id` mounted by an editor's widget decoration
    /// (`crate::editor`): drawn as an inline embed is, given `props` (the
    /// widget's) and `source` (empty); nothing when the extension or the
    /// embed is gone.
    #[component]
    pub fn WidgetView(extension: String, id: String, #[props(default)] props: Value) -> Element {
        let now = EmbedInput {
            claim: Claim::EmbedType("widget".to_owned()).to_string(),
            source: String::new(),
            props,
        };
        let mut input = use_signal(|| now.clone());
        use_effect(use_reactive!(|now| {
            if *input.peek() != now {
                input.set(now);
            }
        }));
        let Some(ui) = try_use_context::<ExtensionUi>() else {
            return rsx! {};
        };
        let _ = ui.revision();
        let host = ui.host();
        let Some(spec) = Embeds::new(host.clone()).of(&extension, &id) else {
            return rsx! {};
        };
        draw(&host, spec, input, || rsx! {})
    }

    /// A base's view of a type an extension adds (`[[base_views]]`, type
    /// `kind`), drawn by it and given `input` (the view's spec and rows),
    /// up to `max_height` points tall; `fallback` when no loaded extension
    /// adds the type, or once it unloads.
    #[component]
    pub fn BaseViewEmbed(
        kind: String,
        input: crate::contrib::BaseViewInput,
        #[props(default = u32::MAX)] max_height: u32,
        #[props(default)] fallback: Option<Element>,
    ) -> Element {
        let now = EmbedInput {
            claim: Claim::EmbedType("base_view".to_owned()).to_string(),
            source: input.base.clone(),
            props: serde_json::to_value(&input).unwrap_or_default(),
        };
        let mut live = use_signal(|| now.clone());
        use_effect(use_reactive!(|now| {
            if *live.peek() != now {
                live.set(now);
            }
        }));
        let plain = move || fallback.clone().unwrap_or_else(|| rsx! {});
        let Some(ui) = try_use_context::<ExtensionUi>() else {
            return plain();
        };
        let _ = ui.revision();
        let host = ui.host();
        let Some(mut spec) = Embeds::new(host.clone()).base_view(&kind) else {
            return plain();
        };
        spec.max_height = max_height;
        draw(&host, spec, live, plain)
    }

    /// `spec` drawn, given `input`: a page's sub-document, a linked view in
    /// the host's document, or a process's; `plain` when none can be.
    fn draw(
        host: &crate::host::Host,
        spec: EmbedSpec,
        input: Signal<EmbedInput>,
        plain: impl Fn() -> Element,
    ) -> Element {
        let Some(loaded) = host.get(&spec.extension) else {
            return plain();
        };
        let key = format!("{}/{}", spec.extension, spec.embed.id);
        let max = spec.max_height;
        if spec.page.is_some()
            && let Some(pages) = try_consume_context::<EmbedPages>()
        {
            let ask = EmbedAsk {
                gate: crate::host::gate(&loaded.manifest),
                input,
                spec: spec.clone(),
                guest: false,
            };
            if let Some(page) = (pages.0)(&ask) {
                return rsx! {
                    div { class: "embed", "data-embed": "{key}", style: "{BOX}", {page} }
                };
            }
        }
        if let Some(view) = spec.embed.view.as_deref()
            && let Some(root) = host.linked_view(&spec.extension, view)
        {
            // In the host's own document: laid out by it, its events its.
            return rsx! {
                div { class: "embed", "data-embed": "{key}",
                    style: "{BOX} max-height: {max}px; overflow-y: auto;",
                    LinkedEmbed { key: "{key}", root, input }
                }
            };
        }
        #[cfg(all(feature = "remote", unix))]
        if let Some(view) = spec.embed.view.clone()
            && let Some(processes) = try_use_context::<crate::process::Processes>()
        {
            return rsx! {
                div { class: "embed", "data-embed": "{key}", style: "{BOX}",
                    crate::remote::RemoteEmbed {
                        key: "{key}",
                        processes,
                        extension: spec.extension,
                        view,
                        input,
                        max_height: max,
                    }
                }
            };
        }
        plain()
    }

    /// An embed's box in the host's flow: as wide as the text, as tall as
    /// what is in it.
    const BOX: &str = "display: flex; flex-direction: column; width: 100%; flex-shrink: 0;";

    /// A linked-in view, given its input.
    #[component]
    fn LinkedEmbed(root: fn() -> Element, input: Signal<EmbedInput>) -> Element {
        use_context_provider(|| EmbedContext(input));
        root()
    }
}
