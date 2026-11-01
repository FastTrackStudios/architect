//! The organization / workspace switcher every app keeps at the head of
//! its sidebar: the current one's mark and name, and a list to switch —
//! "All" first where the app has an aggregate view, then each one, then
//! the app's actions (New organization, Manage).
//!
//! ```rust,ignore
//! Workspaces::new("Organization", move |id| selection.set(id))
//!     .all("All organizations")
//!     .with(Workspace::new("fts", "FastTrackStudio").detail("12 members"))
//!     .with(Workspace::new("hp", "Harbor & Pine").color("#7c2d12"))
//!     .current(selection())
//!     .action("New organization", icon!(Plus), move |_| create.open())
//! ```

use dioxus::prelude::*;
use lucide_dioxus::{Check, ChevronsUpDown, LayoutGrid};

use super::{Icon, IconFn, use_shell};

/// One organization or workspace.
#[derive(Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// A second line in the list: members, a plan.
    pub detail: Option<String>,
    /// The mark's fill.
    pub color: &'static str,
}

impl Workspace {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            detail: None,
            color: "#3f3f46",
        }
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[must_use]
    pub const fn color(mut self, color: &'static str) -> Self {
        self.color = color;
        self
    }
}

/// One of the app's actions at the foot of the list.
#[derive(Clone, PartialEq)]
pub struct WorkspaceAction {
    pub label: String,
    pub icon: IconFn,
    pub on_select: Callback<()>,
}

/// The switcher: what there is, which is current, what picking one does.
#[derive(Clone, PartialEq)]
pub struct Workspaces {
    /// What one is called, over the list: "Organization", "Workspace".
    pub kind: String,
    pub list: Vec<Workspace>,
    /// The current one's id; `None` is "All" (when there is an "All").
    pub current: Option<String>,
    /// The aggregate view's label ("All organizations"); `None` for none.
    pub all: Option<String>,
    pub on_select: Callback<Option<String>>,
    pub actions: Vec<WorkspaceAction>,
}

impl Workspaces {
    /// `on_select` hears the id picked, or `None` for "All".
    pub fn new(kind: impl Into<String>, on_select: impl FnMut(Option<String>) + 'static) -> Self {
        Self {
            kind: kind.into(),
            list: Vec::new(),
            current: None,
            all: None,
            on_select: Callback::new(on_select),
            actions: Vec::new(),
        }
    }

    /// Offer an aggregate view of every one, first in the list.
    #[must_use]
    pub fn all(mut self, label: impl Into<String>) -> Self {
        self.all = Some(label.into());
        self
    }

    #[must_use]
    pub fn with(mut self, workspace: Workspace) -> Self {
        self.list.push(workspace);
        self
    }

    #[must_use]
    pub fn current(mut self, id: Option<impl Into<String>>) -> Self {
        self.current = id.map(Into::into);
        self
    }

    #[must_use]
    pub fn action(
        mut self,
        label: impl Into<String>,
        icon: IconFn,
        on_select: impl FnMut(()) + 'static,
    ) -> Self {
        self.actions.push(WorkspaceAction {
            label: label.into(),
            icon,
            on_select: Callback::new(on_select),
        });
        self
    }

    /// The current one, or `None` for "All" (or an id no longer listed).
    fn chosen(&self) -> Option<&Workspace> {
        let id = self.current.as_ref()?;
        self.list.iter().find(|w| &w.id == id)
    }

    fn label(&self) -> String {
        match (self.chosen(), &self.all) {
            (Some(w), _) => w.name.clone(),
            (None, Some(all)) => all.clone(),
            (None, None) => self
                .list
                .first()
                .map(|w| w.name.clone())
                .unwrap_or_default(),
        }
    }
}

/// An icon's size in whole points (lucide takes a `usize`).
// Rounded and clamped first, so the conversion is exact.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
const fn icon_size(points: f64) -> usize {
    points.round().clamp(1.0, 256.0) as usize
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter(|w| w.chars().next().is_some_and(char::is_alphanumeric))
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

/// A workspace's mark: its initials on its colour, or a grid for "All".
#[component]
fn Mark(name: String, color: &'static str, all: bool, size: f64) -> Element {
    let t = use_shell().theme;
    let radius = (size * 0.28).round();
    let font = (size * 0.4).round();
    if all {
        return rsx! {
            span { style: "width: {size}px; height: {size}px; flex-shrink: 0; border-radius: {radius}px; \
                           background: {t.active}; display: flex; align-items: center; justify-content: center;",
                LayoutGrid { size: icon_size(size * 0.55), color: t.ink }
            }
        };
    }
    rsx! {
        span { style: "width: {size}px; height: {size}px; flex-shrink: 0; border-radius: {radius}px; background: {color}; \
                       display: flex; align-items: center; justify-content: center; color: #fff; \
                       font-size: {font}px; font-weight: 750;",
            "{initials(&name)}"
        }
    }
}

/// Where the switcher sits, which decides its trigger and where the list
/// opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Place {
    /// The head of a docked sidebar or a drawer: mark, name, ⌃⌄.
    Head,
    /// The head of a rail: the mark alone; the list opens to its right.
    Rail,
}

