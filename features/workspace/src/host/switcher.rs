//! A phone's tab switcher, Obsidian's on iOS: a grid of cards (each a
//! live preview of its pane, scaled down, or its view's icon), the title
//! under each, a ✕ on each, the current one ringed in the accent; under
//! them a bar of `+` (a new tab), "N tabs ⌄" (a menu) and Done.
//!
//! Every move is on `architect_shell::motion`: opening zooms out from the
//! pane on show into its card (`Switch::zoom`, 0 the pane, 1 the grid),
//! choosing a card zooms into it, a closed card's neighbours slide
//! together (each card's place is a pair of motions following its slot),
//! a card swiped sideways is held by the finger and flung or sprung back,
//! a card held still is lifted and dragged to another's place, and the
//! grid scrolls under the finger and coasts.
//!
//! Inside a shell it covers the whole screen (`use_shell_cover`), its bars
//! too; elsewhere it covers the host. Which tabs it shows is a [`Place`]'s
//! (`crate::switcher`): the main window's, a dock's or a pop-out's, picked
//! from the menu.

use std::collections::HashMap;

use architect_shell::motion::{Motion, Spring};
use architect_shell::{Icon, IconFn};
use dioxus::prelude::*;
use lucide_dioxus::{Check, ChevronDown, File, Pin, Plus, X};

use super::group::PaneSlot;
use super::{Host, LONG_PRESS_MS, Mount, Op, narrow, point, use_host};
use crate::model::{Area, PaneId, Rect, Workspace};
use crate::switcher::{Card, CardGrid, Place, reorder_dest};

/// A phone's own bar over its pane, when the host draws it (points).
pub(super) const PHONE_HEADER: f32 = 56.0;
/// The switcher's bar at the foot, over the home indicator (points).
const BAR: f32 = 52.0;
/// A place's name over its grid, for any place but the main window's.
const HEAD: f32 = 34.0;
/// How far a press moves before it is a scroll or a swipe, points.
const SLOP: f32 = 8.0;
/// A card's corners, in the grid.
const RADIUS: f32 = 12.0;
/// A swipe this far across its card (or this fast, points a second)
/// closes it.
const SWIPE_SHARE: f32 = 0.45;
const FLING: f64 = 900.0;
/// How many frames the layer's box is waited for.
const LAYER_TRIES: u32 = 30;

/// Where the switcher is drawn, as it last was: its layer's size and
/// place in the window, the safe area, and the pane on show (in the
/// layer), which the zoom goes from and to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct Geo {
    w: f32,
    h: f32,
    ox: f32,
    oy: f32,
    top: f32,
    bottom: f32,
    pane: Rect,
}

impl Geo {
    fn grid(&self) -> CardGrid {
        CardGrid::new(self.w, self.pane.w, self.pane.h)
    }

    fn bar_h(&self) -> f32 {
        BAR + self.bottom
    }

    /// The grid's top in the layer: under the status bar, and the place's
    /// name when it is not the main window's.
    fn grid_top(&self, place: Place) -> f32 {
        self.top + if place == Place::Main { 0.0 } else { HEAD }
    }

    /// How much of the grid shows at once.
    fn view_h(&self, place: Place) -> f32 {
        (self.h - self.grid_top(place) - self.bar_h()).max(0.0)
    }

    /// How far the grid scrolls.
    fn max_scroll(&self, place: Place, n: usize) -> f32 {
        (self.grid().height(n) - self.view_h(place)).max(0.0)
    }

    /// A window point in the layer.
    fn local(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (x - self.ox, y - self.oy)
    }
}

/// A press in the switcher, until it is let go.
#[derive(Debug, Clone, PartialEq)]
struct Press {
    /// The card pressed (`None`: the space between them).
    pane: Option<PaneId>,
    closable: bool,
    /// Its index among the place's cards.
    index: usize,
    x0: f32,
    y0: f32,
    scroll0: f32,
    /// The finger from the card's top left, in the grid.
    grab: (f32, f32),
    stroke: Stroke,
    serial: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stroke {
    Undecided,
    Scroll,
    Swipe,
    Lift,
}

/// A card lifted to be moved: where it came from and where it would go
/// (indices into the place's cards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Lifted {
    pane: PaneId,
    from: usize,
    to: usize,
}

/// The bar's menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SwitchMenu {
    /// "N tabs ⌄": the places, the screensets, closing.
    Tabs,
    /// `+`: which view the new tab shows.
    New,
}

/// The switcher's state, in the [`Host`] (made in the host's scope).
#[derive(Clone, Copy)]
pub(super) struct Switch {
    /// 0: the pane fills its place; 1: the grid.
    zoom: Motion,
    /// The grid's scroll, points.
    scroll: Motion,
    /// The card the zoom goes out of or into.
    focus: Signal<Option<PaneId>>,
    place: Signal<Place>,
    geo: CopyValue<Geo>,
    /// The layer's own box in the window, once measured (a shell's box
    /// may start under an in-window menu bar).
    layer: Signal<Option<Rect>>,
    /// Each card's place in the grid as it moves, by pane.
    cards: CopyValue<HashMap<PaneId, (Motion, Motion)>>,
    /// The card swiped sideways, and how far it is.
    swiped: Signal<Option<PaneId>>,
    swipe: Motion,
    /// A card going from its ✕, and how much of it is left (1–0).
    closing: Signal<Option<PaneId>>,
    fade: Motion,
    lifted: Signal<Option<Lifted>>,
    menu: Signal<Option<SwitchMenu>>,
    press: CopyValue<Option<Press>>,
    serial: CopyValue<u64>,
    /// Counts the switcher's closes: the phone's pane is drawn afresh
    /// after each (a pane mounted while the layer was over it was laid
    /// out at its narrowest by Blitz, and stayed so).
    pub(super) closes: Signal<u64>,
}

