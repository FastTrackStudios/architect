//! The phone's bottom sheet, and the parts sheets are built from (#445).
//!
//! Ported from Task (`task:crates/ui-core/src/sheet.rs`), the look kept
//! and drawn in architect's theme: Linear's iOS sheets. A full-width panel
//! rising from the bottom edge with a large top radius, a grab handle,
//! and content in **grouped inset rows** ([`SheetGroup`]): a label on the
//! left, the value on the right ([`SheetSelect`] opens a check-marked
//! popover, [`SheetToggle`] is a switch), chips for multi-pick sets
//! ([`SheetChip`]), and a search field over a checkbox list for filters
//! ([`SheetSearch`], [`SheetCheckRow`]). Every slide-up menu made of these
//! reads as one system.
//!
//! The colours are the theme's (`architect_theme::use_themed` over
//! [`SheetTheme`]): Task's `bg-popover` is `sheet`, `text-foreground` is
//! `ink`, `text-muted-foreground` is `ink-2`, `bg-primary` is `brand`, the
//! scrim is `scrim`, and Task's `foreground/[0.07]` washes are the ink
//! mixed into transparency ([`wash`]).

use architect_theme::{FromResolved, Resolved};
use dioxus::prelude::*;
use lucide_dioxus::{Check, ChevronsUpDown, ListFilter, X};

/// The theme's colours a sheet draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetTheme {
    /// The panel (`sheet`).
    pub sheet: String,
    /// Text (`ink`), quieter text (`ink-2`), the faintest (`ink-3`).
    pub ink: String,
    pub ink_2: String,
    pub ink_3: String,
    /// The accent and the ink on it (`brand`, `brand-ink`).
    pub brand: String,
    pub brand_ink: String,
    /// The dim behind a sheet (`scrim`).
    pub scrim: String,
    /// Hairlines (`rule`).
    pub rule: String,
    /// `danger`.
    pub danger: String,
    /// The face (`font`).
    pub font: String,
}

impl FromResolved for SheetTheme {
    fn from_resolved(r: &Resolved) -> Self {
        let t = |name: &str| r.value(name).to_owned();
        Self {
            sheet: t("sheet"),
            ink: t("ink"),
            ink_2: t("ink-2"),
            ink_3: t("ink-3"),
            brand: t("brand"),
            brand_ink: t("brand-ink"),
            scrim: t("scrim"),
            rule: t("rule"),
            danger: t("danger"),
            font: t("font"),
        }
    }
}

/// The sheet's colours, from the theme above (the built-in default's
/// with none).
#[must_use]
pub fn use_sheet_theme() -> SheetTheme {
    architect_theme::use_themed::<SheetTheme>().read().clone()
}

/// `color` at `percent` over transparency: Task's `foreground/[0.07]`.
#[must_use]
pub fn wash(color: &str, percent: u8) -> String {
    format!("color-mix(in srgb, {color} {percent}%, transparent)")
}

/// The press and hover washes the parts use: a class reads its colour
/// from `--sheet-press` set inline. Mount once where sheets show (the
/// sheet mounts it itself).
pub const PRESS_CSS: &str = ".sheet-press:active { background: var(--sheet-press) !important; } \
     .sheet-press:hover { background: var(--sheet-hover, transparent); } \
     .sheet-input::placeholder { color: var(--sheet-placeholder); }";

