//! Minimal host-driven WebGL2 paint view for the browser smoke page.

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::{Cell, RefCell};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use architect::action::{ActionCaller, ActionList};
    use architect_workspace::paint::web::WebPaintSurface;
    use architect_workspace::paint::{
        PaintScene, PaintView, Painter, Scene,
        peniko::{
            Color, Fill,
            kurbo::{Affine, Rect},
        },
    };
    use architect_workspace::{
        FrameClock, Geometry, KindId, PaneId, Placement, ViewCtx, ViewHost, ViewInput,
    };
    use wasm_bindgen::prelude::*;
    use web_sys::HtmlCanvasElement;

    thread_local! {
        static SURFACE: RefCell<Option<WebPaintSurface>> = const { RefCell::new(None) };
        static BLUE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        static LAST: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    }

    struct View(Arc<AtomicBool>);
    impl PaintView for View {
        fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
            if let ViewInput::PointerDown { x, y, .. } = event {
                LAST.with(|last| last.set((*x, *y)));
                self.0.store(true, Ordering::Release);
                return true;
            }
            false
        }
        fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
            let color = if self.0.load(Ordering::Acquire) {
                Color::from_rgb8(0, 0, 255)
            } else {
                Color::from_rgb8(255, 0, 0)
            };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color,
                None,
                &Rect::new(
                    0.0,
                    0.0,
                    f64::from(ctx.geometry.width),
                    f64::from(ctx.geometry.height),
                ),
            );
        }
    }

    #[wasm_bindgen]
    pub fn mount(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
        BLUE.with(|blue| blue.store(false, Ordering::Release));
        let ctx = ViewCtx::new(
            PaneId::new("web-demo"),
            KindId::paint(),
            (ViewHost::Web, Placement::Pane),
            (ActionList::new(), ActionCaller::app()),
            FrameClock::new(Duration::from_millis(16), || Duration::ZERO),
        );
        let view = BLUE.with(|blue| View(blue.clone()));
        let geometry = Geometry {
            width: 160.0,
            height: 100.0,
            scale: 1.0,
        };
        let surface = WebPaintSurface::new(canvas, Painter::new(ctx, view), geometry)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        SURFACE.with(|slot| *slot.borrow_mut() = Some(surface));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn draw() -> Result<bool, JsValue> {
        SURFACE.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .ok_or_else(|| JsValue::from_str("paint view is not mounted"))?
                .draw()
                .map_err(|error| JsValue::from_str(&error.to_string()))
        })
    }

    #[wasm_bindgen]
    pub fn press(x: f32, y: f32) {
        SURFACE.with(|slot| {
            if let Some(surface) = slot.borrow().as_ref() {
                surface.input(ViewInput::PointerDown {
                    x,
                    y,
                    button: architect_workspace::Button::Primary,
                });
            }
        });
    }

    #[wasm_bindgen]
    pub fn last_x() -> f32 {
        LAST.with(|last| last.get().0)
    }

    #[wasm_bindgen]
    pub fn last_y() -> f32 {
        LAST.with(|last| last.get().1)
    }

    #[wasm_bindgen]
    pub fn resize(width: f32, height: f32, scale: f32) -> Result<(), JsValue> {
        SURFACE.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .ok_or_else(|| JsValue::from_str("paint view is not mounted"))?
                .resize(Geometry {
                    width,
                    height,
                    scale,
                })
                .map_err(|error| JsValue::from_str(&error.to_string()))
        })
    }
}

fn main() {}
