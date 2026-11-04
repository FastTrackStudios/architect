//! `ImGui` and `Canvas` for Luau scripts, and a live script that draws
//! with them ([`Session`]).
//!
//! `ImGui` is `ReaImGui`'s API, re-declared from Dear `ImGui`'s (`ReaImGui` is
//! LGPL; nothing of its files is here), so a `ReaImGui` script ports by
//! swapping its `reaper.*` calls for actions:
//!
//! ```lua
//! local `ImGui` = require 'imgui' '0.10'
//! local ctx = `ImGui`.`CreateContext`('Tool')
//! local gain = 0.5
//! local function loop()
//!   local visible, open = `ImGui`.Begin(ctx, 'Tool', true)
//!   if visible then
//!     local changed
//!     changed, gain = `ImGui`.`SliderDouble`(ctx, 'Gain', gain, 0, 1)
//!     `ImGui`.End(ctx)
//!   end
//!   if open then defer(loop) end
//! end
//! defer(loop)
//! ```
//!
//! | `ImGui.` | gives |
//! |---|---|
//! | `CreateContext(label)` | a context: every call takes it first |
//! | `Begin(ctx, name, p_open?, flags?)` | `visible, open` (`open` is `nil` without `p_open`) |
//! | `End(ctx)` | — (call it when `Begin` gave `visible`) |
//! | `Text`, `TextDisabled(ctx, text)`, `TextColored(ctx, rgba, text)` | — |
//! | `Button(ctx, label, w?, h?)` | clicked |
//! | `Checkbox(ctx, label, v)` | `changed, v` |
//! | `SliderDouble(ctx, label, v, min, max, format?)`, `SliderInt(…)` | `changed, v` |
//! | `ProgressBar(ctx, fraction, w?, h?, overlay?)` | — |
//! | `SameLine(ctx, offset?, spacing?)`, `Spacing`, `Separator(ctx)` | — |
//! | `PushID(ctx, id)`, `PopID(ctx)` | — |
//! | `IsItemHovered(ctx)`, `IsItemActive(ctx)` | the last item's |
//! | `SetNextWindowSize(ctx, w, h, cond?)`, `SetNextWindowPos(ctx, x, y, cond?)` | — |
//! | `PushStyleColor(ctx, idx, rgba)`, `PopStyleColor(ctx, count?)` | — (a colour of the style, over the theme's, until popped) |
//! | `WindowFlags_*`, `Cond_*`, `Col_*` | Dear `ImGui`'s values |
//!
//! The windows' style follows the app's theme ([`Session::set_theme`],
//! `Style::from_theme`), and a script reads the theme itself through
//! `theme` (`architect_scripting::theme`): `theme.color('brand')` is a
//! colour `TextColored` and `PushStyleColor` take.
//!
//! `Canvas` draws over a view, in its coordinates:
//!
//! | `Canvas.` | gives |
//! |---|---|
//! | `Layer(view, plane?, id?)` | a layer over views of kind `view`, on `under`, `over` (the default) or `top` |
//! | `Spaces(layer)` | `{width, height, time = {start, pixels_per_second}, lanes = {{id, name, top, height}}}`, `nil` while the view is not shown |
//! | `TimeToX(layer, t)`, `XToTime(layer, x)` | across the view; `nil` with no time axis |
//! | `Lane(layer, id_or_name)` | `top, height`, or `nil` |
//! | `Line(layer, x1, y1, x2, y2, rgba, thickness?)` | — |
//! | `Rect(layer, x, y, w, h, rgba, filled?, rounding?)`, `Circle(layer, x, y, r, rgba, filled?)` | — |
//! | `Text(layer, x, y, text, rgba?, size?)` | — |
//! | `Region(layer, id, x, y, w, h)` | `hovered, clicked, held, context`: claims it, so it takes the pointer |
//! | `Clear(layer)` | — |
//!
//! A frame that draws into a layer draws it afresh; a frame that does
//! not leaves it as it was, so a script redraws a layer when what it
//! shows changes (the view's spaces, the pointer over a region).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use architect::action::ActionCaller;
use architect_scripting::luau::mlua::{self, Lua, UserData, UserDataRef, Value as LuaValue};
use architect_scripting::luau::{Live, Module, Report, Runtime, raise};
use architect_scripting::theme::{ThemeCell, ThemeModule};
use architect_theme::Resolved;
use architect_workspace::ViewMeter;
use serde_json::Value;