/// The phone's bottom sheet: scrim, and a rounded panel floating at the
/// bottom edge, safe-area padded, scrolling past 88% of the height.
/// Closes on a scrim tap, Escape, or the close button (44pt). An empty
/// `title` drops the title row (a search field or a group leads
/// instead). Children are only mounted while open.
#[component]
pub fn BottomSheet(
    open: bool,
    on_close: EventHandler<()>,
    title: String,
    children: Element,
) -> Element {
    let t = use_sheet_theme();
    if !open {
        return rsx! {};
    }
    rsx! {
        document::Style { {PRESS_CSS} }
        div {
            style: "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 50; background: {t.scrim};",
            onclick: move |_| on_close.call(()),
        }
        div {
            style: "position: fixed; left: 0; right: 0; bottom: 0; z-index: 50; display: flex; flex-direction: column; \
                    max-height: 88vh; overflow: hidden; border-radius: 30px 30px 0 0; background: {t.sheet}; color: {t.ink}; \
                    font-family: {t.font}; box-shadow: 0 -12px 40px -12px rgba(0,0,0,0.6), inset 0 1px 0 rgba(255,255,255,0.07); \
                    outline: none;",
            tabindex: "-1",
            // `autofocus` fires on a page's first load only, not when the
            // sheet is inserted: focus on mount so Escape lands here.
            onmounted: move |e: Event<MountedData>| {
                spawn(async move {
                    let _ = e.data().set_focus(true).await;
                });
            },
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    on_close.call(());
                }
            },
            // The grab handle.
            div { style: "margin: 8px auto 0; height: 5px; width: 36px; flex-shrink: 0; border-radius: 999px; background: {wash(&t.ink, 20)};" }
            if !title.is_empty() {
                div { style: "display: flex; flex-shrink: 0; align-items: center; justify-content: space-between; gap: 8px; padding: 6px 12px 0 20px;",
                    h2 { style: "margin: 0; font-size: 17px; font-weight: 600; letter-spacing: -0.01em; color: {t.ink};",
                        "{title}"
                    }
                    SheetClose { on_close }
                }
            }
            // Flush with the bottom edge: the home indicator's room is
            // padding inside the sheet, not a gap under it.
            div {
                style: "min-height: 0; flex: 1; overflow-y: auto; overflow-x: hidden; padding: 8px 12px 0; \
                        padding-bottom: max(0.75rem, env(safe-area-inset-bottom, 0px));",
                {children}
            }
        }
    }
}

/// The round ✕ a sheet closes with.
#[component]
pub fn SheetClose(on_close: EventHandler<()>) -> Element {
    let t = use_sheet_theme();
    rsx! {
        button {
            r#type: "button",
            "aria-label": "Close",
            style: "display: flex; width: 44px; height: 44px; flex-shrink: 0; align-items: center; justify-content: center; \
                    border: none; padding: 0; border-radius: 999px; background: {wash(&t.ink, 7)}; color: {t.ink}; cursor: pointer;",
            onclick: move |_| on_close.call(()),
            X { size: 18 }
        }
    }
}

/// A grouped inset list: rows on one rounded surface, hairlines between.
#[component]
pub fn SheetGroup(children: Element) -> Element {
    let t = use_sheet_theme();
    let line = wash(&t.ink, 6);
    rsx! {
        document::Style { ".sheet-group > * + * {{ border-top: 1px solid var(--sheet-line); }}" }
        div {
            class: "sheet-group",
            style: "--sheet-line: {line}; margin-bottom: 12px; display: flex; flex-direction: column; border-radius: 20px; \
                    background: {wash(&t.ink, 5)};",
            {children}
        }
    }
}

/// A small heading above a group ("Type", "Row properties"), with an
/// optional trailing action ("Reset").
#[component]
pub fn SheetSection(
    label: String,
    #[props(default)] action: Option<(String, EventHandler<()>)>,
) -> Element {
    let t = use_sheet_theme();
    rsx! {
        div { style: "display: flex; align-items: center; justify-content: space-between; padding: 4px 8px 8px;",
            span { style: "font-size: 13px; font-weight: 500; color: {t.ink_2};", "{label}" }
            if let Some((text, on)) = action {
                button {
                    r#type: "button",
                    style: "border: none; background: none; padding: 0; font-size: 13px; color: {t.ink_3}; cursor: pointer;",
                    onclick: move |_| on.call(()),
                    "{text}"
                }
            }
        }
    }
}

/// One row: the label left, anything right.
#[component]
pub fn SheetRow(label: String, children: Element) -> Element {
    let t = use_sheet_theme();
    rsx! {
        div { style: "position: relative; display: flex; min-height: 54px; align-items: center; justify-content: space-between; \
                      gap: 12px; padding: 0 16px; font-size: 16px; color: {t.ink};",
            span { style: "min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{label}" }
            div { style: "display: flex; flex-shrink: 0; align-items: center;", {children} }
        }
    }
}

