//! Its views: the calendar (day, week, month, agenda) and the two view
//! types it adds to Bases (Task's `task:features/view/view-calendar`).
//!
//! - **Day and week:** a heading per day (a click opens its daily note),
//!   the all-day strip, and the hours, each block in its column of those
//!   it overlaps ([`crate::layout::day_columns`]). Drag a block to move
//!   it (by 15 minutes, and across days), its top or bottom edge to
//!   resize it; sweep over free time to make one; click one to edit it.
//! - **Month:** six weeks, the blocks in tracks, three a day then "+N";
//!   drag one to another day. A day's number opens its daily note; a
//!   click on a free part of a day makes an event there.
//! - **Agenda:** the next 30 days, as a list.
//! - **Bases:** `calendar`, a month of a base's notes by a date property,
//!   and `timeline`, a schedule of them as bars; drag either.
//!
//! Drags follow the navigator's pattern: a press is noted, a move past a
//! few points makes it a drag, the grid's measured box turns the pointer
//! into minutes and days, and the release writes.

// Pixel math: points, minutes and day counts as floats and back. They
// stay far inside f64's exact range, and a rounded drag is what is meant.
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap
)]

use std::rc::Rc;
use std::time::Duration as Wait;

use chrono::{Datelike as _, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike as _};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::layout::{self, Mode};
use crate::model::{self, Block, Source};
use crate::{Shown, quick, recur, show, shown, today};

/// One hour's height in the day and week views.
const HOUR: f64 = 44.0;
/// The hours' gutter.
const GUTTER: f64 = 52.0;
/// A press moves this far before it is a drag (Task's 4 points).
const SLOP: f64 = 4.0;

const MUTED: &str = "color: var(--muted-foreground);";

/// Re-renders the calling view when what it knows changes, and reads
/// again every few seconds while it shows.
pub(crate) fn use_calendar_revision() -> u64 {
    use_calendar()
}

