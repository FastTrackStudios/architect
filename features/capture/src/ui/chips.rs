//! The property chips the composer's bar is made of (#445): Status,
//! Priority, Assignee, Project and Labels, each a chip that opens its
//! [`PropertyPicker`].
//!
//! Ported from Task (`task:features/task/task-ui/src/views/properties.rs`),
//! in the vocabulary architect's Tasks extension writes: statuses `open`,
//! `in-progress`, `waiting`, `done`, `cancelled` (Linear's words on the
//! chip), priorities `none` … `critical`. A project and a person are
//! named (the project's title, the person's note), as quick add writes
//! them.
//!
//! Each chip lives twice in the composer: its button in the bar
//! (`opens`), its picker outside it (`picker_only`, following `hotkey`),
//! so a phone's scrolling bar never clips a picker.

use dioxus::prelude::*;
use lucide_dioxus as lc;

use super::picker::{PickOption, PropertyPicker};
use super::style::{self, TRUNCATE, use_look};

/// The properties a key opens (`S`, `P`, `A`, `R`, `L`), and what a chip
/// asks the bar's picker to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Status,
    Priority,
    Assignee,
    Project,
    Labels,
}

impl Hotkey {
    /// The property key `c` opens.
    #[must_use]
    pub fn from_key(c: &str) -> Option<Self> {
        match c {
            "s" | "S" => Some(Self::Status),
            "p" | "P" => Some(Self::Priority),
            "a" | "A" => Some(Self::Assignee),
            "r" | "R" => Some(Self::Project),
            "l" | "L" => Some(Self::Labels),
            _ => None,
        }
    }
}

/// Statuses as stored, and as the chip says them.
pub const STATUSES: &[(&str, &str)] = &[
    ("open", "Todo"),
    ("in-progress", "In Progress"),
    ("waiting", "Waiting"),
    ("done", "Done"),
    ("cancelled", "Canceled"),
];

/// Priorities as stored, and as the chip says them.
pub const PRIORITIES: &[(&str, &str)] = &[
    ("none", "No priority"),
    ("critical", "Urgent"),
    ("high", "High"),
    ("normal", "Medium"),
    ("low", "Low"),
];

/// A status as one of [`STATUSES`] (its synonyms read as it).
#[must_use]
pub fn canonical_status(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "in-progress" | "in_progress" | "doing" | "started" => "in-progress",
        "waiting" | "blocked" | "on-hold" | "on_hold" => "waiting",
        "done" | "completed" | "complete" => "done",
        "cancelled" | "canceled" => "cancelled",
        _ => "open",
    }
}

/// A priority as one of [`PRIORITIES`].
#[must_use]
pub fn canonical_priority(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "critical" | "urgent" | "1" => "critical",
        "high" | "2" => "high",
        "normal" | "medium" | "med" | "3" => "normal",
        "low" | "4" => "low",
        _ => "none",
    }
}

fn shown(table: &[(&str, &'static str)], stored: &str) -> &'static str {
    table
        .iter()
        .find(|(s, _)| *s == stored)
        .map_or("", |(_, l)| *l)
}

/// The ring for a status: the same glyph in the chip and the picker.
#[component]
pub fn StatusIcon(status: String, #[props(default = 14)] size: usize) -> Element {
    let l = use_look();
    match canonical_status(&status) {
        "in-progress" => rsx! { lc::CircleDot { size, color: l.warn } },
        "waiting" => rsx! { lc::CirclePause { size, color: l.info } },
        "done" => rsx! { lc::CircleCheck { size, color: l.brand } },
        "cancelled" => rsx! { lc::CircleX { size, color: l.ink_2 } },
        _ => rsx! { lc::Circle { size, color: l.ink } },
    }
}

/// Bars for a priority (an alert for Urgent, an ellipsis for none).
#[component]
pub fn PriorityIcon(priority: String, #[props(default = 14)] size: usize) -> Element {
    let l = use_look();
    match canonical_priority(&priority) {
        "critical" => rsx! { lc::TriangleAlert { size, color: l.danger } },
        "high" => rsx! { lc::SignalHigh { size, color: l.ink } },
        "normal" => rsx! { lc::SignalMedium { size, color: l.ink_2 } },
        "low" => rsx! { lc::SignalLow { size, color: l.ink_3 } },
        _ => rsx! { lc::Ellipsis { size, color: l.ink_2 } },
    }
}

/// A person's initials on a colour that stays theirs.
#[component]
pub fn PersonDot(name: String, #[props(default = 18)] size: u32) -> Element {
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let initials = if initials.is_empty() {
        "?".to_owned()
    } else {
        initials
    };
    let hue = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)))
        % 360;
    rsx! {
        span {
            style: "display: inline-flex; flex-shrink: 0; align-items: center; justify-content: center; border-radius: 999px; \
                    font-size: 9px; font-weight: 600; line-height: 1; color: #fff; width: {size}px; height: {size}px; \
                    background: hsl({hue}, 45%, 42%);",
            "{initials}"
        }
    }
}

