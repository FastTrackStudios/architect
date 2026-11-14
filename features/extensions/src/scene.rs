//! Scenes in a remote view (#175): an element an installed extension
//! paints as an anyrender scene, not as DOM.
//!
//! A scene crosses the boundary as two attributes of its element, so the
//! remote-view wire needs nothing new and Dioxus's own diffing decides
//! what is sent:
//!
//! - [`SCENE_ATTR`]: the scene's content, recorded in content space, as
//!   `<hash>\n<json>` the first time, and as `<hash>` alone after that
//!   while both sides still hold it ([`SceneCache`], the last
//!   [`CACHED`] scenes, the same FIFO on each side). It changes only
//!   when the content does.
//! - [`TRANSFORM_ATTR`]: content space to the element's, `a b c d e f`
//!   (a `kurbo::Affine`'s coefficients). Panning and zooming change only
//!   this, a few dozen bytes a frame, coalesced with the rest of the
//!   frame's edits.
//!
//! The host draws the element as a widget in its own window's scene
//! (Vello through Blitz on the desk; a canvas in a browser,
//! `crate::web`). When it is laid out at a new size it tells the view
//! with a [`RESIZE_EVENT`], a form value `{"width", "height", "scale"}`
//! (CSS pixels). The view's pointer, wheel and key handlers on the
//! element get the input, in the element's coordinates, as for any
//! element (#176). A press holds the pointer.
//!
//! The JSON is a compact form of anyrender's commands: fills, strokes,
//! layers, clips and box shadows, each path as SVG, each colour as RGBA
//! bytes. A gradient crosses as its first stop's colour; text should be
//! drawn as paths for now (a glyph run carries its font's bytes).

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

pub use anyrender::recording::RenderCommand;
pub use anyrender::{PaintScene, Scene};
pub use peniko::kurbo::Affine;

/// The attribute that carries an element's scene.
pub const SCENE_ATTR: &str = "data-architect-scene";

/// The attribute that carries its transform, content to element.
pub const TRANSFORM_ATTR: &str = "data-architect-transform";

/// The event a host sends a scene's element when it is laid out at a new
/// size: a form value, `{"width": …, "height": …, "scale": …}`.
pub const RESIZE_EVENT: &str = "architect-resize";

/// How many scenes each side keeps by hash.
pub const CACHED: usize = 64;

/// A scene's content, shared.
pub type Commands = Arc<Vec<RenderCommand>>;

/// One command as it crosses: a path as SVG, a transform's six numbers, a
/// colour as RGBA bytes. A gradient crosses as its first stop's colour,
/// an image or a backend's own paint not at all, nor a glyph run (a
/// font's bytes don't cross: draw text as paths).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "k")]
enum Wire {
    #[serde(rename = "f")]
    Fill {
        t: [f64; 6],
        c: [u8; 4],
        p: String,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "s")]
    Stroke {
        t: [f64; 6],
        c: [u8; 4],
        p: String,
        w: f64,
        /// Join: 0 bevel, 1 miter, 2 round. Cap: 0 butt, 1 square, 2 round.
        #[serde(default)]
        j: u8,
        #[serde(default)]
        cap: u8,
    },
    #[serde(rename = "c")]
    Clip {
        t: [f64; 6],
        p: String,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "l")]
    Layer {
        t: [f64; 6],
        p: String,
        a: f32,
        #[serde(default)]
        eo: bool,
    },
    #[serde(rename = "o")]
    Pop,
    #[serde(rename = "b")]
    Shadow {
        t: [f64; 6],
        r: [f64; 4],
        c: [u8; 4],
        radius: f64,
        sd: f64,
    },
}

fn rgba(c: peniko::Color) -> [u8; 4] {
    c.to_rgba8().to_u8_array()
}

fn brush_rgba(p: &anyrender::Paint) -> [u8; 4] {
    match p {
        anyrender::Paint::Solid(c) => rgba(*c),
        anyrender::Paint::Gradient(g) => g
            .stops
            .first()
            .map_or([0; 4], |s| rgba(s.color.to_alpha_color())),
        _ => [0; 4],
    }
}

