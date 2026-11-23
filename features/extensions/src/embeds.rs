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
            fill: false,
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

    /// The note types the app's own linked extensions draw, each with the
    /// view that draws it and what its extension declares it reads: every
    /// `[[embeds]]` of a compiled-in extension (`Origin::Builtin`, not
    /// overridden by another copy of its id) that claims a note type with
    /// one of its linked views, in the order they loaded.
    ///
    /// What an app whose views are compiled in (a distribution's web app)
    /// chooses a note's view from, before any separately loaded one.
    #[cfg(feature = "linked")]
    #[must_use]
    pub fn linked_note_views(&self) -> Vec<LinkedNoteView> {
        self.host
            .contributions()
            .embeds()
            .into_iter()
            .filter(|by| !by.item.claims.note_type.is_empty())
            .filter_map(|by| {
                let view = by.item.view.clone()?;
                self.host.linked_view_root(&by.owner, &view)?;
                let loaded = self.host.get(&by.owner)?;
                Some(LinkedNoteView {
                    reads: self
                        .host
                        .capabilities_of(&loaded.manifest, loaded.origin)
                        .reads,
                    extension: by.owner,
                    view,
                    note_types: by.item.claims.note_type,
                    fill: by.item.fill,
                    max_height: by.item.max_height.unwrap_or(DEFAULT_MAX_HEIGHT),
                })
            })
            .collect()
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

    /// The `[[favourites]]` its linked extensions declare (compiled in,
    /// running as built-ins), each with who declared it and what its
    /// capabilities let it run: what an app reads in its own process,
    /// where a node's listing (`/_ext/favourites.json`) is read otherwise.
    #[cfg(feature = "linked")]
    #[must_use]
    pub fn linked_favourites(&self) -> Vec<architect_favourites::Listed> {
        let linked = self.linked();
        self.loaded()
            .into_iter()
            .filter(|l| {
                l.origin == Origin::Builtin && linked.iter().any(|(id, _)| *id == l.manifest.id)
            })
            .flat_map(|l| {
                let by = l.manifest.declarer();
                l.manifest.favourites.into_iter().map(move |favourites| {
                    architect_favourites::Listed {
                        by: by.clone(),
                        favourites,
                    }
                })
            })
            .collect()
    }

    /// Its linked extensions' views, each extension with the ids of the
    /// views its code draws (`(id, views)`): what a favourite's `{view}`
    /// of that extension may open in the app's own document.
    #[cfg(feature = "linked")]
    #[must_use]
    pub fn linked_view_ids(&self) -> Vec<(String, Vec<String>)> {
        self.linked()
            .into_iter()
            .filter(|(id, _)| self.get(id).is_some_and(|l| l.origin == Origin::Builtin))
            .map(|(id, built)| (id, built.view_ids()))
            .collect()
    }
}

/// A note type's view of a linked extension
/// ([`Embeds::linked_note_views`]).
#[cfg(feature = "linked")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedNoteView {
    /// The extension.
    pub extension: String,
    /// Its view that draws the note.
    pub view: String,
    /// The note types the view claims, as declared.
    pub note_types: Vec<String>,
    /// Whether the view is the note's page (its embed's `fill`), not a
    /// card at the note's head.
    pub fill: bool,
    /// The most it grows to at a note's head, points (its embed's
    /// `max_height`, else [`DEFAULT_MAX_HEIGHT`]).
    pub max_height: u32,
    /// What its extension may read (`[capabilities] reads`): the app
    /// hands its view a note's text only with `documents`, a note's files
    /// only with `files` too.
    pub reads: Vec<String>,
}

/// Which views draw a guest's content (a share link's note, #629): the
/// embedding app's choice, in context above [`GuestEmbedView`].
///
/// A guest's content is never drawn with a person's reach. A page embed
/// runs on a host that reaches nothing (`architect_pages::host::AppHost::guest`),
/// so it always may. A linked view runs in the app's process and asks the
/// app's host; a process view asks over its own connection. On a desk the
/// app's host is the person's own session, with their vaults, so neither
/// draws a guest's content there: that is the default, and what an app
/// that provides no policy gets.
///
/// An app whose host is a guest's ([`Host::for_guest`]: no vault, no
/// session, every vault request refused) may let its own linked
/// extensions draw ([`Self::own_linked`]). Those views are compiled into
/// the app: they are the app itself, as much as the page around them is,
/// and there is nothing of a person's in the process for them to reach.
/// What they are handed is still the link's permission: the app gives
/// them the guest's note, never more than the link's node gave it.
/// A process view never draws a guest's content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GuestEmbedPolicy {
    linked: bool,
}

impl GuestEmbedPolicy {
    /// Pages only: what an app that says nothing gets (a desk).
    #[must_use]
    pub const fn pages_only() -> Self {
        Self { linked: false }
    }

    /// The app's own linked extensions too, on a guest's host.
    #[must_use]
    pub const fn own_linked() -> Self {
        Self { linked: true }
    }

    /// Whether a linked view may draw a guest's content on `host`: the
    /// app said so, and `host` runs for a guest. Both, so an app that
    /// holds a person's session never draws one by a policy set in error.
    #[must_use]
    pub fn draws_linked(self, host: &Host) -> bool {
        self.linked && host.is_guest()
    }
}