impl Switch {
    /// Made in the host's own scope.
    pub(super) fn new() -> Self {
        Self {
            zoom: Motion::new(0.0, Spring::PANEL, 0.001),
            scroll: Motion::new(0.0, Spring::PANEL, 0.5),
            focus: Signal::new(None),
            place: Signal::new(Place::Main),
            geo: CopyValue::new(Geo::default()),
            layer: Signal::new(None),
            cards: CopyValue::new(HashMap::new()),
            swiped: Signal::new(None),
            swipe: Motion::new(0.0, Spring::SNAPPY, 0.5),
            closing: Signal::new(None),
            fade: Motion::new(1.0, Spring::SNAPPY, 0.01),
            lifted: Signal::new(None),
            menu: Signal::new(None),
            press: CopyValue::new(None),
            serial: CopyValue::new(0),
            closes: Signal::new(0),
        }
    }
}

/// The pane a phone shows: the active one if it is in the center, else
/// the current document.
pub(super) fn phone_pane(w: &Workspace) -> Option<PaneId> {
    w.active
        .clone()
        .filter(|p| w.locate(p).is_some_and(|l| l.area == Area::Center))
        .or_else(|| w.current_document().cloned())
}

/// Readies the grid for opening: the pane on show's place, scrolled to
/// its card.
fn prepare(host: Host) {
    let s = host.switch;
    let (current, place, at, n) = {
        let w = host.workspace.peek();
        let current = phone_pane(&w);
        let place = current
            .as_ref()
            .and_then(|p| w.place_of(p))
            .unwrap_or(Place::Main);
        let cards = w.cards(place);
        let at = current
            .as_ref()
            .and_then(|c| cards.iter().position(|k| &k.pane == c));
        (current, place, at, cards.len())
    };
    let (mut focus, mut where_, mut menu, mut lifted) = (s.focus, s.place, s.menu, s.lifted);
    focus.set(current);
    where_.set(place);
    menu.set(None);
    lifted.set(None);
    scroll_to(host, place, at, n);
}

/// Scrolls the grid so card `at` (of `n`) is in the middle, as far as it
/// goes.
fn scroll_to(host: Host, place: Place, at: Option<usize>, n: usize) {
    let s = host.switch;
    let geo = *s.geo.peek();
    let grid = geo.grid();
    let to = at.map_or(0.0, |i| {
        let r = grid.rect(i);
        r.y + r.h / 2.0 - geo.view_h(place) / 2.0
    });
    s.scroll
        .jump(f64::from(to.clamp(0.0, geo.max_scroll(place, n))));
}

/// Zooms into the focus card, then closes; `then` runs once closed.
fn zoom_in(host: Host, then: impl FnOnce() + 'static) {
    let s = host.switch;
    let mut then = Some(then);
    let mut switcher = host.switcher;
    let mut menu = s.menu;
    menu.set(None);
    s.zoom.to_then(0.0, move || {
        switcher.set(false);
        closed(host);
        if let Some(then) = then.take() {
            then();
        }
    });
}

/// The layer has gone: the phone's pane is drawn afresh, a frame on (in
/// a layout the layer is no longer in).
fn closed(host: Host) {
    spawn(async move {
        architect_shell::motion::next_frame().await;
        let mut closes = host.switch.closes;
        if let Ok(n) = closes.try_peek().map(|n| n.wrapping_add(1)) {
            closes.set(n);
        }
    });
}

/// Done: back into the pane on show.
fn done(host: Host) {
    let s = host.switch;
    let current = phone_pane(&host.workspace.peek());
    let mut focus = s.focus;
    focus.set(current);
    zoom_in(host, || {});
}

/// A card chosen: it is shown (a docked one as a sheet) and the switcher
/// zooms into it.
fn choose(host: Host, pane: PaneId) {
    let s = host.switch;
    let docked = host
        .workspace
        .peek()
        .locate(&pane)
        .is_some_and(|l| matches!(l.area, Area::Dock(_)));
    let mut focus = s.focus;
    if docked {
        focus.set(None);
        let mut sheet = host.sheet;
        zoom_in(host, move || sheet.set(Some(pane)));
    } else {
        host.run(Op::Show(pane.clone()));
        focus.set(Some(pane));
        zoom_in(host, || {});
    }
}

/// A card's ✕: it fades, then closes; the others slide together.
fn close_card(host: Host, pane: PaneId) {
    let s = host.switch;
    let mut closing = s.closing;
    if closing.peek().is_some() {
        return;
    }
    closing.set(Some(pane.clone()));
    s.fade.jump(1.0);
    s.fade.to_then(0.0, move || {
        host.run(Op::Close(pane.clone()));
        closing.set(None);
        s.fade.jump(1.0);
    });
}

