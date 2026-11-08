//! The calendar at the foot of a pane (Notebook Navigator's `Calendar`,
//! `CalendarHeader`, `CalendarGrid`, `CalendarDayButton`,
//! `styles/sections/calendar.css`): the month, quarter and year in its
//! header with ‹ Today › beside them, the weekdays, and a row per week,
//! its number first when week numbers show.
//!
//! A click on a day opens its daily note, or makes it (asking first when
//! the settings say so); on a week's number, the month, the quarter or the
//! year, their notes. A day with a note has a dot, or the note's feature
//! image; one with tasks left a ring; today is lit; the note open is
//! ringed.

use dioxus::prelude::*;
use lucide_dioxus::{ChevronLeft, ChevronRight};

use super::{Navigator, Open};
use crate::calendar::{CalendarDay, HeaderPeriod};
use crate::list::Thumb;
use crate::period::{Period, PeriodKind};
use crate::state::OpenIn;

/// A day's cell: Notebook Navigator's squares, at a sidebar's width.
const DAY: f64 = 26.0;

/// The calendar.
#[component]
pub(super) fn Calendar(nav: Navigator, on_open: EventHandler<Open>) -> Element {
    let s = nav.state.read();
    let g = s.calendar();
    let settings = s.settings().clone();
    drop(s);
    let source = nav.source();
    let week_numbers = settings.calendar_week_numbers;
    let highlight = settings.calendar_highlight_today;
    let open = move |period: Period, how: OpenIn| {
        let out = nav.state.peek().open_period(period, how);
        nav.open_or_create(out, on_open);
    };
    // Rows of seven equal cells, after a week's number when they show.
    let row = "display:flex; gap:1px;";
    let cell = "flex:1 1 0; min-width:0;";
    rsx! {
        div { class: "nv-calendar", role: "group", "aria-label": "Calendar",
            "data-highlight-today": if highlight { "true" } else { "false" },
            style: "flex:0 0 auto; padding:6px 8px 8px; border-top:1px solid var(--nv-divider); background:var(--nv-background); \
                    display:flex; flex-direction:column; gap:2px; position:relative; z-index:2;",
            div { class: "nv-cal-header", style: "display:flex; align-items:center; gap:2px; height:26px;",
                HeaderButton { head: g.month.clone(), strong: true, on_pick: move |p| open(p, OpenIn::Here) }
                if settings.calendar_quarter {
                    HeaderButton { head: g.quarter.clone(), strong: false, on_pick: move |p| open(p, OpenIn::Here) }
                }
                HeaderButton { head: g.year.clone(), strong: false, on_pick: move |p| open(p, OpenIn::Here) }
                div { style: "flex:1;" }
                button { class: "nv-tool", "aria-label": "Previous", "data-testid": "nv-cal-prev", style: "width:22px; height:22px;",
                    onclick: move |_| nav.update(|s| s.calendar_move(-1)),
                    ChevronLeft { size: 15 }
                }
                button { class: "nv-cal-today", "aria-label": "Today", "data-testid": "nv-cal-today",
                    style: "height:22px; padding:0 4px; border:none; border-radius:6px; background:transparent; color:var(--nv-muted); font-size:11px; cursor:pointer;",
                    onclick: move |_| {
                        let out = nav.update(crate::state::NavigatorState::calendar_today);
                        nav.act(out, on_open);
                    },
                    "Today"
                }
                button { class: "nv-tool", "aria-label": "Next", "data-testid": "nv-cal-next", style: "width:22px; height:22px;",
                    onclick: move |_| nav.update(|s| s.calendar_move(1)),
                    ChevronRight { size: 15 }
                }
            }
            div { class: "nv-cal-weekdays", style: "{row} height:18px; align-items:center; font-size:10px; color:var(--nv-faint); text-align:center;",
                if week_numbers { span { style: "flex:0 0 22px;" } }
                for (i, w) in g.weekdays.iter().enumerate() {
                    span { key: "{i}", style: "{cell}", "{w}" }
                }
            }
            for week in g.weeks {
                div { key: "{week.start}", class: "nv-cal-week", style: "{row} height:{DAY}px;",
                    if week_numbers {
                        {
                            let period = Period { kind: PeriodKind::Week, start: week.start };
                            let class = format!(
                                "nv-cal-weeknumber{}{}",
                                if week.note.is_some() { " has-note" } else { "" },
                                if week.active { " is-active" } else { "" },
                            );
                            rsx! {
                                button { class, "aria-label": "Week {week.number}",
                                    disabled: !g.week_notes,
                                    style: "flex:0 0 22px; position:relative; height:{DAY}px; border-radius:6px; background:transparent; color:var(--nv-faint); font-size:10px; padding:0; cursor:pointer;",
                                    onclick: move |_| open(period, OpenIn::Here),
                                    "{week.number}"
                                    if week.note.is_some() { span { class: "nv-cal-dot" } }
                                }
                            }
                        }
                    }
                    for day in week.days {
                        DayCell { key: "{day.date}", day: day.clone(), image: image_url(&source, day.image.as_ref()),
                            on_pick: move |(date, tab): (chrono::NaiveDate, bool)| {
                                let period = Period { kind: PeriodKind::Day, start: date };
                                open(period, if tab { OpenIn::Tab } else { OpenIn::Here });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The URL a day's image draws from.
fn image_url(
    source: &std::sync::Arc<dyn crate::source::NavigatorSource>,
    image: Option<&Thumb>,
) -> Option<String> {
    match image? {
        Thumb::Url(u) => Some(u.clone()),
        Thumb::File(p) => source.resource_url(p),
        Thumb::Badge(_) => None,
    }
}

#[component]
fn HeaderButton(head: HeaderPeriod, strong: bool, on_pick: EventHandler<Period>) -> Element {
    let class = format!(
        "nv-cal-period{}{}",
        if head.note.is_some() { " has-note" } else { "" },
        if head.active { " is-active" } else { "" },
    );
    let (weight, colour) = if strong {
        (600, "var(--nv-text)")
    } else {
        (400, "var(--nv-muted)")
    };
    let period = head.period;
    rsx! {
        button { class, "data-period": "{period.kind.id()}", disabled: !head.enabled,
            style: "position:relative; height:26px; padding:0 4px; border-radius:6px; background:transparent; \
                    color:{colour}; font-size:13px; font-weight:{weight}; cursor:pointer; white-space:nowrap;",
            onclick: move |_| on_pick.call(period),
            "{head.label}"
            if head.note.is_some() { span { class: "nv-cal-dot" } }
        }
    }
}

#[component]
fn DayCell(
    day: CalendarDay,
    image: Option<String>,
    on_pick: EventHandler<(chrono::NaiveDate, bool)>,
) -> Element {
    if day.hidden {
        return rsx! { span { class: "nv-cal-spacer", style: "flex:1 1 0; min-width:0;" } };
    }
    let mut class = String::from("nv-cal-day");
    for (on, name) in [
        (day.today, " is-today"),
        (!day.in_month, " is-outside"),
        (day.note.is_some(), " has-note"),
        (image.is_some(), " has-image"),
        (day.active, " is-active"),
        (day.unfinished > 0, " has-tasks"),
    ] {
        if on {
            class.push_str(name);
        }
    }
    let n = chrono::Datelike::day(&day.date);
    let date = day.date;
    let background = image.as_deref().map_or_else(String::new, |u| {
        format!(
            "background-image:url(\"{}\"); background-size:cover; background-position:center;",
            u.replace('"', "%22")
        )
    });
    let label = date.format("%A, %B %-d, %Y").to_string();
    rsx! {
        div { class: if day.weekend { "nv-cal-cell is-weekend" } else { "nv-cal-cell" },
            style: "flex:1 1 0; min-width:0; position:relative; height:{DAY}px; border-radius:6px;",
            button { class, "data-date": "{date}", "aria-label": "{label}",
                style: "position:absolute; inset:1px; border-radius:6px; padding:0; cursor:pointer; \
                        display:flex; align-items:center; justify-content:center; font-size:12px; {background}",
                onclick: move |e: MouseEvent| {
                    let m = e.modifiers();
                    let tab = if cfg!(target_os = "macos") { m.meta() } else { m.ctrl() };
                    on_pick.call((date, tab));
                },
                span { class: "nv-cal-number", "{n}" }
                if day.note.is_some() && (image.is_none() || day.unfinished > 0) {
                    span { class: "nv-cal-dot" }
                }
            }
        }
    }
}

// ── A section's month grid (#444) ───────────────────────────────────

/// The grid's padding over its header, and at its sides.
pub(super) const GRID_TOP: f64 = 6.0;
pub(super) const GRID_SIDE: f64 = 8.0;
/// Its header's and weekdays' heights, the gap between its rows.
const GRID_HEADER: f64 = 26.0;
const GRID_WEEKDAYS: f64 = 18.0;
const GRID_GAP: f64 = 2.0;
/// A week's number's column.
const GRID_WEEK_NUMBER: f64 = 22.0;

/// Where the grid's weeks start, down from its top.
pub(super) const GRID_WEEKS_AT: f64 = GRID_TOP + GRID_HEADER + GRID_GAP + GRID_WEEKDAYS + GRID_GAP;

/// The day under `x`, `y` (from the grid row's top left, `width` wide),
/// of `weeks` as drawn: a drop's day.
#[must_use]
pub(super) fn grid_day_at(
    weeks: &[crate::calendar::CalendarWeek],
    week_numbers: bool,
    width: f64,
    x: f64,
    y: f64,
) -> Option<chrono::NaiveDate> {
    let left = GRID_SIDE
        + if week_numbers {
            GRID_WEEK_NUMBER + 1.0
        } else {
            0.0
        };
    let w = width - left - GRID_SIDE;
    if y < GRID_WEEKS_AT || x < left || w <= 0.0 {
        return None;
    }
    let week = ((y - GRID_WEEKS_AT) / (DAY + GRID_GAP)).floor();
    let day = ((x - left) / (w / 7.0)).floor();
    if !(0.0..7.0).contains(&day) || week < 0.0 {
        return None;
    }
    // Both are small and whole by now.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions
    )]
    let (week, day) = (week as usize, day as usize);
    let d = weeks.get(week)?.days.get(day)?;
    (!d.hidden).then_some(d.date)
}

/// The month grid under a provider's row in the left list (the Calendar
/// section, #444): Notebook Navigator's calendar a whole month at a
/// time, with the provider's own on each day: its dots (events, tasks),
/// its mark (the day's template, coloured), and its day selected when
/// that day's list shows. A click selects the day and opens its daily
/// note; an event or a task dropped on a day moves there.
#[component]
pub(super) fn SectionGrid(
    nav: Navigator,
    on_open: EventHandler<Open>,
    mount: String,
    top: f64,
    height: f64,
) -> Element {
    let s = nav.state.read();
    let g = s.section_grid();
    let days = s
        .section_calendar()
        .filter(|(id, _)| *id == mount)
        .map(|(_, c)| c.days.clone())
        .unwrap_or_default();
    let selected = s.selected_section_day();
    let settings = s.settings().clone();
    drop(s);
    let over = nav
        .drag
        .read()
        .as_ref()
        .and_then(|d| d.over.clone())
        .unwrap_or_default();
    let week_numbers = settings.calendar_week_numbers;
    let open = move |period: Period, how: OpenIn| {
        let out = nav.state.peek().open_period(period, how);
        nav.open_or_create(out, on_open);
    };
    let row = "display:flex; gap:1px;";
    let cell = "flex:1 1 0; min-width:0;";
    rsx! {
        div { class: "nv-calendar nv-section-grid", role: "group", "aria-label": "Month",
            "data-testid": "nv-section-grid",
            "data-highlight-today": if settings.calendar_highlight_today { "true" } else { "false" },
            style: "position:absolute; left:0; right:0; top:{top}px; height:{height}px; box-sizing:border-box; \
                    padding:{GRID_TOP}px {GRID_SIDE}px 0; display:flex; flex-direction:column; gap:{GRID_GAP}px;",
            div { class: "nv-cal-header", style: "display:flex; align-items:center; gap:2px; height:{GRID_HEADER}px;",
                HeaderButton { head: g.month.clone(), strong: true, on_pick: move |p| open(p, OpenIn::Here) }
                HeaderButton { head: g.year.clone(), strong: false, on_pick: move |p| open(p, OpenIn::Here) }
                div { style: "flex:1;" }
                button { class: "nv-tool", "aria-label": "Previous month", "data-testid": "nv-grid-prev", style: "width:22px; height:22px;",
                    onclick: move |_| nav.update(|s| s.section_move(-1)),
                    ChevronLeft { size: 15 }
                }
                button { class: "nv-cal-today", "aria-label": "Today", "data-testid": "nv-grid-today",
                    style: "height:22px; padding:0 4px; border:none; border-radius:6px; background:transparent; color:var(--nv-muted); font-size:11px; cursor:pointer;",
                    onclick: move |_| {
                        let out = nav.update(crate::state::NavigatorState::section_today);
                        nav.act(out, on_open);
                    },
                    "Today"
                }
                button { class: "nv-tool", "aria-label": "Next month", "data-testid": "nv-grid-next", style: "width:22px; height:22px;",
                    onclick: move |_| nav.update(|s| s.section_move(1)),
                    ChevronRight { size: 15 }
                }
            }
            div { class: "nv-cal-weekdays", style: "{row} height:{GRID_WEEKDAYS}px; align-items:center; font-size:10px; color:var(--nv-faint); text-align:center;",
                if week_numbers { span { style: "flex:0 0 {GRID_WEEK_NUMBER}px;" } }
                for (i, w) in g.weekdays.iter().enumerate() {
                    span { key: "{i}", style: "{cell}", "{w}" }
                }
            }
            for week in g.weeks {
                div { key: "{week.start}", class: "nv-cal-week", style: "{row} height:{DAY}px;",
                    if week_numbers {
                        {
                            let period = Period { kind: PeriodKind::Week, start: week.start };
                            let class = format!("nv-cal-weeknumber{}", if week.note.is_some() { " has-note" } else { "" });
                            rsx! {
                                button { class, "aria-label": "Week {week.number}", disabled: !g.week_notes,
                                    style: "flex:0 0 {GRID_WEEK_NUMBER}px; position:relative; height:{DAY}px; border-radius:6px; background:transparent; color:var(--nv-faint); font-size:10px; padding:0; cursor:pointer;",
                                    onclick: move |_| open(period, OpenIn::Here),
                                    "{week.number}"
                                    if week.note.is_some() { span { class: "nv-cal-dot" } }
                                }
                            }
                        }
                    }
                    for day in week.days {
                        {
                            let date = day.date;
                            let mark = days.get(&date).cloned();
                            let drop_key = format!("grid:{mount}|{date}");
                            rsx! {
                                GridCell { key: "{date}", day, mark,
                                    selected: selected == Some(date),
                                    drop_here: over == drop_key,
                                    on_pick: move |tab: bool| {
                                        let out = nav.update(|s| s.pick_section_day(date));
                                        nav.act(out, on_open);
                                        open(Period { kind: PeriodKind::Day, start: date }, if tab { OpenIn::Tab } else { OpenIn::Here });
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

/// A day of a section's grid.
#[component]
fn GridCell(
    day: CalendarDay,
    mark: Option<crate::mounts::SectionDay>,
    selected: bool,
    drop_here: bool,
    on_pick: EventHandler<bool>,
) -> Element {
    if day.hidden {
        return rsx! { span { class: "nv-cal-spacer", style: "flex:1 1 0; min-width:0;" } };
    }
    let mut class = String::from("nv-cal-day");
    for (on, name) in [
        (day.today, " is-today"),
        (!day.in_month, " is-outside"),
        (day.note.is_some(), " has-note"),
        (selected, " is-selected"),
        (drop_here, " is-drop"),
    ] {
        if on {
            class.push_str(name);
        }
    }
    let n = chrono::Datelike::day(&day.date);
    let date = day.date;
    let count = mark.as_ref().map_or(0, |m| m.count.min(3));
    let colour = mark.as_ref().and_then(|m| m.colour.clone());
    let mut label = date.format("%A, %B %-d, %Y").to_string();
    if let Some(l) = mark.as_ref().and_then(|m| m.label.as_deref()) {
        label = format!("{label} · {l}");
    }
    if let Some(m) = &mark
        && m.count > 0
    {
        label = format!("{label} · {} on it", m.count);
    }
    rsx! {
        div { class: if day.weekend { "nv-cal-cell is-weekend" } else { "nv-cal-cell" },
            style: "flex:1 1 0; min-width:0; position:relative; height:{DAY}px; border-radius:6px;",
            button { class, "data-date": "{date}", "aria-label": "{label}", title: "{label}",
                style: "position:absolute; inset:1px; border-radius:6px; padding:0; cursor:pointer; \
                        display:flex; align-items:center; justify-content:center; font-size:12px;",
                onclick: move |e: MouseEvent| {
                    let m = e.modifiers();
                    on_pick.call(if cfg!(target_os = "macos") { m.meta() } else { m.ctrl() });
                },
                if let Some(c) = colour {
                    span { class: "nv-grid-mark", style: "background:{c};" }
                }
                span { class: "nv-cal-number", "{n}" }
                if count > 0 {
                    span { class: "nv-grid-dots",
                        for i in 0..count { span { key: "{i}", class: "nv-grid-dot" } }
                    }
                }
            }
        }
    }
}

/// The calendar's sheet, over the roles the root sets. The rings are
/// borders, the dots spans: no pseudo-elements, no inset shadows.
pub(super) const CSS: &str = "
.nv-root .nv-cal-cell.is-weekend { background: var(--nv-calendar-weekend); }
.nv-root .nv-cal-day, .nv-root .nv-cal-period, .nv-root .nv-cal-weeknumber { border: 1.5px solid transparent; }
.nv-root .nv-cal-day { background-color: transparent; color: var(--nv-text); }
.nv-root .nv-cal-day:hover { background-color: var(--nv-hover); }
.nv-root .nv-cal-day.is-outside { color: var(--nv-faint); }
.nv-root [data-highlight-today=true] .nv-cal-day.is-today { background-color: var(--nv-calendar-today); color: var(--nv-calendar-today-text); font-weight: 700; }
.nv-root .nv-cal-day.is-active, .nv-root .nv-cal-period.is-active, .nv-root .nv-cal-weeknumber.is-active { border-color: var(--nv-open); }
.nv-root .nv-cal-period .nv-cal-dot { bottom: 0; }
.nv-root .nv-cal-dot { position: absolute; left: 50%; bottom: 2px; width: 4px; height: 4px; margin-left: -2px; border-radius: 2px; background: var(--nv-calendar-dot); }
.nv-root .nv-cal-day.has-tasks .nv-cal-dot { background: transparent; border: 1px solid var(--nv-calendar-task); width: 3px; height: 3px; }
.nv-root .nv-cal-day.has-image { color: var(--nv-calendar-image-text); font-weight: 600; }
.nv-root .nv-cal-day.has-image .nv-cal-number { background: var(--nv-calendar-image-shade); border-radius: 4px; padding: 0 3px; line-height: 15px; }
.nv-root .nv-cal-period:hover, .nv-root .nv-cal-weeknumber:hover, .nv-root .nv-cal-today:hover { background-color: var(--nv-hover); }
.nv-root .nv-cal-period:disabled, .nv-root .nv-cal-weeknumber:disabled { cursor: default; }
.nv-root .nv-section-grid .nv-cal-day.has-note .nv-cal-number { font-weight: 650; text-decoration: underline; text-decoration-color: var(--nv-calendar-dot); text-underline-offset: 2px; }
.nv-root .nv-section-grid .nv-cal-day.is-selected { border-color: var(--nv-open); background-color: var(--nv-hover); }
.nv-root .nv-section-grid .nv-cal-day.is-drop { border-color: var(--nv-open); border-style: dashed; }
.nv-root .nv-grid-mark { position: absolute; left: 5px; right: 5px; top: 1px; height: 2px; border-radius: 1px; opacity: 0.85; }
.nv-root .nv-grid-dots { position: absolute; left: 0; right: 0; bottom: 2px; display: flex; justify-content: center; gap: 2px; }
.nv-root .nv-grid-dot { width: 3px; height: 3px; border-radius: 2px; background: var(--nv-calendar-dot); }
";