use crate::geometry::{Color, Point, Rect, Size};
use crate::imgui::{Col, Input, Style, Ui, UiError, cond, window_flags};
use crate::layer::{LayerId, Overlays, Plane, ViewPointer};
use crate::space::Spaces;
use crate::{Output, Shape};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One script's UI: its windows and the labels of its contexts.
#[derive(Debug, Default)]
struct Windows {
    ui: Ui,
    contexts: Vec<String>,
}

/// An `ImGui` context, as a script holds it.
struct Context(String);

impl UserData for Context {}

/// A layer, as a script holds it.
struct LayerRef(LayerId);

impl UserData for LayerRef {}

/// `ImGui`, over one [`Ui`].
#[derive(Clone)]
pub struct ImGuiModule {
    windows: Arc<Mutex<Windows>>,
}

/// `Canvas`, over one [`Overlays`].
#[derive(Clone)]
pub struct CanvasModule {
    overlays: Arc<Mutex<Overlays>>,
}

fn ui_error(lua: &Lua, e: &UiError) -> mlua::Error {
    raise(lua, e.to_string())
}

/// Sets `name` on `table` to a function of `f`.
fn set_fn<A, R>(
    lua: &Lua,
    table: &mlua::Table,
    name: &str,
    f: impl Fn(&Lua, A) -> mlua::Result<R> + Send + Sync + 'static,
) -> mlua::Result<()>
where
    A: mlua::FromLuaMulti,
    R: mlua::IntoLuaMulti,
{
    table.set(name, lua.create_function(f)?)
}

impl ImGuiModule {
    /// A UI widget call: `f` on the UI, its refusal raised in the script.
    fn widget<A, R>(
        &self,
        lua: &Lua,
        table: &mlua::Table,
        name: &str,
        f: impl Fn(&mut Ui, &str, A) -> Result<R, UiError> + Send + Sync + 'static,
    ) -> mlua::Result<()>
    where
        A: mlua::FromLuaMulti,
        R: mlua::IntoLuaMulti,
    {
        let windows = Arc::clone(&self.windows);
        set_fn(
            lua,
            table,
            name,
            move |lua, (ctx, args): (UserDataRef<Context>, A)| {
                let mut w = lock(&windows);
                f(&mut w.ui, &ctx.0, args).map_err(|e| ui_error(lua, &e))
            },
        )
    }
}