/// A label's colour: the same for the same name, any case.
#[must_use]
pub fn label_color(name: &str) -> String {
    let hue = name
        .to_lowercase()
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(33).wrapping_add(u32::from(b)))
        % 360;
    format!("hsl({hue}, 65%, 60%)")
}

/// A chip in the bar: an icon and a label; quiet while unset.
#[component]
pub fn Chip(
    icon: Element,
    label: String,
    quiet: bool,
    on_click: EventHandler<()>,
    title: String,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    rsx! {
        button {
            r#type: "button",
            class: "cap-h cap-a",
            style: "{style::chip(&l, phone, quiet)}",
            title: "{title}",
            onclick: move |_| on_click.call(()),
            {icon}
            span { style: "{TRUNCATE}", "{label}" }
        }
    }
}

/// A chip's tap: ask the bar's picker (`opens`), or open its own.
fn tap(opens: Option<Signal<Option<Hotkey>>>, me: Hotkey, mut open: Signal<bool>) {
    match opens {
        Some(mut ask) => ask.set(Some(me)),
        None => open.set(true),
    }
}

/// A picker opens when `hotkey` names it, and takes the ask.
fn follow_hotkey(hotkey: Option<Signal<Option<Hotkey>>>, me: Hotkey, mut open: Signal<bool>) {
    use_effect(move || {
        if let Some(mut hk) = hotkey
            && hk() == Some(me)
        {
            open.set(true);
            hk.set(None);
        }
    });
}

/// Status: the ring and its word.
#[component]
pub fn StatusChip(
    status: String,
    on_change: EventHandler<String>,
    #[props(default)] hotkey: Option<Signal<Option<Hotkey>>>,
    #[props(default)] opens: Option<Signal<Option<Hotkey>>>,
    #[props(default)] picker_only: bool,
) -> Element {
    let open = use_signal(|| false);
    follow_hotkey(hotkey, Hotkey::Status, open);
    let current = canonical_status(&status);
    let options: Vec<PickOption> = STATUSES
        .iter()
        .map(|(id, label)| {
            PickOption::new(*id, *label).icon(rsx! { StatusIcon { status: (*id).to_owned() } })
        })
        .collect();
    let mut open_w = open;
    rsx! {
        if !picker_only {
            Chip {
                icon: rsx! { StatusIcon { status: current.to_owned() } },
                label: shown(STATUSES, current).to_owned(),
                quiet: false,
                title: "Status (S)",
                on_click: move |()| tap(opens, Hotkey::Status, open),
            }
        }
        if opens.is_none() {
            PropertyPicker {
                open: open(),
                title: "Change status…",
                hotkey: "S".to_owned(),
                options,
                selected: vec![current.to_owned()],
                number_from: Some(1),
                on_pick: move |id: String| on_change.call(id),
                on_close: move |()| open_w.set(false),
            }
        }
    }
}