/// A row whose value is one of `options` (`(value, label)`): the current
/// label with ⌃⌄, and a check-marked popover to change it.
#[component]
pub fn SheetSelect(
    label: String,
    value: String,
    options: Vec<(String, String)>,
    on_change: EventHandler<String>,
) -> Element {
    let t = use_sheet_theme();
    let mut open = use_signal(|| false);
    let current = options
        .iter()
        .find(|(v, _)| *v == value)
        .map_or_else(|| value.clone(), |(_, l)| l.clone());
    let press = wash(&t.ink, 4);
    let press_item = wash(&t.ink, 6);
    rsx! {
        document::Style { {PRESS_CSS} }
        div { style: "position: relative;",
            button {
                r#type: "button",
                class: "sheet-press",
                style: "--sheet-press: {press}; display: flex; min-height: 54px; width: 100%; align-items: center; \
                        justify-content: space-between; gap: 12px; padding: 0 16px; border: none; background: transparent; \
                        text-align: left; font-size: 16px; color: {t.ink}; cursor: pointer;",
                onclick: move |_| open.toggle(),
                span { style: "min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{label}" }
                span { style: "display: flex; flex-shrink: 0; align-items: center; gap: 4px; color: {t.ink_2};",
                    "{current}"
                    ChevronsUpDown { size: 15 }
                }
            }
            if open() {
                // A click-away layer, then the menu over the row's value.
                div {
                    style: "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 10;",
                    onclick: move |_| open.set(false),
                }
                div { style: "position: absolute; right: 8px; top: 4px; z-index: 20; display: flex; min-width: 13rem; flex-direction: column; \
                              border-radius: 22px; background: {t.sheet}; padding: 6px 0; \
                              box-shadow: 0 20px 50px -10px rgba(0,0,0,0.75), inset 0 0 0 1px rgba(255,255,255,0.08);",
                    for (v , l) in options.iter().cloned() {
                        {
                            let picked = v == value;
                            rsx! {
                                button {
                                    key: "{v}",
                                    r#type: "button",
                                    class: "sheet-press",
                                    style: "--sheet-press: {press_item}; display: flex; height: 44px; align-items: center; gap: 10px; \
                                            padding: 0 16px; border: none; background: transparent; text-align: left; font-size: 16px; \
                                            color: {t.ink}; cursor: pointer;",
                                    onclick: move |_| {
                                        open.set(false);
                                        on_change.call(v.clone());
                                    },
                                    span { style: "display: flex; width: 16px; justify-content: center;",
                                        if picked {
                                            Check { size: 16 }
                                        }
                                    }
                                    "{l}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// An on/off switch, iOS-sized.
#[component]
pub fn SheetSwitch(on: bool, on_toggle: EventHandler<bool>, label: String) -> Element {
    let t = use_sheet_theme();
    let track = if on { t.brand } else { wash(&t.ink, 15) };
    let knob = if on { 20 } else { 0 };
    rsx! {
        button {
            r#type: "button",
            role: "switch",
            "aria-checked": "{on}",
            "aria-label": "{label}",
            style: "position: relative; height: 31px; width: 51px; flex-shrink: 0; border: none; border-radius: 999px; padding: 2px; \
                    background: {track}; cursor: pointer; transition: background-color 200ms;",
            onclick: move |_| on_toggle.call(!on),
            span { style: "display: block; width: 27px; height: 27px; border-radius: 999px; background: #fff; \
                           box-shadow: 0 2px 6px rgba(0,0,0,0.3); transform: translateX({knob}px); transition: transform 200ms;" }
        }
    }
}

/// A row with a [`SheetSwitch`].
#[component]
pub fn SheetToggle(
    label: String,
    on: bool,
    on_toggle: EventHandler<bool>,
    #[props(default)] hint: Option<String>,
) -> Element {
    let t = use_sheet_theme();
    rsx! {
        div { style: "display: flex; min-height: 54px; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 16px;",
            div { style: "display: flex; min-width: 0; flex-direction: column;",
                span { style: "overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 16px; color: {t.ink};", "{label}" }
                if let Some(h) = hint {
                    span { style: "overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; color: {t.ink_2};", "{h}" }
                }
            }
            SheetSwitch { on, on_toggle, label }
        }
    }
}

/// A pill in a multi-pick set (row properties, quick filters).
#[component]
pub fn SheetChip(label: String, on: bool, on_toggle: EventHandler<bool>) -> Element {
    let t = use_sheet_theme();
    let look = if on {
        format!(
            "background: {}; color: {}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.08);",
            wash(&t.ink, 12),
            t.ink
        )
    } else {
        format!(
            "background: transparent; color: {}; box-shadow: inset 0 0 0 1px {};",
            t.ink_2,
            wash(&t.ink, 10)
        )
    };
    rsx! {
        button {
            r#type: "button",
            "aria-pressed": "{on}",
            style: "height: 40px; border: none; border-radius: 999px; padding: 0 16px; font-size: 15px; cursor: pointer; \
                    transition: background-color 150ms; {look}",
            onclick: move |_| on_toggle.call(!on),
            "{label}"
        }
    }
}

/// The filter sheet's search field, with the sheet's ✕ beside it.
#[component]
pub fn SheetSearch(
    value: String,
    placeholder: String,
    on_input: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let t = use_sheet_theme();
    rsx! {
        document::Style { {PRESS_CSS} }
        div { style: "display: flex; align-items: center; gap: 10px; padding-bottom: 12px;",
            label { style: "display: flex; height: 48px; min-width: 0; flex: 1; align-items: center; gap: 10px; border-radius: 999px; \
                            background: {wash(&t.ink, 7)}; padding: 0 16px; color: {t.ink_2};",
                ListFilter { size: 18 }
                input {
                    class: "sheet-input",
                    style: "--sheet-placeholder: {t.ink_2}; min-width: 0; flex: 1; border: none; background: transparent; \
                            font-size: 16px; color: {t.ink}; outline: none; font-family: inherit;",
                    placeholder: "{placeholder}",
                    value: "{value}",
                    oninput: move |e| on_input.call(e.value()),
                }
            }
            SheetClose { on_close }
        }
    }
}

/// A checkbox row in a filter list.
#[component]
pub fn SheetCheckRow(
    label: String,
    on: bool,
    on_toggle: EventHandler<bool>,
    #[props(default)] lead: Option<Element>,
) -> Element {
    let t = use_sheet_theme();
    let boxed = if on {
        format!("background: {}; color: {};", t.brand, t.brand_ink)
    } else {
        "box-shadow: inset 0 0 0 1.5px rgba(255,255,255,0.18);".to_owned()
    };
    let press = wash(&t.ink, 4);
    rsx! {
        document::Style { {PRESS_CSS} }
        button {
            r#type: "button",
            role: "checkbox",
            "aria-checked": "{on}",
            class: "sheet-press",
            style: "--sheet-press: {press}; display: flex; min-height: 54px; width: 100%; align-items: center; gap: 14px; \
                    padding: 0 16px; border: none; background: transparent; text-align: left; font-size: 16px; color: {t.ink}; cursor: pointer;",
            onclick: move |_| on_toggle.call(!on),
            span { style: "display: flex; width: 22px; height: 22px; flex-shrink: 0; align-items: center; justify-content: center; \
                           border-radius: 7px; {boxed}",
                if on {
                    Check { size: 15 }
                }
            }
            if let Some(l) = lead {
                {l}
            }
            span { style: "min-width: 0; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{label}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wash_is_the_colour_mixed_into_transparency() {
        assert_eq!(wash("#fff", 7), "color-mix(in srgb, #fff 7%, transparent)");
    }

    #[test]
    fn the_sheet_reads_the_theme() {
        let t = SheetTheme::from_resolved(&Resolved::default());
        assert!(!t.sheet.is_empty() && !t.ink.is_empty() && !t.brand.is_empty());
    }
}
