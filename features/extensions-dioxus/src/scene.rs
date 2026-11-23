//! A scene in a process's view (`architect_extensions::scene`): the
//! guest's side. [`RemoteScene`] is an element its view paints, carried to
//! the host by two attributes; [`RemotePaint`] (feature `paint`) a
//! `PaintView` in a process, drawn by its host.

/// The guest's side (feature `guest`): an element its view paints.
mod guest {
    use std::cell::RefCell;
    use std::rc::Rc;

    use dioxus::prelude::*;

    use architect_extensions::scene::{Affine, RenderCommand, SceneCache};

    thread_local! {
        /// What this view's thread has sent (a view is a thread).
        static SENT: RefCell<SceneCache> = RefCell::new(SceneCache::new());
    }

    /// The [`SCENE_ATTR`](architect_extensions::scene::SCENE_ATTR) value for `commands` from this view.
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

    /// A [`architect_extensions::scene::RESIZE_EVENT`]'s size (it reaches the view as `input`).
    #[must_use]
    pub fn size_of(event: &FormEvent) -> Option<SceneSize> {
        let v: serde_json::Value = serde_json::from_str(&event.value()).ok()?;
        Some(SceneSize {
            width: v.get("width")?.as_f64()?,
            height: v.get("height")?.as_f64()?,
            scale: v.get("scale")?.as_f64()?,
        })
    }

    /// The layers a [`RemoteScene`] shows: compared by pointer, so keep
    /// one while nothing changes.
    #[derive(Clone)]
    pub struct Shown(pub Rc<Vec<(architect_scene::layers::Key, architect_scene::Layer)>>);