/// Priority: its bars and its word ("Priority" while none).
#[component]
pub fn PriorityChip(
    priority: String,
    on_change: EventHandler<String>,
    #[props(default)] hotkey: Option<Signal<Option<Hotkey>>>,
    #[props(default)] opens: Option<Signal<Option<Hotkey>>>,
    #[props(default)] picker_only: bool,
) -> Element {
    let open = use_signal(|| false);
    follow_hotkey(hotkey, Hotkey::Priority, open);
    let current = canonical_priority(&priority);
    let options: Vec<PickOption> = PRIORITIES
        .iter()
        .map(|(id, label)| {
            PickOption::new(*id, *label).icon(rsx! { PriorityIcon { priority: (*id).to_owned() } })
        })
        .collect();
    let mut open_w = open;
    rsx! {
        if !picker_only {
            Chip {
                icon: rsx! { PriorityIcon { priority: current.to_owned() } },
                label: if current == "none" { "Priority".to_owned() } else { shown(PRIORITIES, current).to_owned() },
                quiet: current == "none",
                title: "Priority (P)",
                on_click: move |()| tap(opens, Hotkey::Priority, open),
            }
        }
        if opens.is_none() {
            PropertyPicker {
                open: open(),
                title: "Set priority to…",
                hotkey: "P".to_owned(),
                options,
                selected: vec![current.to_owned()],
                number_from: Some(0),
                on_pick: move |id: String| on_change.call(id),
                on_close: move |()| open_w.set(false),
            }
        }
    }
}

/// Assignee: who it goes to, by name (`people`: the vault's).
#[component]
pub fn AssigneeChip(
    assignee: Option<String>,
    people: Vec<String>,
    on_change: EventHandler<Option<String>>,
    #[props(default)] hotkey: Option<Signal<Option<Hotkey>>>,
    #[props(default)] opens: Option<Signal<Option<Hotkey>>>,
    #[props(default)] picker_only: bool,
) -> Element {
    let l = use_look();
    let open = use_signal(|| false);
    follow_hotkey(hotkey, Hotkey::Assignee, open);
    let mut options = vec![
        PickOption::new("", "No assignee")
            .icon(rsx! { lc::UserRound { size: 14, color: l.ink_2.clone() } }),
    ];
    for name in &people {
        options.push(
            PickOption::new(name.clone(), name.clone())
                .icon(rsx! { PersonDot { name: name.clone(), size: 16 } }),
        );
    }
    let label = assignee.clone();
    let mut open_w = open;
    rsx! {
        if !picker_only {
            Chip {
                icon: match label.clone() {
                    Some(n) => rsx! { PersonDot { name: n } },
                    None => rsx! { lc::UserRound { size: 14, color: l.ink_2 } },
                },
                label: label.clone().unwrap_or_else(|| "Assignee".to_owned()),
                quiet: label.is_none(),
                title: "Assignee (A)",
                on_click: move |()| tap(opens, Hotkey::Assignee, open),
            }
        }
        if opens.is_none() {
            PropertyPicker {
                open: open(),
                title: "Assign to…",
                hotkey: "A".to_owned(),
                options,
                selected: vec![assignee.unwrap_or_default()],
                number_from: Some(0),
                on_pick: move |id: String| on_change.call(Some(id).filter(|s| !s.is_empty())),
                on_close: move |()| open_w.set(false),
            }
        }
    }
}

/// Project: where it is filed, by title. `on_create` makes one from a
/// name no project has (`pending` shows it until it exists).
#[component]
pub fn ProjectChip(
    project: Option<String>,
    projects: Vec<String>,
    on_change: EventHandler<Option<String>>,
    #[props(default)] hotkey: Option<Signal<Option<Hotkey>>>,
    #[props(default)] opens: Option<Signal<Option<Hotkey>>>,
    #[props(default)] picker_only: bool,
    #[props(default)] on_create: Option<EventHandler<String>>,
    #[props(default)] pending: Option<String>,
) -> Element {
    let l = use_look();
    let open = use_signal(|| false);
    follow_hotkey(hotkey, Hotkey::Project, open);
    let mut options = vec![
        PickOption::new("", "No project")
            .icon(rsx! { lc::Box { size: 14, color: l.ink_2.clone() } }),
    ];
    for name in &projects {
        options.push(
            PickOption::new(name.clone(), name.clone())
                .icon(rsx! { lc::Box { size: 14, color: l.brand.clone() } }),
        );
    }
    let label = project.clone().or(pending);
    let tint = if label.is_some() { l.brand } else { l.ink_2 };
    let mut open_w = open;
    rsx! {
        if !picker_only {
            Chip {
                icon: rsx! { lc::Box { size: 14, color: tint } },
                label: label.clone().unwrap_or_else(|| "Project".to_owned()),
                quiet: label.is_none(),
                title: "Project (R)",
                on_click: move |()| tap(opens, Hotkey::Project, open),
            }
        }
        if opens.is_none() {
            PropertyPicker {
                open: open(),
                title: "Add to project…",
                hotkey: "R".to_owned(),
                options,
                selected: vec![project.unwrap_or_default()],
                number_from: None,
                on_pick: move |id: String| on_change.call(Some(id).filter(|s| !s.is_empty())),
                on_create,
                on_close: move |()| open_w.set(false),
            }
        }
    }
}