fn use_calendar() -> u64 {
    let mut seen = use_signal(crate::revision);
    use_future(move || async move {
        let _ = crate::refresh().await;
        let mut ticks = 0_u32;
        loop {
            futures_timer::Delay::new(Wait::from_millis(250)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(16) {
                let _ = crate::refresh().await;
            }
            let now = crate::revision();
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// A box measured again while mounted (Blitz fires no resize).
fn use_box(mounted: Signal<Option<Rc<MountedData>>>) -> Signal<Option<(f64, f64, f64, f64)>> {
    let mut at = use_signal(|| None);
    use_future(move || async move {
        loop {
            if let Some(m) = mounted.peek().clone()
                && let Ok(r) = m.get_client_rect().await
            {
                let b = Some((r.origin.x, r.origin.y, r.size.width, r.size.height));
                if *at.peek() != b {
                    at.set(b);
                }
            }
            futures_timer::Delay::new(Wait::from_millis(400)).await;
        }
    });
    at
}

fn open_note(path: String) {
    spawn(async move {
        if let Some(h) = crate::vault::handle() {
            let _ = h
                .host(architect_extensions::hostcall::HostRequest::Open { path })
                .await;
        }
    });
}

fn open_daily(day: NaiveDate, mut problem: Signal<Option<String>>) {
    spawn(async move {
        if let Err(e) = crate::daily(json!({ "date": day.to_string() })).await {
            problem.set(Some(e));
        }
    });
}

fn chip(active: bool) -> String {
    format!(
        "padding: 3px 10px; border-radius: 999px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {};",
        if active {
            "var(--primary)"
        } else {
            "transparent"
        },
        if active {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

const TOOL: &str = "height: 28px; min-width: 28px; padding: 0 8px; border-radius: 7px; border: 1px solid var(--border); \
                    background: transparent; color: var(--foreground); cursor: pointer; font-size: 13px;";

const FIELD: &str = "height: 30px; box-sizing: border-box; padding: 0 8px; border-radius: 7px; border: 1px solid var(--input); \
                     background: var(--background); color: var(--foreground); font-size: 13px; width: 100%;";

/// What the editor is on: a block drawn, or a new event's times.
#[derive(Clone, Debug, PartialEq)]
enum Picked {
    Block(Block),
    New {
        start: NaiveDateTime,
        end: NaiveDateTime,
        all_day: bool,
    },
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grip {
    Move,
    Top,
    Bottom,
    /// A sweep over free time.
    Sweep,
}

/// A press, and the drag it may become.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    block: Option<Block>,
    grip: Grip,
    from: (f64, f64),
    active: bool,
    /// The day pressed on, and (a sweep) the minute.
    day: NaiveDate,
    minute: i64,
    /// Where it is now: minutes and days moved.
    minutes: i64,
    days: i64,
}

impl Drag {
    /// The times it would write.
    fn span(&self) -> Option<(NaiveDateTime, NaiveDateTime)> {
        match (&self.block, self.grip) {
            (Some(b), Grip::Move) => {
                let d = Duration::minutes(self.minutes) + Duration::days(self.days);
                Some((b.start + d, b.end + d))
            }
            (Some(b), Grip::Top) => {
                let s = (b.start + Duration::minutes(self.minutes))
                    .min(b.end - Duration::minutes(layout::SNAP));
                Some((s, b.end))
            }
            (Some(b), Grip::Bottom) => {
                let e = (b.end + Duration::minutes(self.minutes))
                    .max(b.start + Duration::minutes(layout::SNAP));
                Some((b.start, e))
            }
            (None, Grip::Sweep) => {
                let a = layout::snap(self.minute);
                let b = layout::snap(self.minute + self.minutes) + layout::SNAP;
                let (a, b) = if self.minutes < 0 {
                    (
                        layout::snap(self.minute + self.minutes),
                        layout::snap(self.minute) + layout::SNAP,
                    )
                } else {
                    (a, b)
                };
                let m = layout::midnight(self.day);
                Some((
                    m + Duration::minutes(a),
                    m + Duration::minutes(b.max(a + layout::SNAP)),
                ))
            }
            _ => None,
        }
    }
}

/// Writes where a block was dragged to: an event's occurrence (that one,
/// when of a series) or a task's day and time.
async fn commit(
    b: Block,
    start: NaiveDateTime,
    end: NaiveDateTime,
    all_day: bool,
) -> Result<(), String> {
    match &b.source {
        Source::Event { readonly: true, .. } => {
            Err("this event comes from a subscribed calendar: change it there".to_owned())
        }
        Source::Event {
            path,
            series,
            master,
            date,
            ..
        } => {
            let mut args = json!({
                "path": path,
                "start": model::when_text(&Local, start, all_day),
                "end": model::when_text(&Local, end, all_day),
                "all_day": all_day,
            });
            if *series && master.is_none() {
                args["occurrence"] = json!(date.to_string());
                args["scope"] = json!("this");
            }
            crate::reschedule(args).await.map(|_| ())
        }
        Source::Task { path, .. } => {
            let mut args = json!({ "path": path, "date": start.date().to_string() });
            if !all_day {
                args["time"] = json!(start.format("%H:%M").to_string());
            }
            crate::move_task(args).await.map(|_| ())
        }
        Source::Time { .. } => Err("tracked time is moved in the Time Tracker".to_owned()),
        Source::Booking { .. } => {
            Err("a booking keeps the time agreed: cancel it, and book again".to_owned())
        }
    }
}

/// The calendar.
#[component]
pub fn CalendarView() -> Element {
    let _ = use_calendar();
    let picked = use_signal(|| None::<Picked>);
    let mut problem = use_signal(|| None::<String>);
    let mut draft = use_signal(String::new);
    let s = shown();
    let settings = crate::settings();
    let first = settings.week_start;
    let set = move |s: Shown| show(s);
    let mut submit = move || {
        let text = draft.peek().trim().to_owned();
        if text.is_empty() {
            return;
        }
        draft.set(String::new());
        let day = shown().cursor;
        spawn(async move {
            match crate::quick_add(json!({ "text": text, "date": day.to_string() })).await {
                Ok(_) => problem.set(None),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let mut new_now = {
        let mut picked = picked;
        move || {
            let s = shown();
            let now = Local::now().naive_local();
            let day = if s.mode == Mode::Month || s.mode == Mode::Agenda {
                now.date().max(s.cursor)
            } else {
                s.cursor
            };
            let hour = if day == now.date() {
                (now.time().hour() + 1).min(23)
            } else {
                9
            };
            let start = day
                .and_hms_opt(hour, 0, 0)
                .unwrap_or_else(|| layout::midnight(day));
            picked.set(Some(Picked::New {
                start,
                end: start + Duration::minutes(crate::settings().event_minutes),
                all_day: false,
            }));
        }
    };
    let (error, read) = {
        let c = crate::cache();
        (c.error.clone(), c.read)
    };
    rsx! {
        div { class: "cal-root", "data-view": "{s.mode.id()}", "data-date": "{s.cursor}", tabindex: "0",
            style: "display: flex; flex-direction: column; height: 100%; box-sizing: border-box; color: var(--foreground); \
                    font-size: 13px; outline: none;",
            onkeydown: move |e: KeyboardEvent| {
                let s = shown();
                if let Key::Character(c) = e.key() {
                    match c.as_str() {
                        "t" => set(Shown { cursor: today(), ..s }),
                        "1" | "d" => set(Shown { mode: Mode::Day, ..s }),
                        "2" | "w" => set(Shown { mode: Mode::Week, ..s }),
                        "3" | "m" => set(Shown { mode: Mode::Month, ..s }),
                        "4" | "a" => set(Shown { mode: Mode::Agenda, ..s }),
                        "n" => new_now(),
                        _ => {}
                    }
                } else if e.key() == Key::Named(NamedKey::ArrowLeft) {
                    set(Shown { cursor: layout::step(s.mode, s.cursor, -1), ..s });
                } else if e.key() == Key::Named(NamedKey::ArrowRight) {
                    set(Shown { cursor: layout::step(s.mode, s.cursor, 1), ..s });
                }
            },
            div { class: "cal-toolbar",
                style: "display: flex; align-items: center; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--border); flex-wrap: wrap;",
                button { class: "cal-prev", style: TOOL, "aria-label": "Previous",
                    onclick: move |_| { let s = shown(); set(Shown { cursor: layout::step(s.mode, s.cursor, -1), ..s }); }, "‹" }
                button { class: "cal-today", style: TOOL, onclick: move |_| { let s = shown(); set(Shown { cursor: today(), ..s }); }, "Today" }
                button { class: "cal-next", style: TOOL, "aria-label": "Next",
                    onclick: move |_| { let s = shown(); set(Shown { cursor: layout::step(s.mode, s.cursor, 1), ..s }); }, "›" }
                span { class: "cal-range", style: "font-size: 17px; font-weight: 700; margin: 0 6px; white-space: nowrap;",
                    {layout::range_label(s.mode, s.cursor, first)} }
                span { style: "flex: 1;" }
                div { class: "cal-modes", style: "display: flex; gap: 4px;",
                    for m in Mode::ALL {
                        button { key: "{m.id()}", class: "cal-mode", "data-mode": "{m.id()}", style: chip(s.mode == m),
                            onclick: move |_| { let s = shown(); set(Shown { mode: m, ..s }); }, "{m.label()}" }
                    }
                }
                input { class: "cal-quick", r#type: "text", "aria-label": "New event",
                    placeholder: "New event — e.g. Lunch with Sam tomorrow 1pm for 90m",
                    value: "{draft}",
                    style: "{FIELD} width: 300px;",
                    oninput: move |e: FormEvent| draft.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        e.stop_propagation();
                        if e.key() == Key::Named(NamedKey::Enter) { submit() }
                    },
                }
                button { class: "cal-new", style: "{TOOL} background: var(--primary); color: var(--primary-foreground); border: none;",
                    onclick: move |_| new_now(), "+ New" }
            }
            if let Some(p) = problem() {
                div { class: "cal-problem", style: "padding: 6px 16px; color: var(--destructive);",
                    "{p} " button { style: TOOL, onclick: move |_| problem.set(None), "×" } }
            }
            if let Some(e) = error {
                div { style: "padding: 6px 16px; color: var(--destructive);", "The vault could not be read: {e}" }
            } else if !read {
                div { style: "padding: 6px 16px; {MUTED}", "Reading the vault…" }
            }
            div { style: "flex: 1; min-height: 0; display: flex; position: relative;",
                div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column;",
                    match s.mode {
                        Mode::Day | Mode::Week => rsx! { TimeGrid { shown: s, picked, problem } },
                        Mode::Month => rsx! { MonthGrid { shown: s, picked, problem } },
                        Mode::Agenda => rsx! { Agenda { shown: s, picked } },
                    }
                }
                if picked().is_some() {
                    Editor { picked, problem }
                }
            }
        }
    }
}

/// The day and week views.
// Both lints point into `rsx!`'s own expansion of the ghost and the grid.
#[allow(clippy::redundant_clone, clippy::suboptimal_flops)]
#[component]
fn TimeGrid(
    shown: Shown,
    picked: Signal<Option<Picked>>,
    problem: Signal<Option<String>>,
) -> Element {
    let settings = crate::settings();
    let days = layout::days(shown.mode, shown.cursor, settings.week_start);
    let (Some(a), Some(z)) = (days.first().copied(), days.last().copied()) else {
        return rsx! {};
    };
    let blocks = crate::blocks(layout::midnight(a), layout::midnight(z + Duration::days(1)));
    let mounted = use_signal(|| None::<Rc<MountedData>>);
    let grid = use_box(mounted);
    let mut drag = use_signal(|| None::<Drag>);
    let now = Local::now().naive_local();
    // From the setting's hour, or earlier when something is.
    let earliest = blocks
        .iter()
        .filter(|b| !b.in_strip())
        .map(|b| {
            if b.start.date() < a {
                0
            } else {
                b.start.hour()
            }
        })
        .min();
    let first_hour = earliest.map_or(settings.day_starts, |h| h.min(settings.day_starts));
    let top0 = i64::from(first_hour) * 60;
    let px = HOUR / 60.0;
    let n = days.len();
    let strip = layout::tracks(a, n, &blocks, false);
    let strip_rows = strip.iter().map(|t| t.track + 1).max().unwrap_or(0);
    let days2 = days.clone();
    let on_move = move |e: PointerEvent| {
        let Some(mut d) = drag.peek().clone() else {
            return;
        };
        let p = e.data().client_coordinates();
        let (dx, dy) = (p.x - d.from.0, p.y - d.from.1);
        if !d.active && dx.hypot(dy) < SLOP {
            return;
        }
        d.active = true;
        #[allow(clippy::cast_possible_truncation)]
        {
            d.minutes = layout::snap_round((dy / px).round() as i64);
            let col = grid
                .peek()
                .map_or(1.0, |(_, _, w, _)| ((w - GUTTER) / n as f64).max(1.0));
            d.days = if d.grip == Grip::Move {
                (dx / col).round() as i64
            } else {
                0
            };
        }
        if d.grip == Grip::Sweep {
            d.minutes = (dy / px).round() as i64;
        }
        drag.set(Some(d));
    };
    let mut on_up = move || {
        let Some(d) = drag.take() else { return };
        let mut picked = picked;
        let mut problem = problem;
        match (&d.block, d.active) {
            (Some(b), false) => picked.set(Some(Picked::Block(b.clone()))),
            (Some(b), true) => {
                if let Some((s, e)) = d.span()
                    && (s, e) != (b.start, b.end)
                {
                    let b = b.clone();
                    spawn(async move {
                        if let Err(e) = commit(b, s, e, false).await {
                            problem.set(Some(e));
                        }
                    });
                }
            }
            (None, active) => {
                let (s, e) = if active {
                    d.span().unwrap_or_default()
                } else {
                    let s = layout::midnight(d.day) + Duration::minutes(layout::snap(d.minute));
                    (s, s + Duration::minutes(crate::settings().event_minutes))
                };
                picked.set(Some(Picked::New {
                    start: s,
                    end: e,
                    all_day: false,
                }));
            }
        }
    };
    let ghost = drag().filter(|d| d.active).and_then(|d| d.span());
    let height = (24.0 - f64::from(first_hour)) * HOUR;
    rsx! {
        div { class: "cal-grid", "data-days": "{n}",
            style: "flex: 1; min-height: 0; display: flex; flex-direction: column; user-select: none;",
            onpointermove: on_move,
            onpointerup: move |_| on_up(),
            // The heads: a click opens the day's daily note.
            div { class: "cal-heads", style: "display: flex; border-bottom: 1px solid var(--border);",
                div { style: "flex: 0 0 {GUTTER}px;" }
                for day in days.clone() {
                    {
                        let is_today = day == today();
                        let weekday = day.format("%a").to_string();
                        let number = day.day();
                        rsx! {
                            div { key: "{day}", style: "flex: 1 1 0; min-width: 0; padding: 6px 0; text-align: center;",
                                button { class: "cal-daily", "data-date": "{day}", title: "Open the daily note",
                                    style: "border: none; background: transparent; color: inherit; cursor: pointer; display: inline-flex; align-items: baseline; gap: 6px; padding: 2px 8px; border-radius: 7px;",
                                    onclick: move |_| open_daily(day, problem),
                                    span { style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; {MUTED}", "{weekday}" }
                                    span { style: if is_today { "font-size: 17px; font-weight: 700; color: var(--primary-foreground); background: var(--primary); border-radius: 999px; padding: 0 7px;" } else { "font-size: 17px; font-weight: 600;" },
                                        "{number}" }
                                }
                            }
                        }
                    }
                }
            }
            // The all-day strip.
            if strip_rows > 0 {
                div { class: "cal-strip", style: "position: relative; display: flex; border-bottom: 1px solid var(--border); height: {strip_rows * 22 + 4}px;",
                    div { style: "flex: 0 0 {GUTTER}px; font-size: 10px; padding: 4px; box-sizing: border-box; {MUTED}", "all day" }
                    div { style: "flex: 1; position: relative;",
                        for t in strip.clone() {
                            {
                                let b = blocks[t.index].clone();
                                let left = t.first as f64 * 100.0 / n as f64;
                                let width = (t.last - t.first + 1) as f64 * 100.0 / n as f64;
                                let top = t.track * 22 + 2;
                                rsx! {
                                    div { key: "{b.key}", style: "position: absolute; left: calc({left}% + 2px); width: calc({width}% - 4px); top: {top}px; height: 20px;",
                                        BlockChip { block: b, picked }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // The hours.
            div { style: "flex: 1; min-height: 0; overflow-y: auto;",
                div { style: "display: flex; position: relative; height: {height}px;",
                    div { style: "flex: 0 0 {GUTTER}px; position: relative;",
                        for h in first_hour..24 {
                            div { key: "{h}", style: "position: absolute; top: {f64::from(h - first_hour) * HOUR - 6.0}px; right: 6px; font-size: 10px; {MUTED}",
                                if h > first_hour { "{h:02}:00" }
                            }
                        }
                    }
                    div { class: "cal-columns", style: "flex: 1; display: flex; position: relative;",
                        onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
                        for (ci, day) in days2.iter().copied().enumerate() {
                            {
                                let placed = layout::day_columns(day, &blocks);
                                let is_today = day == now.date();
                                let now_top = (f64::from(now.hour() * 60 + now.minute()) - top0 as f64) * px;
                                rsx! {
                                    div { key: "{day}", class: "cal-day", "data-date": "{day}",
                                        style: "flex: 1 1 0; min-width: 0; position: relative; border-left: 1px solid var(--border); \
                                                background: repeating-linear-gradient(to bottom, transparent 0, transparent {HOUR - 1.0}px, var(--border) {HOUR - 1.0}px, var(--border) {HOUR}px);",
                                        onpointerdown: move |e: PointerEvent| {
                                            if e.data().pointer_type() != "mouse" { return; }
                                            let p = e.data().client_coordinates();
                                            let y = e.data().element_coordinates().y;
                                            #[allow(clippy::cast_possible_truncation)]
                                            let minute = top0 + (y / px) as i64;
                                            drag.set(Some(Drag { block: None, grip: Grip::Sweep, from: (p.x, p.y), active: false, day, minute, minutes: 0, days: 0 }));
                                        },
                                        if is_today {
                                            div { class: "cal-now", style: "position: absolute; left: 0; right: 0; top: {now_top}px; height: 2px; background: var(--destructive); z-index: 3; pointer-events: none;" }
                                        }
                                        for p in placed {
                                            {
                                                let b = blocks[p.index].clone();
                                                let dragging = drag().is_some_and(|d| d.active && d.block.as_ref().is_some_and(|x| x.key == b.key));
                                                let top = (p.top - top0) as f64 * px;
                                                let h = (p.height as f64 * px).max(14.0);
                                                let left = p.column as f64 * 100.0 / p.columns as f64;
                                                let width = 100.0 / p.columns as f64;
                                                rsx! {
                                                    TimedBlock { key: "{b.key}", block: b, top, height: h, left, width, faded: dragging, ci, drag, picked }
                                                }
                                            }
                                        }
                                        // The ghost of a drag, on its day.
                                        if let Some((gs, ge)) = ghost.filter(|(s, _)| s.date() == day) {
                                            {
                                            let ghost_top = ((gs - layout::midnight(day)).num_minutes() - top0) as f64 * px;
                                            let ghost_h = (ge - gs).num_minutes() as f64 * px;
                                            let ghost_text = format!("{}–{}", gs.format("%H:%M"), ge.format("%H:%M"));
                                            rsx! {
                                            div { class: "cal-ghost",
                                                style: "position: absolute; left: 2px; right: 2px; top: {ghost_top}px; \
                                                        height: {ghost_h}px; border-radius: 6px; border: 2px dashed var(--primary); \
                                                        background: color-mix(in srgb, var(--primary) 18%, transparent); pointer-events: none; z-index: 4; \
                                                        font-size: 11px; padding: 2px 4px; box-sizing: border-box;",
                                                "{ghost_text}"
                                            }
                                            }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A block in the hours: a press on its body moves it, on its edges
/// resizes it.
#[component]
fn TimedBlock(
    block: Block,
    top: f64,
    height: f64,
    left: f64,
    width: f64,
    faded: bool,
    ci: usize,
    drag: Signal<Option<Drag>>,
    picked: Signal<Option<Picked>>,
) -> Element {
    let colour = model::color_css(&block.color);
    let (kind, done) = match &block.source {
        Source::Event { .. } => ("event", false),
        Source::Task { done, .. } => ("task", *done),
        Source::Time { .. } => ("time", false),
        Source::Booking { .. } => ("booking", false),
    };
    let movable = block.movable();
    let style = match kind {
        "time" => format!(
            "background: repeating-linear-gradient(135deg, color-mix(in srgb, {colour} 22%, transparent) 0 6px, transparent 6px 12px); \
             border: 1px dashed {colour}; color: var(--foreground);"
        ),
        "task" => format!(
            "background: var(--card); border: 1px solid var(--border); border-left: 3px solid {colour}; color: var(--card-foreground);"
        ),
        _ => format!(
            "background: color-mix(in srgb, {colour} 82%, black); border: 1px solid {colour}; color: white;"
        ),
    };
    let press = move |grip: Grip, b: Block| {
        move |e: PointerEvent| {
            e.stop_propagation();
            if e.data().pointer_type() != "mouse" {
                return;
            }
            let p = e.data().client_coordinates();
            let mut drag = drag;
            drag.set(Some(Drag {
                block: Some(b.clone()),
                grip: if b.movable() { grip } else { Grip::Move },
                from: (p.x, p.y),
                active: false,
                day: b.start.date(),
                minute: 0,
                minutes: 0,
                days: 0,
            }));
        }
    };
    let b = block.clone();
    let title = block.title.clone();
    let data_start = block.start.format("%Y-%m-%dT%H:%M").to_string();
    let source_path = block.source.path().to_owned();
    let cursor = if movable { "grab" } else { "pointer" };
    let opacity = if faded { 0.45 } else { 1.0 };
    let deco = if done { "line-through" } else { "none" };
    let times = format!(
        "{}–{}",
        block.start.format("%H:%M"),
        block.end.format("%H:%M")
    );
    rsx! {
        div { class: "cal-block", "data-kind": kind, "data-path": "{source_path}", "data-start": "{data_start}",
            "data-column": "{ci}",
            style: "position: absolute; top: {top}px; height: {height}px; left: calc({left}% + 2px); width: calc({width}% - 4px); \
                    box-sizing: border-box; border-radius: 6px; padding: 2px 6px; overflow: hidden; font-size: 11px; line-height: 14px; \
                    cursor: {cursor}; z-index: 2; opacity: {opacity}; text-decoration: {deco}; {style}",
            onpointerdown: press(Grip::Move, b),
            if movable && height > 24.0 {
                div { class: "cal-grip-top", style: "position: absolute; left: 0; right: 0; top: 0; height: 5px; cursor: ns-resize;",
                    onpointerdown: press(Grip::Top, b.clone()) }
            }
            div { style: "font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;", "{title}" }
            if height > 28.0 {
                div { style: "opacity: 0.85; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                    "{times}"
                    if let Some(l) = &block.location { " · {l}" }
                    if block.repeats.is_some() { " ↻" }
                }
            }
            if movable {
                div { class: "cal-grip-bottom", style: "position: absolute; left: 0; right: 0; bottom: 0; height: 6px; cursor: ns-resize;",
                    onpointerdown: press(Grip::Bottom, b.clone()) }
            }
        }
    }
}

/// A block as a chip: in the strip, a month's day, the agenda.
#[component]
fn BlockChip(block: Block, picked: Signal<Option<Picked>>) -> Element {
    let colour = model::color_css(&block.color);
    let (kind, done) = match &block.source {
        Source::Event { .. } => ("event", false),
        Source::Task { done, .. } => ("task", *done),
        Source::Time { .. } => ("time", false),
        Source::Booking { .. } => ("booking", false),
    };
    let label = match kind {
        "task" => block.title.clone(),
        _ if block.in_strip() => block.title.clone(),
        _ => format!("{} {}", block.start.format("%H:%M"), block.title),
    };
    let solid = kind == "event" && block.in_strip();
    let b = block.clone();
    let deco = if done { "line-through" } else { "none" };
    let look = if solid {
        format!("background: {colour}; color: white;")
    } else {
        format!(
            "border-left: 3px solid {colour}; background: var(--card); color: var(--card-foreground);"
        )
    };
    let source_path = block.source.path().to_owned();
    rsx! {
        div { class: "cal-chip", "data-kind": kind, "data-path": "{source_path}",
            style: "height: 20px; box-sizing: border-box; border-radius: 5px; padding: 0 6px; font-size: 11px; line-height: 18px; \
                    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; cursor: pointer; \
                    text-decoration: {deco}; {look}",
            onclick: move |e| { e.stop_propagation(); let mut p = picked; p.set(Some(Picked::Block(b.clone()))); },
            "{label}"
        }
    }
}

/// How many chips a month's day shows before "+N".
const MONTH_TRACKS: usize = 3;

/// The month.
#[component]
fn MonthGrid(
    shown: Shown,
    picked: Signal<Option<Picked>>,
    problem: Signal<Option<String>>,
) -> Element {
    let settings = crate::settings();
    let days = layout::days(Mode::Month, shown.cursor, settings.week_start);
    let Some(a) = days.first().copied() else {
        return rsx! {};
    };
    let blocks = crate::blocks(
        layout::midnight(a),
        layout::midnight(a + Duration::days(42)),
    );
    let mounted = use_signal(|| None::<Rc<MountedData>>);
    let grid = use_box(mounted);
    // A chip pressed: its block, where, and the day it is over.
    let mut press = use_signal(|| None::<(Block, (f64, f64), bool, Option<NaiveDate>)>);
    let over_day = move |x: f64, y: f64| -> Option<NaiveDate> {
        let (bx, by, w, h) = (*grid.peek())?;
        let (cw, ch) = (w / 7.0, h / 6.0);
        #[allow(clippy::cast_possible_truncation)]
        let (c, r) = (
            ((x - bx) / cw).floor() as i64,
            ((y - by) / ch).floor() as i64,
        );
        ((0..7).contains(&c) && (0..6).contains(&r)).then(|| a + Duration::days(r * 7 + c))
    };
    let heads = days.iter().take(7).map(|d| d.format("%a").to_string());
    let month = shown.cursor.month();
    let over = press().filter(|p| p.2).and_then(|p| p.3);
    rsx! {
        div { class: "cal-month", style: "flex: 1; min-height: 0; display: flex; flex-direction: column; user-select: none;",
            onpointermove: move |e: PointerEvent| {
                let Some(mut p) = press.peek().clone() else { return };
                let c = e.data().client_coordinates();
                if !p.2 && (c.x - p.1.0).hypot(c.y - p.1.1) < SLOP { return; }
                p.2 = true;
                p.3 = over_day(c.x, c.y);
                press.set(Some(p));
            },
            onpointerup: move |_| {
                let Some((b, _, active, to)) = press.take() else { return };
                let mut picked = picked;
                let mut problem = problem;
                if !active {
                    picked.set(Some(Picked::Block(b)));
                    return;
                }
                if let Some(to) = to && to != b.start.date() && b.movable() {
                    let d = to - b.start.date();
                    spawn(async move {
                        let all_day = b.in_strip();
                        if let Err(e) = commit(b.clone(), b.start + d, b.end + d, all_day).await {
                            problem.set(Some(e));
                        }
                    });
                }
            },
            div { style: "display: flex; border-bottom: 1px solid var(--border);",
                for (i, h) in heads.enumerate() {
                    div { key: "{i}", style: "flex: 1 1 0; padding: 6px; font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; {MUTED}", "{h}" }
                }
            }
            div { class: "cal-month-grid", style: "flex: 1; min-height: 0; display: flex; flex-direction: column;",
                onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
                for w in 0..6_usize {
                    {
                        let start = a + Duration::days(i64::try_from(w * 7).unwrap_or(0));
                        let rows = layout::tracks(start, 7, &blocks, true);
                        rsx! {
                            div { key: "{start}", class: "cal-week", style: "flex: 1 1 0; min-height: 0; display: flex; position: relative; border-bottom: 1px solid var(--border);",
                                for c in 0..7_usize {
                                    {
                                        let day = start + Duration::days(i64::try_from(c).unwrap_or(0));
                                        let hidden = rows.iter().filter(|t| t.track >= MONTH_TRACKS && (t.first..=t.last).contains(&c)).count();
                                        let outside = day.month() != month;
                                        let is_today = day == today();
                                        let lit = over == Some(day);
                                        let s = settings.event_minutes;
                                        let bg = if lit { "color-mix(in srgb, var(--primary) 15%, transparent)" } else if outside { "var(--muted)" } else { "transparent" };
                                        let num_bg = if is_today { "var(--primary)" } else { "transparent" };
                                        let num_fg = if is_today { "var(--primary-foreground)" } else if outside { "var(--muted-foreground)" } else { "inherit" };
                                        let number = day.day();
                                        rsx! {
                                            div { key: "{day}", class: "cal-cell", "data-date": "{day}",
                                                style: "flex: 1 1 0; min-width: 0; border-left: 1px solid var(--border); padding: 3px 4px; box-sizing: border-box; \
                                                        background: {bg};",
                                                onclick: move |_| {
                                                    let mut picked = picked;
                                                    let start = day.and_hms_opt(9, 0, 0).unwrap_or_else(|| layout::midnight(day));
                                                    picked.set(Some(Picked::New { start, end: start + Duration::minutes(s), all_day: false }));
                                                },
                                                div { style: "display: flex; justify-content: space-between; align-items: center; height: 20px;",
                                                    button { class: "cal-daily", "data-date": "{day}", title: "Open the daily note",
                                                        style: "border: none; background: {num_bg}; color: {num_fg}; border-radius: 999px; padding: 0 6px; font-size: 12px; font-weight: 600; cursor: pointer;",
                                                        onclick: move |e| { e.stop_propagation(); open_daily(day, problem); },
                                                        "{number}"
                                                    }
                                                    if hidden > 0 {
                                                        button { class: "cal-more", style: "border: none; background: transparent; font-size: 11px; cursor: pointer; {MUTED}",
                                                            onclick: move |e| { e.stop_propagation(); show(Shown { mode: Mode::Day, cursor: day }); },
                                                            "+{hidden}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                for t in rows.into_iter().filter(|t| t.track < MONTH_TRACKS) {
                                    {
                                        let b = blocks[t.index].clone();
                                        let left = t.first as f64 * 100.0 / 7.0;
                                        let width = (t.last - t.first + 1) as f64 * 100.0 / 7.0;
                                        let top = 24 + t.track * 22;
                                        let pressed = press().is_some_and(|p| p.2 && p.0.key == b.key);
                                        let opacity = if pressed { 0.45 } else { 1.0 };
                                        let bb = b.clone();
                                        rsx! {
                                            div { key: "{b.key}", style: "position: absolute; left: calc({left}% + 3px); width: calc({width}% - 6px); top: {top}px; opacity: {opacity};",
                                                onpointerdown: move |e: PointerEvent| {
                                                    e.stop_propagation();
                                                    if e.data().pointer_type() != "mouse" { return; }
                                                    let c = e.data().client_coordinates();
                                                    press.set(Some((bb.clone(), (c.x, c.y), false, None)));
                                                },
                                                onclick: move |e| e.stop_propagation(),
                                                BlockChip { block: b, picked }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The agenda: the days ahead with something on them.
#[component]
fn Agenda(shown: Shown, picked: Signal<Option<Picked>>) -> Element {
    let from = shown.cursor;
    let to = from + Duration::days(layout::AGENDA_DAYS);
    let blocks = crate::blocks(layout::midnight(from), layout::midnight(to));
    let mut by_day: Vec<(NaiveDate, Vec<Block>)> = Vec::new();
    for b in blocks {
        let day = b.start.date().max(from);
        match by_day.last_mut() {
            Some((d, list)) if *d == day => list.push(b),
            _ => by_day.push((day, vec![b])),
        }
    }
    rsx! {
        div { class: "cal-agenda", style: "flex: 1; min-height: 0; overflow-y: auto; padding: 8px 16px 24px;",
            if by_day.is_empty() {
                div { style: "padding: 24px 0; {MUTED}", "Nothing in the next {layout::AGENDA_DAYS} days." }
            }
            for (day, list) in by_day {
                div { key: "{day}", class: "cal-agenda-day", "data-date": "{day}", style: "display: flex; gap: 16px; padding: 10px 0; border-bottom: 1px solid var(--border);",
                    div { style: "flex: 0 0 110px;",
                        div { style: "font-weight: 700;", {day.format("%a %-d %b").to_string()} }
                        if day == today() { div { style: "font-size: 11px; color: var(--primary);", "Today" } }
                    }
                    div { style: "flex: 1; display: flex; flex-direction: column; gap: 4px;",
                        for b in list {
                            {
                                let bb = b.clone();
                                let colour = model::color_css(&b.color);
                                let when = if b.in_strip() { "All day".to_owned() } else { format!("{}–{}", b.start.format("%H:%M"), b.end.format("%H:%M")) };
                                rsx! {
                                    div { key: "{b.key}", class: "cal-agenda-row", "data-path": "{b.source.path()}",
                                        style: "display: flex; align-items: center; gap: 10px; cursor: pointer; padding: 2px 0;",
                                        onclick: move |_| { let mut p = picked; p.set(Some(Picked::Block(bb.clone()))); },
                                        span { style: "flex: 0 0 96px; font-variant-numeric: tabular-nums; {MUTED}", "{when}" }
                                        span { style: "width: 8px; height: 8px; border-radius: 4px; background: {colour};" }
                                        span { style: "font-weight: 600;", "{b.title}" }
                                        if let Some(l) = &b.location { span { style: MUTED, "{l}" } }
                                        if let Some(r) = &b.repeats { span { style: MUTED, "↻ {r}" } }
                                        if matches!(b.source, Source::Task { .. }) { span { style: MUTED, "task" } }
                                        if matches!(b.source, Source::Time { .. }) { span { style: MUTED, "tracked" } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The editor's fields.
#[derive(Clone, Debug, PartialEq, Default)]
struct Form {
    title: String,
    date: String,
    end_date: String,
    start: String,
    end: String,
    all_day: bool,
    color: String,
    recurrence: String,
    location: String,
    description: String,
}

impl Form {
    fn of(p: &Picked) -> Self {
        let (start, end, all_day) = match p {
            Picked::Block(b) => (b.start, b.end, b.in_strip()),
            Picked::New {
                start,
                end,
                all_day,
            } => (*start, *end, *all_day),
        };
        let event = match p {
            Picked::Block(Block {
                source: Source::Event { path, .. },
                ..
            }) => crate::events().into_iter().find(|e| &e.path == path),
            _ => None,
        };
        let last = if all_day {
            (end - Duration::seconds(1)).date()
        } else {
            end.date()
        };
        Self {
            title: match p {
                Picked::Block(b) => b.title.clone(),
                Picked::New { .. } => String::new(),
            },
            date: start.date().to_string(),
            end_date: last.to_string(),
            start: start.format("%H:%M").to_string(),
            end: end.format("%H:%M").to_string(),
            all_day,
            color: event
                .as_ref()
                .map_or_else(|| "primary".to_owned(), |e| e.color.clone()),
            recurrence: event
                .as_ref()
                .and_then(|e| e.recurrence.clone())
                .unwrap_or_default(),
            location: event
                .as_ref()
                .and_then(|e| e.location.clone())
                .unwrap_or_default(),
            description: event
                .as_ref()
                .and_then(|e| e.description.clone())
                .unwrap_or_default(),
        }
    }

    /// The times the fields say.
    fn span(&self) -> Result<(NaiveDateTime, NaiveDateTime), String> {
        let day = NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d")
            .map_err(|_| "the date is YYYY-MM-DD".to_owned())?;
        if self.all_day {
            let last = NaiveDate::parse_from_str(self.end_date.trim(), "%Y-%m-%d")
                .unwrap_or(day)
                .max(day);
            return Ok((
                layout::midnight(day),
                layout::midnight(last + Duration::days(1)),
            ));
        }
        let t = |s: &str| {
            quick::time_of_day(s).or_else(|| NaiveTime::parse_from_str(s.trim(), "%H:%M").ok())
        };
        let s = t(&self.start).ok_or("the start is a time: 14:00, 2pm")?;
        let e = t(&self.end).ok_or("the end is a time: 15:00, 3pm")?;
        let (s, mut e) = (day.and_time(s), day.and_time(e));
        if e <= s {
            // Past midnight.
            e += Duration::days(1);
        }
        Ok((s, e))
    }
}

/// How far an edit of an occurrence reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    This,
    Following,
    Series,
}

impl Scope {
    const fn id(self) -> &'static str {
        match self {
            Self::This => "this",
            Self::Following => "following",
            Self::Series => "series",
        }
    }
}

/// Saves the editor: a new event, or the picked one's changes.
async fn save(p: Picked, f: Form, scope: Scope) -> Result<(), String> {
    let (start, end) = f.span()?;
    let title = f.title.trim().to_owned();
    if title.is_empty() {
        return Err("give it a title".to_owned());
    }
    let when = |t: NaiveDateTime| model::when_text(&Local, t, f.all_day);
    match p {
        Picked::New { .. } => {
            let mut args = json!({ "title": title, "start": when(start), "end": when(end), "all_day": f.all_day, "color": f.color });
            for (k, v) in [
                ("recurrence", &f.recurrence),
                ("location", &f.location),
                ("description", &f.description),
            ] {
                if !v.trim().is_empty() {
                    args[k] = json!(v.trim());
                }
            }
            crate::create(args).await.map(|_| ())
        }
        Picked::Block(b) => match &b.source {
            Source::Event {
                path,
                series,
                master,
                date,
                ..
            } => {
                let occurrence = *series && master.is_none();
                let moved = (start, end) != (b.start, b.end) || f.all_day != b.in_strip();
                let mut target = path.clone();
                if moved || (occurrence && scope != Scope::Series) {
                    let mut args = json!({ "path": path, "start": when(start), "end": when(end), "all_day": f.all_day });
                    if occurrence {
                        args["occurrence"] = json!(date.to_string());
                        args["scope"] = json!(scope.id());
                    }
                    let out = crate::reschedule(args).await?;
                    if let Some(p) = out["path"].as_str() {
                        p.clone_into(&mut target);
                    }
                }
                let mut args = json!({
                    "path": target, "title": title, "color": f.color,
                    "location": f.location.trim(), "description": f.description.trim(),
                });
                // An occurrence alone keeps no rule of its own.
                if !(occurrence && scope == Scope::This) && master.is_none() {
                    args["recurrence"] = json!(f.recurrence.trim());
                }
                crate::update(args).await.map(|_| ())
            }
            Source::Task { path, .. } => {
                let mut args = json!({ "path": path, "date": start.date().to_string() });
                if !f.all_day {
                    args["time"] = json!(start.format("%H:%M").to_string());
                }
                crate::move_task(args).await.map(|_| ())
            }
            Source::Time { .. } | Source::Booking { .. } => Ok(()),
        },
    }
}

/// The editor beside the calendar.
#[component]
fn Editor(picked: Signal<Option<Picked>>, problem: Signal<Option<String>>) -> Element {
    let Some(p) = picked() else { return rsx! {} };
    let readonly = matches!(
        &p,
        Picked::Block(Block {
            source: Source::Event { readonly: true, .. },
            ..
        })
    );
    let mut form = use_signal(|| Form::of(&p));
    let mut scope = use_signal(|| Scope::This);
    let mut was = use_signal(|| p.clone());
    if *was.peek() != p {
        was.set(p.clone());
        form.set(Form::of(&p));
        scope.set(Scope::This);
    }
    let f = form();
    let (kind, path, occurrence, day) = match &p {
        Picked::New { .. } => ("new", None, false, None),
        Picked::Block(b) => match &b.source {
            Source::Event {
                path,
                series,
                master,
                date,
                ..
            } => (
                "event",
                Some(path.clone()),
                *series && master.is_none(),
                Some(*date),
            ),
            Source::Task { path, .. } => ("task", Some(path.clone()), false, None),
            Source::Time { path, .. } => ("time", Some(path.clone()), false, None),
            Source::Booking { path, .. } => ("booking", Some(path.clone()), false, None),
        },
    };
    let heading = match kind {
        "new" => "New event",
        "task" => "Task",
        "time" => "Tracked time",
        "booking" => "Booking",
        _ if occurrence => "Event in a series",
        _ => "Event",
    };
    let mut close = move || picked.set(None);
    let label = "font-size: 11px; font-weight: 700; letter-spacing: 0.05em; text-transform: uppercase; color: var(--muted-foreground);";
    let pp = p;
    let on_save = move |_| {
        let (p, f, sc) = (pp.clone(), form.peek().clone(), *scope.peek());
        let mut problem = problem;
        let mut picked = picked;
        spawn(async move {
            match save(p, f, sc).await {
                Ok(()) => {
                    problem.set(None);
                    picked.set(None);
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let path2 = path.clone();
    rsx! {
        div { class: "cal-editor", "data-kind": kind,
            // Over the calendar's right side, so the grid keeps its width.
            style: "position: absolute; top: 0; right: 0; bottom: 0; width: 300px; max-width: 90%; box-sizing: border-box; z-index: 10; \
                    border-left: 1px solid var(--border); box-shadow: -8px 0 24px rgba(0, 0, 0, 0.25); padding: 14px 16px; overflow-y: auto; \
                    display: flex; flex-direction: column; gap: 10px; background: var(--card); color: var(--card-foreground);",
            div { style: "display: flex; align-items: center;",
                span { style: "flex: 1; font-weight: 700; font-size: 15px;", "{heading}" }
                button { class: "cal-editor-close", style: TOOL, onclick: move |_| close(), "×" }
            }
            if readonly {
                div { class: "cal-readonly", style: MUTED, "From a subscribed calendar: it changes there, and comes here when the calendar syncs (CALENDAR_SYNC)." }
            } else if kind == "booking" {
                div { style: MUTED, "A booking of an event type. Cancel it in Bookings (CALENDAR_BOOKINGS_OPEN)." }
            } else if kind == "time" {
                div { style: MUTED, "Time tracked on this note. Change it in the Time Tracker." }
            } else {
                input { class: "cal-title", r#type: "text", "aria-label": "Title", placeholder: "Title", value: "{f.title}",
                    disabled: kind == "task", style: FIELD,
                    oninput: move |e: FormEvent| form.write().title = e.value() }
                div { style: "display: flex; gap: 6px; align-items: center;",
                    span { style: label, "When" }
                    span { style: "flex: 1;" }
                    button { class: "cal-all-day", style: chip(f.all_day), onclick: move |_| { let mut w = form.write(); w.all_day = !w.all_day; }, "All day" }
                }
                input { class: "cal-date", r#type: "text", "aria-label": "Date", value: "{f.date}", style: FIELD,
                    oninput: move |e: FormEvent| form.write().date = e.value() }
                if f.all_day {
                    if kind != "task" {
                        input { class: "cal-end-date", r#type: "text", "aria-label": "Last day", value: "{f.end_date}", style: FIELD,
                            oninput: move |e: FormEvent| form.write().end_date = e.value() }
                    }
                } else {
                    div { style: "display: flex; gap: 6px; align-items: center;",
                        input { class: "cal-start", r#type: "text", "aria-label": "Starts", value: "{f.start}", style: FIELD,
                            oninput: move |e: FormEvent| form.write().start = e.value() }
                        span { "–" }
                        input { class: "cal-end", r#type: "text", "aria-label": "Ends", value: "{f.end}", style: FIELD,
                            disabled: kind == "task",
                            oninput: move |e: FormEvent| form.write().end = e.value() }
                    }
                }
                if kind != "task" {
                    div { style: label, "Colour" }
                    div { style: "display: flex; gap: 6px;",
                        for (name, css) in model::COLORS.iter().copied() {
                            {
                            let ring = if f.color == name { "var(--foreground)" } else { "transparent" };
                            rsx! {
                            button { key: "{name}", class: "cal-color", "data-color": "{name}", title: "{name}",
                                style: "width: 20px; height: 20px; border-radius: 10px; background: {css}; cursor: pointer; \
                                        border: 2px solid {ring};",
                                onclick: move |_| name.clone_into(&mut form.write().color) }
                            }
                            }
                        }
                    }
                    if !(occurrence && scope() == Scope::This) {
                        div { style: label, "Repeats" }
                        div { class: "cal-repeats", style: "display: flex; flex-wrap: wrap; gap: 4px;",
                            for (rule, name) in recur::PRESETS.iter().copied() {
                                button { key: "{rule}", style: chip(f.recurrence == rule), onclick: move |_| rule.clone_into(&mut form.write().recurrence), "{name}" }
                            }
                        }
                        if !f.recurrence.is_empty() && !recur::PRESETS.iter().any(|(r, _)| *r == f.recurrence) {
                            div { style: MUTED, {recur::describe(&f.recurrence)} }
                        }
                    }
                    input { class: "cal-location", r#type: "text", "aria-label": "Location", placeholder: "Location", value: "{f.location}", style: FIELD,
                        oninput: move |e: FormEvent| form.write().location = e.value() }
                    textarea { class: "cal-description", "aria-label": "Notes", placeholder: "Notes", value: "{f.description}",
                        style: "{FIELD} height: 64px; padding: 6px 8px;",
                        oninput: move |e: FormEvent| form.write().description = e.value() }
                }
                if occurrence {
                    div { style: label, "Change" }
                    div { class: "cal-scope", style: "display: flex; gap: 4px; flex-wrap: wrap;",
                        for (sc, name) in [(Scope::This, "This event"), (Scope::Following, "This and following"), (Scope::Series, "All events")] {
                            button { key: "{sc.id()}", "data-scope": "{sc.id()}", style: chip(scope() == sc), onclick: move |_| scope.set(sc), "{name}" }
                        }
                    }
                }
                button { class: "cal-save", style: "{TOOL} background: var(--primary); color: var(--primary-foreground); border: none; height: 32px;",
                    onclick: on_save, if kind == "new" { "Create" } else { "Save" } }
            }
            if let Some(path) = path2 {
                button { class: "cal-open", style: TOOL, onclick: move |_| open_note(path.clone()), "Open note" }
            }
            if kind == "event" {
                {
                    let path = path.unwrap_or_default();
                    let p2 = path.clone();
                    rsx! {
                        if occurrence {
                            button { class: "cal-skip", style: "{TOOL} color: var(--destructive);",
                                onclick: move |_| {
                                    let (path, day) = (path.clone(), day);
                                    spawn(async move {
                                        let mut problem = problem;
                                        let mut picked = picked;
                                        match crate::skip(json!({ "path": path, "occurrence": day.map(|d| d.to_string()) })).await {
                                            Ok(_) => picked.set(None),
                                            Err(e) => problem.set(Some(e)),
                                        }
                                    });
                                },
                                "Delete this event"
                            }
                        }
                        button { class: "cal-cancel", style: "{TOOL} color: var(--destructive);",
                            onclick: move |_| {
                                let path = p2.clone();
                                spawn(async move {
                                    let mut problem = problem;
                                    let mut picked = picked;
                                    match crate::cancel(json!({ "path": path })).await {
                                        Ok(_) => picked.set(None),
                                        Err(e) => problem.set(Some(e)),
                                    }
                                });
                            },
                            if occurrence { "Delete the series" } else { "Delete" }
                        }
                    }
                }
            }
        }
    }
}

/// A date or time property's text moved by `days`, in the form it was
/// written in (`2026-11-06`, `2026-11-06T14:00`, with an offset or not).
#[must_use]
pub fn shift_text(text: &str, days: i64) -> Option<String> {
    let t = text.trim();
    let d = Duration::days(days);
    if let Ok(x) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some((x + d).to_string());
    }
    if let Ok(x) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some((x + d).format("%Y-%m-%dT%H:%M:%S%:z").to_string());
    }
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(x) = NaiveDateTime::parse_from_str(t, f) {
            return Some((x + d).format(f).to_string());
        }
    }
    None
}

/// A base's row, placed: its note, title, the property its start is in,
/// and its days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: String,
    pub title: String,
    pub start_key: String,
    pub start_text: String,
    pub end_key: Option<String>,
    pub end_text: Option<String>,
    pub first: NaiveDate,
    /// Inclusive.
    pub last: NaiveDate,
    pub color: String,
    pub group: String,
}

/// The rows of a base view's input placed by `startBy` (else `start`,
/// `scheduled`, `due` or `date`) and `endBy` (else `end`).
#[must_use]
pub fn base_rows(input: &architect_extensions::contrib::BaseViewInput) -> Vec<Row> {
    let opt = |k: &str| {
        input.view["options"][k]
            .as_str()
            .or_else(|| input.view[k].as_str())
            .map(|s| s.strip_prefix("note.").unwrap_or(s).to_owned())
            .filter(|s| !s.is_empty())
    };
    let start_by = opt("startBy");
    let end_by = opt("endBy");
    let mut out = Vec::new();
    let groups = input.rows["groups"].as_array().cloned().unwrap_or_default();
    for g in groups {
        let group = g["label"].as_str().unwrap_or_default().to_owned();
        let rows = crate::vault::rows_of(&json!({ "rows": { "groups": [g] } }));
        for r in rows {
            let pick = |keys: &[String]| {
                keys.iter()
                    .find_map(|k| Some((k.clone(), r.values.get(k)?.as_str()?.to_owned())))
            };
            let mut starts: Vec<String> = start_by.iter().cloned().collect();
            starts.extend(
                ["start", "scheduled", "due", "date"]
                    .iter()
                    .map(|s| (*s).to_owned()),
            );
            let Some((start_key, start_text)) = pick(&starts) else {
                continue;
            };
            let Some(first) =
                model::When::parse(&start_text, &Local).map(|w| w.local(&Local).date())
            else {
                continue;
            };
            let mut ends: Vec<String> = end_by.iter().cloned().collect();
            ends.push("end".to_owned());
            let end = pick(&ends).filter(|(k, _)| *k != start_key);
            let last = end
                .as_ref()
                .and_then(|(_, t)| model::When::parse(t, &Local))
                .map_or(first, |w| match w {
                    // An end date is the day after an all-day one's last.
                    model::When::Day(d) if start_key == "start" => {
                        (d - Duration::days(1)).max(first)
                    }
                    model::When::Day(d) => d.max(first),
                    model::When::At(_) => {
                        let e = w.local(&Local);
                        (if e.time() == NaiveTime::MIN {
                            e.date() - Duration::days(1)
                        } else {
                            e.date()
                        })
                        .max(first)
                    }
                });
            let title = r
                .values
                .get("title")
                .and_then(Value::as_str)
                .map_or_else(|| r.name.clone(), str::to_owned);
            out.push(Row {
                color: r
                    .values
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("primary")
                    .to_owned(),
                path: r.path,
                title,
                start_key,
                start_text,
                end_key: end.as_ref().map(|(k, _)| k.clone()),
                end_text: end.map(|(_, t)| t),
                first,
                last,
                group: group.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.first.cmp(&b.first).then_with(|| a.title.cmp(&b.title)));
    out
}

/// Moves a row by `days` (its start, and its end with it unless `end_only`).
async fn shift_row(r: Row, days: i64, end_only: bool) -> Result<(), String> {
    if !end_only {
        let s = shift_text(&r.start_text, days).ok_or("its start is not a date")?;
        crate::vault::set(&r.path, &r.start_key, &json!(s)).await?;
    }
    if let (Some(k), Some(t)) = (&r.end_key, &r.end_text)
        && let Some(e) = shift_text(t, days)
    {
        crate::vault::set(&r.path, k, &json!(e)).await?;
    }
    let _ = crate::refresh().await;
    Ok(())
}

/// Its Bases view type `calendar` (`[[base_views]] id = "calendar"`): a
/// month of the base's notes on their days; drag one to another.
#[component]
pub fn MonthBase() -> Element {
    let Some(input) = architect_extensions::linked::use_base_view() else {
        return rsx! { div { style: MUTED, "No base." } };
    };
    let rows = base_rows(&input);
    let mut cursor = use_signal(|| {
        rows.iter()
            .map(|r| r.first)
            .find(|d| *d >= today())
            .unwrap_or_else(today)
    });
    let first = crate::settings().week_start;
    let days = layout::days(Mode::Month, cursor(), first);
    let a = days[0];
    let mounted = use_signal(|| None::<Rc<MountedData>>);
    let grid = use_box(mounted);
    let mut press = use_signal(|| None::<(Row, (f64, f64), bool, Option<NaiveDate>)>);
    let mut problem = use_signal(|| None::<String>);
    let over_day = move |x: f64, y: f64| -> Option<NaiveDate> {
        let (bx, by, w, h) = (*grid.peek())?;
        #[allow(clippy::cast_possible_truncation)]
        let (c, r) = (
            ((x - bx) / (w / 7.0)).floor() as i64,
            ((y - by) / (h / 6.0)).floor() as i64,
        );
        ((0..7).contains(&c) && (0..6).contains(&r)).then(|| a + Duration::days(r * 7 + c))
    };
    let lit = press().filter(|p| p.2).and_then(|p| p.3);
    let month = cursor().month();
    rsx! {
        div { class: "cal-base-month", style: "display: flex; flex-direction: column; height: 100%; min-height: 420px; user-select: none; color: var(--foreground); font-size: 12px;",
            onpointermove: move |e: PointerEvent| {
                let Some(mut p) = press.peek().clone() else { return };
                let c = e.data().client_coordinates();
                if !p.2 && (c.x - p.1.0).hypot(c.y - p.1.1) < SLOP { return; }
                p.2 = true;
                p.3 = over_day(c.x, c.y);
                press.set(Some(p));
            },
            onpointerup: move |_| {
                let Some((r, _, active, to)) = press.take() else { return };
                if !active { open_note(r.path); return; }
                if let Some(to) = to && to != r.first {
                    let days = (to - r.first).num_days();
                    spawn(async move { if let Err(e) = shift_row(r, days, false).await { problem.set(Some(e)); } });
                }
            },
            div { style: "display: flex; align-items: center; gap: 6px; padding: 6px 0;",
                button { style: TOOL, onclick: move |_| cursor.set(layout::step(Mode::Month, cursor(), -1)), "‹" }
                button { style: TOOL, onclick: move |_| cursor.set(today()), "Today" }
                button { style: TOOL, onclick: move |_| cursor.set(layout::step(Mode::Month, cursor(), 1)), "›" }
                span { style: "font-weight: 700; font-size: 14px; margin-left: 6px;", {cursor().format("%B %Y").to_string()} }
                if let Some(p) = problem() { span { style: "color: var(--destructive);", "{p}" } }
            }
            div { style: "flex: 1; display: flex; flex-direction: column; border-top: 1px solid var(--border);",
                onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
                for w in 0..6_i64 {
                    div { key: "{w}", style: "flex: 1 1 0; display: flex; min-height: 64px; border-bottom: 1px solid var(--border);",
                        for c in 0..7_i64 {
                            {
                                let day = a + Duration::days(w * 7 + c);
                                let on_day = rows.iter().filter(|r| r.first <= day && day <= r.last).cloned();
                                let bg = if lit == Some(day) { "color-mix(in srgb, var(--primary) 15%, transparent)" } else if day.month() != month { "var(--muted)" } else { "transparent" };
                                let number = day.day();
                                rsx! {
                                    div { key: "{day}", class: "cal-cell", "data-date": "{day}",
                                        style: "flex: 1 1 0; min-width: 0; padding: 2px 3px; border-left: 1px solid var(--border); display: flex; flex-direction: column; gap: 2px; overflow: hidden; \
                                                background: {bg};",
                                        span { style: if day == today() { "font-weight: 700; color: var(--primary);" } else { MUTED }, "{number}" }
                                        for r in on_day {
                                            {
                                                let rr = r.clone();
                                                let chip_colour = model::color_css(&r.color);
                                                rsx! {
                                                    div { key: "{r.path}", class: "cal-base-chip", "data-path": "{r.path}",
                                                        style: "border-left: 3px solid {chip_colour}; background: var(--card); padding: 0 4px; border-radius: 4px; \
                                                                white-space: nowrap; overflow: hidden; text-overflow: ellipsis; cursor: grab;",
                                                        onpointerdown: move |e: PointerEvent| {
                                                            if e.data().pointer_type() != "mouse" { return; }
                                                            let c = e.data().client_coordinates();
                                                            press.set(Some((rr.clone(), (c.x, c.y), false, None)));
                                                        },
                                                        "{r.title}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// How many days the timeline shows at once.
const TIMELINE_DAYS: i64 = 42;

/// The timeline's titles' width.
const LABEL: f64 = 180.0;

/// Its Bases view type `timeline` (`[[base_views]] id = "timeline"`): a
/// schedule, each note a bar from its start to its end over six weeks;
/// drag a bar to move it, its right end to change its end.
#[component]
pub fn TimelineBase() -> Element {
    let Some(input) = architect_extensions::linked::use_base_view() else {
        return rsx! { div { style: MUTED, "No base." } };
    };
    let rows = base_rows(&input);
    let first_day = crate::settings().week_start;
    let mut from = use_signal(|| layout::week_start(today(), first_day) - Duration::days(7));
    let a = from();
    let z = a + Duration::days(TIMELINE_DAYS);
    let shown: Vec<Row> = rows
        .iter()
        .filter(|r| r.last >= a && r.first < z)
        .cloned()
        .collect();
    let mounted = use_signal(|| None::<Rc<MountedData>>);
    let track = use_box(mounted);
    // A bar pressed: its row, where, the end alone, active, days moved.
    let mut press = use_signal(|| None::<(Row, (f64, f64), bool, bool, i64)>);
    let mut problem = use_signal(|| None::<String>);
    let per = |w: f64| ((w - LABEL) / TIMELINE_DAYS as f64).max(1.0);
    let today_left = (today() - a).num_days() as f64 * 100.0 / TIMELINE_DAYS as f64;
    let week_left = |w: i64| w as f64 * 700.0 / TIMELINE_DAYS as f64;
    rsx! {
        div { class: "cal-timeline", style: "display: flex; flex-direction: column; min-height: 240px; user-select: none; color: var(--foreground); font-size: 12px;",
            onpointermove: move |e: PointerEvent| {
                let Some(mut p) = press.peek().clone() else { return };
                let c = e.data().client_coordinates();
                let dx = c.x - p.1.0;
                if !p.3 && dx.hypot(c.y - p.1.1) < SLOP { return; }
                p.3 = true;
                #[allow(clippy::cast_possible_truncation)]
                { p.4 = (dx / per(track.peek().map_or(800.0, |b| b.2))).round() as i64; }
                press.set(Some(p));
            },
            onpointerup: move |_| {
                let Some((r, _, end_only, active, days)) = press.take() else { return };
                if !active { open_note(r.path); return; }
                if days != 0 {
                    spawn(async move { if let Err(e) = shift_row(r, days, end_only).await { problem.set(Some(e)); } });
                }
            },
            div { style: "display: flex; align-items: center; gap: 6px; padding: 6px 0;",
                button { style: TOOL, onclick: move |_| from.set(from() - Duration::weeks(2)), "‹" }
                button { style: TOOL, onclick: move |_| from.set(layout::week_start(today(), first_day) - Duration::days(7)), "Today" }
                button { style: TOOL, onclick: move |_| from.set(from() + Duration::weeks(2)), "›" }
                span { style: "font-weight: 700; font-size: 14px; margin-left: 6px;",
                    {format!("{} – {}", a.format("%b %-d"), (z - Duration::days(1)).format("%b %-d, %Y"))} }
                if let Some(p) = problem() { span { style: "color: var(--destructive);", "{p}" } }
            }
            div { style: "position: relative; display: flex; flex-direction: column;",
                onmounted: move |e: MountedEvent| { let mut m = mounted; m.set(Some(e.data())); },
                // The weeks.
                div { style: "display: flex; border-bottom: 1px solid var(--border); height: 22px;",
                    div { style: "flex: 0 0 {LABEL}px;" }
                    div { style: "flex: 1; position: relative;",
                        for w in 0..(TIMELINE_DAYS / 7) {
                            div { key: "{w}", style: "position: absolute; top: 3px; left: {week_left(w)}%; {MUTED} font-size: 11px; border-left: 1px solid var(--border); padding-left: 4px;",
                                {(a + Duration::weeks(w)).format("%b %-d").to_string()} }
                        }
                        div { style: "position: absolute; top: 0; bottom: -2000px; left: {today_left}%; width: 2px; background: var(--destructive); z-index: 2; pointer-events: none;" }
                    }
                }
                if shown.is_empty() {
                    div { style: "padding: 16px {LABEL}px; {MUTED}", "Nothing in these weeks." }
                }
                for r in shown {
                    {
                        let moved = press().filter(|p| p.3 && p.0.path == r.path).map(|p| (p.2, p.4));
                        let (s_off, e_off) = match moved { Some((true, d)) => (0, d), Some((false, d)) => (d, d), None => (0, 0) };
                        let first = (r.first - a).num_days() + s_off;
                        let last = (r.last - a).num_days() + e_off;
                        let left = first.max(0) as f64 * 100.0 / TIMELINE_DAYS as f64;
                        let width = ((last.min(TIMELINE_DAYS - 1) - first.max(0) + 1).max(1)) as f64 * 100.0 / TIMELINE_DAYS as f64;
                        let colour = model::color_css(&r.color);
                        let (r1, r2) = (r.clone(), r.clone());
                        let opacity = if moved.is_some() { 0.7 } else { 1.0 };
                        let when = if r.first == r.last { r.first.format("%b %-d").to_string() } else { format!("{} – {}", r.first.format("%b %-d"), r.last.format("%b %-d")) };
                        rsx! {
                            div { key: "{r.path}", class: "cal-timeline-row", "data-path": "{r.path}", style: "display: flex; height: 28px; align-items: center; border-bottom: 1px solid var(--border);",
                                div { style: "flex: 0 0 {LABEL}px; padding: 0 8px; box-sizing: border-box; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 600;", "{r.title}" }
                                div { style: "flex: 1; position: relative; height: 100%;",
                                    div { class: "cal-bar", title: "{when}",
                                        style: "position: absolute; top: 5px; height: 18px; left: {left}%; width: {width}%; border-radius: 5px; background: {colour}; color: white; \
                                                padding: 0 6px; box-sizing: border-box; font-size: 11px; line-height: 18px; white-space: nowrap; overflow: hidden; cursor: grab; \
                                                opacity: {opacity};",
                                        onpointerdown: move |e: PointerEvent| {
                                            if e.data().pointer_type() != "mouse" { return; }
                                            let c = e.data().client_coordinates();
                                            press.set(Some((r1.clone(), (c.x, c.y), false, false, 0)));
                                        },
                                        "{when}"
                                        if r.end_key.is_some() {
                                            div { class: "cal-bar-end", style: "position: absolute; right: 0; top: 0; bottom: 0; width: 6px; cursor: ew-resize;",
                                                onpointerdown: move |e: PointerEvent| {
                                                    e.stop_propagation();
                                                    if e.data().pointer_type() != "mouse" { return; }
                                                    let c = e.data().client_coordinates();
                                                    press.set(Some((r2.clone(), (c.x, c.y), true, false, 0)));
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifting_keeps_the_form() {
        assert_eq!(shift_text("2026-11-06", 3).as_deref(), Some("2026-11-09"));
        assert_eq!(
            shift_text("2026-11-06T14:00", -1).as_deref(),
            Some("2026-11-05T14:00")
        );
        assert_eq!(
            shift_text("2026-11-06T14:00:00-06:00", 1).as_deref(),
            Some("2026-11-07T14:00:00-06:00")
        );
        assert_eq!(shift_text("soon", 1), None);
    }

    #[test]
    fn the_form_reads_its_times() {
        let f = Form {
            date: "2026-11-06".into(),
            start: "2pm".into(),
            end: "15:30".into(),
            ..Form::default()
        };
        let (s, e) = f.span().unwrap();
        assert_eq!(
            (s.to_string(), e.to_string()),
            ("2026-11-06 14:00:00".into(), "2026-11-06 15:30:00".into())
        );
        let f = Form {
            date: "2026-11-06".into(),
            start: "23:00".into(),
            end: "01:00".into(),
            ..Form::default()
        };
        assert_eq!(f.span().unwrap().1.date().to_string(), "2026-11-07");
        let f = Form {
            date: "2026-11-06".into(),
            end_date: "2026-11-08".into(),
            all_day: true,
            ..Form::default()
        };
        assert_eq!((f.span().unwrap().1 - f.span().unwrap().0).num_days(), 3);
    }

    #[test]
    fn a_sweep_is_snapped() {
        let d = Drag {
            block: None,
            grip: Grip::Sweep,
            from: (0.0, 0.0),
            active: true,
            day: NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            minute: 9 * 60 + 7,
            minutes: 50,
            days: 0,
        };
        let (s, e) = d.span().unwrap();
        assert_eq!(
            (s.format("%H:%M").to_string(), e.format("%H:%M").to_string()),
            ("09:00".into(), "10:00".into())
        );
    }
}
