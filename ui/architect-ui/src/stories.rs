//! `architect-story` registrations for architect-ui components.
//!
//! Each `#[story]` block expands to a static `Story` value plus a
//! `linkme` registration into the global `STORIES` slice. The shell
//! enumerates them; the snapshot harness iterates them × their auto
//! state matrix.
//!
//! Stories assume an enclosing `ThemeProvider` is supplied by the host
//! app (the Lookbook shell wraps everything in one global
//! `ThemeProvider`, which lets the host pick the active preset / mode
//! from the top bar). Story render bodies should NOT instantiate their
//! own provider; doing so breaks the global theme switcher.

use crate::prelude::*;
use dioxus::prelude::*;
use architect_story_runtime::story;

// ── Buttons ──────────────────────────────────────────────────────────────────

/// Single button rendered with the active theme preset.
#[story(
    category = "Buttons",
    name = "primary",
    knobs(label = "Click me", disabled = false,)
)]
pub fn button_primary(label: &str, disabled: bool) -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Button { variant: ButtonVariant::Primary, disabled, "{label}" }
        }
    }
}

/// All `ButtonVariant` values laid out side by side.
#[story(category = "Buttons", name = "variants")]
pub fn button_variants() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-wrap gap-2",
            Button { variant: ButtonVariant::Primary, "Primary" }
            Button { variant: ButtonVariant::Secondary, "Secondary" }
            Button { variant: ButtonVariant::Outline, "Outline" }
            Button { variant: ButtonVariant::Ghost, "Ghost" }
            Button { variant: ButtonVariant::Destructive, "Destructive" }
            Button { variant: ButtonVariant::Link, "Link" }
        }
    }
}

/// Matrix of every `ButtonVariant` × every state — Enabled, Disabled,
/// Loading. Designed to surface cross-renderer styling drift in one
/// snapshot: rows are variants, columns are states. Each cell is
/// labelled so you can map a divergence in the composite back to the
/// exact (variant, state) pair without counting.
#[story(category = "Buttons", name = "matrix")]
pub fn button_matrix() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            table { class: "border-collapse",
                thead {
                    tr { class: "text-xs uppercase tracking-wider text-muted-foreground",
                        th { class: "px-3 py-2 text-left", "Variant" }
                        th { class: "px-3 py-2 text-left", "Enabled" }
                        th { class: "px-3 py-2 text-left", "Disabled" }
                        th { class: "px-3 py-2 text-left", "Loading" }
                    }
                }
                tbody {
                    ButtonRow { label: "Primary", variant: ButtonVariant::Primary }
                    ButtonRow { label: "Secondary", variant: ButtonVariant::Secondary }
                    ButtonRow { label: "Outline", variant: ButtonVariant::Outline }
                    ButtonRow { label: "Ghost", variant: ButtonVariant::Ghost }
                    ButtonRow { label: "Destructive", variant: ButtonVariant::Destructive }
                    ButtonRow { label: "Link", variant: ButtonVariant::Link }
                }
            }
        }
    }
}

#[component]
fn ButtonRow(label: String, variant: ButtonVariant) -> Element {
    rsx! {
        tr {
            td { class: "px-3 py-2 text-sm text-muted-foreground", "{label}" }
            td { class: "px-3 py-2",
                Button { variant: variant.clone(), "Click me" }
            }
            td { class: "px-3 py-2",
                Button { variant: variant.clone(), disabled: true, "Click me" }
            }
            td { class: "px-3 py-2",
                Button { variant: variant.clone(), loading: true, "Click me" }
            }
        }
    }
}

/// All button sizes for the Primary variant. Catches per-renderer
/// drift in font scaling, padding, and intrinsic sizing.
#[story(category = "Buttons", name = "sizes")]
pub fn button_sizes() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-wrap items-center gap-3",
            Button { size: ButtonSize::Small, "Small" }
            Button { size: ButtonSize::Medium, "Medium" }
            Button { size: ButtonSize::Large, "Large" }
        }
    }
}

// ── Cards ────────────────────────────────────────────────────────────────────

/// Card with header, content, and footer regions.
#[story(
    category = "Card",
    name = "basic",
    knobs(title = "Project Alpha", description = "A sample project card.",)
)]
pub fn card_basic(title: &str, description: &str) -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Card {
                CardHeader {
                    CardTitle { "{title}" }
                    CardDescription { "{description}" }
                }
                CardContent {
                    p { class: "text-sm text-muted-foreground", "Card body content." }
                }
                CardFooter {
                    Button { size: ButtonSize::Small, variant: ButtonVariant::Primary, "Action" }
                    Button { size: ButtonSize::Small, variant: ButtonVariant::Ghost, "Cancel" }
                }
            }
        }
    }
}