    impl PartialEq for Shown {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.0, &other.0)
        }
    }

    /// The named values a [`RemoteScene`] shows with its layers
    /// (`architect_scene::anim::Values`): compared by pointer, so keep
    /// one while nothing changes.
    #[derive(Clone)]
    pub struct ShownValues(pub Rc<Vec<(architect_scene::anim::Name, f64)>>);

    impl PartialEq for ShownValues {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.0, &other.0)
        }
    }

    /// What a [`RemoteScene`] has told its host of its layers.
    #[derive(Default)]
    struct Told {
        sender: architect_scene::ops::Sender,
        batches: u64,
        value: String,
        shown: Option<(Shown, Option<ShownValues>)>,
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
        /// Its layers, in place of `content`: each crosses when it comes
        /// or changes, and a layer that only moved as its transform.
        /// Text, images and gradients cross as drawn.
        #[props(default)]
        pub layers: Option<Shown>,
        /// Values its layers' animations may be bound to: each crosses
        /// as its name and number when it changes. Only with `layers`.
        #[props(default)]
        pub values: Option<ShownValues>,
        #[props(default)]
        pub style: String,
        #[props(default)]
        pub onresize: Option<EventHandler<super::guest::SceneSize>>,
        /// What a pointer did to one of its layers' hit regions, as its
        /// host saw it (`architect_scene::region`).
        #[props(default)]
        pub onregion: Option<EventHandler<architect_scene::region::RegionEvent>>,
        #[props(default)]
        pub onpointerdown: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onpointermove: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onpointerup: Option<EventHandler<PointerEvent>>,
        /// The press ended where the view couldn't see it (outside the
        /// pane, the window or the focus lost): let go of what it held.
        #[props(default)]
        pub onpointercancel: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onwheel: Option<EventHandler<WheelEvent>>,
        #[props(default)]
        pub onkeydown: Option<EventHandler<KeyboardEvent>>,
        #[props(default)]
        pub onkeyup: Option<EventHandler<KeyboardEvent>>,
        /// The pointer left the element (no button held: a held one's
        /// moves keep coming).
        #[props(default)]
        pub onpointerleave: Option<EventHandler<PointerEvent>>,
        #[props(default)]
        pub onfocus: Option<EventHandler<FocusEvent>>,
        #[props(default)]
        pub onblur: Option<EventHandler<FocusEvent>>,
        /// An input method's text as it is composed.
        #[props(default)]
        pub oncompositionupdate: Option<EventHandler<CompositionEvent>>,
        /// An input method's text, committed.
        #[props(default)]
        pub oncompositionend: Option<EventHandler<CompositionEvent>>,
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
            if e.0 != at && props.layers.is_none() {
                *e = (at, scene_attr(&props.content));
            }
            e.1.clone()
        };
        // What changed since the layers it last showed, as one batch.
        let told = use_hook(|| Rc::new(RefCell::new(Told::default())));
        let layers = props.layers.as_ref().map(|shown| {
            let mut t = told.borrow_mut();
            let now = (shown.clone(), props.values.clone());
            if t.shown.as_ref() != Some(&now) {
                let values = props.values.as_ref().map_or(&[][..], |v| v.0.as_slice());
                if let Some(batch) = t.sender.frame_with(&shown.0, values) {
                    t.batches = t.batches.wrapping_add(1);
                    // As bytes beside the edits where its host takes them
                    // (`wire::BYTES_WIRE`), else in the attribute.
                    t.value = match architect_extensions::scene::outbox::put(batch) {
                        Ok(marker) => marker,
                        Err(batch) => architect_extensions::scene::layers_value(t.batches, &batch),
                    };
                }
                t.shown = Some(now);
            }
            t.value.clone()
        });
        let scene = if layers.is_some() {
            String::new()
        } else {
            scene
        };
        let transform = architect_extensions::scene::transform_attr(props.transform);
        let RemoteSceneProps {
            onresize,
            onregion,
            onpointerdown,
            onpointermove,
            onpointerup,
            onpointercancel,
            onwheel,
            onkeydown,
            onkeyup,
            onpointerleave,
            onfocus,
            onblur,
            oncompositionupdate,
            oncompositionend,
            style,
            ..
        } = props;
        rsx! {
            object {
                style: "display: block; position: absolute; left: 0; top: 0; width: 100%; height: 100%; outline: none; {style}",
                tabindex: "0",
                "data-architect-scene": if layers.is_none() { "{scene}" },
                "data-architect-layers": layers,
                "data-architect-transform": "{transform}",
                // The host's `RESIZE_EVENT` arrives as an `input`.
                oninput: move |e: FormEvent| {
                    if let (Some(size), Some(on)) = (size_of(&e), onresize) {
                        on.call(size);
                    } else if let (Some(event), Some(on)) = (architect_extensions::scene::region_of_value(&e.value()), onregion) {
                        on.call(event);
                    }
                },
                onpointerdown: move |e| if let Some(h) = onpointerdown { h.call(e) },
                onpointermove: move |e| if let Some(h) = onpointermove { h.call(e) },
                onpointerup: move |e| if let Some(h) = onpointerup { h.call(e) },
                onpointercancel: move |e| if let Some(h) = onpointercancel { h.call(e) },
                onwheel: move |e| if let Some(h) = onwheel { h.call(e) },
                onkeydown: move |e| if let Some(h) = onkeydown { h.call(e) },
                onkeyup: move |e| if let Some(h) = onkeyup { h.call(e) },
                onpointerleave: move |e| if let Some(h) = onpointerleave { h.call(e) },
                onfocus: move |e| if let Some(h) = onfocus { h.call(e) },
                onblur: move |e| if let Some(h) = onblur { h.call(e) },
                oncompositionupdate: move |e| if let Some(h) = oncompositionupdate { h.call(e) },
                oncompositionend: move |e| if let Some(h) = oncompositionend { h.call(e) },
            }
        }
    }
}

pub use guest::{
    RemoteScene, RemoteSceneProps, SceneSize, Shown, ShownValues, scene_attr, size_of,
};

/// A linked extension's paint view in its process copy (#591, #175):
/// `V` made for the pane, as `linked::Builder::paint` makes it linked in,
/// its `PaintWake` in its extras.
#[cfg(feature = "paint")]
pub fn remote_paint_root<V>() -> dioxus::prelude::Element
where
    V: architect_scene::paint::PaintView + Default,
{
    use dioxus::prelude::*;
    let mount = use_hook(|| {
        let mount = architect_scene::paint::PaintMount::new(|_| V::default());
        let wake = mount.wake_handle();
        mount.extras(architect_scene::view::Extras::default().with(wake))
    });
    rsx! { RemotePaint { mount } }
}

/// What a process's paint view keeps between frames.
#[cfg(feature = "paint")]
mod remote_paint {
    use std::rc::Rc;
    use std::sync::Arc;

    use architect_scene::paint::{FrameStats, PaintView, Scene};
    use architect_scene::view::{ViewCtx, ViewInput};

    use super::{Shown, ShownValues};

    pub(super) struct Live {
        pub(super) view: Box<dyn PaintView>,
        pub(super) ctx: ViewCtx,
        /// What it shows: its layers, or one layer of the scene it painted.
        pub(super) shown: Shown,
        /// The values its layers read, as it last said them.
        pub(super) values: ShownValues,
        pub(super) stats: FrameStats,
    }

