// `#[architect::rpc]` writes `async fn` into its trait.
#![allow(async_fn_in_trait)]

//! Spaces (`docs/content/architecture/spaces.md`): a scope below the org
//! that people are invited into, with its own roles.
//!
//! A [`Space`] names part of an org — a project's subtree, a wiki, a
//! folder, a document, a session — by the permission resources the org's
//! services already check (`vault/Alpha/`, `doc/<id>`,
//! `files/<root>/Mixes/`). Its [`Member`]s are people in the org, people
//! from other orgs, and link guests ([`Subject`]). A member's role (a
//! [`SpaceRole`]: a named set of actions) holds over the whole scope; a
//! [`SpaceGrant`] adds actions on one resource inside it. What a space
//! gives compiles to permission rules ([`Space::rules_for`]), and the
//! org's [`SpacePolicy`] caps what it may give someone from outside:
//! effective access is the org role ∪ the space role, capped by the
//! policy.
//!
//! - default (wasm-clean): the model and its resolution
//!   ([`OrgSpaces::allows`]).
//! - `vox`: [`SpacesService`] and its gate table ([`PERMITS`]).
//! - `server` (native): [`server::Spaces`], every org's spaces, the gate's
//!   engine for them ([`server::SpacesEngine`]) and the service for the
//!   gate's caller; every change is audited first, and every check reads
//!   the current state, so a removal binds on the next call.
//! - Share links ([`share`], #197/#374): a scope of a vault on a link,
//!   for an audience, with an expiry and an access; [`ShareLinks`] is
//!   its wire (`vox`), [`share_note`] how it is kept in the vault
//!   (`server`).
//! - `files`: a space over a folder grants its members that folder in
//!   `architect-files`, generated per call.

pub mod guest;
mod model;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "vox")]
mod service;
pub mod share;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
mod share_note;
#[cfg(feature = "vox")]
mod share_service;

pub use guest::{
    BlockKind, GUEST_PERMITS, GuestFault, GuestLink, GuestNote, ListedFile, ListedNote, PlainBlock,
    Refused, ShareGuest, ShareUrl,
};
pub use model::{
    COMMENTER, EDITOR, FILES_PREFIX, Holding, MANAGER, Member, NewSpace, OrgSpaces, SPACES_PREFIX,
    Space, SpaceAudit, SpaceChange, SpaceGrant, SpaceId, SpacePatch, SpacePolicy, SpaceRole,
    SpaceSummary, SpacesFault, Standing, Subject, VIEWER, builtin_roles, capped, head, is_pattern,
    overlaps, validate_pattern, validate_reach,
};
#[cfg(feature = "vox")]
pub use service::{
    PERMITS, SpacesService, SpacesServiceClient, layer, serve,
    spaces_service_rpc_service_descriptor,
};
pub use share::{
    Audience, KeptLayers, LayersReport, Listen, NewAudience, NewShare, Share, ShareAccess,
    ShareAudience, ShareChange, ShareComment, ShareFault, ShareScope, ShareState, Verdict, Viewer,
};
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub use share_note::{comments_note, parse_comments, parse_share_note, share_note};
#[cfg(feature = "vox")]
pub use share_service::{
    SHARE_PERMITS, ShareLinks, ShareLinksClient, ShareLinksDispatcher,
    share_links_rpc_service_descriptor,
};
