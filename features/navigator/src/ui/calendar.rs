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
";
