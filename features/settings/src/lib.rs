//! Settings (`docs/content/architecture/settings-page.md`): declared
//! once — by the app, or by an extension while it is loaded — stored,
//! validated, searched, and drawn as T3 Code's settings page.
//!
//! The default build is the model: a [`Section`] of [`Setting`]s is data
//! (its key, title, description, [`Control`], default and [`Scope`]), so
//! an extension declares settings at run time the way the app does at
//! build time; the [`Registry`] holds every section with its [`Owner`]
//! (an extension's go when it does); [`Values`] keeps what is set,
//! checked against each setting, and puts defaults back; [`ScopedValues`]
//! holds them by [`Scope`] (this device, the person's, the project's)
//! over an organization's defaults and locks ([`OrgSettings`]).
//!
//! `#[derive(Settings)]` declares a section from a facet struct
//! ([`Settings`], `derive`): the doc comments are the words, the types
//! the controls, `Default` the defaults; `T::read` and `entries` are
//! the typed value.
//!
//! Feature `appearance` keeps a theme store's org, person and device
//! layers as settings, and declares the Appearance section (`appearance`;
//! with `ui`, its page: `use_appearance_section`).
//!
//! Feature `store` keeps the device's and a project's values on disk;
//! `synced` keeps the person's in a CRDT document every device syncs;
//! `ui` draws the page and gives `use_settings::<T>()`.

#[cfg(feature = "appearance")]
pub mod appearance;
#[cfg(all(feature = "appearance", feature = "ui"))]
mod appearance_ui;
pub mod derive;
mod model;
mod registry;
#[cfg(feature = "store")]
mod store;
#[cfg(feature = "synced")]
mod synced;
#[cfg(feature = "ui")]
mod ui;
mod values;

pub use model::{Block, Choice, Control, Owner, Scope, Section, Setting, Value};
pub use registry::{Hit, Registry, RegistryError};
#[cfg(feature = "store")]
pub use store::{DeviceValues, ProjectValues};
#[cfg(feature = "synced")]
pub use synced::{DocValues, ROOT as SYNCED_ROOT, synced_doc_id};
#[cfg(all(feature = "appearance", feature = "ui"))]
pub use appearance_ui::{AppearancePage, use_appearance_section};
#[cfg(feature = "ui")]
pub use ui::{
    SettingsChange, SettingsHandle, SettingsPage, UseSettings, try_use_settings, use_section,
    use_section_page, use_setting, use_settings, use_settings_provider,
};
pub use values::{OrgSettings, ScopedValues, ValueError, Values};

pub use architect_settings_derive::Settings;
pub use derive::Settings;
