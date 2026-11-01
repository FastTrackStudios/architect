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

pub mod catalog;
pub mod index;
pub mod manifest;
pub mod package;
pub mod project;
pub mod reapack;

#[cfg(feature = "host")]
pub mod actions;
#[cfg(feature = "crdt")]
pub mod doc;
#[cfg(feature = "host")]
pub mod extensions;
#[cfg(all(feature = "guest", unix))]
pub mod guest;
#[cfg(feature = "host")]
pub mod host;
#[cfg(feature = "host")]
pub mod install;
#[cfg(all(feature = "process", unix))]
pub mod process;
#[cfg(feature = "vox")]
pub mod proto;
#[cfg(all(feature = "remote", unix))]
pub mod remote;
#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "host")]
pub mod watch;
#[cfg(feature = "vox")]
pub mod wire;

pub use catalog::{Candidate, Chosen, Origin, choose};
pub use index::{Hit, Index, Listing, Location};
pub use manifest::{API, Kind, MANIFEST, Manifest, ManifestError, Reach};
pub use package::{Files, Package, PackageError};
pub use project::{Missing, Plan, Policies, Policy, Requirement, Requirements, plan};
pub use semver::{Version, VersionReq};
