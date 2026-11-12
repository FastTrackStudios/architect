//! Extensions: adding to an app at run time what its own features add —
//! actions, settings sections, views, openers, keys, document data —
//! without rebuilding it (`docs/content/architecture/extensions.md`).
//!
//! An extension is a package: a [`Manifest`] (`extension.toml`: its id,
//! version, [`Kind`], the extension API it needs, the capabilities it asks
//! for and everything it adds) and its files. Four kinds:
//!
//! | kind | runs as | where |
//! |---|---|---|
//! | `data` | nothing: settings, presets, keys, frame views | everywhere |
//! | `luau` | scripts on `architect-scripting`'s runtime, over the Action List | everywhere |
//! | `wasm` | a component against `architect:extension` | a [`host::Runner`] the app adds |
//! | `process` | a program speaking vox | a [`host::Runner`] the app adds; desk only |
//!
//! The default build is the model, pure data and wasm-clean: [`Manifest`]
//! and its declarations, a registry's [`Index`] and its search, `ReaPack`'s
//! `index.xml` ([`reapack`]), which copy runs when one is built in, one
//! installed and one in development ([`catalog`]), and what a project
//! needs and an organization allows ([`project`]).
//!
//! Feature `host` runs them: a [`host::Host`] puts an extension's actions
//! on the app's Action List and its sections on the settings registry,
//! takes them off on unload, and swaps them as a diff on a reload (the
//! last good copy staying when the new one fails); [`install`] fetches
//! packages from git registries, folders and `ReaPack` repositories;
//! [`watch`] notices a package's files changing; and
//! [`extensions::Extensions`] puts it together — sources, built-ins,
//! installs, development copies, the organization's policy, hot reload,
//! and a project's requirements when it is opened or joined — with the
//! `EXT_*` actions ([`actions`]) to search, install and remove from ⌘K or
//! MCP. Feature `crdt` keeps a project's requirements and an extension's
//! data in its document ([`doc`]); `ui` brings loaded extensions' sections
//! onto the settings page ([`ui::use_extension_sections`]).

// A linked-in view's root is a fn pointer compared by Dioxus's props: a
// stale compare only redraws.
#![cfg_attr(feature = "ui", allow(unpredictable_function_pointer_comparisons))]

pub mod capabilities;
pub mod catalog;
pub mod contrib;
pub mod documents;
// The editor being typed in, for extensions (#312).
pub mod editor;
pub mod events;
pub mod hostcall;
pub mod index;
pub mod interop;
pub mod manifest;
pub mod package;
pub mod project;
pub mod reapack;
pub mod share_layers;
/// `[[favourites]]`: a note type's tabs and primary action.
pub use architect_favourites as favourites;
pub mod toolbar;
pub mod toolbar_edit;

#[cfg(feature = "host")]
pub mod actions;
#[cfg(feature = "host")]
pub mod actions_manage;
#[cfg(feature = "host")]
pub mod browse;
#[cfg(feature = "host")]
pub mod check;
#[cfg(feature = "host")]
pub mod clone;
#[cfg(feature = "host")]
pub mod consent;
#[cfg(feature = "host")]
pub mod contrib_actions;
#[cfg(feature = "crdt")]
pub mod doc;
#[cfg(feature = "host")]
pub mod embeds;
#[cfg(feature = "host")]
pub mod extensions;
#[cfg(feature = "host")]
mod forward;
#[cfg(all(feature = "guest", unix))]
pub mod guest;
#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod host_events;
#[cfg(feature = "vox")]
pub mod host_proto;
#[cfg(feature = "ui")]
pub mod host_ui_view;
#[cfg(feature = "host")]
pub mod hostapi;
#[cfg(feature = "host")]
pub mod hostui;
#[cfg(feature = "host")]
pub mod install;
#[cfg(feature = "ui")]
pub mod interface;
#[cfg(feature = "host")]
pub mod interop_actions;
#[cfg(any(feature = "host", feature = "web"))]
mod invocation;
#[cfg(any(feature = "linked", all(feature = "guest", unix)))]
pub mod linked;
#[cfg(feature = "host")]
pub mod logs;
#[cfg(feature = "host")]
mod luau_host;
#[cfg(feature = "host")]
pub mod manage;
#[cfg(feature = "manager")]
pub mod manager;
#[cfg(feature = "host")]
pub mod net;
// Language servers through the host (#512): `Host::open_code`, the
// `EDITOR_*` actions. Native only.
#[cfg(all(feature = "lsp", not(target_family = "wasm")))]
pub mod lsp;
#[cfg(feature = "page")]
pub mod page;
#[cfg(feature = "host")]
pub mod points;
#[cfg(feature = "host")]
pub mod ports;
#[cfg(all(feature = "process", unix))]
pub mod process;
#[cfg(feature = "vox")]
pub mod proto;
#[cfg(feature = "host")]
pub mod registries;
#[cfg(feature = "host")]
mod registry_actions;
#[cfg(all(feature = "remote", unix))]
pub mod remote;
#[cfg(feature = "host")]
pub mod source;
#[cfg(feature = "host")]
pub mod toolbar_actions;
#[cfg(feature = "ui")]
pub mod toolbar_editor;
#[cfg(feature = "ui")]
pub mod toolbar_ui;
#[cfg(feature = "host")]
pub mod toolbars;
#[cfg(feature = "ui")]
pub mod typing_ui;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "host")]
pub mod views;
#[cfg(feature = "wasi")]
pub mod wasi;
#[cfg(feature = "host")]
pub mod watch;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;
#[cfg(feature = "ui")]
pub mod window_ui;
#[cfg(feature = "host")]
pub mod windows;
#[cfg(feature = "vox")]
pub mod wire;
#[cfg(feature = "guest")]
pub mod work;

pub use catalog::{Candidate, Chosen, Origin, choose};
pub use contrib::{Claim, Contribution, Contributions, EmbedDecl, EmbedInput};
pub use index::{Hit, Index, Listing, Location};
pub use manifest::{API, Kind, MANIFEST, Manifest, ManifestError, Reach, SourceDecl};
pub use package::{Files, Package, PackageError};
pub use project::{Missing, Plan, Policies, Policy, Requirement, Requirements, plan};
pub use semver::{Version, VersionReq};
