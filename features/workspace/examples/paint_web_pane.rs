//! Dioxus web pane exercise; the companion HTML page drives browser checks.

#[cfg(target_arch = "wasm32")]
mod browser {
    use architect::action::{ActionCaller, ActionList};
    use architect_workspace::paint::web_ui::{WebPaintMount, WebPaintPane, WebPaintWake};
    use architect_workspace::paint::{
        FrameStats, PaintScene, PaintView, Painter, Scene,
        peniko::{
            Color, Fill,
            kurbo::{Affine, Rect},
        },
    };
    use architect_workspace::{
        FrameClock, KindId, PaneId, Placement, ThemeTokens, ViewCtx, ViewHost, ViewInput,
    };
    use dioxus::prelude::*;
    use std::cell::{Cell, RefCell};
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };
    use std::time::Duration;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static MODE: Arc<AtomicU8> = Arc::new(AtomicU8::new(0));
        static WAKE: RefCell<Option<WebPaintWake>> = const { RefCell::new(None) };
        static STATS: RefCell<Option<FrameStats>> = const { RefCell::new(None) };
        static LAST: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    }

    struct View(Arc<AtomicU8>);
    impl PaintView for View {
        fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
            if let ViewInput::PointerDown { x, y, .. } = event {
                LAST.with(|last| last.set((*x, *y)));
                self.0.store(2, Ordering::Release);
                return true;
            }
            false
        }
        fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
            let color = match self.0.load(Ordering::Acquire) {
                1 => Color::from_rgb8(0, 255, 0),
                2 => Color::from_rgb8(255, 255, 0),
                _ => ctx
                    .theme
                    .color("accent")
                    .unwrap_or(Color::from_rgb8(255, 0, 0)),
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

    fn app() -> Element {
        let mut blue = use_signal(|| false);
        let mount = use_hook(|| {
            let stats = FrameStats::new();
            STATS.with(|slot| *slot.borrow_mut() = Some(stats.clone()));
            let mount = WebPaintMount::new(move || {
                let ctx = ViewCtx::new(
                    PaneId::new("web-pane"),
                    KindId::paint(),
                    (ViewHost::Web, Placement::Pane),
                    (ActionList::new(), ActionCaller::app()),
                    FrameClock::new(Duration::from_millis(16), || Duration::ZERO),
                );
                let view = MODE.with(|mode| View(mode.clone()));
                Painter::new(ctx, view).with_stats(stats.clone())
            });
            WAKE.with(|slot| *slot.borrow_mut() = Some(mount.wake_handle()));
            mount
        });
        let theme =
            ThemeTokens::default().with("accent", if blue() { "#0000ff" } else { "#ff0000" });
        rsx! {
            button { id: "theme", onclick: move |_| blue.toggle(), "Toggle theme" }
            div { id: "area", style: "width:160px;height:100px;", WebPaintPane { mount, theme } }
        }
    }

    #[wasm_bindgen(start)]
    pub fn start() {
        dioxus::web::launch::launch_cfg(app, Default::default());
    }
    #[wasm_bindgen]
    pub fn external_update() {
        MODE.with(|mode| mode.store(1, Ordering::Release));
        WAKE.with(|wake| {
            if let Some(wake) = wake.borrow().as_ref() {
                wake.wake();
                wake.wake();
                wake.wake();
            }
        });
    }
    #[wasm_bindgen]
    pub fn frames() -> u32 {
        STATS.with(|stats| {
            stats.borrow().as_ref().map_or(0, |stats| {
                u32::try_from(stats.summary().frames).unwrap_or(u32::MAX)
            })
        })
    }
    #[wasm_bindgen]
    pub fn last_x() -> f32 {
        LAST.with(|last| last.get().0)
    }
    #[wasm_bindgen]
    pub fn last_y() -> f32 {
        LAST.with(|last| last.get().1)
    }
}

fn main() {}
