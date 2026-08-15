//! Date picker — primitive-backed date input and calendar popover.

use dioxus::prelude::*;
use dioxus_primitives::date_picker::{
    DatePicker as PrimitiveDatePicker, DatePickerCalendar as PrimitiveDatePickerCalendar,
    DatePickerInput as PrimitiveDatePickerInput, DatePickerPopover as PrimitiveDatePickerPopover,
};
use dioxus_primitives::popover::{
    PopoverContent as PrimitivePopoverContent, PopoverTrigger as PrimitivePopoverTrigger,
};
use dioxus_primitives::ContentAlign;
use architect_story_runtime::story;
use time::{macros::date, Date};

#[component]
pub fn DatePicker(
    #[props(default)] selected_date: Option<Date>,
    #[props(default)] on_value_change: Option<Callback<Option<Date>>>,
    #[props(default = false)] disabled: bool,
    #[props(default = false)] read_only: bool,
    #[props(default = date!(1925-01-01))] min_date: Date,
    #[props(default = date!(2050-12-31))] max_date: Date,
    #[props(default = false)] roving_loop: bool,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitiveDatePicker {
            selected_date,
            disabled,
            read_only,
            min_date,
            max_date,
            roving_loop,
            on_value_change: move |date| {
                if let Some(callback) = &on_value_change {
                    callback.call(date);
                }
            },
            class: crate::cn::merge_slice(&["relative inline-flex flex-col gap-2", class.as_str()]),
            {children}
        }
    }
}

#[component]
pub fn DatePickerPopover(
    #[props(default = true)] is_modal: bool,
    #[props(default)] open: Option<bool>,
    #[props(default = false)] default_open: bool,
    #[props(default)] on_open_change: Option<Callback<bool>>,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitiveDatePickerPopover {
            is_modal,
            open,
            default_open,
            on_open_change: move |open| {
                if let Some(callback) = &on_open_change {
                    callback.call(open);
                }
            },
            class,
            {children}
        }
    }
}

#[component]
pub fn DatePickerInput(
    #[props(default)] class: String,
    #[props(default)] children: Element,
) -> Element {
    rsx! {
        PrimitiveDatePickerInput {
            class: crate::cn::merge_slice(&[
                "inline-flex h-9 items-center gap-1 rounded-lg border border-input bg-transparent px-3 text-sm shadow-xs outline-none focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50",
                "[&_.date-segment]:min-w-[2ch] [&_.date-segment]:rounded [&_.date-segment]:px-0.5 [&_.date-segment]:text-center [&_.date-segment:focus]:bg-accent [&_.date-segment:focus]:outline-none [&_.date-segment[no-date=true]]:text-muted-foreground",
                class.as_str(),
            ]),
            {children}
        }
    }
}

#[component]
pub fn DatePickerTrigger(#[props(default)] class: String, children: Element) -> Element {
    rsx! {
        PrimitivePopoverTrigger {
            class: crate::cn::merge_slice(&["inline-flex cursor-pointer", class.as_str()]),
            {children}
        }
    }
}

#[component]
pub fn DatePickerContent(
    #[props(default = ContentAlign::End)] align: ContentAlign,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitivePopoverContent {
            align,
            class: crate::cn::merge_slice(&[
                "absolute z-50 mt-2 rounded-lg border border-border bg-popover p-2 text-popover-foreground shadow-md",
                class.as_str(),
            ]),
            {children}
        }
    }
}

#[component]
pub fn DatePickerCalendar(#[props(default)] class: String) -> Element {
    rsx! {
        PrimitiveDatePickerCalendar {
            class,
            crate::components::Calendar {}
        }
    }
}

/// Default DatePicker story rendering input + popover calendar.
#[story(category = "DatePicker", name = "default")]
pub fn date_picker_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            DatePicker {
                DatePickerPopover {
                    DatePickerTrigger {
                        DatePickerInput {}
                    }
                    DatePickerContent {
                        DatePickerCalendar {}
                    }
                }
            }
        }
    }
}
