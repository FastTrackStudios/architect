//! Runtime themes (`docs/content/architecture/theming.md`), modelled on
//! herdr's: a theme is a small [`Palette`] plus overrides; [`resolve`]
//! derives every semantic token architect-ui, the shell and the view
//! kinds read (mixing in `OKLab`, branching on light and dark, fixing the
//! ink tokens' contrast) into one flat [`Resolved`] map; a [`ThemeStore`]
//! composes the [`Layers`] of the cascade (built in, app, org, person,
//! device, workspace, pick, preview) and tells its subscribers once per
//! real change.
//!
//! Consumers read a typed slice ([`FromResolved`]): `ShellTheme`
//! (`architect-shell`), `ThemeStyle` (`architect-ui`'s CSS variables),
//! `ThemeTokens` (a view kind's). A slice kept in [`Projected`] says
//! whether it changed, so a view redraws only for its own part.
//!
//! The default build is the model: wasm-clean, no I/O, no Dioxus.
//! Feature `fs` reads theme files (themes folders, extensions'), watches
//! them and installs an app's shipped ones; `actions` puts the `THEME_*`
//! actions on an Action List; `dioxus` gives the store to a Dioxus app.

// A write answers whether it changed anything: a courtesy most callers
// ignore, so not `#[must_use]`.
#![allow(clippy::must_use_candidate)]

pub mod builtin;
pub mod color;
pub mod expr;
mod model;
pub mod resolve;
mod store;

#[cfg(feature = "actions")]
pub mod actions;
#[cfg(feature = "dioxus")]
mod dioxus;
#[cfg(all(feature = "fs", not(target_arch = "wasm32")))]
pub mod fs;

pub use color::Rgba;
pub use model::{
    Appearance, Palette, Problem, Registry, Scheme, Source, TOKEN_NAME_RULE, Theme, ThemeChoice,
    ThemeFile, valid_id, valid_token,
};
pub use resolve::{
    FromResolved, Origin, Projected, Resolved, TokenSource, css_name, derive, names, resolve,
};
pub use store::{
    Composed, Contribution, Layer, LayerKind, Layers, Reloader, Subscription, ThemeInfo, ThemeStore,
};

#[cfg(feature = "dioxus")]
pub use dioxus::{
    RootCss, ThemeHandle, try_use_root_css, try_use_theme, use_resolved, use_theme,
    use_theme_store, use_themed,
};
