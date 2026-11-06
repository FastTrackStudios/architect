//! Accounts on an issuer: who is signed in on this device, and how.
//!
//! An app signs in at the issuer (FTS's is `auth.fasttrackstudio.app`, the
//! `apps/auth-server` deployment) and may hold several accounts, one of them
//! active. Signing in is optional: an app works signed out.
//!
//! - [`Account`], [`AccountsFile`]: the accounts and the active one, the
//!   metadata a switcher shows. No secrets: it is a plain JSON file.
//! - [`SecretStore`]: where each account's session lives. [`Keychain`]
//!   (feature `keychain`) is the platform's; [`MemorySecrets`] is for tests.
//!   A session is never written to a plain file.
//! - [`AccountStore`]: the two together, the file under the app's folder and
//!   the sessions in the store; [`ActiveSession`] is the active account's
//!   session as an `auth_client::TokenStore`, which is what an org list or
//!   a vox client takes.
//! - [`Accounts`] (feature `http`): the controller an app and its actions
//!   drive, thread-safe — sign in by a browser code (the device
//!   authorization grant, which signs in with whatever the issuer offers:
//!   passkeys, social, password) or by email and password, switch, sign out
//!   (revoked at the issuer), and the account's devices.
//! - [`actions`] (feature `actions`): `ACCOUNT_*` on an Action List, so MCP,
//!   the menus and ⌘K reach them.
//! - [`ui`] (feature `ui`): the shell's Account item with the switcher, and
//!   the sign-in sheet with the devices.

mod model;
mod secrets;
mod store;

#[cfg(all(feature = "keychain", not(target_arch = "wasm32")))]
mod keychain;

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
mod controller;
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
mod enrolment;
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
mod issuer;

#[cfg(all(feature = "actions", not(target_arch = "wasm32")))]
pub mod actions;
#[cfg(all(feature = "ui", not(target_arch = "wasm32")))]
pub mod ui;

pub use model::{Account, AccountsFile, DEVICE_ORG, Device, OrgHost, device_org, with_device_org};
pub use secrets::{MemorySecrets, SecretError, SecretStore};
pub use store::{AccountStore, ActiveSession, StoreError};

#[cfg(all(feature = "keychain", not(target_arch = "wasm32")))]
pub use keychain::{DEFAULT_SERVICE, Keychain, SERVICE_ENV};

#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use controller::{Accounts, BrowserWait, Sheet, Status};
#[cfg(all(feature = "http", not(target_arch = "wasm32")))]
pub use issuer::{DEFAULT_ISSUER, ISSUER_ENV, Issuer, IssuerError, SignedIn, issuer_from_env};