/// Measures the host again, then runs `then`: the zoom starts from the
/// pane's real box (a box measured as it mounts may have had no layout
/// yet, and nothing measures it again on iOS).
fn measure_then(host: Host, then: impl FnOnce() + 'static) {
    spawn(async move {
        if let Some(r) = host.rect(Mount::Host).await
            && r.w > 0.0
            && *host.float_area.peek() != Some(r)
        {
            let mut area = host.float_area;
            area.set(Some(r));
        }
        then();
    });
}

/// Measures the layer's own box, once it has one.
async fn measure_layer(host: Host) {
    for _ in 0..LAYER_TRIES {
        if let Some(r) = host.rect(Mount::Switcher).await
            && r.w > 0.0
            && r.h > 0.0
        {
            let mut layer = host.switch.layer;
            if *layer.peek() != Some(r) {
                layer.set(Some(r));
            }
            return;
        }
        architect_shell::motion::next_frame().await;
    }
}

/// Opens the switcher (the host's [`Host::show_switcher`], or the app's
/// own signal), and closes it when the app says so.
fn use_open_close(host: Host) {
    use_effect(move || {
        let open = (host.switcher)();
        let s = host.switch;
        if open {
            if s.zoom.target() < 1.0 {
                if s.zoom.peek() < 0.001 {
                    measure_then(host, move || {
                        prepare(host);
                        s.zoom.to(1.0);
                    });
                } else {
                    s.zoom.to(1.0);
                }
            }
        } else if s.zoom.target() > 0.0 {
            s.zoom.to_then(0.0, move || closed(host));
        }
    });
}

/// One card as drawn: what it shows.
#[derive(Clone, PartialEq)]
struct Shown {
    pane: PaneId,
    title: String,
    icon: Option<IconFn>,
    pinned: bool,
    /// Its view draws a live preview.
    live: bool,
}