const fn color([r, g, b, a]: [u8; 4]) -> peniko::Color {
    peniko::Color::from_rgba8(r, g, b, a)
}

const fn even_odd(fill: peniko::Fill) -> bool {
    matches!(fill, peniko::Fill::EvenOdd)
}

const fn fill_of(eo: bool) -> peniko::Fill {
    if eo {
        peniko::Fill::EvenOdd
    } else {
        peniko::Fill::NonZero
    }
}

fn path_of(svg: &str) -> peniko::kurbo::BezPath {
    peniko::kurbo::BezPath::from_svg(svg).unwrap_or_default()
}

fn to_wire(c: &RenderCommand) -> Option<Wire> {
    use peniko::kurbo::{Cap, Join};
    Some(match c {
        RenderCommand::Fill(f) => Wire::Fill {
            t: f.transform.as_coeffs(),
            c: brush_rgba(&f.brush),
            p: f.shape.to_svg(),
            eo: even_odd(f.fill),
        },
        RenderCommand::Stroke(s) => Wire::Stroke {
            t: s.transform.as_coeffs(),
            c: brush_rgba(&s.brush),
            p: s.shape.to_svg(),
            w: s.style.width,
            j: match s.style.join {
                Join::Bevel => 0,
                Join::Miter => 1,
                Join::Round => 2,
            },
            cap: match s.style.start_cap {
                Cap::Butt => 0,
                Cap::Square => 1,
                Cap::Round => 2,
            },
        },
        RenderCommand::PushClipLayer(l) => Wire::Clip {
            t: l.transform.as_coeffs(),
            p: l.clip.to_svg(),
            eo: even_odd(l.fill),
        },
        RenderCommand::PushLayer(l) => Wire::Layer {
            t: l.transform.as_coeffs(),
            p: l.clip.to_svg(),
            a: l.alpha,
            eo: even_odd(l.fill),
        },
        RenderCommand::PopLayer => Wire::Pop,
        RenderCommand::BoxShadow(b) => Wire::Shadow {
            t: b.transform.as_coeffs(),
            r: [b.rect.x0, b.rect.y0, b.rect.x1, b.rect.y1],
            c: rgba(b.brush),
            radius: b.radius,
            sd: b.std_dev,
        },
        RenderCommand::GlyphRun(_) => return None,
    })
}

fn from_wire(w: Wire) -> RenderCommand {
    use anyrender::recording::{
        BoxShadowCommand, ClipCommand, FillCommand, LayerCommand, StrokeCommand,
    };
    use peniko::kurbo::{Cap, Join, Rect, Stroke};
    match w {
        Wire::Fill { t, c, p, eo } => RenderCommand::Fill(FillCommand {
            fill: fill_of(eo),
            transform: Affine::new(t),
            brush: anyrender::Paint::Solid(color(c)),
            brush_transform: None,
            shape: path_of(&p),
        }),
        Wire::Stroke { t, c, p, w, j, cap } => {
            let cap = match cap {
                1 => Cap::Square,
                2 => Cap::Round,
                _ => Cap::Butt,
            };
            let join = match j {
                0 => Join::Bevel,
                2 => Join::Round,
                _ => Join::Miter,
            };
            RenderCommand::Stroke(StrokeCommand {
                style: Stroke::new(w).with_join(join).with_caps(cap),
                transform: Affine::new(t),
                brush: anyrender::Paint::Solid(color(c)),
                brush_transform: None,
                shape: path_of(&p),
            })
        }
        Wire::Clip { t, p, eo } => RenderCommand::PushClipLayer(ClipCommand {
            fill: fill_of(eo),
            transform: Affine::new(t),
            clip: path_of(&p),
        }),
        Wire::Layer { t, p, a, eo } => RenderCommand::PushLayer(LayerCommand {
            fill: fill_of(eo),
            blend: peniko::BlendMode::default(),
            alpha: a,
            transform: Affine::new(t),
            clip: path_of(&p),
            filter: None,
            backdrop_filter: None,
        }),
        Wire::Pop => RenderCommand::PopLayer,
        Wire::Shadow { t, r: [x0, y0, x1, y1], c, radius, sd } => RenderCommand::BoxShadow(BoxShadowCommand {
            transform: Affine::new(t),
            rect: Rect::new(x0, y0, x1, y1),
            brush: color(c),
            radius,
            std_dev: sd,
        }),
    }
}