/// The switcher: its trigger, and while open, its list.
#[component]
pub(super) fn Switcher(workspaces: Workspaces, place: Place) -> Element {
    let t = use_shell().theme;
    let mut open = use_signal(|| false);
    let chosen = workspaces.chosen().cloned();
    let all = chosen.is_none() && workspaces.all.is_some();
    let label = workspaces.label();
    let color = chosen.as_ref().map_or("#3f3f46", |w| w.color);
    let (trigger, list_at) = match place {
        Place::Head => (
            rsx! {
                button {
                    class: "lp",
                    "aria-label": "Switch {workspaces.kind}",
                    style: "flex: 1; min-width: 0; height: 44px; display: flex; align-items: center; justify-content: flex-start; \
                            gap: 10px; padding: 0 8px; border: none; border-radius: 12px; background: transparent; \
                            font-family: {t.font}; text-align: left;",
                    onclick: move |_| open.toggle(),
                    Mark { name: label.clone(), color, all, size: 30.0 }
                    span { style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; font-size: 16px; \
                                   font-weight: 700; letter-spacing: -0.01em; color: {t.ink};",
                        "{label}"
                    }
                    ChevronsUpDown { size: 15, color: t.ink_3 }
                }
            },
            "top: 48px; left: 8px;",
        ),
        Place::Rail => (
            rsx! {
                button {
                    class: "lp",
                    "aria-label": "Switch {workspaces.kind}: {label}",
                    style: "height: 52px; border: none; border-radius: 12px; background: transparent; \
                            display: flex; align-items: center; justify-content: center;",
                    onclick: move |_| open.toggle(),
                    Mark { name: label.clone(), color, all, size: 34.0 }
                }
            },
            "top: 0px; left: 70px;",
        ),
    };
    let pick = workspaces.on_select;
    rsx! {
        div { style: "position: relative; display: flex; flex: 1; min-width: 0;",
            {trigger}
            if open() {
                // Far past every edge: Blitz has no `position: fixed`.
                div {
                    style: "position: absolute; top: -2000px; left: -4000px; width: 8000px; height: 8000px; z-index: 60;",
                    onclick: move |_| open.set(false),
                }
                div {
                    style: "position: absolute; {list_at} z-index: 70; width: 280px; display: flex; flex-direction: column; \
                            padding: 6px 0; border-radius: 16px; background: {t.pill}; border: 1px solid {t.rule}; \
                            box-shadow: 0 20px 50px rgba(0,0,0,0.6); font-family: {t.font};",
                    div { style: "padding: 8px 14px 6px; font-size: 12px; font-weight: 650; color: {t.ink_3};",
                        "{workspaces.kind}"
                    }
                    if let Some(all_label) = workspaces.all.clone() {
                        Choice { name: all_label, detail: None, color: "#3f3f46", all: true, on: all,
                            on_pick: move |()| { open.set(false); pick.call(None); } }
                    }
                    for w in workspaces.list.iter().cloned() {
                        Choice {
                            key: "{w.id}",
                            name: w.name.clone(),
                            detail: w.detail.clone(),
                            color: w.color,
                            all: false,
                            on: workspaces.current.as_deref() == Some(w.id.as_str()),
                            on_pick: move |()| { open.set(false); pick.call(Some(w.id.clone())); },
                        }
                    }
                    if !workspaces.actions.is_empty() {
                        div { style: "height: 1px; margin: 6px 14px; background: {t.rule};" }
                        for (i, a) in workspaces.actions.iter().cloned().enumerate() {
                            button {
                                key: "{i}",
                                class: "lp",
                                style: "height: 42px; display: flex; align-items: center; justify-content: flex-start; gap: 12px; \
                                        padding: 0 14px; border: none; background: transparent; color: {t.ink_2}; \
                                        font-family: {t.font}; font-size: 14px; text-align: left;",
                                onclick: move |_| { open.set(false); a.on_select.call(()); },
                                span { style: "width: 28px; display: flex; justify-content: center;",
                                    {(a.icon)(Icon { size: 17, color: t.ink_3 })}
                                }
                                span { "{a.label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Choice(
    name: String,
    detail: Option<String>,
    color: &'static str,
    all: bool,
    on: bool,
    on_pick: EventHandler<()>,
) -> Element {
    let t = use_shell().theme;
    let bg = if on { t.active } else { "transparent" };
    rsx! {
        button {
            class: "lp",
            style: "min-height: 48px; margin: 0 6px; display: flex; align-items: center; justify-content: flex-start; gap: 12px; \
                    padding: 6px 8px; border: none; border-radius: 10px; background: {bg}; font-family: {t.font}; text-align: left;",
            onclick: move |_| on_pick.call(()),
            Mark { name: name.clone(), color, all, size: 28.0 }
            div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px;",
                span { style: "font-size: 15px; font-weight: 600; color: {t.ink}; white-space: nowrap; overflow: hidden;", "{name}" }
                if let Some(d) = detail {
                    span { style: "font-size: 12px; color: {t.ink_3};", "{d}" }
                }
            }
            if on { Check { size: 16, color: t.ink } }
        }
    }
}
