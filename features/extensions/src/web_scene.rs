//! A remote view's scenes in a browser (#175; `crate::scene`): the same
//! two attributes `RemoteDocument` draws as a Blitz widget, drawn by
//! [`crate::web::Pane`] into a `<canvas>` inside the element with the
//! page's Canvas 2D.
//!
//! Each command is replayed at the element's transform and the device's
//! pixel ratio: fills and strokes as `Path2D`s from their SVG path, clips
//! and layers as the context's save, clip and restore, a layer's alpha as
//! `globalAlpha`, a box shadow as a blurred rectangle. A gradient is drawn
//! in its first stop's colour (WebGL2 through `vello_hybrid`,
//! `architect_workspace::paint::web`, is the follow-up for full fidelity).
//! When the canvas is laid out at a new size the view is sent
//! [`crate::scene::RESIZE_EVENT`].

// kurbo's transforms compose with `*`: no overflow to guard.
#![allow(clippy::arithmetic_side_effects)]

use std::cell::RefCell;

use peniko::kurbo::{Affine, Shape as _};
use peniko::{Color, Fill};
use wasm_bindgen::JsCast as _;

use crate::scene::{Commands, RESIZE_EVENT, RenderCommand, SceneCache, parse_transform};
use crate::wire::{EventData, ViewEvent};

/// One painted element: its canvas, its scene, its transform.
struct Painted {
    element: web_sys::Element,
    id: u64,
    canvas: web_sys::HtmlCanvasElement,
    commands: Commands,
    transform: Affine,
    size: (u32, u32),
}

/// A pane's scenes: what it was sent, and the elements it paints.
#[derive(Default)]
pub(crate) struct Scenes {
    cache: RefCell<SceneCache>,
    painted: RefCell<Vec<Painted>>,
}

impl Scenes {
    /// `element` (view element `id`) had its scene or transform set.
    pub(crate) fn set(
        &self,
        element: &web_sys::Element,
        id: u64,
        name: &str,
        value: Option<&str>,
        on_event: &dyn Fn(ViewEvent),
    ) -> Result<(), String> {
        let mut painted = self.painted.borrow_mut();
        let at = painted
            .iter()
            .position(|p| p.element.is_same_node(Some(element)));
        let index = match at {
            Some(i) => i,
            None => {
                let document = element.owner_document().ok_or("no document")?;
                let canvas: web_sys::HtmlCanvasElement = document
                    .create_element("canvas")
                    .map_err(|e| format!("{e:?}"))?
                    .dyn_into()
                    .map_err(|_| "not a canvas")?;
                let _ = canvas.set_attribute(
                    "style",
                    "display: block; width: 100%; height: 100%; pointer-events: none;",
                );
                element
                    .append_child(&canvas)
                    .map_err(|e| format!("{e:?}"))?;
                painted.push(Painted {
                    element: element.clone(),
                    id,
                    canvas,
                    commands: Commands::default(),
                    transform: Affine::IDENTITY,
                    size: (0, 0),
                });
                painted.len() - 1
            }
        };
        let p = painted.get_mut(index).ok_or("no painted element")?;
        if name == crate::scene::SCENE_ATTR {
            match value {
                Some(v) => p.commands = self.cache.borrow_mut().receive(v)?,
                None => p.commands = Commands::default(),
            }
        } else {
            p.transform = value.map_or(Affine::IDENTITY, parse_transform);
        }
        p.id = id;
        draw(p, on_event);
        Ok(())
    }

    /// Draws every painted element again (the page was laid out anew).
    pub(crate) fn redraw(&self, on_event: &dyn Fn(ViewEvent)) {
        self.painted
            .borrow_mut()
            .retain(|p| p.element.is_connected());
        for p in self.painted.borrow_mut().iter_mut() {
            draw(p, on_event);
        }
    }
}

fn css(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    format!("rgba({r}, {g}, {b}, {})", f64::from(a) / 255.0)
}