impl Module for ImGuiModule {
    fn global(&self) -> &'static str {
        "ImGui"
    }

    fn path(&self) -> &'static str {
        "imgui"
    }

    #[allow(clippy::too_many_lines)]
    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        let t = lua.create_table()?;
        let windows = Arc::clone(&self.windows);
        set_fn(
            lua,
            &t,
            "CreateContext",
            move |_, (label, _flags): (String, Option<u32>)| {
                let mut w = lock(&windows);
                // Two contexts of one label each get their own windows.
                let taken = w
                    .contexts
                    .iter()
                    .filter(|c| c.split('#').next() == Some(&label))
                    .count();
                let name = if taken == 0 {
                    label
                } else {
                    format!("{label}#{}", taken.saturating_add(1))
                };
                w.contexts.push(name.clone());
                drop(w);
                Ok(Context(name))
            },
        )?;
        self.widget(
            lua,
            &t,
            "Begin",
            |ui, ctx, (name, open, flags): (String, Option<bool>, Option<u32>)| {
                if open == Some(false) {
                    return Ok((false, Some(false)));
                }
                let (visible, now) = ui.begin(ctx, &name, open.is_some(), flags.unwrap_or(0));
                Ok((visible, open.map(|_| now)))
            },
        )?;
        self.widget(lua, &t, "End", |ui, _, (): ()| {
            ui.end();
            Ok(())
        })?;
        self.widget(lua, &t, "Text", |ui, _, text: String| ui.text(&text))?;
        self.widget(lua, &t, "TextDisabled", |ui, _, text: String| {
            ui.text_disabled(&text)
        })?;
        self.widget(
            lua,
            &t,
            "TextColored",
            |ui, _, (rgba, text): (u32, String)| ui.text_colored(Color(rgba), &text),
        )?;
        self.widget(
            lua,
            &t,
            "Button",
            |ui, _, (label, w, h): (String, Option<f32>, Option<f32>)| {
                ui.button(&label, Size::new(w.unwrap_or(0.0), h.unwrap_or(0.0)))
            },
        )?;
        self.widget(lua, &t, "Checkbox", |ui, _, (label, v): (String, bool)| {
            ui.checkbox(&label, v)
        })?;
        self.widget(
            lua,
            &t,
            "SliderDouble",
            |ui, _, (label, v, min, max, format): (String, f64, f64, f64, Option<String>)| {
                ui.slider_double(&label, v, min, max, format.as_deref().unwrap_or(""))
            },
        )?;
        self.widget(
            lua,
            &t,
            "SliderInt",
            |ui, _, (label, v, min, max, format): (String, i64, i64, i64, Option<String>)| {
                ui.slider_int(&label, v, min, max, format.as_deref().unwrap_or(""))
            },
        )?;
        self.widget(
            lua,
            &t,
            "ProgressBar",
            |ui, _, (fraction, w, h, overlay): (f32, Option<f32>, Option<f32>, Option<String>)| {
                ui.progress_bar(
                    fraction,
                    Size::new(w.unwrap_or(0.0), h.unwrap_or(0.0)),
                    overlay.as_deref().unwrap_or(""),
                )
            },
        )?;
        self.widget(
            lua,
            &t,
            "SameLine",
            |ui, _, (offset, spacing): (Option<f32>, Option<f32>)| {
                ui.same_line(offset.unwrap_or(0.0), spacing.unwrap_or(-1.0))
            },
        )?;
        self.widget(lua, &t, "Spacing", |ui, _, (): ()| ui.spacing())?;
        self.widget(lua, &t, "Separator", |ui, _, (): ()| ui.separator())?;
        self.widget(lua, &t, "PushID", |ui, _, id: LuaValue| {
            ui.push_id(&id.to_string().unwrap_or_default())
        })?;
        self.widget(lua, &t, "PopID", |ui, _, (): ()| ui.pop_id())?;
        self.widget(lua, &t, "IsItemHovered", |ui, _, _flags: Option<u32>| {
            Ok(ui.is_item_hovered())
        })?;
        self.widget(lua, &t, "IsItemActive", |ui, _, (): ()| {
            Ok(ui.is_item_active())
        })?;
        self.widget(
            lua,
            &t,
            "SetNextWindowSize",
            |ui, _, (w, h, c): (f32, f32, Option<u32>)| {
                ui.set_next_window_size(Size::new(w, h), c.unwrap_or(0));
                Ok(())
            },
        )?;
        self.widget(
            lua,
            &t,
            "SetNextWindowPos",
            |ui, _, (x, y, c): (f32, f32, Option<u32>)| {
                ui.set_next_window_pos(Point::new(x, y), c.unwrap_or(0));
                Ok(())
            },
        )?;
        self.widget(
            lua,
            &t,
            "PushStyleColor",
            |ui, _, (idx, rgba): (u32, u32)| {
                // An index this UI does not draw is taken and left alone.
                if let Some(col) = Col::from_index(idx) {
                    ui.push_style_color(col, Color(rgba));
                }
                Ok(())
            },
        )?;
        self.widget(lua, &t, "PopStyleColor", |ui, _, count: Option<usize>| {
            ui.pop_style_color(count.unwrap_or(1))
        })?;
        for col in Col::ALL {
            t.set(format!("Col_{}", col.name()), col.index())?;
        }
        for (name, value) in [
            ("WindowFlags_None", window_flags::NONE),
            ("WindowFlags_NoTitleBar", window_flags::NO_TITLE_BAR),
            ("WindowFlags_NoResize", window_flags::NO_RESIZE),
            ("WindowFlags_NoMove", window_flags::NO_MOVE),
            ("WindowFlags_NoScrollbar", window_flags::NO_SCROLLBAR),
            ("WindowFlags_NoCollapse", window_flags::NO_COLLAPSE),
            (
                "WindowFlags_AlwaysAutoResize",
                window_flags::ALWAYS_AUTO_RESIZE,
            ),
            ("WindowFlags_NoBackground", window_flags::NO_BACKGROUND),
            ("Cond_Always", cond::ALWAYS),
            ("Cond_Once", cond::ONCE),
            ("Cond_FirstUseEver", cond::FIRST_USE_EVER),
            ("Cond_Appearing", cond::APPEARING),
        ] {
            t.set(name, value)?;
        }
        // `require 'imgui' '0.10'`: the module called with a version is
        // itself (one API, whatever version a script was written for).
        let meta = lua.create_table()?;
        meta.set(
            "__call",
            lua.create_function(|_, (this, _version): (LuaValue, Option<String>)| Ok(this))?,
        )?;
        t.set_metatable(Some(meta))?;
        Ok(LuaValue::Table(t))
    }
}