/// A scene's content as JSON, and its hash.
#[must_use]
pub fn encode(commands: &[RenderCommand]) -> (u64, String) {
    let wire: Vec<Wire> = commands.iter().filter_map(to_wire).collect();
    let json = serde_json::to_string(&wire).unwrap_or_else(|_| "[]".to_owned());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut h);
    (h.finish(), json)
}

/// A transform as [`TRANSFORM_ATTR`] carries it.
#[must_use]
pub fn transform_attr(transform: Affine) -> String {
    transform
        .as_coeffs()
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`TRANSFORM_ATTR`]'s value as a transform; the identity when it is not
/// six numbers.
#[must_use]
pub fn parse_transform(value: &str) -> Affine {
    let numbers: Vec<f64> = value
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    match <[f64; 6]>::try_from(numbers.as_slice()) {
        Ok(coeffs) if coeffs.iter().all(|v| v.is_finite()) => Affine::new(coeffs),
        _ => Affine::IDENTITY,
    }
}

/// The last [`CACHED`] scenes one side has sent or received, by hash.
/// Both sides insert only full scenes and evict the oldest first, so the
/// sender knows what the receiver still holds.
#[derive(Debug, Default)]
pub struct SceneCache {
    order: VecDeque<u64>,
    held: HashMap<u64, Commands>,
}

impl SceneCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether it holds `hash`.
    #[must_use]
    pub fn holds(&self, hash: u64) -> bool {
        self.held.contains_key(&hash)
    }

    fn insert(&mut self, hash: u64, commands: Commands) {
        if self.held.insert(hash, commands).is_none() {
            self.order.push_back(hash);
            if self.order.len() > CACHED
                && let Some(old) = self.order.pop_front()
            {
                self.held.remove(&old);
            }
        }
    }

    /// The sender's side: the [`SCENE_ATTR`] value for `commands`, the
    /// content only when the receiver doesn't hold it.
    pub fn send(&mut self, commands: &[RenderCommand]) -> String {
        let (hash, json) = encode(commands);
        if self.holds(hash) {
            return format!("{hash:x}");
        }
        self.insert(hash, Arc::new(Vec::new()));
        format!("{hash:x}\n{json}")
    }

    /// The receiver's side: the scene a [`SCENE_ATTR`] value names.
    ///
    /// # Errors
    ///
    /// It names a scene not held, or its content is not a scene.
    pub fn receive(&mut self, value: &str) -> Result<Commands, String> {
        let (hash, json) = value.split_once('\n').unwrap_or((value, ""));
        let hash = u64::from_str_radix(hash.trim(), 16)
            .map_err(|_| "a scene with no hash".to_owned())?;
        if json.is_empty() {
            return self
                .held
                .get(&hash)
                .cloned()
                .ok_or_else(|| format!("a scene {hash:x} it was never sent"));
        }
        let wire: Vec<Wire> =
            serde_json::from_str(json).map_err(|e| format!("a scene that isn't one: {e}"))?;
        let commands = Arc::new(wire.into_iter().map(from_wire).collect::<Vec<_>>());
        self.insert(hash, Arc::clone(&commands));
        Ok(commands)
    }
}

/// `commands` drawn at `transform` into a scene.
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
#[must_use]
pub fn scene_of(commands: &[RenderCommand], transform: Affine) -> Scene {
    let mut scene = Scene::new();
    scene.append_scene(
        Scene {
            commands: commands.to_vec(),
            ..Scene::new()
        },
        transform,
    );
    scene
}

/// The host's widget: a scene's content at its transform, scaled to the
/// display, telling the view when its size changes.
#[cfg(feature = "remote")]
pub(crate) struct SceneWidget {
    pub(crate) commands: Commands,
    pub(crate) transform: Affine,
    pub(crate) element: u64,
    pub(crate) events: futures::channel::mpsc::UnboundedSender<crate::wire::ViewEvent>,
    pub(crate) size: Option<(u32, u32)>,
    pub(crate) dirty: bool,
}

