//! The `web` view kind's hosts (#222;
//! `docs/content/architecture/embedded-views.md`, "Web views").
//!
//! An extension ships a full TypeScript/npm app — its built `dist/` — in
//! its package, and names it in a view:
//!
//! ```toml
//! [[views]]
//! id = "board"
//! title = "Board"
//! kind = "web"
//! src = "dist/index.html"
//! ```
//!
//! The model, the bridge's messages, the package protocol and its policy
//! are `architect_extensions::webview`'s, shared by every host. This crate
//! is the hosts an app links:
//!
//! - **`desk`** ([`desk::use_desk_web_views`]): a wry child web view of
//!   the pane's window (`WebKitGTK` on Linux under X11 or `XWayland`,
//!   `WKWebView` on macOS, `WebView2` on Windows), clipped to the pane.
//! - **a browser**: a sandboxed `<iframe>` on the same messages, from the
//!   npm package (`features/extensions/web/webview-host.js` `mountWebView`),
//!   which the web app mounts once it hosts extensions (#495).
//! - **iOS** puts a `WKWebView` in the pane's `UIView` on the same messages
//!   (the iOS host's, beside `apps/mobile`).
//!
//! An app registers the kind with [`register`] so extensions naming it
//! load, and provides its host near its root.

use architect_workspace::{KindSpec, ViewKinds};

#[cfg(feature = "desk")]
pub mod desk;

/// Adds the `web` kind to `kinds` (an app that hosts it on a desk and in
/// a browser): extensions whose views name it then load. A kind already
/// there is kept.
pub fn register(kinds: &mut ViewKinds) {
    let _ = kinds.register(KindSpec::web());
}
