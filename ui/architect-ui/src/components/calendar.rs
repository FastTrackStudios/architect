//! Calendar — primitive-backed month calendar for date picking.

use architect_story_runtime::story;
use chrono::{Datelike, Local, NaiveDate};
use dioxus::prelude::*;
use dioxus_primitives::calendar::{
    Calendar as PrimitiveCalendar, CalendarGrid, CalendarHeader, CalendarMonthTitle,
    CalendarNavigation, CalendarNextMonthButton, CalendarPreviousMonthButton,
};
use time::{Date, Month, UtcDateTime, Weekday};

#[derive(Props, Clone, PartialEq)]
pub struct CalendarProps {
    /// The currently selected date, if any.
    #[props(default)]
    pub selected: Option<NaiveDate>,

    /// Callback fired when the user selects a day.
    #[props(default)]
    pub on_select: Option<Callback<NaiveDate>>,

    /// Whether the calendar is disabled.
    #[props(default = false)]
    pub disabled: bool,

    /// First day of the week.
    #[props(default = Weekday::Monday)]
    pub first_day_of_week: Weekday,

    /// Extra CSS classes on the root element.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn Calendar(props: CalendarProps) -> Element {
    let today = chrono_to_time(Local::now().date_naive());
    let selected = props.selected.and_then(chrono_to_time_checked);
    let mut view_date = use_signal(|| selected.unwrap_or_else(|| UtcDateTime::now().date()));

    rsx! {
        PrimitiveCalendar {
            selected_date: selected,
            today,
            view_date: view_date(),
            disabled: props.disabled,
            first_day_of_week: props.first_day_of_week,
            on_view_change: move |date| view_date.set(date),
            on_format_weekday: move |weekday| weekday_abbreviation(weekday).to_string(),
            on_format_month: move |month| month_name(month).to_string(),
            on_date_change: move |date: Option<Date>| {
                if let (Some(callback), Some(date)) = (&props.on_select, date.and_then(time_to_chrono_checked)) {
                    callback.call(date);
                }
            },
            class: crate::cn::merge_slice(&["p-3 bg-background w-fit", props.class.as_str()]),
            CalendarHeader {
                class: "flex items-center justify-between mb-4",
                CalendarNavigation {
                    class: "flex w-full items-center justify-between",
                    CalendarPreviousMonthButton {
                        class: "inline-flex items-center justify-center size-8 rounded-lg hover:bg-muted transition-colors",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            width: "16",
                            height: "16",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            path { d: "m15 18-6-6 6-6" }
                        }
                    }
                    CalendarMonthTitle {
                        class: "text-sm font-medium select-none",
                    }
                    CalendarNextMonthButton {
                        class: "inline-flex items-center justify-center size-8 rounded-lg hover:bg-muted transition-colors",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            width: "16",
                            height: "16",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            path { d: "m9 18 6-6-6-6" }
                        }
                    }
                }
            }
            CalendarGrid {
                class: "border-separate border-spacing-1 text-sm [&_.calendar-grid-day-header]:h-8 [&_.calendar-grid-day-header]:w-8 [&_.calendar-grid-day-header]:text-xs [&_.calendar-grid-day-header]:font-normal [&_.calendar-grid-day-header]:text-muted-foreground [&_.calendar-grid-cell]:flex [&_.calendar-grid-cell]:size-8 [&_.calendar-grid-cell]:items-center [&_.calendar-grid-cell]:justify-center [&_.calendar-grid-cell]:rounded-full [&_.calendar-grid-cell]:transition-colors [&_.calendar-grid-cell]:hover:bg-muted [&_.calendar-grid-cell[data-month=last]]:text-muted-foreground/50 [&_.calendar-grid-cell[data-month=next]]:text-muted-foreground/50 [&_.calendar-grid-cell[data-selected=true]]:bg-primary [&_.calendar-grid-cell[data-selected=true]]:text-primary-foreground [&_.calendar-grid-cell[data-today=true]]:font-semibold",
            }
        }
    }
}

fn chrono_to_time(date: NaiveDate) -> Date {
    chrono_to_time_checked(date).expect("chrono NaiveDate should convert to time Date")
}

fn chrono_to_time_checked(date: NaiveDate) -> Option<Date> {
    let month = Month::try_from(date.month() as u8).ok()?;
    Date::from_calendar_date(date.year(), month, date.day() as u8).ok()
}

fn time_to_chrono_checked(date: Date) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year(), date.month() as u32, date.day() as u32)
}

fn weekday_abbreviation(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

fn month_name(month: Month) -> &'static str {
    match month {
        Month::January => "January",
        Month::February => "February",
        Month::March => "March",
        Month::April => "April",
        Month::May => "May",
        Month::June => "June",
        Month::July => "July",
        Month::August => "August",
        Month::September => "September",
        Month::October => "October",
        Month::November => "November",
        Month::December => "December",
    }
}

/// Default Calendar story rendering today's month grid.
#[story(category = "Calendar", name = "default")]
pub fn calendar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            div { class: "rounded-lg border border-border bg-card text-card-foreground shadow-sm w-fit",
                Calendar {}
            }
        }
    }
}