impl CanvasModule {
    /// A call on a layer: `f` with the overlays and the layer.
    fn on_layer<A, R>(
        &self,
        lua: &Lua,
        table: &mlua::Table,
        name: &str,
        f: impl Fn(&Lua, &mut Overlays, LayerId, A) -> mlua::Result<R> + Send + Sync + 'static,
    ) -> mlua::Result<()>
    where
        A: mlua::FromLuaMulti,
        R: mlua::IntoLuaMulti,
    {
        let overlays = Arc::clone(&self.overlays);
        set_fn(
            lua,
            table,
            name,
            move |lua, (layer, args): (UserDataRef<LayerRef>, A)| {
                f(lua, &mut lock(&overlays), layer.0, args)
            },
        )
    }

    /// A drawing call: `shape` of the arguments, onto the layer.
    fn shape<A>(
        &self,
        lua: &Lua,
        table: &mlua::Table,
        name: &str,
        shape: impl Fn(A) -> Shape + Send + Sync + 'static,
    ) -> mlua::Result<()>
    where
        A: mlua::FromLuaMulti,
    {
        self.on_layer(lua, table, name, move |_, o, layer, args: A| {
            if let Some(draw) = o.draw(layer) {
                draw.push(shape(args));
            }
            Ok(())
        })
    }
}

impl Module for CanvasModule {
    fn global(&self) -> &'static str {
        "Canvas"
    }

    fn path(&self) -> &'static str {
        "canvas"
    }

    fn install(&self, lua: &Lua) -> mlua::Result<LuaValue> {
        let t = lua.create_table()?;
        let overlays = Arc::clone(&self.overlays);
        set_fn(
            lua,
            &t,
            "Layer",
            move |lua, (view, plane, id): (String, Option<String>, Option<String>)| {
                let plane = match plane.as_deref() {
                    None => Plane::Over,
                    Some(name) => Plane::parse(name).ok_or_else(|| {
                        raise(lua, format!("{name:?} is not a plane (under, over, top)"))
                    })?,
                };
                let id = id.unwrap_or_else(|| "layer".to_owned());
                Ok(LayerRef(lock(&overlays).layer(&id, &view, plane)))
            },
        )?;
        self.on_layer(lua, &t, "Spaces", |lua, o, layer, (): ()| {
            o.view_of(layer)
                .and_then(|(_, s)| s)
                .map_or(Ok(LuaValue::Nil), |spaces| {
                    spaces_table(lua, spaces).map(LuaValue::Table)
                })
        })?;
        self.on_layer(lua, &t, "TimeToX", |_, o, layer, t: f64| {
            Ok(time_axis(o, layer).map(|axis| axis.x(t)))
        })?;
        self.on_layer(lua, &t, "XToTime", |_, o, layer, x: f32| {
            Ok(time_axis(o, layer).map(|axis| axis.time(x)))
        })?;
        self.on_layer(lua, &t, "Lane", |_, o, layer, key: String| {
            let lane = o
                .view_of(layer)
                .and_then(|(_, s)| s)
                .and_then(|s| s.lane(&key))
                .map(|l| (l.top, l.height));
            Ok(lane.map_or((None, None), |(top, h)| (Some(top), Some(h))))
        })?;
        self.shape(
            lua,
            &t,
            "Line",
            |(x1, y1, x2, y2, rgba, thickness): (f32, f32, f32, f32, u32, Option<f32>)| {
                Shape::Line {
                    from: Point::new(x1, y1),
                    to: Point::new(x2, y2),
                    color: Color(rgba),
                    thickness: thickness.unwrap_or(1.0),
                }
            },
        )?;
        self.shape(
            lua,
            &t,
            "Rect",
            |(x, y, w, h, rgba, filled, rounding): (
                f32,
                f32,
                f32,
                f32,
                u32,
                Option<bool>,
                Option<f32>,
            )| {
                Shape::Rect {
                    rect: Rect::new(x, y, w, h),
                    color: Color(rgba),
                    rounding: rounding.unwrap_or(0.0),
                    fill: filled.unwrap_or(true),
                    thickness: 1.0,
                }
            },
        )?;
        self.shape(
            lua,
            &t,
            "Circle",
            |(x, y, r, rgba, filled): (f32, f32, f32, u32, Option<bool>)| Shape::Circle {
                center: Point::new(x, y),
                radius: r,
                color: Color(rgba),
                fill: filled.unwrap_or(true),
                thickness: 1.0,
            },
        )?;
        self.shape(
            lua,
            &t,
            "Text",
            |(x, y, text, rgba, size): (f32, f32, String, Option<u32>, Option<f32>)| Shape::Text {
                at: Point::new(x, y),
                text,
                color: rgba.map_or(Color::WHITE, Color),
                size: size.unwrap_or(12.0),
            },
        )?;
        self.on_layer(
            lua,
            &t,
            "Region",
            |_, o, layer, (id, x, y, w, h): (String, f32, f32, f32, f32)| {
                let s = o.region(layer, &id, Rect::new(x, y, w, h));
                Ok((s.hovered, s.clicked, s.held, s.context))
            },
        )?;
        self.on_layer(lua, &t, "Clear", |_, o, layer, (): ()| {
            o.clear(layer);
            Ok(())
        })?;
        Ok(LuaValue::Table(t))
    }
}