/// The switcher, over the screen (`cover`: the shell's) or the host.
#[component]
pub(super) fn SwitcherLayer(cover: bool) -> Element {
    let host = use_host();
    let s = host.switch;
    let t = *host.theme.read();
    let shell = architect_shell::try_use_shell();
    let area = (host.float_area)().unwrap_or_default();
    let header = if (host.phone_header)() {
        PHONE_HEADER
    } else {
        0.0
    };
    let geo = match shell.filter(|_| cover) {
        Some(sh) => {
            // Its own box, else the screen's.
            let layer = (s.layer)().unwrap_or_else(|| {
                Rect::new(0.0, 0.0, narrow(sh.screen.width), narrow(sh.screen.height))
            });
            Geo {
                w: layer.w,
                h: layer.h,
                ox: layer.x,
                oy: layer.y,
                top: narrow(sh.screen.insets.top),
                bottom: narrow(sh.screen.insets.bottom),
                pane: Rect::new(
                    area.x - layer.x,
                    area.y - layer.y + header,
                    area.w,
                    area.h - header,
                ),
            }
        }
        None => Geo {
            w: area.w,
            h: area.h,
            ox: area.x,
            oy: area.y,
            top: 0.0,
            bottom: 0.0,
            pane: Rect::new(0.0, header, area.w, area.h - header),
        },
    };
    // Not measured yet: the layer's own box, so a preview is never laid
    // out at nothing.
    let geo = if geo.pane.w > 0.0 && geo.pane.h > 0.0 {
        geo
    } else {
        Geo {
            pane: Rect::new(0.0, geo.top, geo.w, geo.h - geo.top - geo.bottom),
            ..geo
        }
    };
    let mut kept = s.geo;
    if *kept.peek() != geo {
        kept.set(geo);
    }
    use_open_close(host);
    let open = (host.switcher)();
    let z = narrow(s.zoom.value()).clamp(0.0, 1.0);
    if !open && z <= 0.001 {
        return rsx! {};
    }

    let place = (s.place)();
    let w = host.workspace.read();
    let cards = w.cards(place);
    let current = phone_pane(&w);
    let views = host.views.read();
    let shown: Vec<Shown> = cards
        .iter()
        .filter_map(|c| {
            let p = w.pane(&c.pane)?;
            let view = views.get(&p.view.0);
            Some(Shown {
                pane: c.pane.clone(),
                title: host.title(p),
                icon: view.and_then(|v| v.icon),
                pinned: p.pinned,
                live: view.is_some_and(|v| !v.still),
            })
        })
        .collect();
    drop(views);
    drop(w);
    let n = shown.len();

    // Lifted, a card is drawn where it would go and the others make room.
    let lifted = (s.lifted)();
    let mut order: Vec<usize> = (0..n).collect();
    if let Some(l) = lifted.as_ref().filter(|l| l.from < n) {
        let i = order.remove(l.from);
        order.insert(l.to.min(n.saturating_sub(1)), i);
    }
    let grid = geo.grid();
    let top = geo.grid_top(place);
    let view_h = geo.view_h(place);
    let scroll = narrow(s.scroll.value());
    let focus = (s.focus)();
    let lifted_pane = lifted.as_ref().map(|l| l.pane.clone());
    let swiped = (s.swiped)();
    let dx = narrow(s.swipe.value());
    let closing = (s.closing)();
    let fade = narrow(s.fade.value());
    let drawn: Vec<(usize, Shown, Rect)> = order
        .iter()
        .enumerate()
        .filter_map(|(slot, &i)| {
            let card = shown.get(i)?.clone();
            let r = grid.rect(slot);
            let near = r.y + r.h + grid.title_h >= scroll - grid.row_h()
                && r.y <= scroll + view_h + grid.row_h();
            let always =
                focus.as_ref() == Some(&card.pane) || lifted_pane.as_ref() == Some(&card.pane);
            (near || always).then_some((i, card, r))
        })
        .collect();

    let bar_h = geo.bar_h();
    let bar_top = (1.0 - z).mul_add(bar_h, geo.h - bar_h);
    let count = match n {
        1 => "1 tab".to_owned(),
        n => format!("{n} tabs"),
    };
    let menu = (s.menu)();

    let onpointerdown = move |e: PointerEvent| {
        e.stop_propagation();
        let (x, y) = geo.local(point(&e));
        let mut press = s.press;
        press.set(Some(Press {
            pane: None,
            closable: false,
            index: 0,
            x0: x,
            y0: y,
            scroll0: narrow(s.scroll.peek()),
            grab: (0.0, 0.0),
            stroke: Stroke::Undecided,
            serial: 0,
        }));
    };
    let onpointermove = move |e: PointerEvent| on_move(host, geo.local(point(&e)));
    let onpointerup = move |_: PointerEvent| on_up(host, true);
    let onpointercancel = move |_: PointerEvent| on_up(host, false);

    rsx! {
        div {
            "data-switcher": "open",
            style: "position:absolute; top:0; left:0; right:0; bottom:0; \
                    z-index:{architect_shell::layer::SHEET}; overflow:hidden; color:{t.ink}; font-family:{t.font};",
            onmounted: move |e| {
                host.mount(Mount::Switcher, e.data());
                if cover {
                    spawn(measure_layer(host));
                }
            },
            onpointerdown,
            onpointermove,
            onpointerup,
            onpointercancel,
            div { style: "position:absolute; top:0; left:0; right:0; bottom:0; background:{t.desk}; opacity:{z};" }
            for (i, card, slot) in drawn {
                CardView {
                    key: "{card.pane.0}",
                    current: current.as_ref() == Some(&card.pane),
                    zooming: focus.as_ref() == Some(&card.pane),
                    lifted: lifted_pane.as_ref() == Some(&card.pane),
                    dx: if swiped.as_ref() == Some(&card.pane) { dx } else { 0.0 },
                    fade: if closing.as_ref() == Some(&card.pane) { fade } else { 1.0 },
                    index: i,
                    card,
                    slot,
                    scroll,
                    top,
                    zoom: z,
                    geo,
                }
            }
            if n == 0 {
                div {
                    style: "position:absolute; left:0; right:0; top:{top + 80.0}px; text-align:center; \
                            color:{t.ink_3}; font-size:14px; line-height:1.6; opacity:{z};",
                    span { style: "color:{t.ink_2}; font-size:15px; font-weight:600;", "No open tabs" }
                    br {}
                    "Press + for a new one."
                }
            }
            // Over the status bar, and the place's name.
            div {
                style: "position:absolute; top:0; left:0; right:0; height:{top}px; z-index:3; background:{t.desk}; \
                        opacity:{z}; display:flex; align-items:flex-end; justify-content:center;",
                if place != Place::Main {
                    div { style: "height:{HEAD}px; display:flex; align-items:center; font-size:15px; font-weight:650;",
                        "{place.name()}"
                    }
                }
            }
            div {
                "data-switcher": "bar",
                style: "position:absolute; left:0; right:0; top:{bar_top}px; height:{bar_h}px; box-sizing:border-box; \
                        padding:0 8px {geo.bottom}px; z-index:3; display:flex; align-items:center; \
                        justify-content:space-between; background:{t.desk}; border-top:1px solid {t.rule};",
                BarButton { name: "new", label: "New tab", on: move |()| toggle_menu(host, SwitchMenu::New),
                    Plus { size: 22, color: t.ink }
                }
                BarButton { name: "tabs", label: "{count}, more", on: move |()| toggle_menu(host, SwitchMenu::Tabs),
                    span { style: "font-size:15px; font-weight:600;", "{count}" }
                    ChevronDown { size: 16, color: t.ink_2 }
                }
                BarButton { name: "done", label: "Done", on: move |()| done(host),
                    span { style: "font-size:16px; font-weight:650; color:{t.accent};", "Done" }
                }
            }
            if let Some(menu) = menu {
                SwitcherMenu { menu, bar_h, w: geo.w }
            }
        }
    }
}

fn toggle_menu(host: Host, which: SwitchMenu) {
    let mut menu = host.switch.menu;
    let now = *menu.peek();
    menu.set(if now == Some(which) {
        None
    } else {
        Some(which)
    });
}

/// A button on the switcher's bar.
#[component]
fn BarButton(
    name: &'static str,
    label: String,
    on: EventHandler<()>,
    children: Element,
) -> Element {
    let t = *use_host().theme.read();
    rsx! {
        button {
            "data-switcher": "{name}",
            "aria-label": "{label}",
            style: "min-width:44px; height:44px; padding:0 12px; display:flex; align-items:center; \
                    justify-content:center; gap:4px; border:none; border-radius:10px; \
                    background:transparent; color:{t.ink}; cursor:pointer;",
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |e| {
                e.stop_propagation();
                on.call(());
            },
            {children}
        }
    }
}