#[cfg(feature = "remote")]
// kurbo's transforms compose with `*`: no overflow to guard.
#[allow(clippy::arithmetic_side_effects)]
impl blitz_dom::Widget for SceneWidget {
    fn needs_redraw(&self) -> bool {
        self.dirty
    }

    fn paint(
        &mut self,
        _: &mut dyn anyrender::RenderContext,
        _: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        self.dirty = false;
        let scale = if scale > 0.0 { scale } else { 1.0 };
        if self.size != Some((width, height)) {
            self.size = Some((width, height));
            let size = serde_json::json!({
                "width": f64::from(width) / scale,
                "height": f64::from(height) / scale,
                "scale": scale,
            });
            let _ = self.events.unbounded_send(crate::wire::ViewEvent {
                element: self.element,
                name: RESIZE_EVENT.to_owned(),
                bubbles: false,
                data: crate::wire::EventData::Form(size.to_string()),
            });
        }
        scene_of(&self.commands, Affine::scale(scale) * self.transform)
    }
}

/// The guest's side (feature `guest`): an element its view paints.
#[cfg(feature = "guest")]
mod guest {
    use std::cell::RefCell;
    use std::rc::Rc;

    use dioxus::prelude::*;

    use super::{Affine, RenderCommand, SceneCache};

    thread_local! {
        /// What this view's thread has sent (a view is a thread).
        static SENT: RefCell<SceneCache> = RefCell::new(SceneCache::new());
    }

    /// The [`SCENE_ATTR`] value for `commands` from this view.
    #[must_use]
    pub fn scene_attr(commands: &[RenderCommand]) -> String {
        SENT.with(|s| s.borrow_mut().send(commands))
    }

    /// An element's size, as its host last laid it out (CSS pixels).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SceneSize {
        pub width: f64,
        pub height: f64,
        pub scale: f64,
    }

    /// A [`super::RESIZE_EVENT`]'s size (it reaches the view as `input`).
    #[must_use]
    pub fn size_of(event: &FormEvent) -> Option<SceneSize> {
        let v: serde_json::Value = serde_json::from_str(&event.value()).ok()?;
        Some(SceneSize {
            width: v.get("width")?.as_f64()?,
            height: v.get("height")?.as_f64()?,
            scale: v.get("scale")?.as_f64()?,
        })
    }

    /// The handlers a [`RemoteScene`] passes on.
    #[derive(Clone, PartialEq, Props)]
    pub struct RemoteSceneProps {
        /// What it draws, in content space; compared by pointer, so keep
        /// one `Rc` while it doesn't change.
        pub content: Rc<Vec<RenderCommand>>,
        /// Content space to the element's: pan and zoom.
        #[props(default = Affine::IDENTITY)]
        pub transform: Affine,
        #[props(default)]
        pub style: String,
        #[props(default)]
        pub onresize: Option<EventHandler<super::guest::SceneSize>>,
        #[props(default)]
        pub onpointerdown: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onpointermove: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onpointerup: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onwheel: Option<EventHandler<WheelEvent>>,
        #[props(default)]
        pub onkeydown: Option<EventHandler<KeyboardEvent>>,
    }

    /// An element painted with `content` at `transform`.
    ///
    /// It fills the view (or its nearest positioned parent) unless `style`
    /// says otherwise. Its content crosses once (by hash after that); a
    /// new `transform` alone is a few bytes. Its size, as the host lays
    /// it out, comes to `onresize`.
    #[allow(non_snake_case)]
    pub fn RemoteScene(props: RemoteSceneProps) -> Element {
        // Encoded once per content (by pointer), not on every pan.
        let encoded = use_hook(|| Rc::new(RefCell::new((std::ptr::null(), String::new()))));
        let scene = {
            let mut e = encoded.borrow_mut();
            let at = Rc::as_ptr(&props.content);
            if e.0 != at {
                *e = (at, scene_attr(&props.content));
            }
            e.1.clone()
        };
        let transform = super::transform_attr(props.transform);
        let RemoteSceneProps {
            onresize,
            onpointerdown,
            onpointermove,
            onpointerup,
            onwheel,
            onkeydown,
            style,
            ..
        } = props;
        rsx! {
            object {
                style: "display: block; position: absolute; left: 0; top: 0; width: 100%; height: 100%; outline: none; {style}",
                tabindex: "0",
                "data-architect-scene": "{scene}",
                "data-architect-transform": "{transform}",
                // The host's `RESIZE_EVENT` arrives as an `input`.
                oninput: move |e: FormEvent| {
                    if let (Some(size), Some(on)) = (size_of(&e), onresize) {
                        on.call(size);
                    }
                },
                onpointerdown: move |e| if let Some(h) = onpointerdown { h.call(e) },
                onpointermove: move |e| if let Some(h) = onpointermove { h.call(e) },
                onpointerup: move |e| if let Some(h) = onpointerup { h.call(e) },
                onwheel: move |e| if let Some(h) = onwheel { h.call(e) },
                onkeydown: move |e| if let Some(h) = onkeydown { h.call(e) },
            }
        }
    }
}