fn time_axis(o: &Overlays, layer: LayerId) -> Option<crate::TimeAxis> {
    o.view_of(layer)?.1?.time
}

fn spaces_table(lua: &Lua, s: &Spaces) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    t.set("width", s.width)?;
    t.set("height", s.height)?;
    if let Some(time) = s.time {
        let axis = lua.create_table()?;
        axis.set("start", time.start)?;
        axis.set("pixels_per_second", time.pixels_per_second)?;
        axis.set("finish", time.time(s.width))?;
        t.set("time", axis)?;
    }
    let lanes = lua.create_table()?;
    for lane in &s.lanes {
        let l = lua.create_table()?;
        l.set("id", lane.id.as_str())?;
        l.set("name", lane.name.as_str())?;
        l.set("top", lane.top)?;
        l.set("height", lane.height)?;
        lanes.push(l)?;
    }
    t.set("lanes", lanes)?;
    Ok(t)
}

/// A live script with `ImGui` and `Canvas`: its windows and its layers,
/// a frame at a time.
///
/// The host publishes what its views show ([`Self::publish`]) and the
/// pointer over them ([`Self::pointer`]), and asks for a frame when
/// something changed — a pointer press or release, a view scrolled, a
/// pane resized ([`Self::frame`]), a worker's news ([`Self::woken`]),
/// an animation's next step ([`Self::wake_after`]) — not on a timer.
///
/// Its script runs on a thread of its own (`architect_scripting::luau`):
/// a frame is awaited, never run on the host's thread, so a script that
/// runs long keeps the host's last good frame on screen and the app
/// answering ([`Self::frame_metered`]).
pub struct Session {
    live: Live,
    windows: Arc<Mutex<Windows>>,
    overlays: Arc<Mutex<Overlays>>,
    output: Output,
    /// The theme its script reads (`theme`) and its windows are styled by.
    theme: ThemeCell,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("live", &self.live)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts `source` on `runtime` (its gate, its transactions) as
    /// `caller`, with `ImGui` and `Canvas`. Its first run is a frame: what
    /// it draws then is [`Self::output`]. `None` when that run failed.
    pub async fn start(
        runtime: &Runtime,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
    ) -> (Report, Option<Self>) {
        Self::launch(runtime, caller, source, args, step, None).await
    }

    /// [`Self::start`], its windows styled by `theme` and its script
    /// reading it (`theme`) from the first frame on; then
    /// [`Self::set_theme`] on every change.
    pub async fn start_themed(
        runtime: &Runtime,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
        theme: &Resolved,
    ) -> (Report, Option<Self>) {
        Self::launch(runtime, caller, source, args, step, Some(theme)).await
    }