    impl Live {
        /// Asks the view for this frame: its time goes into its stats, as
        /// a linked painter's does.
        pub(super) fn record(&mut self) {
            let began = self.ctx.clock.now();
            let layers = self.view.layers(&self.ctx).unwrap_or_else(|| {
                let mut scene = Scene::new();
                self.view.paint(&self.ctx, &mut scene);
                vec![(0, architect_scene::Layer::new(Arc::new(scene.commands)))]
            });
            self.shown = Shown(Rc::new(layers));
            let values = self.view.values(&self.ctx);
            if *self.values.0 != values {
                self.values = ShownValues(Rc::new(values));
            }
            self.stats
                .record(self.ctx.clock.now().saturating_sub(began));
        }

        /// `input` given to the view: whether it drew again.
        pub(super) fn input(&mut self, input: &ViewInput) -> bool {
            let changed = self.view.input(&self.ctx, input);
            if changed {
                self.record();
            }
            changed
        }

        /// What one of its hit regions heard (the host handled the press
        /// or the drag, `architect_scene::region`): whether it drew again.
        pub(super) fn region(&mut self, event: &architect_scene::region::RegionEvent) -> bool {
            let changed = self.view.region(&self.ctx, event);
            if changed {
                self.record();
            }
            changed
        }
    }

    /// A view's wakes, as its host's redraw: the component drawn again
    /// on the view's own thread, from whichever thread woke it.
    pub(super) struct Redraw(pub(super) Arc<dyn Fn() + Send + Sync>);

    impl blitz_traits::shell::ShellProvider for Redraw {
        fn request_redraw(&self) {
            (self.0)();
        }
    }
}

#[cfg(feature = "paint")]
mod remote_paint_ui {
    use dioxus::prelude::*;

    /// A [`PaintView`](architect_scene::paint::PaintView) in a process,
    /// drawn by its host: what `linked::PaintPane` is in a process copy.
    ///
    /// The view is made from `mount` and given the same [`ViewCtx`] a linked
    /// `PaintPane` gives it: the pane's id, the host's theme as tokens (kept
    /// current), `mount`'s extras with its `PaintWake` (a wake from any
    /// thread asks it for a frame), and every input kind with its modifiers
    /// (pointer, wheel in logical pixels, touch, keys down and up, focus,
    /// composed text). What it records crosses as layers ([`RemoteScene`]):
    /// a fragment once, then what changed. Its frames' times go into
    /// `mount`'s stats, as linked.
    ///
    /// [`ViewCtx`]: architect_scene::view::ViewCtx
    #[component]
    pub fn RemotePaint(mount: architect_scene::paint::PaintMount) -> Element {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::Arc;
        use std::time::Duration;

        use super::remote_paint::{Live, Redraw};
        use super::{RemoteScene, SceneSize, Shown, ShownValues, futures_timer_delay};
        use architect_scene::view::{
            Button, FrameClock, Geometry, ThemeTokens, ViewCtx, ViewInput,
        };
        use architect_views::{KindId, PaneId, Placement, ViewHost};
        use dioxus::html::geometry::WheelDelta;
        use dioxus::prelude::*;

        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        const fn at(p: dioxus::html::geometry::ElementPoint) -> (f32, f32) {
            (p.x as f32, p.y as f32)
        }

        /// The held modifiers by [`ViewInput::Key`]'s names (`"Mod"` is ⌘ on
        /// a Mac, Ctrl elsewhere), as a linked pane names them.
        fn held(m: Modifiers) -> Vec<&'static str> {
            let command = if cfg!(target_os = "macos") {
                Modifiers::META
            } else {
                Modifiers::CONTROL
            };
            [
                (command, "Mod"),
                (Modifiers::SHIFT, "Shift"),
                (Modifiers::ALT, "Alt"),
                (Modifiers::CONTROL, "Ctrl"),
            ]
            .into_iter()
            .filter(|(k, _)| m.contains(*k))
            .map(|(_, name)| name)
            .collect()
        }

        fn button(e: &PointerEvent) -> Button {
            match e.trigger_button() {
                Some(dioxus::html::input_data::MouseButton::Secondary) => Button::Secondary,
                Some(dioxus::html::input_data::MouseButton::Auxiliary) => Button::Middle,
                _ => Button::Primary,
            }
        }

