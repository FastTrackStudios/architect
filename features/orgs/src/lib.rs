//! Organizations (`docs/content/architecture/organizations.md`): the
//! person's orgs, from the one identity server every app signs in to, as
//! the scope everything else is in.
//!
//! The default build is the model: [`Org`]s with the person's [`Role`],
//! the window's [`Selection`] (one org, or All), the app's
//! [`Capabilities`] by role (roles are the org's, what they let a member
//! do is the app's), and [`Orgs`], cached on the device so the switcher
//! works offline. Feature `auth` reads the identity server's answer;
//! `ui` is [`use_org`] and the shell's switcher.

#[cfg(feature = "auth")]
mod auth;
mod model;
#[cfg(feature = "ui")]
mod ui;

#[cfg(feature = "auth")]
pub use auth::from_bundles;
pub use model::{Capabilities, Org, OrgId, Orgs, Role, Selection};
#[cfg(feature = "ui")]
pub use ui::{OrgHandle, use_org, use_orgs_provider};
