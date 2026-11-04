//! Pages as editable files (`docs/content/architecture/pages.md`).
//!
//! A page is a view written as a file — TSX or JS on `QuickJS`, Luau
//! elements, or A2UI JSON — that the app re-renders live when it is
//! saved. It never runs on the UI thread: each page has a runtime of its
//! own on a thread of its own, and what it renders reaches the window as
//! the same edit stream a process extension's view sends
//! (`architect_extensions::wire::Edit`), applied to a Blitz sub-document
//! with the host's live theme CSS, events, focus and IME.
//!
//! The default set is the model, pure data:
//!
//! - [`api`]: the page API, defined once — `#[architect::rpc]` services
//!   ([`api::PageHost`], [`api::PageView`]) over facet types, with
//!   `#[subscribe]` streams for theme changes, live data and worker
//!   messages;
//! - [`diff`]: an element tree drawn as edits ([`diff::Mirror`]);
//! - [`files`]: where a view's page file is — shipped by its extension
//!   (`[[views]] page = "pages/x.tsx"`), or an override in the app's
//!   folder (`<app dir>/pages/<view-id>.tsx`) — and what a broken one
//!   leaves (`<file>.error`).
//!
//! Features: `codegen` writes `architect.d.ts` and `architect.d.luau`
//! from the services' descriptors; `js` runs TSX/JS pages on `QuickJS`
//! (oxc transpiles TypeScript and JSX in Rust); `luau` runs Luau element
//! pages; `ui` mounts a page in a pane; `a2ui` draws `.a2ui.json` pages.

pub mod api;
pub mod diff;
pub mod files;

#[cfg(feature = "codegen")]
pub mod codegen;
#[cfg(feature = "runner")]
pub mod host;
#[cfg(feature = "js")]
pub mod js;
#[cfg(feature = "runner")]
pub mod runner;
#[cfg(feature = "js")]
pub mod tsx;
#[cfg(feature = "ui")]
pub mod ui;

pub use api::{Attr, Element, Listener, PageError, PageEvent};
pub use diff::Mirror;
pub use files::{PageFile, PageKind};
