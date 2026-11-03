//! Exercise an external wake through a mounted Blitz document and its painter.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyrender::{PaintScene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, ShellProvider, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use peniko::{
    Color, Fill,
    kurbo::{Affine, Rect},
};

use super::{PaintMount, PaintPane, PaintView, PaintWake, Scene, ViewCtx, ViewInput};

thread_local! {
    static WAKE: RefCell<Option<PaintWake>> = const { RefCell::new(None) };
    static BLUE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    static THEME: RefCell<Option<Signal<architect_shell::ShellTheme>>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct Window(AtomicUsize);

impl ShellProvider for Window {
    fn request_redraw(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

struct Solid(Arc<AtomicBool>);

impl PaintView for Solid {
    fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
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

#[component]
fn SolidPane() -> Element {
    let mount = use_hook(|| {
        let blue = BLUE.with(Arc::clone);
        let mount = PaintMount::new(move |_| Solid(blue.clone()));
        WAKE.with(|slot| *slot.borrow_mut() = Some(mount.wake_handle()));
        mount
    });
    rsx! { PaintPane { mount } }
}

fn app() -> Element {
    let workspace = use_signal(|| {
        let mut workspace = crate::Workspace::new();
        workspace
            .open(
                crate::Pane::new("solid", "solid-test", "Solid"),
                crate::Target::Tab,
            )
            .unwrap();
        workspace
    });
    let views = use_hook(|| {
        crate::Views::new().view(crate::View::new(
            "solid-test",
            "Solid",
            |_| rsx! { SolidPane {} },
        ))
    });
    rsx! { div { style: "width: 400px; height: 300px;", crate::WorkspaceView { workspace, views } } }
}

fn paint(doc: &DioxusDocument, scale: u32) -> Vec<u8> {
    render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            blitz_paint::paint_scene(
                scene,
                &mut doc.inner.borrow_mut(),
                f64::from(scale),
                400 * scale,
                300 * scale,
                0,
                0,
            )
        },
        400 * scale,
        300 * scale,
    )
}

fn has_solid_color(pixels: &[u8], color: [u8; 4]) -> bool {
    pixels
        .chunks_exact(4)
        .filter(|pixel| *pixel == color)
        .count()
        > 1_000
}

#[test]
fn external_wake_changes_pixels_in_a_blitz_document() {
    let one = wake_at_scale(1);
    let two = wake_at_scale(2);
    assert!(
        two >= one * 3 && two <= one * 5,
        "a 2x scale should cover about four times as many pixels"
    );
}

fn wake_at_scale(scale: u32) -> usize {
    BLUE.with(|color| color.store(false, Ordering::Release));
    let window = Arc::new(Window::default());
    let shell: Arc<dyn ShellProvider> = window.clone();
    let vdom = VirtualDom::new(app).with_root_context(shell.clone());
    let config = DocumentConfig {
        viewport: Some(Viewport::new(
            400 * scale,
            300 * scale,
            scale as f32,
            ColorScheme::Dark,
        )),
        shell_provider: Some(shell),
        ..Default::default()
    };
    let mut doc = DioxusDocument::new(vdom, config);
    doc.initial_build();
    for _ in 0..16 {
        if !doc.poll(None) {
            break;
        }
    }
    doc.inner.borrow_mut().resolve(0.0);
    let before = paint(&doc, scale);
    assert!(has_solid_color(&before, [255, 0, 0, 255]));
    assert!(!doc.inner.borrow().widgets_need_redraw());
    let requested = window.0.load(Ordering::Relaxed);
    BLUE.with(|color| color.store(true, Ordering::Release));
    WAKE.with(|wake| wake.borrow().as_ref().unwrap().wake());
    assert!(window.0.load(Ordering::Relaxed) > requested);
    assert!(doc.inner.borrow().widgets_need_redraw());
    let after = paint(&doc, scale);
    assert!(has_solid_color(&after, [0, 0, 255, 255]));
    assert!(!doc.inner.borrow().widgets_need_redraw());
    before
        .chunks_exact(4)
        .filter(|pixel| *pixel == [255, 0, 0, 255])
        .count()
}

struct ThemedSolid;

impl PaintView for ThemedSolid {
    fn input(&mut self, _: &ViewCtx, _: &ViewInput) -> bool {
        false
    }

    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
        let color = ctx.theme.color("accent").unwrap();
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

#[component]
fn ThemedPane() -> Element {
    let mount = use_hook(|| PaintMount::new(|_| ThemedSolid));
    rsx! { PaintPane { mount } }
}

fn themed_app() -> Element {
    let theme = use_signal(|| architect_shell::ShellTheme {
        accent: "#ff0000",
        ..Default::default()
    });
    THEME.with(|slot| *slot.borrow_mut() = Some(theme));
    let workspace = use_signal(|| {
        let mut workspace = crate::Workspace::new();
        workspace
            .open(
                crate::Pane::new("themed", "themed-test", "Themed"),
                crate::Target::Tab,
            )
            .unwrap();
        workspace
    });
    let views = use_hook(|| {
        crate::Views::new().view(crate::View::new(
            "themed-test",
            "Themed",
            |_| rsx! { ThemedPane {} },
        ))
    });
    let screen = architect_shell::Screen {
        width: 1000.0,
        height: 800.0,
        touch: false,
        insets: Default::default(),
    };
    rsx! {
        div { style: "width: 400px; height: 300px; position: relative;",
            architect_shell::AppShell { screen, theme: theme(), tabs: vec![], current: None, on_tab: |_| (),
                crate::WorkspaceView { workspace, views }
            }
        }
    }
}

#[test]
fn shell_theme_change_repaints_a_blitz_document() {
    let window = Arc::new(Window::default());
    let shell: Arc<dyn ShellProvider> = window;
    let vdom = VirtualDom::new(themed_app).with_root_context(shell.clone());
    let config = DocumentConfig {
        viewport: Some(Viewport::new(400, 300, 1.0, ColorScheme::Dark)),
        shell_provider: Some(shell),
        ..Default::default()
    };
    let mut doc = DioxusDocument::new(vdom, config);
    doc.initial_build();
    for _ in 0..16 {
        if !doc.poll(None) {
            break;
        }
    }
    doc.inner.borrow_mut().resolve(0.0);
    assert!(has_solid_color(&paint(&doc, 1), [255, 0, 0, 255]));
    doc.vdom.in_runtime(|| {
        THEME.with(|slot| {
            let mut theme = slot.borrow().unwrap();
            theme.write().accent = "#0000ff";
        })
    });
    for _ in 0..16 {
        if !doc.poll(None) {
            break;
        }
    }
    doc.inner.borrow_mut().resolve(0.1);
    assert!(doc.inner.borrow().widgets_need_redraw());
    assert!(has_solid_color(&paint(&doc, 1), [0, 0, 255, 255]));
    assert!(!doc.inner.borrow().widgets_need_redraw());
}
