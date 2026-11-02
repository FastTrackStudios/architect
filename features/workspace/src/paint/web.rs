//! A host-driven WebGL2 canvas for a [`PaintView`](super::PaintView).
//!
//! The embedding app owns browser events and animation-frame scheduling. It
//! forwards pane-local logical input, calls `draw` on an animation frame, and
//! schedules another only while `wants_frame` is true or state changes.

use anyrender_vello_hybrid::{WebGlImageManager, WebGlScenePainter};
use peniko::{
    Color,
    kurbo::{Affine, Point},
};
use rustc_hash::FxHashMap;
use vello_common::paint::ImageId;
use vello_gpu::{RenderSize, Resources, Scene, WebGlRenderer, WebGlTextureBindings};
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext};

use super::{FrameStats, PaintScene, Painter};
use crate::{Geometry, ThemeTokens, ViewInput};

/// A canvas initialization, sizing, or WebGL rendering failure.
#[derive(Debug, thiserror::Error)]
pub enum WebPaintError {
    /// Logical size and scale must produce nonzero dimensions within 65535 pixels.
    #[error("paint canvas dimensions must be finite, positive and at most 65535 physical pixels")]
    InvalidGeometry,
    /// WebGL2 initialization or drawing failed.
    #[error("WebGL2 paint: {0}")]
    WebGl(String),
}

/// WebGL2 renderer for one canvas and one existing paint view.
///
/// This does not install DOM listeners or a frame loop: the host must wake its
/// own `requestAnimationFrame` after external changes, input, or resizing.
/// The surface owns the canvas's width and height attributes; size it with
/// [`Self::resize`] and set its CSS size separately.
pub struct WebPaintSurface {
    canvas: HtmlCanvasElement,
    renderer: WebGlRenderer,
    gl: WebGl2RenderingContext,
    resources: Resources,
    scene: Scene,
    images: FxHashMap<u64, ImageId>,
    painter: Painter,
    geometry: Geometry,
    size: RenderSize,
}

impl WebPaintSurface {
    /// Attach `painter` to a canvas at `geometry` in logical pixels.
    ///
    /// # Errors
    /// Returns an error for invalid geometry or if WebGL2 cannot initialize.
    pub fn new(
        canvas: HtmlCanvasElement,
        painter: Painter,
        geometry: Geometry,
    ) -> Result<Self, WebPaintError> {
        let size = physical_size(geometry)?;
        canvas.set_width(u32::from(size.width));
        canvas.set_height(u32::from(size.height));
        let (renderer, resources) =
            WebGlRenderer::new(&canvas).map_err(|error| WebPaintError::WebGl(error.to_string()))?;
        let gl = canvas
            .get_context("webgl2")
            .map_err(|error| WebPaintError::WebGl(format!("{error:?}")))?
            .ok_or_else(|| WebPaintError::WebGl("WebGL2 context is unavailable".to_owned()))?
            .dyn_into::<WebGl2RenderingContext>()
            .map_err(|_| WebPaintError::WebGl("canvas context is not WebGL2".to_owned()))?;
        Ok(Self {
            canvas,
            renderer,
            gl,
            resources,
            scene: Scene::new(size.width, size.height),
            images: FxHashMap::default(),
            painter,
            geometry,
            size,
        })
    }

    /// Change the logical size or device scale and request a frame.
    ///
    /// # Errors
    /// Returns an error if the resulting physical dimensions are invalid.
    pub fn resize(&mut self, geometry: Geometry) -> Result<(), WebPaintError> {
        let size = physical_size(geometry)?;
        if self.geometry == geometry {
            return Ok(());
        }
        if self.size != size {
            self.canvas.set_width(u32::from(size.width));
            self.canvas.set_height(u32::from(size.height));
            self.scene = Scene::new(size.width, size.height);
            self.size = size;
        }
        self.geometry = geometry;
        self.painter.invalidate();
        Ok(())
    }

    /// Queue input in pane-local logical pixels. Schedule a browser frame.
    pub fn input(&self, event: ViewInput) {
        self.painter.ctx().input.push(event);
    }

    /// Update theme tokens. Schedule a browser frame if they changed.
    pub fn set_theme(&mut self, theme: ThemeTokens) {
        self.painter.set_theme(theme);
    }

    /// Mark externally changed data dirty. Schedule a browser frame.
    pub fn invalidate(&mut self) {
        self.painter.invalidate();
    }

    /// Whether input, external changes or animation require a frame.
    #[must_use]
    pub fn wants_frame(&self) -> bool {
        self.painter.wants_frame()
    }

    /// The topmost view-defined hit at a pane-local logical point.
    #[must_use]
    pub fn hit(&self, at: Point) -> Option<u64> {
        self.painter.hit(at)
    }

    /// The painter's frame-recording timings (not GPU timing).
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.painter.stats()
    }

    /// Render a frame if needed; `true` if a frame was rendered.
    ///
    /// # Errors
    /// Returns a WebGL error and leaves the view dirty for a retry.
    pub fn draw(&mut self) -> Result<bool, WebPaintError> {
        if !self.wants_frame() {
            return Ok(false);
        }
        if self.gl.is_context_lost()
            || self.gl.drawing_buffer_width() != i32::from(self.size.width)
            || self.gl.drawing_buffer_height() != i32::from(self.size.height)
        {
            return Err(WebPaintError::WebGl(
                "WebGL2 drawing buffer is unavailable or smaller than requested".to_owned(),
            ));
        }
        let recorded = self.painter.frame(self.geometry);
        {
            let images =
                WebGlImageManager::new(&mut self.renderer, &mut self.resources, &mut self.images);
            let mut painter = WebGlScenePainter::new(&mut self.scene, images);
            painter.append_scene(recorded, Affine::IDENTITY);
        }
        let result = self.renderer.render(
            &self.scene,
            &mut self.resources,
            &self.size,
            &WebGlTextureBindings::new(),
            Color::TRANSPARENT,
        );
        self.scene.reset();
        if let Err(error) = result {
            self.painter.invalidate();
            return Err(WebPaintError::WebGl(error.to_string()));
        }
        Ok(true)
    }
}

fn physical_size(geometry: Geometry) -> Result<RenderSize, WebPaintError> {
    let (width, height) =
        super::web_geometry::physical_size(geometry).ok_or(WebPaintError::InvalidGeometry)?;
    Ok(RenderSize { width, height })
}