/// Labels: a chip per label chosen (or one quiet "Labels"), the vault's
/// tags to pick from, and a new one made by typing it.
#[component]
pub fn LabelsChip(
    chosen: Vec<String>,
    known: Vec<String>,
    on_toggle: EventHandler<String>,
    #[props(default)] hotkey: Option<Signal<Option<Hotkey>>>,
    #[props(default)] opens: Option<Signal<Option<Hotkey>>>,
    #[props(default)] picker_only: bool,
) -> Element {
    let l = use_look();
    let open = use_signal(|| false);
    follow_hotkey(hotkey, Hotkey::Labels, open);
    let mut names: Vec<String> = known;
    for c in &chosen {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(c)) {
            names.push(c.clone());
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    let options: Vec<PickOption> = names
        .iter()
        .map(|n| PickOption::new(n.clone(), n.clone()).dot(label_color(n)))
        .collect();
    let mut open_w = open;
    rsx! {
        if !picker_only {
            if chosen.is_empty() {
                Chip {
                    icon: rsx! { lc::Tag { size: 14, color: l.ink_2 } },
                    label: "Labels".to_owned(),
                    quiet: true,
                    title: "Labels (L)",
                    on_click: move |()| tap(opens, Hotkey::Labels, open),
                }
            } else {
                for c in chosen.iter() {
                    Chip {
                        key: "{c}",
                        icon: rsx! { span { style: "width: 8px; height: 8px; flex-shrink: 0; border-radius: 999px; background: {label_color(c)};" } },
                        label: c.clone(),
                        quiet: false,
                        title: "Labels (L)",
                        on_click: move |()| tap(opens, Hotkey::Labels, open),
                    }
                }
            }
        }
        if opens.is_none() {
            PropertyPicker {
                open: open(),
                title: "Add labels…",
                hotkey: "L".to_owned(),
                options,
                selected: chosen,
                multi: true,
                number_from: None,
                on_create: move |name: String| on_toggle.call(name),
                on_pick: move |id: String| on_toggle.call(id),
                on_close: move |()| open_w.set(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_read_in_the_linear_vocabulary() {
        assert_eq!(shown(STATUSES, canonical_status("open")), "Todo");
        assert_eq!(
            shown(STATUSES, canonical_status("in_progress")),
            "In Progress"
        );
        assert_eq!(shown(STATUSES, canonical_status("canceled")), "Canceled");
        assert_eq!(shown(PRIORITIES, canonical_priority("critical")), "Urgent");
        assert_eq!(shown(PRIORITIES, canonical_priority("normal")), "Medium");
        assert_eq!(shown(PRIORITIES, canonical_priority("")), "No priority");
    }

    #[test]
    fn the_five_keys_and_nothing_else() {
        assert_eq!(Hotkey::from_key("s"), Some(Hotkey::Status));
        assert_eq!(Hotkey::from_key("P"), Some(Hotkey::Priority));
        assert_eq!(Hotkey::from_key("l"), Some(Hotkey::Labels));
        assert_eq!(Hotkey::from_key("x"), None);
    }

    #[test]
    fn a_label_keeps_its_colour() {
        assert_eq!(label_color("mixing"), label_color("Mixing"));
        assert_ne!(label_color("mixing"), label_color("mastering"));
    }
}
