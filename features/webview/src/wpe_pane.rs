//! A `web` view drawn by WPE WebKit as a texture in its pane (feature `wpe`,
//! Linux): the native-Wayland desk host.
//!
//! [`WpeWidget`] is a Blitz custom widget whose picture is the page's last
//! frame: a shared-memory frame is uploaded to a BGRA texture, and one pass
//! (`wgpu_context::AlphaConvertBlitter`) turns it into the straight-alpha
//! RGBA texture the window's renderer composites (`Widget::composite_texture`),
//! so ⌘K, menus and popovers draw over it like over any element: nothing to
//! hide, no snapshot. Each frame is reported shown once drawn, which paces
//! WebKit to the window's own frames.
//!
//! [`WpePane`] is the pane: it makes the view, iterates GLib for it, passes
//! its messages to the same `architect_extensions::webview::Bridge` as the
//! wry host (the extension cannot tell them apart), and gives it the theme,
//! the focus, the cursor and the keys. Typed keys stay in the page; an
//! Escape it did not take gives the focus back to the app.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyrender::{PaintScene as _, RenderContext, ResourceId, Scene};
use architect_extensions::interface::WebAsk;
use architect_extensions::webview::{self, Bridge, Outbound};
use architect_workspace::use_pane;
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{BlitzImeEvent, BlitzPointerId, BlitzWheelDelta, KeyState, Modifiers, MouseEventButton, UiEvent};
use blitz_traits::shell::ShellProvider;
use dioxus::prelude::*;
use dioxus_native_dom::CustomWidgetAttr;
use wgpu_context::{AlphaConversion, AlphaConvertBlitter, DeviceHandle};

use crate::desk::HostRef;
use crate::wpe::{Answer, Frame, FramePixels, Mods, PointerKind, Renderer, WpeWeb};

/// How often the pane iterates WPE's main context and reads its messages.
const TICK: Duration = Duration::from_millis(4);

/// How long a view whose pane went away is kept (a tab switch).
const PARKED: Duration = Duration::from_secs(60);

/// What the widget and the pane share.
pub struct Shared {
    web: WpeWeb,
    /// The newest frame, not drawn yet.
    frame: RefCell<Option<Frame>>,
    /// The size the page is laid out at, logical pixels.
    size: Cell<(u32, u32)>,
    shell: RefCell<Option<Arc<dyn ShellProvider>>>,
}

thread_local! {
    static VIEWS: RefCell<HashMap<String, (Rc<Shared>, Rc<RefCell<Option<Bridge>>>, Option<Instant>)>> =
        RefCell::new(HashMap::new());
}

/// The GPU side of a widget.
struct Gpu {
    handle: DeviceHandle,
    blitter: AlphaConvertBlitter,
}

/// The textures one frame size takes.
struct Textures {
    size: (u32, u32),
    /// BGRA, as WPE renders (premultiplied).
    source: wgpu::Texture,
    /// RGBA, straight alpha: what the renderer composites.
    target: wgpu::Texture,
    id: ResourceId,
}

/// The page's frames as a Blitz custom widget.
pub struct WpeWidget {
    shared: Rc<Shared>,
    gpu: Option<Gpu>,
    textures: Option<Textures>,
}

impl WpeWidget {
    fn upload(&mut self, ctx: &mut dyn RenderContext, frame: &Frame) {
        let Some(gpu) = &self.gpu else { return };
        let FramePixels::Shm { pixels, stride } = &frame.pixels else {
            // dmabuf frames are imported by the GPU layer (`dmabuf.rs`).
            return;
        };
        let size = (frame.width.max(1), frame.height.max(1));
        if self.textures.as_ref().is_none_or(|t| t.size != size) {
            if let Some(old) = self.textures.take() {
                ctx.unregister_resource(old.id);
            }
            let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
            let make = |label, format, usage| {
                gpu.handle.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let source = make(
                "web view frame (BGRA)",
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let target = make(
                "web view frame",
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            );
            let Ok(id) = ctx.try_register_custom_resource(Box::new(target.clone())) else {
                return;
            };
            self.textures = Some(Textures { size, source, target, id });
        }
        let Some(t) = &self.textures else { return };
        gpu.handle.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(*stride), rows_per_image: Some(size.1) },
            wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        );
        let mut encoder = gpu.handle.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("web view frame") });
        let source = t.source.create_view(&wgpu::TextureViewDescriptor::default());
        let target = t.target.create_view(&wgpu::TextureViewDescriptor::default());
        gpu.blitter.copy(&gpu.handle.device, &mut encoder, &source, &target);
        gpu.handle.queue.submit([encoder.finish()]);
    }

    fn mods(m: Modifiers) -> Mods {
        Mods {
            control: m.contains(Modifiers::CONTROL),
            shift: m.contains(Modifiers::SHIFT),
            alt: m.contains(Modifiers::ALT),
            meta: m.contains(Modifiers::META),
        }
    }

    const fn button(b: MouseEventButton) -> u32 {
        match b {
            MouseEventButton::Main => 1,
            MouseEventButton::Auxiliary => 2,
            MouseEventButton::Secondary => 3,
            MouseEventButton::Fourth => 8,
            MouseEventButton::Fifth => 9,
        }
    }
}