    async fn launch(
        runtime: &Runtime,
        caller: &ActionCaller,
        source: &str,
        args: Value,
        step: &str,
        theme: Option<&Resolved>,
    ) -> (Report, Option<Self>) {
        let windows = Arc::new(Mutex::new(Windows::default()));
        let overlays = Arc::new(Mutex::new(Overlays::new()));
        let cell = ThemeCell::new(theme.cloned().unwrap_or_default());
        if let Some(theme) = theme {
            lock(&windows).ui.set_style(Style::from_theme(theme));
        }
        let runtime = runtime
            .clone()
            .with_module(Arc::new(ImGuiModule {
                windows: Arc::clone(&windows),
            }))
            .with_module(Arc::new(CanvasModule {
                overlays: Arc::clone(&overlays),
            }))
            .with_module(Arc::new(ThemeModule::new(cell.clone())));
        begin(&windows, &overlays, Input::default());
        let (report, live) = runtime.start(caller, source, args, step).await;
        let output = end(&windows, &overlays);
        let session = live.map(|live| Self {
            live,
            windows,
            overlays,
            output,
            theme: cell,
        });
        (report, session)
    }

    /// Runs one frame with `input` (the pointer over its windows, their
    /// panes' sizes, the ones closed): what it deferred runs, and what it
    /// draws is [`Self::output`].
    pub async fn frame(&mut self, input: Input) -> Report {
        begin(&self.windows, &self.overlays, input);
        let report = self.live.frame().await;
        self.output = end(&self.windows, &self.overlays);
        report
    }

    /// What the last frame drew.
    #[must_use]
    pub const fn output(&self) -> &Output {
        &self.output
    }

    /// [`Self::frame`], measured by `meter`: once the frame has run past
    /// the meter's budget, `busy` is called and the meter is busy until
    /// it ends (its host keeps showing the last good frame, marked busy).
    /// The frame runs on the script's thread; this only waits. What the
    /// meter records is the frame's time on that thread
    /// ([`Live::last_frame_time`]), not the host's wait to hear of it.
    pub async fn frame_metered(
        &mut self,
        input: Input,
        meter: &ViewMeter,
        busy: impl FnOnce(),
    ) -> Report {
        use futures::future::{Either, select};
        let budget = meter.budget();
        let report = {
            let frame = std::pin::pin!(self.frame(input));
            match select(frame, futures_timer::Delay::new(budget)).await {
                Either::Left((report, _)) => report,
                Either::Right(((), frame)) => {
                    meter.set_busy(true);
                    busy();
                    frame.await
                }
            }
        };
        meter.record(self.live.last_frame_time());
        report
    }

    /// Resolves when its script asks for a frame with no input: one of
    /// its workers posted news ([`Live::woken`]).
    pub async fn woken(&self) {
        self.live.woken().await;
    }

    /// The frame its script asked for with no input (`wake_after(ms)`,
    /// an animation's next step): how soon, taking the ask.
    #[must_use]
    pub fn wake_after(&self) -> Option<Duration> {
        self.live.wake_after()
    }

    /// Whether it has a next frame: false once it stops deferring.
    #[must_use]
    pub fn running(&self) -> bool {
        self.live.running()
    }

    /// Stops it.
    pub fn stop(&self) {
        self.live.stop();
    }

    /// A view of kind `view` shows with `spaces` (`None`: it is gone).
    pub fn publish(&self, view: &str, spaces: Option<Spaces>) {
        lock(&self.overlays).publish(view, spaces);
    }

    /// The pointer over `view`'s layers.
    pub fn pointer(&self, view: &str, pointer: ViewPointer) {
        lock(&self.overlays).pointer(view, pointer);
    }

    /// Its windows' look, as given (what the script pushed is dropped).
    pub fn set_style(&self, style: Style) {
        lock(&self.windows).ui.set_style(style);
    }

    /// The app's theme changed: its windows restyled
    /// ([`Style::from_theme`]) and its script's `theme` moved on, from the
    /// next frame (the host asks for one). False when it is the theme it
    /// had.
    #[allow(clippy::must_use_candidate)] // A courtesy most callers ignore.
    pub fn set_theme(&self, theme: &Resolved) -> bool {
        if !self.theme.set(theme) {
            return false;
        }
        self.set_style(Style::from_theme(theme));
        true
    }
}

fn begin(windows: &Mutex<Windows>, overlays: &Mutex<Overlays>, input: Input) {
    lock(windows).ui.begin_frame(input);
    lock(overlays).begin_frame();
}

fn end(windows: &Mutex<Windows>, overlays: &Mutex<Overlays>) -> Output {
    Output {
        windows: lock(windows).ui.end_frame(),
        layers: lock(overlays).end_frame(),
    }
}