        /// A finger's pointer as the touch a linked pane gives (its host
        /// numbers fingers from 10), or the mouse's or pen's input.
        fn pointer(
            e: &PointerEvent,
            phase: &'static str,
            mouse: impl FnOnce(f32, f32) -> ViewInput,
        ) -> ViewInput {
            let (x, y) = at(e.element_coordinates());
            if e.pointer_type() == "touch" {
                let id = u64::try_from(e.pointer_id().saturating_sub(10)).unwrap_or(0);
                return ViewInput::Touch { id, x, y, phase };
            }
            mouse(x, y)
        }

        /// The theme's tokens as a linked pane reads them from the app's
        /// store: the host's resolved theme, every token.
        fn tokens(theme: Option<&architect_theme::Resolved>) -> ThemeTokens {
            theme.map(ThemeTokens::from).unwrap_or_default()
        }

        let theme = tokens(crate::guest::use_theme().as_ref());
        let pane = crate::pane::use_pane_state().map(|(id, _)| id);
        let mut drawn = use_signal(|| 0_u64);
        // The view is made at its pane's first size, as a linked pane
        // makes it as it mounts: by then its host has told it its pane
        // (`use_pane_state`, which comes after its first render).
        let live: Rc<RefCell<Option<Live>>> = use_hook(|| Rc::new(RefCell::new(None)));
        let make = {
            let mount = mount.clone();
            move |theme: ThemeTokens, pane: Option<String>| {
                // The view's thread's own clock, its animations' wall time.
                let clock =
                    FrameClock::new(mount.frame_budget(), architect_scene::anim::wall_elapsed);
                let pane = pane
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| "remote".to_owned());
                let ctx = ViewCtx::new(
                    PaneId::new(pane),
                    KindId::paint(),
                    (ViewHost::Blitz, Placement::Pane),
                    // The app's actions it may run, through its host: what
                    // a linked view's list is, as far as it may reach.
                    (
                        architect_extensions::guest::app_actions(),
                        architect::action::ActionCaller::app(),
                    ),
                    clock,
                )
                .with_theme(theme)
                .with_extras(mount.host_extras());
                let view = mount.make_view(&ctx);
                // A wake, from any thread: this component draws again and
                // asks the view (below).
                let update = dioxus::core::schedule_update();
                mount
                    .wake_handle()
                    .attach(Some(Arc::new(Redraw(Arc::new(move || update())))));
                Live {
                    view,
                    ctx,
                    shown: Shown(Rc::new(Vec::new())),
                    values: ShownValues(Rc::new(Vec::new())),
                    stats: mount.frame_stats(),
                }
            }
        };
        let mut redraw = move || drawn.with_mut(|d| *d = d.wrapping_add(1));
        if let Some(l) = live.borrow_mut().as_mut() {
            // A wake, or a new theme: a frame from the view now.
            let themed = l.ctx.theme != theme;
            if themed {
                l.ctx.theme = theme.clone();
            }
            if let Some(id) = pane.as_ref().filter(|p| !p.is_empty())
                && l.ctx.pane.0 != *id
            {
                l.ctx.pane = PaneId::new(id.clone());
            }
            if mount.wake_handle().take() || themed {
                l.record();
            }
        }
        // A view that says it is moving is asked every frame. Layers that
        // move by their animations need not: the host runs them.
        {
            let live = Rc::clone(&live);
            use_future(move || {
                let live = Rc::clone(&live);
                async move {
                    loop {
                        futures_timer_delay(Duration::from_micros(16_667)).await;
                        let mut slot = live.borrow_mut();
                        let Some(l) = slot.as_mut() else { continue };
                        if l.view.animating(&l.ctx) {
                            l.record();
                            drop(slot);
                            redraw();
                        }
                    }
                }
            });
        }
        let _ = drawn();
        let (shown, values) = live.borrow().as_ref().map_or_else(
            || (Shown(Rc::new(Vec::new())), ShownValues(Rc::new(Vec::new()))),
            |l| (l.shown.clone(), l.values.clone()),
        );
        let sized = Rc::clone(&live);
        let (sized_theme, sized_pane) = (theme, pane);
        let on = {
            let live = Rc::clone(&live);
            move |input: ViewInput| {
                let changed = live.borrow_mut().as_mut().is_some_and(|l| l.input(&input));
                if changed {
                    redraw();
                }
            }
        };
        // What its hit regions heard, before (and instead of) its input.
        let regioned = {
            let live = Rc::clone(&live);
            move |event: architect_scene::region::RegionEvent| {
                let changed = live.borrow_mut().as_mut().is_some_and(|l| l.region(&event));
                if changed {
                    redraw();
                }
            }
        };
        let (mut down, mut moved, mut up, mut cancel, mut leave, mut wheel) = (
            on.clone(),
            on.clone(),
            on.clone(),
            on.clone(),
            on.clone(),
            on.clone(),
        );
        let (mut key_down, mut key_up, mut focus, mut blur, mut composing, mut committed) = (
            on.clone(),
            on.clone(),
            on.clone(),
            on.clone(),
            on.clone(),
            on,
        );
        // In a box of its own, positioned: the scene's element fills its
        // nearest positioned parent, and a linked `PaintPane` fills the
        // box it is mounted in, not the view around it.
        rsx! {
            div { style: "position: relative; display: block; width: 100%; height: 100%; overflow: hidden;",
            RemoteScene {
                content: Rc::new(Vec::new()),
                layers: shown,
                values,
                onregion: regioned,
                onresize: move |size: SceneSize| {
                    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                    let geometry = Geometry {
                        width: size.width as f32,
                        height: size.height as f32,
                        scale: size.scale as f32,
                    };
                    let mut slot = sized.borrow_mut();
                    let l = slot.get_or_insert_with(|| make(sized_theme.clone(), sized_pane.clone()));
                    if l.ctx.geometry != geometry || l.shown.0.is_empty() {
                        l.ctx.geometry = geometry;
                        l.record();
                        drop(slot);
                        redraw();
                    }
                },
                onpointerdown: move |e: PointerEvent| {
                    down(pointer(&e, "start", |x, y| ViewInput::PointerDown { x, y, button: button(&e), mods: held(e.modifiers()) }));
                },
                onpointermove: move |e: PointerEvent| {
                    moved(pointer(&e, "move", |x, y| ViewInput::PointerMove { x, y, mods: held(e.modifiers()) }));
                },
                onpointerup: move |e: PointerEvent| {
                    up(pointer(&e, "end", |x, y| ViewInput::PointerUp { x, y, button: button(&e), mods: held(e.modifiers()) }));
                },
                // A press it won't see the end of: the view hears a release
                // (a finger's, as a linked pane's, a cancelled touch).
                onpointercancel: move |e: PointerEvent| {
                    cancel(pointer(&e, "cancel", |x, y| ViewInput::PointerUp { x, y, button: button(&e), mods: held(e.modifiers()) }));
                },
                // Gone, as a linked pane says it: where it went, so nothing
                // is under it.
                onpointerleave: move |e: PointerEvent| {
                    let finger = e.pointer_type() == "touch";
                    leave(pointer(&e, "cancel", |x, y| ViewInput::PointerMove { x, y, mods: held(e.modifiers()) }));
                    // Then that it left, as a linked pane says it.
                    if !finger {
                        leave(ViewInput::PointerLeave);
                    }
                },
                onwheel: move |e: WheelEvent| {
                    let (x, y) = at(e.element_coordinates());
                    // Logical pixels, as a linked pane gives them: a line is
                    // `WHEEL_LINE` of them (a page crosses as lines).
                    let line = architect_scene::paint::WHEEL_LINE;
                    let (dx, dy) = match e.delta() {
                        WheelDelta::Pixels(d) => (d.x, d.y),
                        WheelDelta::Lines(d) => (d.x * line, d.y * line),
                        WheelDelta::Pages(d) => (d.x * line, d.y * line),
                    };
                    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                    wheel(ViewInput::Wheel { dx: dx as f32, dy: dy as f32, x, y, mods: held(e.modifiers()) });
                },
                onkeydown: move |e: KeyboardEvent| {
                    key_down(ViewInput::Key { key: e.key().to_string(), down: true, modifiers: held(e.modifiers()) });
                },
                onkeyup: move |e: KeyboardEvent| {
                    key_up(ViewInput::Key { key: e.key().to_string(), down: false, modifiers: held(e.modifiers()) });
                },
                onfocus: move |_| focus(ViewInput::Focus(true)),
                onblur: move |_| blur(ViewInput::Focus(false)),
                oncompositionupdate: move |e: CompositionEvent| {
                    composing(ViewInput::Ime { preedit: e.data().data(), commit: None });
                },
                oncompositionend: move |e: CompositionEvent| {
                    committed(ViewInput::Ime { preedit: String::new(), commit: Some(e.data().data()) });
                },
            }
            }
        }
    }
}

#[cfg(feature = "paint")]
pub use remote_paint_ui::{RemotePaint, RemotePaintProps};

/// Waits `d` on the view's own runtime (the guest's tokio).
#[cfg(feature = "paint")]
async fn futures_timer_delay(d: std::time::Duration) {
    tokio::time::sleep(d).await;
}
