//! A desk's status line: small facts and switches at the foot of the
//! window — a document's save state, a mode, a connection, a count.
//!
//! One list of [`StatusItem`]s, drawn either as a [`StatusStyle::Bar`]
//! along the window's foot (Task's, an IDE's: the page's items on the
//! left, the app's on the right) or as a [`StatusStyle::Badge`] in the
//! bottom right corner (Obsidian's). The app gives its items to
//! [`AppShell`](super::AppShell); a page adds its own while it is mounted
//! with [`use_status`].

use dioxus::prelude::*;

use super::{Icon, IconFn, use_shell};

/// How a desk shows its status items.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StatusStyle {
    /// A line along the window's foot, right of the ribbon.
    #[default]
    Bar,
    /// A badge in the window's bottom right corner, over the page.
    Badge,
    /// None.
    Hidden,
}

/// Which end of the bar an item keeps to (a badge keeps them in order).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StatusSide {
    /// The page's: what is being looked at.
    Left,
    /// The app's: where it stands.
    #[default]
    Right,
}

/// One fact or switch on the status line.
#[derive(Clone, PartialEq)]
pub struct StatusItem {
    pub id: String,
    pub text: String,
    pub icon: Option<IconFn>,
    /// On hover.
    pub hint: Option<String>,
    /// A dot before it, in this colour (unsaved, offline, live).
    pub dot: Option<&'static str>,
    /// Shown as a filled chip (a mode: `NORMAL`, `INSERT`).
    pub chip: bool,
    pub side: StatusSide,
    pub on_press: Option<Callback<()>>,
}

impl StatusItem {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            icon: None,
            hint: None,
            dot: None,
            chip: false,
            side: StatusSide::Right,
            on_press: None,
        }
    }

    #[must_use]
    pub const fn icon(mut self, icon: IconFn) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[must_use]
    pub const fn dot(mut self, colour: &'static str) -> Self {
        self.dot = Some(colour);
        self
    }

    #[must_use]
    pub const fn chip(mut self) -> Self {
        self.chip = true;
        self
    }

    /// Keep to the left end of a bar (the page's side).
    #[must_use]
    pub const fn left(mut self) -> Self {
        self.side = StatusSide::Left;
        self
    }

    #[must_use]
    pub fn on_press(mut self, on_press: impl FnMut(()) + 'static) -> Self {
        self.on_press = Some(Callback::new(on_press));
        self
    }
}

/// Adds the page's items to the status line while the page is mounted
/// (and keeps them current: call it every render with what is true now).
pub fn use_status(items: Vec<StatusItem>) {
    let shell = try_use_context::<Signal<super::Shell>>();
    let mut page = shell.map(|s| s.peek().page_status);
    if let Some(page) = page.as_mut() {
        if *page.peek() != items {
            page.set(items);
        }
    }
    use_drop(move || {
        if let Some(mut page) = page {
            page.set(Vec::new());
        }
    });
}

/// The bar's height, points (Task's 24).
pub const STATUS_BAR: f64 = 24.0;
/// The badge's height, points (Obsidian's).
pub const STATUS_BADGE: f64 = 26.0;

/// A desk's status line along the foot.
#[component]
pub(super) fn StatusBar(items: Vec<StatusItem>) -> Element {
    let t = use_shell().theme;
    let (left, right): (Vec<StatusItem>, Vec<StatusItem>) =
        items.into_iter().partition(|i| i.side == StatusSide::Left);
    rsx! {
        div {
            style: "height: {STATUS_BAR}px; flex-shrink: 0; box-sizing: border-box; display: flex; align-items: center; \
                    gap: 12px; padding: 0 10px; background: {t.title_bar}; border-top: 1px solid {t.rule}; \
                    color: {t.ink_3}; font-family: {t.font}; font-size: 11px; white-space: nowrap; overflow: hidden;",
            for item in left {
                StatusEntry { key: "{item.id}", item }
            }
            span { style: "flex: 1;" }
            for item in right {
                StatusEntry { key: "{item.id}", item }
            }
        }
    }
}

/// A desk's status line as a badge in the bottom right corner.
#[component]
pub(super) fn StatusBadge(items: Vec<StatusItem>) -> Element {
    let t = use_shell().theme;
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            style: "position: absolute; right: 0; bottom: 0; z-index: {super::layer::STATUS}; height: {STATUS_BADGE}px; box-sizing: border-box; \
                    display: flex; align-items: center; gap: 12px; padding: 0 10px; background: {t.title_bar}; \
                    border-top: 1px solid {t.rule}; border-left: 1px solid {t.rule}; border-radius: 8px 0 0 0; \
                    color: {t.ink_3}; font-family: {t.font}; font-size: 12px; white-space: nowrap;",
            for item in items {
                StatusEntry { key: "{item.id}", item }
            }
        }
    }
}

#[component]
fn StatusEntry(item: StatusItem) -> Element {
    let t = use_shell().theme;
    let (fill, ink, pad, weight) = if item.chip {
        (&*t.active, &*t.ink, "0 6px", 650)
    } else {
        ("transparent", &*t.ink_3, "0 2px", 500)
    };
    let hint = item.hint.clone().unwrap_or_default();
    let press = item.on_press;
    let cursor = if press.is_some() {
        "pointer"
    } else {
        "default"
    };
    rsx! {
        div {
            class: if press.is_some() { "lp" } else { "" },
            title: "{hint}",
            style: "display: flex; align-items: center; gap: 5px; height: 18px; padding: {pad}; border-radius: 4px; \
                    background: {fill}; color: {ink}; font-weight: {weight}; cursor: {cursor}; flex-shrink: 0;",
            onclick: move |_| {
                if let Some(press) = press {
                    press.call(());
                }
            },
            if let Some(dot) = item.dot {
                span { style: "width: 6px; height: 6px; border-radius: 3px; background: {dot};" }
            }
            if let Some(icon) = item.icon {
                {icon(Icon { size: 13, color: ink.into() })}
            }
            if !item.text.is_empty() {
                span { "{item.text}" }
            }
        }
    }
}