/// Force-link helper — referenced from the binary's `main` so LTO
/// can't drop the static registrations. Each `#[story]` macro emits
/// a registration as a `static` item; without a code path that
/// touches it, the linker may strip it from the final binary.
pub fn force_link() {
    use crate::components::*;

    // Tuples can be wide; nested tuples avoid the rustc tuple-arity limit
    // and group symbols by category for readability.
    let _ = (
        // Buttons + Cards (live in this file)
        (
            &BUTTON_PRIMARY_STORY,
            &BUTTON_VARIANTS_STORY,
            &BUTTON_MATRIX_STORY,
            &BUTTON_SIZES_STORY,
            &CARD_BASIC_STORY,
        ),
        // Badges + Avatar
        (&BADGE_VARIANTS_STORY, &AVATAR_SIZES_STORY),
        // Form controls
        (
            &CHECKBOX_DEFAULT_STORY,
            &SWITCH_DEFAULT_STORY,
            &RADIO_GROUP_DEFAULT_STORY,
            &SLIDER_DEFAULT_STORY,
            &TOGGLE_DEFAULT_STORY,
            &TOGGLE_VARIANTS_STORY,
            &TOGGLE_GROUP_DEFAULT_STORY,
            &INPUT_DEFAULT_STORY,
            &INPUT_VARIANTS_STORY,
            &TEXTAREA_DEFAULT_STORY,
            &LABEL_DEFAULT_STORY,
            &INPUT_GROUP_DEFAULT_STORY,
        ),
        (
            &INPUT_OTP_DEFAULT_STORY,
            &NATIVE_SELECT_DEFAULT_STORY,
            &SELECT_DEFAULT_STORY,
            &SEGMENTED_CONTROL_DEFAULT_STORY,
            &SEGMENTED_CONTROL_SIZES_STORY,
        ),
        // Overlays
        (
            &DIALOG_DEFAULT_STORY,
            &ALERT_DIALOG_DEFAULT_STORY,
            &DRAWER_DEFAULT_STORY,
            &SIDE_SHEET_DEFAULT_STORY,
            &POPOVER_DEFAULT_STORY,
            &HOVER_CARD_DEFAULT_STORY,
            &TOOLTIP_DEFAULT_STORY,
            &DROPDOWN_DEFAULT_STORY,
            &CONTEXT_MENU_DEFAULT_STORY,
            &MENUBAR_DEFAULT_STORY,
            &COMMAND_DEFAULT_STORY,
            &COMBOBOX_DEFAULT_STORY,
        ),
        // Navigation
        (
            &BREADCRUMB_DEFAULT_STORY,
            &PAGINATION_DEFAULT_STORY,
            &TABS_DEFAULT_STORY,
            &NAVIGATION_MENU_DEFAULT_STORY,
            &SIDEBAR_DEFAULT_STORY,
            &NAV_TAB_DEFAULT_STORY,
        ),
        // Data display
        (
            &ASPECT_RATIO_DEFAULT_STORY,
            &SCROLL_AREA_DEFAULT_STORY,
            &RESIZABLE_DEFAULT_STORY,
            &TABLE_DEFAULT_STORY,
            &ACCORDION_DEFAULT_STORY,
            &COLLAPSIBLE_DEFAULT_STORY,
            &DATA_TABLE_DEFAULT_STORY,
            &CALENDAR_DEFAULT_STORY,
            &CAROUSEL_DEFAULT_STORY,
            &DATE_PICKER_DEFAULT_STORY,
        ),
        // Feedback + misc
        (
            &ALERT_DEFAULT_STORY,
            &ALERT_VARIANTS_STORY,
            &PROGRESS_DEFAULT_STORY,
            &PROGRESS_VARIANTS_STORY,
            &PROGRESS_BAR_DEFAULT_STORY,
            &SPINNER_SIZES_STORY,
            &SKELETON_DEFAULT_STORY,
            &STATUS_DEFAULT_STORY,
            &KBD_DEFAULT_STORY,
            &TOAST_DEFAULT_STORY,
            &EMPTY_STATE_DEFAULT_STORY,
            &BUTTON_GROUP_DEFAULT_STORY,
        ),
        (
            &ITEM_DEFAULT_STORY,
            &KEY_VALUE_ROW_DEFAULT_STORY,
            &LIST_ROW_DEFAULT_STORY,
            &SECTION_HEADER_DEFAULT_STORY,
            &SEARCHABLE_DROPDOWN_DEFAULT_STORY,
            &SEARCHABLE_LIST_DEFAULT_STORY,
            &INLINE_EDIT_DEFAULT_STORY,
            &FIELD_DEFAULT_STORY,
            &FORM_DEFAULT_STORY,
            &TOOLBAR_DEFAULT_STORY,
        ),
        // Typography + layout (different module paths)
        (
            &crate::typography::HEADING_LEVELS_STORY,
            &crate::typography::TEXT_VARIANTS_STORY,
            &crate::layout::DIVIDER_DEFAULT_STORY,
        ),
        // Matrices — variants × states / sizes / sides etc.
        (
            &INPUT_SIZES_STORY,
            &TEXTAREA_STATES_STORY,
            &SLIDER_RANGE_STORY,
            &SWITCH_STATES_STORY,
            &CHECKBOX_STATES_STORY,
            &RADIO_GROUP_STATES_STORY,
            &SELECT_STATES_STORY,
            &TOGGLE_GROUP_VARIANTS_STORY,
            &SEGMENTED_CONTROL_STATES_STORY,
            &AVATAR_FALLBACKS_STORY,
        ),
        (
            &ALERT_DIALOG_DESTRUCTIVE_STORY,
            &DRAWER_SIDES_STORY,
            &SIDE_SHEET_SIDES_STORY,
            &TABS_VERTICAL_STORY,
            &BREADCRUMB_WITH_ELLIPSIS_STORY,
            &PAGINATION_MANY_PAGES_STORY,
            &TOAST_TYPES_STORY,
            &STATUS_DOTS_AND_BADGES_STORY,
            &CAROUSEL_WITH_CONTROLS_STORY,
        ),
    );

    #[cfg(feature = "router")]
    let _ = &NAVBAR_DEFAULT_STORY;
}
