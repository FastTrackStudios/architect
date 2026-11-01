//! Organizations (`docs/content/architecture/organizations.md`): the
//! person's orgs, from the one identity server every app signs in to, as
//! the scope everything else is in.
//!
//! The default build is the model: [`Org`]s with the person's [`Role`],
//! the window's [`Selection`] (one org, or All), the app's
//! [`Capabilities`] by role (roles are the org's, what they let a member
//! do is the app's), and [`Orgs`], cached on the device so the switcher
//! works offline. A [`Scope`] is what a read covers (the window's org,
//! or every org in All), [`OrgId::path`] the key every layer stores an
//! org's data under, and [`read_all`] the All-organizations read.
//! [`Refresh`] says when the list is fetched again (every reconnect).
//!
//! Feature `auth` reads the identity server's answer ([`from_bundles`])
//! and fetches it ([`fetch`]); `ui` is [`use_org`], the shell's switcher
//! and [`use_orgs_live`]; `commands` is ⌘O across orgs
//! ([`scoped_opener`]); `server` confines every service to the caller's
//! org ([`server`]).

#[cfg(feature = "auth")]
mod auth;
mod model;
#[cfg(feature = "commands")]
mod opener;
mod refresh;
mod scope;
#[cfg(all(feature = "server", not(target_arch = "wasm32")))]
pub mod server;
#[cfg(feature = "ui")]
mod ui;

#[cfg(feature = "auth")]
pub use auth::{fetch, from_bundles};
pub use model::{Capabilities, Org, OrgId, Orgs, Role, Selection};
#[cfg(feature = "commands")]
pub use opener::scoped_opener;
pub use refresh::Refresh;
pub use scope::{Aggregate, Scope, Scoped, read_all};
#[cfg(feature = "ui")]
pub use ui::{OrgHandle, use_org, use_orgs_live, use_orgs_provider};
