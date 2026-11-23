//! A `PaintView` in an extension's own view, linked in or as a process.

use dioxus::prelude::*;

/// A view's [`PaintView`](architect_workspace::paint::PaintView) in its
/// own Dioxus view, the same code linked in or as a process: linked, it
/// is `architect_workspace::PaintPane`; in a process, the host composes
/// what it draws (`scene::RemotePaint`), with the same `ViewCtx`
/// (the theme, `mount`'s extras and its `PaintWake`) and the same input.
///
/// Mount it among the view's own elements, as `PaintPane` is: it fills
/// its parent.
#[component]
pub fn PaintPane(mount: architect_workspace::paint::PaintMount) -> Element {
    #[cfg(feature = "guest")]
    if architect_extensions::linked::in_process() {
        return rsx! { crate::scene::RemotePaint { mount } };
    }
    rsx! { architect_workspace::paint::PaintPane { mount } }
}
