//! Views as Dioxus components on a linked extension's
//! [`Builder`](architect_extensions::linked::Builder), which keeps them
//! type-erased.

use std::sync::Arc;

use architect_extensions::linked::Builder;
use dioxus::prelude::Element;

/// What a Dioxus extension adds to its [`Builder`].
pub trait BuilderExt {
    /// Its view `id` (one of its manifest's `[[views]]`), drawn by `root`.
    fn view(&mut self, id: impl Into<String>, root: fn() -> Element) -> &mut Self;

    /// The code that draws its base view type `id` (one of its manifest's
    /// `[[base_views]]` that names no `view` or `page`): drawn as view
    /// `base.<id>`, given the base's rows (`linked::use_base_view`), linked
    /// in or as a process.
    fn base_view(&mut self, id: &str, root: fn() -> Element) -> &mut Self;

    /// A value its views read with `use_context`, provided at the app's
    /// root when it is linked in. It stays until the app ends.
    fn context<T: Clone + 'static>(&mut self, make: fn() -> T) -> &mut Self;

    /// Its view `id`'s component.
    #[must_use]
    fn view_root(&self, id: &str) -> Option<fn() -> Element>;

    /// Its paint view `id` (one of its manifest's `[[views]]`, `kind =
    /// "paint"`), drawn by a `V` made for each pane it opens in (feature
    /// `paint`, #591).
    ///
    /// `V` records an anyrender scene for the pane's size and scale
    /// (`architect_scene::paint::PaintView::paint`) and is given the
    /// pane's input in its own logical pixels: pointer, wheel, touch,
    /// keys and composed text (`PaintView::input`). It is hosted as a
    /// Blitz widget in the pane (`architect_scene::paint::PaintPane`).
    /// A frame is drawn after input that changed something, every frame
    /// while it says it is `animating`, and when it asks: its
    /// `ctx.extras` holds the pane's `PaintWake`.
    #[cfg(feature = "paint")]
    fn paint<V>(&mut self, id: impl Into<String>) -> &mut Self
    where
        V: architect_scene::paint::PaintView + Default;
}

impl BuilderExt for Builder {
    fn view(&mut self, id: impl Into<String>, root: fn() -> Element) -> &mut Self {
        self.view_any(id, Arc::new(root))
    }

    fn base_view(&mut self, id: &str, root: fn() -> Element) -> &mut Self {
        self.view_any(
            format!("{}{id}", architect_extensions::contrib::BASE_VIEW_PREFIX),
            Arc::new(root),
        )
    }

    fn context<T: Clone + 'static>(&mut self, make: fn() -> T) -> &mut Self {
        self.provide_with(Arc::new(move || {
            dioxus::prelude::provide_context(make());
        }))
    }

    fn view_root(&self, id: &str) -> Option<fn() -> Element> {
        self.view_root_any(id)
            .and_then(|root| root.downcast_ref::<fn() -> Element>().copied())
    }

    #[cfg(feature = "paint")]
    fn paint<V>(&mut self, id: impl Into<String>) -> &mut Self
    where
        V: architect_scene::paint::PaintView + Default,
    {
        let id = id.into();
        #[cfg(feature = "guest")]
        self.remote_view_any(
            id.clone(),
            Arc::new(crate::scene::remote_paint_root::<V> as fn() -> Element),
        );
        self.view_any(id, Arc::new(paint_root::<V> as fn() -> Element))
    }
}

#[cfg(feature = "paint")]
/// A paint view's pane: a `V`, hosted as a Blitz widget, its wake in
/// its `ctx.extras`.
fn paint_root<V>() -> Element
where
    V: architect_scene::paint::PaintView + Default,
{
    use architect_scene::paint::{PaintMount, PaintPane};
    use dioxus::prelude::*;
    let mount = use_hook(|| {
        let mount = PaintMount::new(|_| V::default());
        let wake = mount.wake_handle();
        mount.extras(architect_scene::view::Extras::default().with(wake))
    });
    rsx! { PaintPane { mount } }
}
