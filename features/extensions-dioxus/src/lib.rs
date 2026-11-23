//! The Dioxus side of Architect extensions.
//!
//! `architect-extensions` is the runtime: manifests, the host, linked,
//! process, Luau, WASI and data extensions, with views kept type-erased
//! (`linked::ViewRoot`). This crate is what an extension whose views are
//! Dioxus components links:
//!
//! - [`deferred`]: a linked extension linked by its manifest alone, its
//!   code built (in a browser, fetched) the first time it is used;
//! - [`EmbeddedView`]: another extension's view held in this one.
//!
//! The host's chrome (panes, toolbars, the extensions page) is not here:
//! it is the shell's (app-shell's `app-shell-extensions`).

pub mod deferred;
mod embedded;

pub use embedded::EmbeddedView;
