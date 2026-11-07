//! How the ported components look: Task's Tailwind classes as inline
//! styles over architect's theme (#445).
//!
//! | Task | here |
//! |---|---|
//! | `bg-background` | `desk` ([`Look::page`]) |
//! | `bg-popover`, `bg-card` | `sheet` |
//! | `text-foreground` | `ink` |
//! | `text-muted-foreground` | `ink-2` (`/70`: `ink-3`) |
//! | `border-border` | `rule` |
//! | `bg-accent` (a row's hover) | `active` |
//! | `bg-primary`, `text-primary-foreground` | `brand`, `brand-ink` |
//! | `bg-primary/15` | `brand-soft` |
//! | `foreground/[0.07]` | `wash(ink, 7)` ([`wash`]) |
//! | `bg-black/50` | `scrim` |
//! | `shadow-xl`, `shadow-2xl` | `shadow-lift`, `shadow-float` |
//! | `rose-400`, `amber-400`, `sky-400` | `danger`, `warn`, `info` |
//!
//! Task's `md:` breakpoint is the shell's form factor ([`phone`]): a
//! phone gets Task's small-screen classes, everything else its `md:`
//! ones. Hover and press washes, placeholders and hidden scrollbars are
//! classes in [`CSS`] reading a colour set inline (`--cap-h`, `--cap-a`).

use architect_theme::{FromResolved, Resolved};
use dioxus::prelude::*;

/// The theme's colours the capture components draw with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub page: String,
    pub sheet: String,
    pub ink: String,
    pub ink_2: String,
    pub ink_3: String,
    pub rule: String,
    pub hover: String,
    pub brand: String,
    pub brand_ink: String,
    pub brand_soft: String,
    pub danger: String,
    pub warn: String,
    pub info: String,
    pub scrim: String,
    pub lift: String,
    pub float: String,
    pub font: String,
}

impl FromResolved for Look {
    fn from_resolved(r: &Resolved) -> Self {
        let t = |name: &str| r.value(name).to_owned();
        Self {
            page: t("desk"),
            sheet: t("sheet"),
            ink: t("ink"),
            ink_2: t("ink-2"),
            ink_3: t("ink-3"),
            rule: t("rule"),
            hover: t("active"),
            brand: t("brand"),
            brand_ink: t("brand-ink"),
            brand_soft: t("brand-soft"),
            danger: t("danger"),
            warn: t("warn"),
            info: t("info"),
            scrim: t("scrim"),
            lift: t("shadow-lift"),
            float: t("shadow-float"),
            font: t("font"),
        }
    }
}

/// The colours, from the theme above (the built-in default's with none).
#[must_use]
pub fn use_look() -> Look {
    architect_theme::use_themed::<Look>().read().clone()
}

/// `color` at `percent` over transparency: Task's `foreground/[0.07]`.
#[must_use]
pub fn wash(color: &str, percent: u8) -> String {
    format!("color-mix(in srgb, {color} {percent}%, transparent)")
}

/// Whether this is a phone (Task's small screen): the shell's form
/// factor, else the window's screen (the capture layer sits beside the
/// shell, not in it); a capture window with neither is not.
#[must_use]
pub fn phone() -> bool {
    use architect_shell::FormFactor;
    architect_shell::try_use_shell().map_or_else(
        || {
            try_consume_context::<Signal<architect_shell::Screen>>()
                .is_some_and(|s| s.peek().form() == FormFactor::Phone)
        },
        |s| s.form() == FormFactor::Phone,
    )
}

/// The screen's safe insets (the status bar, the home indicator), from
/// the shell or the window's screen: Task's `env(safe-area-inset-*)`,
/// which Blitz does not have. Zero on a desk.
#[must_use]
pub fn insets() -> architect_shell::Insets {
    architect_shell::try_use_shell().map_or_else(
        || {
            try_consume_context::<Signal<architect_shell::Screen>>()
                .map(|s| s.peek().insets)
                .unwrap_or_default()
        },
        |s| s.screen.insets,
    )
}

/// The hover, press, placeholder and scrollbar rules (see the module).
pub const CSS: &str = "\
.cap-h:hover { background: var(--cap-h) !important; color: var(--cap-hc, inherit); } \
.cap-a:active { background: var(--cap-a) !important; } \
.cap-in::placeholder { color: var(--cap-ph); } \
.cap-in:focus { outline: none; } \
.cap-btn:disabled { opacity: 0.4; cursor: default; } \
.cap-noscroll { scrollbar-width: none; } \
.cap-noscroll::-webkit-scrollbar { display: none; } \
@keyframes cap-ping { 75%, 100% { transform: scale(2); opacity: 0; } } \
.cap-ping { animation: cap-ping 1s cubic-bezier(0, 0, 0.2, 1) infinite; }";

/// Mounts [`CSS`]: once in each document the components draw in.
#[component]
pub fn CaptureCss() -> Element {
    rsx! { document::Style { {CSS} } }
}

/// A button's reset: no border, no fill, the text's font and colour.
pub const RESET: &str = "border: none; background: transparent; font-family: inherit; color: inherit; \
     padding: 0; margin: 0; cursor: pointer; box-sizing: border-box;";

/// A text field's reset.
pub const FIELD_RESET: &str = "border: none; background: transparent; font-family: inherit; outline: none; \
     margin: 0; box-sizing: border-box;";

/// One line, cut with an ellipsis.
pub const TRUNCATE: &str = "overflow: hidden; text-overflow: ellipsis; white-space: nowrap;";

/// A property chip in the composer's bar (`CHIP_ROW_CLASS` and
/// `CHIP_ROW_QUIET_CLASS`): a round segment of the keyboard bar on a
/// phone, a bordered chip in a row on a desk. With [`hover_vars`].
#[must_use]
pub fn chip(l: &Look, phone: bool, quiet: bool) -> String {
    let ink = if quiet { &l.ink_2 } else { &l.ink };
    if phone {
        format!(
            "{RESET} display: inline-flex; height: 40px; max-width: 14rem; flex-shrink: 0; align-items: center; gap: 6px; \
             white-space: nowrap; border-radius: 999px; padding: 0 12px; font-size: 15px; color: {ink}; --cap-a: {};",
            wash(&l.ink, 10)
        )
    } else {
        format!(
            "{RESET} display: inline-flex; height: 28px; max-width: 12rem; flex-shrink: 0; align-items: center; gap: 6px; \
             white-space: nowrap; border-radius: 6px; border: 1px solid {}; padding: 0 8px; font-size: 13px; font-weight: 500; \
             color: {ink}; --cap-h: {}; --cap-hc: {};",
            wash(&l.rule, 70),
            l.hover,
            l.ink
        )
    }
}