impl Widget for WpeWidget {
    fn can_create_surfaces(&mut self, ctx: &mut dyn RenderContext) {
        self.textures = None;
        self.gpu = ctx
            .renderer_specific_context()
            .and_then(|any| any.downcast::<DeviceHandle>().ok())
            .map(|handle| Gpu {
                blitter: AlphaConvertBlitter::new(&handle.device, wgpu::TextureFormat::Rgba8Unorm, AlphaConversion::Unpremultiply),
                handle: *handle,
            });
    }

    fn destroy_surfaces(&mut self) {
        self.textures = None;
        self.gpu = None;
    }

    fn needs_redraw(&self) -> bool {
        self.shared.frame.borrow().is_some()
    }

    fn requires_redraw(&self) -> bool {
        self.shared.frame.borrow().is_some()
    }

    fn composite_texture(&self) -> Option<ResourceId> {
        self.textures.as_ref().map(|t| t.id)
    }

    fn handle_event(&mut self, event: &UiEvent) {
        let web = &self.shared.web;
        match event {
            UiEvent::PointerDown(p) | UiEvent::PointerUp(p) | UiEvent::PointerMove(p) | UiEvent::PointerLeave(p) => {
                let kind = match event {
                    UiEvent::PointerDown(_) => PointerKind::Down,
                    UiEvent::PointerUp(_) => PointerKind::Up,
                    UiEvent::PointerLeave(_) => PointerKind::Leave,
                    _ => PointerKind::Move,
                };
                let touch = matches!(p.id, BlitzPointerId::Finger(_));
                web.pointer(
                    kind,
                    f64::from(p.coords.client_x),
                    f64::from(p.coords.client_y),
                    Self::button(p.button),
                    Self::mods(p.mods),
                    1,
                    touch,
                );
            }
            UiEvent::Wheel(w) => {
                let (dx, dy, precise) = match w.delta {
                    BlitzWheelDelta::Lines(x, y) => (x, y, false),
                    BlitzWheelDelta::Pixels(x, y) => (x, y, true),
                };
                web.scroll(f64::from(w.coords.client_x), f64::from(w.coords.client_y), dx, dy, precise, Self::mods(w.mods));
            }
            UiEvent::KeyDown(k) | UiEvent::KeyUp(k) => {
                let name = k.key.to_string();
                let text = match &k.key {
                    keyboard_types::Key::Character(c) => Some(c.as_str()),
                    _ => k.text.as_deref(),
                };
                let named = if matches!(k.key, keyboard_types::Key::Character(_)) { "Character" } else { name.as_str() };
                if let Some(sym) = crate::wpe::keysym(Some(named), text) {
                    web.key(k.state == KeyState::Pressed, sym, Self::mods(k.modifiers));
                }
            }
            UiEvent::Ime(BlitzImeEvent::Commit(text)) | UiEvent::Paste(text) => web.ime_commit(text),
            UiEvent::Ime(BlitzImeEvent::Preedit(text, cursor)) => {
                let at = cursor.map_or(0, |(start, _)| u32::try_from(start).unwrap_or(0));
                web.ime_preedit(text, at);
            }
            UiEvent::Ime(BlitzImeEvent::Disabled) => web.ime_preedit("", 0),
            _ => {}
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::as_conversions)]
    fn paint(&mut self, ctx: &mut dyn RenderContext, _: &ComputedStyles, width: u32, height: u32, scale: f64) -> Scene {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let logical = ((f64::from(width) / scale).round() as u32, (f64::from(height) / scale).round() as u32);
        if logical != self.shared.size.get() && logical.0 > 0 && logical.1 > 0 {
            self.shared.size.set(logical);
            self.shared.web.resize(logical.0, logical.1);
        }
        let frame = self.shared.frame.borrow_mut().take();
        if let Some(frame) = frame {
            self.upload(ctx, &frame);
            self.shared.web.frame_done(frame.id);
        }
        let mut scene = Scene::new();
        if let Some(t) = &self.textures {
            let sx = f64::from(width) / f64::from(t.size.0);
            let sy = f64::from(height) / f64::from(t.size.1);
            scene.fill(
                peniko::Fill::NonZero,
                kurbo::Affine::IDENTITY,
                anyrender::Paint::Resource(peniko::ImageBrush { image: t.id, sampler: peniko::ImageSampler::default() }),
                Some(kurbo::Affine::scale_non_uniform(sx, sy)),
                &kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
        }
        scene
    }
}

/// The CSS cursor names WPE gives, as the window's cursors.
fn cursor(name: &str) -> Option<cursor_icon::CursorIcon> {
    name.parse().ok()
}

/// A `web` view in its pane, drawn by WPE WebKit.
#[component]
pub fn WpePane(ask: WebAsk, host: HostRef, theme: (String, String)) -> Element {
    let pane = use_pane();
    let key = format!("{}/{}#{}", ask.extension, ask.view, pane.id.0);
    let shell = try_use_context::<Arc<dyn ShellProvider>>();
    let mut problem = use_signal(|| None::<String>);
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let mut theme_now = use_signal(|| theme.clone());
    if theme_now.peek().1 != theme.1 {
        theme_now.set(theme.clone());
    }

    let made = use_hook({
        let (key, ask, host) = (key.clone(), ask.clone(), host.0.clone());
        let shell = shell.clone();
        move || -> Result<(Rc<Shared>, Rc<RefCell<Option<Bridge>>>), String> {
            if let Some((shared, bridge)) = VIEWS.with(|v| {
                v.borrow_mut().get_mut(&key).map(|(s, b, parked)| {
                    *parked = None;
                    (Rc::clone(s), Rc::clone(b))
                })
            }) {
                shared.web.set_visible(true);
                return Ok((shared, bridge));
            }
            let dir = ask.dir.clone().ok_or_else(|| format!("{} has no package folder to serve", ask.extension))?;
            let own = ask.extension.clone();
            let protocol: crate::wpe::Protocol = Box::new(move |uri: &str| {
                let path = uri.split_once("://").map_or(uri, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
                Some(match webview::serve(&own, &dir, &format!("/{path}")) {
                    Ok(a) => Answer {
                        status: 200,
                        mime: a.mime.to_owned(),
                        body: a.body,
                        headers: a.headers.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect(),
                    },
                    Err(e) => Answer { status: e.status(), mime: "text/plain".to_owned(), body: e.to_string().into_bytes(), headers: Vec::new() },
                })
            });
            let wake_shell = shell.clone();
            let web = WpeWeb::new(
                &Renderer::Cpu,
                (800, 600),
                webview::SCHEME,
                &webview::boot_script(),
                protocol,
                move || {
                    if let Some(s) = &wake_shell {
                        s.request_redraw();
                    }
                },
            )?;
            web.load(&webview::url(&ask.extension, &ask.src));
            let shared = Rc::new(Shared {
                web,
                frame: RefCell::new(None),
                size: Cell::new((0, 0)),
                shell: RefCell::new(shell.clone()),
            });
            let bridge = Rc::new(RefCell::new(None::<Bridge>));
            VIEWS.with(|v| v.borrow_mut().insert(key.clone(), (Rc::clone(&shared), Rc::clone(&bridge), None)));
            let _ = host;
            Ok((shared, bridge))
        }
    });

    let driven = made.clone();
    let drive_host = host.0.clone();
    let drive_ext = ask.extension.clone();
    use_future(move || {
        let made = driven.clone();
        let host = drive_host.clone();
        let ext = drive_ext.clone();
        async move {
            let Ok((shared, bridge_slot)) = made else { return };
            let (out_tx, out_rx) = std::sync::mpsc::channel::<Outbound>();
            let escape = Rc::new(Cell::new(false));
            let escaped = Arc::new(std::sync::atomic::AtomicBool::new(false));
            if bridge_slot.borrow().is_none() {
                let flag = Arc::clone(&escaped);
                *bridge_slot.borrow_mut() = Some(Bridge::new(
                    &host,
                    &ext,
                    Arc::new(move |m| {
                        let _ = out_tx.send(m);
                    }),
                    move || flag.store(true, std::sync::atomic::Ordering::SeqCst),
                ));
            }
            let mut sent_theme = String::new();
            loop {
                futures_timer::Delay::new(TICK).await;
                crate::wpe::iterate();
                let inbox = shared.web.take();
                if let Some(frame) = inbox.frame {
                    *shared.frame.borrow_mut() = Some(frame);
                    if let Some(s) = shared.shell.borrow().as_ref() {
                        s.request_redraw();
                    }
                }
                if let (Some(name), Some(s)) = (inbox.cursor.as_deref(), shared.shell.borrow().as_ref()) {
                    s.set_cursor(cursor(name));
                }
                if let (Some((x, y, w, h)), Some(s)) = (inbox.ime_area, shared.shell.borrow().as_ref()) {
                    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
                    s.set_ime_cursor_area(x as f32, y as f32, w as f32, h as f32);
                }
                if let Some(reason) = inbox.crashed {
                    problem.set(Some(format!("The web view's page stopped (reason {reason}): reopen the pane to start it again.")));
                }
                let mut work = Vec::new();
                if let Some(bridge) = bridge_slot.borrow().as_ref() {
                    let (scheme, css) = theme_now.peek().clone();
                    if inbox.loads > 0 || sent_theme != css {
                        bridge.send(Outbound::Theme { scheme, css: css.clone() });
                        bridge.send(Outbound::Shown { shown: true });
                        sent_theme = css;
                    }
                    for text in &inbox.messages {
                        if let Some(f) = bridge.receive(text) {
                            work.push(f);
                        }
                    }
                }
                for f in work {
                    spawn(f);
                }
                while let Ok(out) = out_rx.try_recv() {
                    let text = serde_json::to_string(&out.to_json()).unwrap_or_default();
                    shared.web.eval(&format!("window.__architectReceive&&window.__architectReceive({text})"));
                }
                if escaped.swap(false, std::sync::atomic::Ordering::SeqCst) || escape.replace(false) {
                    if let Some(m) = mounted.peek().clone() {
                        let _ = m.set_focus(false).await;
                    }
                }
            }
        }
    });

    let drop_key = key.clone();
    use_drop(move || {
        VIEWS.with(|v| {
            let mut v = v.borrow_mut();
            if let Some((shared, _, parked)) = v.get_mut(&drop_key) {
                shared.web.set_visible(false);
                *parked = Some(Instant::now());
            }
            v.retain(|_, (_, _, parked)| parked.is_none_or(|t| t.elapsed() < PARKED));
        });
    });

    let (shared, _) = match &made {
        Ok(m) => m.clone(),
        Err(why) => {
            return rsx! {
                div { style: "flex: 1; display: flex; align-items: center; justify-content: center; padding: 24px; font-size: 13px; color: var(--muted-foreground); text-align: center;",
                    "The web view could not start: {why}"
                }
            };
        }
    };
    let widget = use_hook(|| CustomWidgetAttr::new(WpeWidget { shared: Rc::clone(&shared), gpu: None, textures: None }));
    let focus = Rc::clone(&shared);
    let blur = Rc::clone(&shared);
    let ime_on = shell.clone();
    let ime_off = shell.clone();
    let message = problem();
    rsx! {
        div { style: "flex: 1; min-height: 0; position: relative; display: flex;",
            object {
                style: "display: block; flex: 1; width: 100%; height: 100%; outline: none;",
                tabindex: "0",
                data: widget,
                onmounted: move |e| mounted.set(Some(e.data())),
                onfocus: move |_| {
                    focus.web.focus(true);
                    if let Some(s) = &ime_on {
                        s.set_ime_enabled(true);
                    }
                },
                onblur: move |_| {
                    blur.web.focus(false);
                    if let Some(s) = &ime_off {
                        s.set_ime_enabled(false);
                    }
                },
            }
            if let Some(why) = message {
                div { style: "position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; padding: 24px; font-size: 13px; color: var(--muted-foreground); background: var(--background);",
                    "{why}"
                }
            }
        }
    }
}
