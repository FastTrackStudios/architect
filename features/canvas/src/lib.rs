//! Canvases: what an extension or a script draws over a view, and the
//! immediate-mode windows a script draws (`docs/content/architecture/extensions.md`,
//! "Overlays: drawing over another view" and "Immediate-mode UI for
//! scripts").
//!
//! - **Layers over a view.** A view publishes its coordinate spaces
//!   ([`Spaces`]: time across, lanes down, already scrolled); a layer
//!   contributed to that kind of view ([`Layer`], on a [`Plane`]) draws
//!   [`Shape`]s in them, and claims the [`Region`]s that take the pointer
//!   — everywhere else the pointer passes through to the view
//!   ([`Layer::hit`]). [`Overlays`] keeps one owner's layers, frame by
//!   frame.
//! - **Immediate-mode windows.** [`imgui::Ui`] is a UI in `ReaImGui`'s
//!   shape — `Begin`, `Text`, `Button`, `Checkbox`, `SliderDouble`,
//!   `SameLine`, `End`, called every frame — laying each window out into
//!   shapes for the pane it is docked, tabbed or floated in.
//!
//! The default set is that model: pure data, no I/O, wasm-clean. Feature
//! `luau` puts both in front of scripts, as `ImGui` (`ReaImGui`'s names:
//! `local ImGui = require 'imgui' '0.10'`) and `Canvas`, on
//! `architect-scripting`'s runtime: a [`luau::Session`] is one live
//! script, a frame at a time. Feature `ui` draws layers and windows on
//! Blitz ([`ui::LayerView`], [`ui::WindowView`]), a window as a workspace
//! pane ([`ui::window_view`]); with `luau` too, [`ui::use_script_canvas`]
//! drives a session from Dioxus.

pub mod dock;
pub mod draw;
pub mod geometry;
pub mod imgui;
pub mod layer;
pub mod space;

#[cfg(feature = "luau")]
pub mod luau;
#[cfg(feature = "ui")]
pub mod ui;

pub use draw::{DrawList, Shape, text_width};
pub use geometry::{Color, Point, Rect, Size};
pub use layer::{Layer, LayerId, Overlays, Plane, Region, RegionState, ViewPointer};
pub use space::{Lane, Spaces, TimeAxis};

/// What one frame drew: every layer, every window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub layers: Vec<Layer>,
    pub windows: Vec<imgui::WindowFrame>,
}

impl Output {
    /// The layers over `view` on `plane`, in the order drawn.
    pub fn layers_on<'a>(&'a self, view: &'a str, plane: Plane) -> impl Iterator<Item = &'a Layer> {
        self.layers
            .iter()
            .filter(move |l| l.view == view && l.plane == plane)
    }

    /// The window `id`, when it was drawn.
    #[must_use]
    pub fn window(&self, id: &str) -> Option<&imgui::WindowFrame> {
        self.windows.iter().find(|w| w.id == id)
    }
}
