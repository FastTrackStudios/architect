//! The Dioxus side of Architect extensions.
//!
//! `architect-extensions` is the runtime: manifests, the host, linked,
//! process, Luau, WASI and data extensions, with views kept type-erased
//! (`linked::ViewRoot`). This crate is what an extension whose views are
//! Dioxus components links:
//!
//! - [`BuilderExt`]: what a linked extension's `Builder` takes that is
//!   Dioxus's (a paint view, feature `paint`);
//! - [`guest`] (feature `guest`): a process extension's views, each in a
//!   `VirtualDom` of its own, served to its host; [`scene`] the scenes
//!   they paint;
//! - [`deferred`]: a linked extension linked by its manifest alone, its
//!   code built (in a browser, fetched) the first time it is used;
//! - [`EmbeddedView`]: another extension's view held in this one;
//! - [`pane`]: the pane a view is drawn in, and where it is in it;
//! - [`overlay::spaces`]: an overlay view's spaces;
//! - [`use_work`] (feature `guest`): a process view's heavy work, off its
//!   thread.
//!
//! The host's chrome (panes, toolbars, the extensions page) is not here:
//! it is the shell's (app-shell's `app-shell-extensions`).

mod builder;
pub mod deferred;
mod embedded;
#[cfg(feature = "guest")]
pub mod guest;
pub mod overlay;
#[cfg(feature = "paint")]
mod paint_pane;
pub mod pane;
#[cfg(feature = "guest")]
pub mod scene;
#[cfg(feature = "guest")]
mod work;

pub use builder::BuilderExt;
pub use embedded::EmbeddedView;
#[cfg(feature = "paint")]
pub use paint_pane::{PaintPane, PaintPaneProps};
#[cfg(feature = "guest")]
pub use work::{UseWork, use_work};
