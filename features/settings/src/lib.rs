//! Settings (`docs/content/architecture/settings-page.md`): declared
//! once — by the app, or by an extension while it is loaded — stored,
//! validated, searched, and drawn as T3 Code's settings page.
//!
//! The default build is the model: a [`Section`] of [`Setting`]s is data
//! (its key, title, description, [`Control`], default and [`Scope`]), so
//! an extension declares settings at run time the way the app does at
//! build time; the [`Registry`] holds every section with its [`Owner`]
//! (an extension's go when it does); [`Values`] keeps what is set,
//! checked against each setting, and puts defaults back. Feature `store`
//! keeps the device's values on disk; `ui` draws the page.

mod model;
mod registry;
#[cfg(feature = "store")]
mod store;
#[cfg(feature = "ui")]
mod ui;
mod values;

pub use model::{Block, Choice, Control, Owner, Scope, Section, Setting, Value};
pub use registry::{Hit, Registry, RegistryError};
#[cfg(feature = "store")]
pub use store::DeviceValues;
#[cfg(feature = "ui")]
pub use ui::{
    SettingsHandle, SettingsPage, try_use_settings, use_section, use_setting, use_settings_provider,
};
pub use values::{ValueError, Values};