/// One card: its preview (the pane's own view scaled down, or its icon),
/// its title under it, its ✕; ringed when it is the pane on show.
#[component]
fn CardView(
    card: Shown,
    index: usize,
    /// Its place in the grid (the grid's coordinates).
    slot: Rect,
    scroll: f32,
    top: f32,
    zoom: f32,
    geo: Geo,
    current: bool,
    /// The zoom goes out of it or into it.
    zooming: bool,
    lifted: bool,
    dx: f32,
    fade: f32,
) -> Element {
    let host = use_host();
    let s = host.switch;
    let t = *host.theme.read();
    // Where it is in the grid, as it moves: it follows its slot.
    let pane = card.pane.clone();
    let place = use_hook(|| {
        let m = (
            Motion::new(f64::from(slot.x), Spring::SNAPPY, 0.25),
            Motion::new(f64::from(slot.y), Spring::SNAPPY, 0.25),
        );
        let mut cards = s.cards;
        cards.write().insert(pane.clone(), m);
        m
    });
    let gone = pane;
    use_drop(move || {
        let mut cards = s.cards;
        if let Ok(mut all) = cards.try_write()
            && all.get(&gone) == Some(&place)
        {
            all.remove(&gone);
        }
    });
    use_effect(use_reactive!(|slot, lifted| {
        // A lifted card is the finger's.
        if !lifted {
            let (x, y) = place;
            if (x.target() - f64::from(slot.x)).abs() > 0.1 {
                x.to(f64::from(slot.x));
            }
            if (y.target() - f64::from(slot.y)).abs() > 0.1 {
                y.to(f64::from(slot.y));
            }
        }
    }));
    let (gx, gy) = (narrow(place.0.value()), narrow(place.1.value()));
    let grid_frame = Rect::new(gx + dx, gy - scroll + top, slot.w, slot.h);
    // Zooming, it is between the pane's own place and its card.
    let z = if zooming { zoom } else { 1.0 };
    let frame = lerp(geo.pane, grid_frame, z);
    // Going from its ✕, it shrinks a little into its middle as it fades.
    let shrink = (1.0 - fade) * 0.08;
    let frame = Rect::new(
        frame.w.mul_add(shrink, frame.x),
        frame.h.mul_add(shrink, frame.y),
        frame.w * 2.0f32.mul_add(-shrink, 1.0),
        frame.h * 2.0f32.mul_add(-shrink, 1.0),
    );
    let radius = RADIUS * z;
    let scale = if geo.pane.w > 0.0 {
        frame.w / geo.pane.w
    } else {
        1.0
    };
    let swiped = (dx.abs() / slot.w.max(1.0)).min(1.0);
    let opacity = if zooming { 1.0 } else { zoom } * fade * 0.7f32.mul_add(-swiped, 1.0);
    let chrome = if zooming { zoom } else { 1.0 };
    let layer = if zooming || lifted { 2 } else { 1 };
    let lift_shadow = if lifted { t.shadow_float } else { "none" };
    let (pw, ph) = (geo.pane.w, geo.pane.h);
    let ring = 3.0_f32;
    let (ring_w, ring_h) = (
        2.0f32.mul_add(ring, frame.w + 2.0),
        2.0f32.mul_add(ring, frame.h + 2.0),
    );
    let named = card.pane.0.clone();
    let pressed = card.pane.clone();
    let closed = card.pane.clone();
    let closable = !card.pinned;
    let title = card.title.clone();
    let live = card.live;
    let icon = card.icon;
    rsx! {
        div {
            "data-card": "{named}",
            "aria-current": if current { "page" } else { "false" },
            style: "position:absolute; left:{frame.x}px; top:{frame.y}px; width:{frame.w}px; \
                    height:{frame.h + CardGrid::TITLE}px; opacity:{opacity}; z-index:{layer};",
            if current {
                div {
                    style: "position:absolute; left:{-ring - 1.0}px; top:{-ring - 1.0}px; \
                            width:{ring_w}px; height:{ring_h}px; \
                            box-sizing:border-box; border:{ring}px solid {t.accent}; \
                            border-radius:{radius + ring + 1.0}px; opacity:{chrome};",
                }
            }
            div {
                style: "position:absolute; left:0; top:0; width:{frame.w}px; height:{frame.h}px; \
                        box-sizing:border-box; border-radius:{radius}px; overflow:hidden; \
                        background:{t.desk}; border:1px solid {t.rule}; box-shadow:{lift_shadow};",
                if live {
                    // The pane's own view, laid out at the pane's size and
                    // scaled down (a scale is the same in device pixels).
                    div {
                        style: "position:absolute; left:0; top:0; width:{pw}px; height:{ph}px; \
                                transform-origin:0 0; transform:scale({scale}); display:flex; \
                                flex-direction:column; pointer-events:none;",
                        Preview { pane: card.pane }
                    }
                } else {
                    div {
                        style: "position:absolute; left:0; top:0; right:0; bottom:0; display:flex; \
                                align-items:center; justify-content:center; background:{t.sheet};",
                        {icon.map_or_else(
                            || rsx! { File { size: 40, color: t.ink_3 } },
                            |icon| icon(Icon { size: 40, color: t.ink_3 }),
                        )}
                    }
                }
            }
            // Presses land here, over the preview (whose own handlers
            // must not hear them).
            div {
                style: "position:absolute; left:0; top:0; width:{frame.w}px; \
                        height:{frame.h + CardGrid::TITLE}px; z-index:1;",
                onpointerdown: move |e| {
                    e.stop_propagation();
                    press_card(host, pressed.clone(), index, closable, point(&e));
                },
            }
            if closable {
                button {
                    "data-card-close": "{named}",
                    "aria-label": "Close {title}",
                    style: "position:absolute; top:6px; right:6px; width:26px; height:26px; padding:0; \
                            z-index:2; border:none; border-radius:13px; display:flex; align-items:center; \
                            justify-content:center; background:rgba(0,0,0,0.55); opacity:{chrome}; cursor:pointer;",
                    onpointerdown: move |e| e.stop_propagation(),
                    onclick: move |e| {
                        e.stop_propagation();
                        close_card(host, closed.clone());
                    },
                    X { size: 15, color: "#ffffff" }
                }
            } else {
                div {
                    "aria-label": "Pinned",
                    style: "position:absolute; top:6px; right:6px; width:26px; height:26px; z-index:2; \
                            border-radius:13px; display:flex; align-items:center; justify-content:center; \
                            background:rgba(0,0,0,0.55); opacity:{chrome};",
                    Pin { size: 13, color: "#ffffff" }
                }
            }
            div {
                style: "position:absolute; left:0; top:{frame.h + 8.0}px; width:{frame.w}px; \
                        text-align:center; white-space:nowrap; overflow:hidden; font-size:13px; \
                        color:{t.ink_2}; opacity:{chrome};",
                "{title}"
            }
        }
    }
}