fn brush_css(brush: &anyrender::Paint) -> String {
    match brush {
        anyrender::Paint::Solid(c) => css(*c),
        anyrender::Paint::Gradient(g) => g.stops.first().map_or_else(
            || "transparent".to_owned(),
            |s| css(s.color.to_alpha_color()),
        ),
        _ => "transparent".to_owned(),
    }
}

fn set(cx: &web_sys::CanvasRenderingContext2d, t: Affine) {
    let [a, b, c, d, e, f] = t.as_coeffs();
    let _ = cx.set_transform(a, b, c, d, e, f);
}

fn path(shape: &peniko::kurbo::BezPath) -> Option<web_sys::Path2d> {
    web_sys::Path2d::new_with_path_string(&shape.to_svg()).ok()
}

/// Sizes `p`'s canvas to its box at the device's ratio, tells the view
/// when that changed, and replays its scene.
fn draw(p: &mut Painted, on_event: &dyn Fn(ViewEvent)) {
    let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
    let (w, h) = (p.canvas.client_width(), p.canvas.client_height());
    let (w, h) = (u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0));
    if (w, h) != p.size && w > 0 && h > 0 {
        p.size = (w, h);
        let size = serde_json::json!({ "width": w, "height": h, "scale": ratio });
        on_event(ViewEvent {
            element: p.id,
            name: RESIZE_EVENT.to_owned(),
            bubbles: false,
            data: EventData::Form(size.to_string()),
        });
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    {
        p.canvas.set_width((f64::from(w) * ratio) as u32);
        p.canvas.set_height((f64::from(h) * ratio) as u32);
    }
    let Ok(Some(cx)) = p.canvas.get_context("2d") else {
        return;
    };
    let Ok(cx) = cx.dyn_into::<web_sys::CanvasRenderingContext2d>() else {
        return;
    };
    let base = Affine::scale(ratio) * p.transform;
    for command in p.commands.iter() {
        match command {
            RenderCommand::Fill(f) => {
                set(&cx, base * f.transform);
                cx.set_fill_style_str(&brush_css(&f.brush));
                if let Some(path) = path(&f.shape) {
                    let rule = match f.fill {
                        Fill::NonZero => web_sys::CanvasWindingRule::Nonzero,
                        Fill::EvenOdd => web_sys::CanvasWindingRule::Evenodd,
                    };
                    cx.fill_with_path_2d_and_winding(&path, rule);
                }
            }
            RenderCommand::Stroke(s) => {
                set(&cx, base * s.transform);
                cx.set_stroke_style_str(&brush_css(&s.brush));
                cx.set_line_width(s.style.width);
                if let Some(path) = path(&s.shape) {
                    cx.stroke_with_path(&path);
                }
            }
            RenderCommand::PushClipLayer(c) => {
                cx.save();
                set(&cx, base * c.transform);
                if let Some(path) = path(&c.clip) {
                    cx.clip_with_path_2d(&path);
                }
            }
            RenderCommand::PushLayer(l) => {
                cx.save();
                cx.set_global_alpha(f64::from(l.alpha));
                set(&cx, base * l.transform);
                if let Some(path) = path(&l.clip) {
                    cx.clip_with_path_2d(&path);
                }
            }
            RenderCommand::PopLayer => cx.restore(),
            RenderCommand::BoxShadow(b) => {
                set(&cx, base * b.transform);
                cx.save();
                cx.set_shadow_blur(b.std_dev * 2.0);
                cx.set_shadow_color(&css(b.brush));
                cx.set_fill_style_str(&css(b.brush));
                let r = b.rect;
                if let Some(path) = path(&r.to_rounded_rect(b.radius).to_path(0.1)) {
                    cx.fill_with_path_2d(&path);
                }
                cx.restore();
            }
            RenderCommand::GlyphRun(_) => {}
        }
    }
}
