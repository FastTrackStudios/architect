//! Screenshots of Kanban-lite for the showcase (#710): the host's own
//! Blitz document, the board drawn by Kanban-lite's process in a pane
//! filling the window, a card held mid-drag, rendered on the CPU (as `APP_SHOT` does) at 390 px (2×) and
//! 1280 px, in the app's built-in light and dark themes.
//!
//! `SHOTS_DIR=<folder> cargo test -p architect-sandbox-kanban --test shots
//! -- --ignored --nocapture`.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    clippy::print_stdout,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyrender::PaintScene as _;
use architect::action::ActionList;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteView, mounted_views};
use architect_extensions::{Origin, Package};
use architect_sandbox_kanban::ID;
use architect_theme::{Registry, RootCss, Scheme};
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};

#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    architect_sandbox_kanban::guest().serve().unwrap();
}

fn package(dir: &Path) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let manifest = include_str!("../extension.toml").replace(
        "path = \"architect-sandbox-kanban\"",
        "path = \"guest\"\nargs = [\"guest\", \"--exact\", \"--nocapture\", \"--test-threads=1\"]",
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

/// The app's built-in theme for `scheme`, as the `:root` sheet.
fn root_css(scheme: Scheme) -> String {
    let theme = architect_theme::builtin::fallback(scheme);
    let resolved = architect_theme::resolve(&theme, &Registry::builtin());
    let mut vars = String::new();
    for (k, v) in resolved.css_tokens() {
        let _ = write!(vars, "--{k}: {v}; ");
    }
    format!(":root {{ {vars} }}")
}

#[derive(Props, Clone, PartialEq)]
struct AppProps {
    processes: Processes,
    view: String,
    css: String,
}

#[allow(non_snake_case)]
fn App(props: AppProps) -> Element {
    let css = props.css.clone();
    use_context_provider(|| RootCss(Signal::new(css)));
    let sheet = format!(
        "{} html, body {{ margin: 0; background: var(--background); color: var(--foreground); }}",
        props.css
    );
    rsx! {
        style { "{sheet}" }
        div { style: "position: absolute; left: 0; top: 0; right: 0; bottom: 0;",
            RemoteView { processes: props.processes, extension: ID, view: props.view }
        }
    }
}

fn pointer((x, y): (f32, f32), buttons: MouseEventButtons, mods: Modifiers) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons,
        mods,
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: Arc::default(),
    }
}

fn pump(doc: &mut DioxusDocument, rounds: u32) {
    let waker = futures::task::noop_waker();
    for _ in 0..rounds {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        // Painting lays out the widgets: they tell their views their size.
        let mut scene = anyrender::Scene::new();
        blitz_paint::paint_scene(&mut scene, &mut doc.inner.borrow_mut(), 1.0, 10, 10, 0, 0);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn shoot(doc: &DioxusDocument, (w, h, scale): (u32, u32, f64), bg: Color, path: &Path) {
    let (pw, ph) = ((f64::from(w) * scale) as u32, (f64::from(h) * scale) as u32);
    let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                bg,
                None,
                &Rect::new(0.0, 0.0, f64::from(pw), f64::from(ph)),
            );
            blitz_paint::paint_scene(scene, &mut doc.inner.borrow_mut(), scale, pw, ph, 0, 0);
        },
        pw,
        ph,
    );
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, pw, ph);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
    println!("{}", path.display());
}

/// A card picked up in "To do" and held over "Doing".
fn act(doc: &mut DioxusDocument) {
    let none = Modifiers::empty();
    let (from, to) = ((60.0_f32, 45.0_f32), (190.0_f32, 120.0_f32));
    doc.handle_ui_event(UiEvent::PointerMove(pointer(
        from,
        MouseEventButtons::None,
        none,
    )));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        from,
        MouseEventButtons::Primary,
        none,
    )));
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        doc.handle_ui_event(UiEvent::PointerMove(pointer(
            at,
            MouseEventButtons::Primary,
            none,
        )));
        pump(doc, 2);
    }
    pump(doc, 20);
}

#[test]
#[ignore = "writes screenshots: SHOTS_DIR=… --ignored --nocapture"]
fn shots() {
    let Some(out) = std::env::var_os("SHOTS_DIR").map(PathBuf::from) else {
        println!("set SHOTS_DIR");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    for view in ["board"] {
        for (w, h, scale) in [(390_u32, 844_u32, 2.0_f64), (1280, 800, 1.0)] {
            for (scheme, cs, name) in [
                (Scheme::Light, ColorScheme::Light, "light"),
                (Scheme::Dark, ColorScheme::Dark, "dark"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let runner = ProcessRunner::new()
                    .unwrap()
                    .with_socket_dir(dir.path())
                    .with_restart(Restart::never());
                let processes = runner.processes();
                let host =
                    Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
                host.load(&package(dir.path()), Origin::Installed).unwrap();
                let css = root_css(scheme);
                let vdom = VirtualDom::new_with_props(
                    App,
                    AppProps {
                        processes,
                        view: view.to_owned(),
                        css,
                    },
                );
                let mut doc = DioxusDocument::new(
                    vdom,
                    DocumentConfig {
                        viewport: Some(Viewport::new(w, h, 1.0, cs)),
                        ..Default::default()
                    },
                );
                doc.initial_build();
                let deadline = Instant::now() + Duration::from_secs(30);
                while mounted_views(&doc.inner.borrow()).is_empty() {
                    assert!(Instant::now() < deadline, "no view");
                    pump(&mut doc, 1);
                }
                pump(&mut doc, 30);
                act(&mut doc);
                let bg = if name == "dark" {
                    Color::from_rgb8(0x14, 0x14, 0x16)
                } else {
                    Color::WHITE
                };
                shoot(
                    &doc,
                    (w, h, scale),
                    bg,
                    &out.join(format!("kanban-{view}-{w}-{name}.png")),
                );
                drop(host);
            }
        }
    }
}