/// Between `a` (at 0) and `b` (at 1).
fn lerp(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::new(
        (b.x - a.x).mul_add(t, a.x),
        (b.y - a.y).mul_add(t, a.y),
        (b.w - a.w).mul_add(t, a.w),
        (b.h - a.h).mul_add(t, a.h),
    )
}

/// A pane drawn by its view inside a card: [`use_pane`](super::use_pane)
/// says so ([`PaneHandle::is_preview`](super::PaneHandle::is_preview)),
/// and its status items stay the pane's own.
#[component]
fn Preview(pane: PaneId) -> Element {
    use_context_provider(|| super::Previewing);
    rsx! { PaneSlot { pane } }
}

/// A card pressed: a tap chooses it, a sideways stroke swipes it, an
/// upright one scrolls, a press held still lifts it.
fn press_card(host: Host, pane: PaneId, index: usize, closable: bool, at: (f32, f32)) {
    let s = host.switch;
    let geo = *s.geo.peek();
    let place = *s.place.peek();
    let (x, y) = geo.local(at);
    let scroll = narrow(s.scroll.peek());
    let (gx, gy) = (x, y - geo.grid_top(place) + scroll);
    let origin = s
        .cards
        .peek()
        .get(&pane)
        .map_or((gx, gy), |(mx, my)| (narrow(mx.peek()), narrow(my.peek())));
    let mut serial = s.serial;
    let n = serial.peek().saturating_add(1);
    serial.set(n);
    let mut press = s.press;
    press.set(Some(Press {
        pane: Some(pane.clone()),
        closable,
        index,
        x0: x,
        y0: y,
        scroll0: scroll,
        grab: (gx - origin.0, gy - origin.1),
        stroke: Stroke::Undecided,
        serial: n,
    }));
    spawn(async move {
        futures_timer::Delay::new(std::time::Duration::from_millis(LONG_PRESS_MS)).await;
        let mut press = s.press;
        let held = press
            .peek()
            .as_ref()
            .is_some_and(|p| p.serial == n && p.stroke == Stroke::Undecided);
        if held {
            if let Some(p) = press.write().as_mut() {
                p.stroke = Stroke::Lift;
            }
            let mut lifted = s.lifted;
            lifted.set(Some(Lifted {
                pane,
                from: index,
                to: index,
            }));
        }
    });
}

/// A finger moving in the switcher (layer points).
fn on_move(host: Host, (x, y): (f32, f32)) {
    let state = host.switch;
    let mut press = state.press;
    let Some(mut pressed) = press.peek().clone() else {
        return;
    };
    let (dx, dy) = (x - pressed.x0, y - pressed.y0);
    if pressed.stroke == Stroke::Undecided && dx.hypot(dy) >= SLOP {
        pressed.stroke = if dx.abs() > dy.abs() && pressed.closable && pressed.pane.is_some() {
            let mut swiped = state.swiped;
            swiped.set(pressed.pane.clone());
            Stroke::Swipe
        } else {
            Stroke::Scroll
        };
        press.set(Some(pressed.clone()));
    }
    let geo = *state.geo.peek();
    let place = *state.place.peek();
    match pressed.stroke {
        Stroke::Undecided => {}
        Stroke::Scroll => {
            let count = host.workspace.peek().cards(place).len();
            let max = geo.max_scroll(place, count);
            let to = pressed.scroll0 - dy;
            // Past an end it gives, half as far.
            let to = if to < 0.0 {
                to / 2.0
            } else if to > max {
                (to - max).mul_add(0.5, max)
            } else {
                to
            };
            state.scroll.hold(f64::from(to));
        }
        Stroke::Swipe => state.swipe.hold(f64::from(dx)),
        Stroke::Lift => {
            let Some(pane) = pressed.pane.clone() else {
                return;
            };
            let grid = geo.grid();
            let scroll = narrow(state.scroll.peek());
            let (gx, gy) = (
                x - pressed.grab.0,
                y - geo.grid_top(place) + scroll - pressed.grab.1,
            );
            if let Some((mx, my)) = state.cards.peek().get(&pane) {
                mx.hold(f64::from(gx));
                my.hold(f64::from(gy));
            }
            let count = host.workspace.peek().cards(place).len();
            let to = grid.index_at(gx + grid.card_w / 2.0, gy + grid.card_h / 2.0, count);
            let mut lifted = state.lifted;
            let moved = lifted.peek().as_ref().is_some_and(|l| l.to != to);
            if moved {
                if let Some(l) = lifted.write().as_mut() {
                    l.to = to;
                }
            }
        }
    }
}