#[cfg(feature = "guest")]
pub use guest::{RemoteScene, RemoteSceneProps, SceneSize, scene_attr, size_of};

/// A linked extension's paint view in its process copy (#591, #175).
///
/// The `PaintView` runs in the process; each frame it records is sent as
/// a [`RemoteScene`]'s content, and the element's input comes back to it
/// as `ViewInput`s. It is the same view either way.
#[cfg(all(feature = "guest", feature = "paint"))]
pub fn remote_paint_root<V>() -> dioxus::prelude::Element
where
    V: architect_workspace::paint::PaintView + Default,
{
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use architect_workspace::paint::PaintView;
    use architect_workspace::{
        Button, FrameClock, Geometry, KindId, PaneId, Placement, ViewCtx, ViewHost, ViewInput,
    };
    use dioxus::prelude::*;

    struct Live<V> {
        view: V,
        ctx: ViewCtx,
        content: Rc<Vec<RenderCommand>>,
    }

    impl<V: PaintView> Live<V> {
        fn record(&mut self) {
            let mut scene = Scene::new();
            self.view.paint(&self.ctx, &mut scene);
            self.content = Rc::new(scene.commands);
        }

        /// `input` given to the view: whether it drew again.
        fn input(&mut self, input: &ViewInput) -> bool {
            let changed = self.view.input(&self.ctx, input);
            if changed {
                self.record();
            }
            changed
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    const fn at(p: dioxus::html::geometry::ElementPoint) -> (f32, f32) {
        (p.x as f32, p.y as f32)
    }

    fn button(e: &PointerEvent) -> Button {
        match e.trigger_button() {
            Some(dioxus::html::input_data::MouseButton::Secondary) => Button::Secondary,
            Some(dioxus::html::input_data::MouseButton::Auxiliary) => Button::Middle,
            _ => Button::Primary,
        }
    }

    let live = use_hook(|| {
        let began = Instant::now();
        let clock = FrameClock::new(Duration::from_micros(16_667), move || began.elapsed());
        let ctx = ViewCtx::new(
            PaneId::new("remote"),
            KindId::paint(),
            (ViewHost::Blitz, Placement::Pane),
            (
                architect::action::ActionList::new(),
                architect::action::ActionCaller::app(),
            ),
            clock,
        );
        let mut live = Live {
            view: V::default(),
            ctx,
            content: Rc::new(Vec::new()),
        };
        live.record();
        Rc::new(RefCell::new(live))
    });
    let mut drawn = use_signal(|| 0_u64);
    let mut redraw = move || drawn.with_mut(|d| *d = d.wrapping_add(1));
    // A view that is moving draws every frame.
    {
        let live = Rc::clone(&live);
        use_future(move || {
            let live = Rc::clone(&live);
            async move {
                loop {
                    futures_timer_delay(Duration::from_micros(16_667)).await;
                    let mut l = live.borrow_mut();
                    if l.view.animating(&l.ctx) {
                        l.record();
                        drop(l);
                        redraw();
                    }
                }
            }
        });
    }
    let _ = drawn();
    let content = Rc::clone(&live.borrow().content);
    let sized = Rc::clone(&live);
    let on = {
        let live = Rc::clone(&live);
        move |input: ViewInput| {
            if live.borrow_mut().input(&input) {
                redraw();
            }
        }
    };
    let (mut down, mut moved, mut up, mut wheel, mut key) =
        (on.clone(), on.clone(), on.clone(), on.clone(), on);
    rsx! {
        RemoteScene {
            content,
            onresize: move |size: SceneSize| {
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                let geometry = Geometry {
                    width: size.width as f32,
                    height: size.height as f32,
                    scale: size.scale as f32,
                };
                let mut l = sized.borrow_mut();
                if l.ctx.geometry != geometry {
                    l.ctx.geometry = geometry;
                    l.record();
                    drop(l);
                    redraw();
                }
            },
            onpointerdown: move |e: PointerEvent| {
                let (x, y) = at(e.element_coordinates());
                down(ViewInput::PointerDown { x, y, button: button(&e) });
            },
            onpointermove: move |e: PointerEvent| {
                let (x, y) = at(e.element_coordinates());
                moved(ViewInput::PointerMove { x, y });
            },
            onpointerup: move |e: PointerEvent| {
                let (x, y) = at(e.element_coordinates());
                up(ViewInput::PointerUp { x, y, button: button(&e) });
            },
            onwheel: move |e: WheelEvent| {
                let (x, y) = at(e.element_coordinates());
                let d = e.delta().strip_units();
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                wheel(ViewInput::Wheel { dx: d.x as f32, dy: d.y as f32, x, y, mods: Vec::new() });
            },
            onkeydown: move |e: KeyboardEvent| {
                key(ViewInput::Key { key: e.key().to_string(), down: true, modifiers: Vec::new() });
            },
        }
    }
}

/// Waits `d` on the view's own runtime (the guest's tokio).
#[cfg(all(feature = "guest", feature = "paint"))]
async fn futures_timer_delay(d: std::time::Duration) {
    tokio::time::sleep(d).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::kurbo::Rect;
    use peniko::{Color, Fill};

    fn square(x: f64) -> Vec<RenderCommand> {
        let mut s = Scene::new();
        s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(1, 2, 3),
            None,
            &Rect::new(x, 0.0, x + 10.0, 10.0),
        );
        s.commands
    }

    #[test]
    fn a_scene_crosses_once_then_by_its_hash() {
        let (mut sent, mut got) = (SceneCache::new(), SceneCache::new());
        let first = sent.send(&square(0.0));
        assert!(first.contains('\n'));
        let a = got.receive(&first).unwrap();
        let again = sent.send(&square(0.0));
        assert!(!again.contains('\n'), "the same scene is its hash: {again}");
        assert_eq!(got.receive(&again).unwrap(), a);
        assert_eq!(*a, square(0.0));
    }

    #[test]
    fn both_sides_forget_the_same_scenes() {
        let (mut sent, mut got) = (SceneCache::new(), SceneCache::new());
        for i in 0..=u32::try_from(CACHED).unwrap_or(u32::MAX) {
            let v = sent.send(&square(f64::from(i)));
            got.receive(&v).unwrap();
        }
        // The first is gone on both sides: it is sent whole again.
        let v = sent.send(&square(0.0));
        assert!(v.contains('\n'));
        got.receive(&v).unwrap();
        assert!(got.receive("deadbeef").is_err());
    }

    #[test]
    fn a_transform_round_trips_and_nonsense_is_the_identity() {
        let t = Affine::translate((3.5, -2.0)) * Affine::scale(1.25);
        assert_eq!(parse_transform(&transform_attr(t)), t);
        assert_eq!(parse_transform("1 2 x"), Affine::IDENTITY);
        assert_eq!(parse_transform("1 0 0 1 NaN 0"), Affine::IDENTITY);
    }
}
