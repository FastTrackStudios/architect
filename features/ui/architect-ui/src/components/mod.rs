//! FTS UI components.

// --- Wrapped components (FTS defaults applied) ---
mod button;
mod input;

pub use button::*;
pub use input::*;

// --- Standalone reimplementations (were lumen-blocks) ---
mod checkbox;
mod context_menu;
mod dropdown;
mod label;
mod progress;
mod side_sheet;
mod switch;
pub mod toast; // toast stays pub mod for toast::use_toast() access

pub use checkbox::*;
pub use context_menu::*;
pub use dropdown::*;
pub use label::*;
pub use progress::*;
pub use side_sheet::*;
pub use switch::*;
pub use toast::*;

// --- Migrated from signal-ui ---
mod dialog;
mod tabs;

pub use dialog::*;
pub use tabs::*;

// --- New FTS components ---
mod nav_tab;
mod progress_bar;

pub use nav_tab::*;
pub use progress_bar::*;

mod badge;
mod button_group;
mod card;
mod data_table;
mod direction;
mod empty_state;
mod form;
mod form_field;
mod inline_edit;
mod key_value_row;
mod list_row;
mod searchable_dropdown;
mod searchable_list;
mod section_header;
mod segmented_control;
mod status;

pub use badge::*;
pub use button_group::*;
pub use card::*;
pub use data_table::*;
pub use direction::*;
pub use empty_state::*;
pub use form::*;
pub use form_field::*;
pub use inline_edit::*;
pub use key_value_row::*;
pub use list_row::*;
pub use searchable_dropdown::*;
pub use searchable_list::*;
pub use section_header::*;
pub use segmented_control::*;
pub use status::*;

mod alert;
mod alert_dialog;
mod breadcrumb;
mod drawer;
mod field;
mod item;
mod kbd;
mod native_select;
mod scroll_area;
mod skeleton;
mod spinner;
mod textarea;

pub use alert::*;
pub use alert_dialog::*;
pub use breadcrumb::*;
pub use drawer::*;
pub use field::*;
pub use item::*;
pub use kbd::*;
pub use native_select::*;
pub use scroll_area::*;
pub use skeleton::*;
pub use spinner::*;
pub use textarea::*;

mod popover;
mod table;
mod toggle;
mod toggle_group;

pub use popover::*;
pub use table::*;
pub use toggle::*;
pub use toggle_group::*;

mod color_picker;
mod radio_group;
mod select;
mod slider;
mod tooltip;

pub use color_picker::*;
pub use radio_group::*;
pub use select::*;
pub use slider::*;
pub use tooltip::*;

mod accordion;
mod collapsible;
mod hover_card;

pub use accordion::*;
pub use collapsible::*;
pub use hover_card::*;

mod avatar;
mod input_group;
mod pagination;

pub use avatar::*;
pub use input_group::*;
pub use pagination::*;

mod combobox;
mod command;
mod menubar;

pub use combobox::*;
pub use command::*;
pub use menubar::*;

mod aspect_ratio;
#[cfg(feature = "router")]
mod navbar;
mod navigation_menu;

pub use aspect_ratio::*;
#[cfg(feature = "router")]
pub use navbar::*;
pub use navigation_menu::*;

mod calendar;
mod date_picker;
mod resizable;
mod toolbar;

pub use calendar::*;
pub use date_picker::*;
pub use resizable::*;
pub use toolbar::*;

mod sidebar;

pub use sidebar::*;

mod carousel;
mod input_otp;

pub use carousel::*;
pub use input_otp::*;