/// The finger let go (`released`), or the press was cancelled.
fn on_up(host: Host, released: bool) {
    let s = host.switch;
    let mut press = s.press;
    let Some(p) = press.write().take() else {
        return;
    };
    let geo = *s.geo.peek();
    let place = *s.place.peek();
    match p.stroke {
        Stroke::Undecided => {
            if let (true, Some(pane)) = (released, p.pane) {
                // Only a grid that is there takes a tap.
                if s.zoom.target() >= 1.0 {
                    choose(host, pane);
                }
            }
        }
        Stroke::Scroll => {
            let n = host.workspace.peek().cards(place).len();
            s.scroll.coast(0.0, f64::from(geo.max_scroll(place, n)));
        }
        Stroke::Swipe => {
            let mut swiped = s.swiped;
            let at = s.swipe.peek();
            let v = s.swipe.velocity();
            let card_w = f64::from(geo.grid().card_w);
            let far = at.abs() > card_w * f64::from(SWIPE_SHARE);
            let flung = v.abs() > FLING && v.signum() == at.signum();
            match p.pane {
                Some(pane) if released && (far || flung) => {
                    let off = f64::from(geo.w).copysign(at);
                    s.swipe.to_then(off, move || {
                        host.run(Op::Close(pane.clone()));
                        swiped.set(None);
                        s.swipe.jump(0.0);
                    });
                }
                _ => s.swipe.to_then(0.0, move || swiped.set(None)),
            }
        }
        Stroke::Lift => drop_lifted(host),
    }
}

/// A lifted card let go: moved to where it was over, and sprung into its
/// slot from where the finger left it.
fn drop_lifted(host: Host) {
    let s = host.switch;
    let mut lifted = s.lifted;
    let Some(l) = lifted.peek().clone() else {
        return;
    };
    let geo = *s.geo.peek();
    let place = *s.place.peek();
    let cards: Vec<Card> = host.workspace.peek().cards(place);
    if let Some(dest) = reorder_dest(&cards, l.from, l.to) {
        let active = host.workspace.peek().active.clone();
        let _ = host.edit(|w| {
            let moved = w.move_pane(&l.pane, dest);
            // A reorder leaves the pane on show as it was.
            if let Some(a) = &active {
                let _ = w.activate(a);
            }
            moved
        });
    }
    let slot = geo.grid().rect(l.to);
    if let Some((mx, my)) = s.cards.peek().get(&l.pane) {
        mx.to(f64::from(slot.x));
        my.to(f64::from(slot.y));
    }
    lifted.set(None);
}

/// A row of the bar's menus.
#[derive(Clone, PartialEq)]
enum Row {
    Heading(String),
    Act {
        label: String,
        act: Act,
        checked: bool,
        danger: bool,
    },
}

#[derive(Clone, PartialEq)]
enum Act {
    Place(Place),
    Recall(u8),
    Reopen,
    CloseOthers,
    CloseAll,
    Open(String),
}

fn act(label: impl Into<String>, act: Act) -> Row {
    Row::Act {
        label: label.into(),
        act,
        checked: false,
        danger: false,
    }
}

/// The cards in the switcher's place that a close may take: every one but
/// the pinned.
fn closable(host: Host) -> Vec<PaneId> {
    let w = host.workspace.peek();
    w.cards(*host.switch.place.peek())
        .into_iter()
        .filter(|c| w.pane(&c.pane).is_some_and(|p| !p.pinned))
        .map(|c| c.pane)
        .collect()
}

fn tabs_rows(host: Host) -> Vec<Row> {
    let place = *host.switch.place.peek();
    let mut rows = Vec::new();
    let (places, counts, reopen) = {
        let w = host.workspace.peek();
        let places = w.places();
        let counts: Vec<usize> = places.iter().map(|p| w.cards(*p).len()).collect();
        (places, counts, !w.closed.is_empty())
    };
    if places.len() > 1 {
        rows.push(Row::Heading("Show".to_owned()));
        for (p, n) in places.into_iter().zip(counts) {
            rows.push(Row::Act {
                label: format!("{} · {n}", p.name()),
                act: Act::Place(p),
                checked: p == place,
                danger: false,
            });
        }
    }
    let sets = host.screensets.peek();
    let slotted: Vec<(u8, String, bool)> = sets
        .sets
        .iter()
        .filter_map(|set| {
            let slot = set.slot?;
            Some((
                slot,
                set.name.clone(),
                sets.active.as_deref() == Some(set.id.as_str()),
            ))
        })
        .collect();
    drop(sets);
    if !slotted.is_empty() {
        rows.push(Row::Heading("Screensets".to_owned()));
        for (slot, name, on) in slotted {
            rows.push(Row::Act {
                label: name,
                act: Act::Recall(slot),
                checked: on,
                danger: false,
            });
        }
    }
    rows.push(Row::Heading("Tabs".to_owned()));
    if reopen {
        rows.push(act("Reopen closed tab", Act::Reopen));
    }
    let can_close = closable(host);
    let current = phone_pane(&host.workspace.peek());
    if can_close.iter().any(|p| Some(p) != current.as_ref()) && can_close.len() > 1 {
        rows.push(Row::Act {
            label: "Close other tabs".to_owned(),
            act: Act::CloseOthers,
            checked: false,
            danger: true,
        });
    }
    if !can_close.is_empty() {
        rows.push(Row::Act {
            label: "Close all tabs".to_owned(),
            act: Act::CloseAll,
            checked: false,
            danger: true,
        });
    }
    rows
}

fn new_rows(host: Host) -> Vec<Row> {
    let mut rows = vec![Row::Heading("New tab".to_owned())];
    rows.extend(
        host.views
            .peek()
            .iter()
            .map(|v| act(v.title.clone(), Act::Open(v.kind.0.clone()))),
    );
    rows
}

fn run(host: Host, a: Act) {
    let s = host.switch;
    let mut menu = s.menu;
    menu.set(None);
    let mut place = s.place;
    match a {
        Act::Place(p) => {
            place.set(p);
            s.scroll.jump(0.0);
        }
        Act::Recall(slot) => {
            host.run(Op::Recall(slot));
            place.set(Place::Main);
            s.scroll.jump(0.0);
        }
        Act::Reopen => host.run(Op::Reopen),
        Act::CloseOthers => {
            let keep = phone_pane(&host.workspace.peek());
            for pane in closable(host) {
                if Some(&pane) != keep.as_ref() {
                    host.run(Op::Close(pane));
                }
            }
        }
        Act::CloseAll => {
            for pane in closable(host) {
                host.run(Op::Close(pane));
            }
        }
        Act::Open(kind) => {
            let Some(opened) = host.open(&kind) else {
                return;
            };
            let (at, n, docked) = {
                let w = host.workspace.peek();
                let p = w.place_of(&opened).unwrap_or(Place::Main);
                let cards = w.cards(p);
                place.set(p);
                (
                    cards.iter().position(|c| c.pane == opened),
                    cards.len(),
                    matches!(p, Place::Dock(_)),
                )
            };
            scroll_to(host, *place.peek(), at, n);
            let mut focus = s.focus;
            if docked {
                focus.set(None);
                let mut sheet = host.sheet;
                zoom_in(host, move || sheet.set(Some(opened)));
            } else {
                focus.set(Some(opened));
                zoom_in(host, || {});
            }
        }
    }
}

/// A bar menu, over the bar: the `+`'s at the left, "N tabs"'s in the
/// middle.
#[component]
fn SwitcherMenu(menu: SwitchMenu, bar_h: f32, w: f32) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let rows = match menu {
        SwitchMenu::Tabs => tabs_rows(host),
        SwitchMenu::New => new_rows(host),
    };
    let width = 260.0_f32.min(w - 24.0);
    let left = match menu {
        SwitchMenu::New => 12.0,
        SwitchMenu::Tabs => (w - width) / 2.0,
    };
    let mut open = host.switch.menu;
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:4;",
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |_| open.set(None),
        }
        div {
            "data-switcher": "menu",
            style: "position:absolute; left:{left}px; bottom:{bar_h + 6.0}px; width:{width}px; max-height:70%; \
                    z-index:5; overflow-y:scroll; box-sizing:border-box; display:flex; flex-direction:column; \
                    padding:6px; border-radius:14px; background:{t.pill}; border:1px solid {t.rule}; \
                    box-shadow:{t.shadow_float};",
            onpointerdown: move |e| e.stop_propagation(),
            onclick: move |e| e.stop_propagation(),
            for (i, row) in rows.into_iter().enumerate() {
                MenuRow { key: "{i}", row }
            }
        }
    }
}

#[component]
fn MenuRow(row: Row) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    match row {
        Row::Heading(text) => rsx! {
            div { style: "padding:8px 12px 4px; color:{t.ink_3}; font-size:12px; font-weight:600;", "{text}" }
        },
        Row::Act {
            label,
            act,
            checked,
            danger,
        } => {
            let ink = if danger { t.danger } else { t.ink };
            rsx! {
                button {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:8px; \
                            min-height:44px; padding:0 12px; border:none; border-radius:8px; \
                            background:transparent; color:{ink}; font-size:15px; text-align:left; cursor:pointer;",
                    onclick: move |_| run(host, act.clone()),
                    span { "{label}" }
                    if checked {
                        Check { size: 16, color: t.accent }
                    }
                }
            }
        }
    }
}
