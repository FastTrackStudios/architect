//! The bars that hold the tabs: a phone's foot, the rail, a docked
//! sidebar's rows, and the separate button in each.

use dioxus::prelude::*;

use super::{Action, Icon, RailSideButton, Screen, ShellStyle, Tab, use_shell};

/// A phone's foot, in the shell's style: the tabs and the separate button.
#[component]
pub(super) fn BottomBar(
    tabs: Vec<Tab>,
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
    separate: Option<Action>,
    screen: Screen,
) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let ins = screen.insets;
    let tabs: Vec<Tab> = tabs.into_iter().take(4).collect();
    if shell.style == ShellStyle::TabBar {
        let bottom = ins.bottom.max(4.0);
        return rsx! {
            div {
                style: "flex-shrink: 0; display: flex; align-items: stretch; height: {54.0 + bottom}px; box-sizing: border-box; \
                        padding: 0 {8.0 + ins.right}px {bottom}px {8.0 + ins.left}px; \
                        background: {t.sheet}; border-top: 1px solid {t.rule};",
                for tab in tabs {
                    BarTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab, labelled: true }
                }
                if let Some(sep) = separate {
                    div { style: "width: 1px; margin: 12px 6px; background: {t.rule};" }
                    PlainButton { action: sep, labelled: true }
                }
            }
        };
    }
    // Task's: the pill, and the round button beside it, floating just above
    // the home indicator.
    let bottom = (ins.bottom - 4.0).max(12.0);
    rsx! {
        div {
            style: "flex-shrink: 0; display: flex; align-items: flex-end; gap: 8px; \
                    padding: 8px {20.0 + ins.right}px {bottom}px {20.0 + ins.left}px;",
            onpointerdown: move |e: PointerEvent| super::input_debug::hit("the tabs", &e),
            div {
                style: "flex: 1; min-width: 0; height: 54px; box-sizing: border-box; display: flex; align-items: center; gap: 2px; \
                        padding: 4px; border-radius: 27px; background: {t.pill}; border: 1px solid {t.rule}; \
                        box-shadow: {t.shadow_lift};",
                for tab in tabs {
                    BarTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab, labelled: false }
                }
            }
            if let Some(sep) = separate {
                RoundButton { action: sep }
            }
        }
    }
}

#[component]
fn BarTab(tab: Tab, on: bool, on_tab: EventHandler<&'static str>, labelled: bool) -> Element {
    let t = use_shell().theme;
    let colour = if on { &*t.ink } else { &*t.ink_3 };
    let (fill, radius, height) = if labelled {
        ("transparent", 10, "100%")
    } else if on {
        (&*t.active, 23, "46px")
    } else {
        ("transparent", 23, "46px")
    };
    let id = tab.id;
    rsx! {
        button {
            "aria-label": tab.label,
            "data-tab": id,
            "aria-current": if on { "page" } else { "false" },
            style: "position: relative; flex: 1 1 0; height: {height}; display: flex; flex-direction: column; \
                    align-items: center; justify-content: center; gap: 3px; border: none; border-radius: {radius}px; \
                    background: {fill}; color: {colour}; font-family: {t.font}; font-size: 11px; font-weight: 650;",
            onclick: move |_| on_tab.call(id),
            {tab.draw(Icon { size: 22, color: colour.into() })}
            if labelled { span { "{tab.label}" } }
            if let Some(b) = tab.badge.clone() { Badge { text: b } }
        }
    }
}

/// The round separate button: Task's capture, Studio's search.
///
/// With a [`SwipeUp`](super::SwipeUp) (Task's `BottomTabBar` capture
/// button): a press and a slide lifts it with the pointer, and past
/// [`SWIPE_UP`](super::SWIPE_UP) it takes the swipe's icon and the accent
/// and lets go into the swipe (a timer) instead of the press (capture).
/// With a badge, the count rides its corner.
#[component]
pub(super) fn RoundButton(action: Action) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    // A finger's size on a touch screen, a mouse's on a desk.
    let (size, icon) = if shell.desk() { (44.0, 18) } else { (54.0, 22) };
    let press = action.on_press;
    let swipe = action.swipe_up.clone();
    let can_swipe = swipe.is_some();
    // The slide: where it started, how far it has lifted, whether that is
    // far enough, and the click a finished slide leaves to swallow.
    let mut start_y = use_signal(|| None::<f64>);
    let mut lift = use_signal(|| 0.0_f64);
    let mut swiping = use_signal(|| false);
    let mut swallow = use_signal(|| false);
    let on_swipe = swipe.as_ref().map(|s| s.on_swipe);
    let mut settle = move || {
        start_y.set(None);
        lift.set(0.0);
        swiping.set(false);
    };
    let on_move = move |e: PointerEvent| {
        if let Some(y0) = start_y() {
            let up = (y0 - e.client_coordinates().y).max(0.0);
            lift.set(up);
            swiping.set(up >= super::SWIPE_UP);
        }
    };
    let mut on_up = move |_: PointerEvent| {
        let go = swiping();
        settle();
        if go && let Some(f) = on_swipe {
            swallow.set(true);
            f.call(());
        }
    };
    let (bg, fg, border) = if swiping() {
        (t.accent.clone(), t.accent_ink.clone(), t.accent.clone())
    } else {
        (t.pill.clone(), t.ink.clone(), t.rule.clone())
    };
    let shift = if lift() > 0.0 {
        format!("transform: translateY(-{:.0}px);", lift().min(48.0))
    } else {
        String::new()
    };
    let label = match (&swipe, swiping()) {
        (Some(s), true) => s.label.clone(),
        _ => action.label.clone(),
    };
    rsx! {
        button {
            class: "tip-host",
            "aria-label": "{label}",
            "data-primary": "round",
            "data-swiping": "{swiping()}",
            style: "position: relative; width: {size}px; height: {size}px; flex-shrink: 0; box-sizing: border-box; border-radius: {size / 2.0}px; \
                    display: flex; align-items: center; justify-content: center; padding: 0; background: {bg}; \
                    border: 1px solid {border}; color: {fg}; box-shadow: {t.shadow_lift}; {shift}",
            onpointerdown: move |e: PointerEvent| {
                // A slide let go off the button left its click elsewhere:
                // nothing to swallow from before this press.
                swallow.set(false);
                if can_swipe {
                    start_y.set(Some(e.client_coordinates().y));
                }
            },
            onpointermove: on_move,
            onpointercancel: move |_| settle(),
            onpointerup: on_up,
            onclick: move |_| {
                if swallow() {
                    swallow.set(false);
                    return;
                }
                press.call(());
            },
            match (&swipe, swiping()) {
                (Some(s), true) => (s.icon)(Icon { size: icon, color: fg.clone() }),
                _ => action.draw(Icon { size: icon, color: fg.clone() }),
            }
            if let Some(n) = action.badge {
                span {
                    style: "position: absolute; top: -3px; right: -3px; min-width: 18px; height: 18px; padding: 0 5px; \
                            box-sizing: border-box; border-radius: 9px; display: flex; align-items: center; justify-content: center; \
                            background: {t.accent}; color: {t.accent_ink}; font-size: 10px; font-weight: 600; \
                            font-variant-numeric: tabular-nums; pointer-events: none;",
                    "{n}"
                }
            }
            if shell.desk() {
                span { class: "tip", style: "right: 0; bottom: {size + 8.0}px;", "{label}" }
            }
        }
        // While it slides, the pointer is the button's wherever it goes
        // (a touch keeps its target; a pointer would leave it above the
        // lifted button and let go elsewhere).
        // Only once it moves: a tap's down and up stay the button's, so its
        // click comes.
        if start_y().is_some() && lift() > 4.0 {
            div {
                style: "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 10000;",
                onpointermove: on_move,
                onpointerup: move |e: PointerEvent| {
                    e.stop_propagation();
                    on_up(e);
                },
                onpointercancel: move |_| settle(),
            }
        }
    }
}

/// The separate button as one more item of a bar.
#[component]
fn PlainButton(action: Action, labelled: bool) -> Element {
    let t = use_shell().theme;
    let press = action.on_press;
    rsx! {
        button {
            "aria-label": "{action.label}",
            "data-primary": "plain",
            style: "flex: 0 0 64px; min-height: 52px; display: flex; flex-direction: column; align-items: center; \
                    justify-content: center; gap: 3px; border: none; border-radius: 10px; background: transparent; \
                    color: {t.ink}; font-family: {t.font}; font-size: 11px; font-weight: 650;",
            onclick: move |_| press.call(()),
            {action.draw(Icon { size: 22, color: t.ink.clone() })}
            if labelled { span { "{action.label}" } }
        }
    }
}

/// The tabs down the left: a phone on its side, an iPad upright, a folded
/// sidebar, `RailOnly`. At its head the button that brings the left side
/// out; at its foot the separate button.
#[component]
pub(super) fn Rail(
    tabs: Vec<Tab>,
    current: Option<&'static str>,
    on_tab: EventHandler<&'static str>,
    screen: Screen,
) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let ins = screen.insets;
    // On a tablet the separate button floats at the page's bottom right
    // instead, and on a desk it is in the top bar: the rail's foot is the
    // account's.
    let elsewhere = shell.floats_separate || shell.form() == super::FormFactor::Desk;
    let separate = if elsewhere {
        None
    } else {
        shell.separate.clone()
    };
    let account = shell.account.clone();
    rsx! {
        div {
            style: "width: {76.0 + ins.left}px; flex-shrink: 0; box-sizing: border-box; display: flex; flex-direction: column; \
                    align-items: stretch; gap: 4px; padding: {ins.top + 10.0}px 6px {ins.bottom + 10.0}px {6.0 + ins.left}px; \
                    background: {t.sheet}; border-right: 1px solid {t.rule};",
            RailSideButton {}
            if let Some(workspaces) = shell.workspaces {
                div { style: "display: flex; margin-bottom: 6px;",
                    super::Switcher { workspaces, place: super::Place::Rail }
                }
            }
            for tab in tabs {
                RailTab { key: "{tab.id}", on: current == Some(tab.id), tab, on_tab }
            }
            span { style: "flex: 1;" }
            for action in shell.foot.clone() {
                super::FootIcon { key: "{action.label}", action }
            }
            if let Some(account) = account {
                super::AccountChip { account }
            }
            if let Some(sep) = separate {
                div { style: "display: flex; justify-content: center;", RoundButton { action: sep } }
            }
        }
    }
}

#[component]
fn RailTab(tab: Tab, on: bool, on_tab: EventHandler<&'static str>) -> Element {
    let t = use_shell().theme;
    let (fill, colour) = if on {
        (&*t.active, &*t.ink)
    } else {
        ("transparent", &*t.ink_3)
    };
    let id = tab.id;
    rsx! {
        button {
            "aria-label": tab.label,
            style: "position: relative; height: 58px; display: flex; flex-direction: column; align-items: center; \
                    justify-content: center; gap: 4px; border: none; border-radius: 12px; background: {fill}; \
                    color: {colour}; font-family: {t.font}; font-size: 11px; font-weight: 650;",
            onclick: move |_| on_tab.call(id),
            {tab.draw(Icon { size: 21, color: colour.into() })}
            span { "{tab.label}" }
            if let Some(b) = tab.badge.clone() { Badge { text: b } }
        }
    }
}

/// A tab as a row at the head of a docked sidebar.
#[component]
pub(super) fn SideTab(tab: Tab, on: bool, on_tab: EventHandler<&'static str>) -> Element {
    let t = use_shell().theme;
    let (fill, colour, weight) = if on {
        (&*t.accent_soft, &*t.ink, 700)
    } else {
        ("transparent", &*t.ink_2, 560)
    };
    let id = tab.id;
    rsx! {
        button {
            "aria-label": tab.label,
            style: "height: 40px; display: flex; align-items: center; justify-content: flex-start; gap: 12px; padding: 0 12px; \
                    border: none; border-radius: 10px; background: {fill}; color: {colour}; font-family: {t.font}; \
                    font-size: 15px; font-weight: {weight}; text-align: left;",
            onclick: move |_| on_tab.call(id),
            {tab.draw(Icon { size: 19, color: colour.into() })}
            span { style: "flex: 1; text-align: left;", "{tab.label}" }
            if let Some(b) = tab.badge.clone() {
                span { style: "min-width: 18px; height: 18px; padding: 0 5px; box-sizing: border-box; border-radius: 9px; \
                               display: flex; align-items: center; justify-content: center; \
                               background: {t.accent}; color: {t.accent_ink}; font-size: 11px; font-weight: 700;",
                    "{b}"
                }
            }
        }
    }
}

#[component]
fn Badge(text: String) -> Element {
    let t = use_shell().theme;
    rsx! {
        span {
            style: "position: absolute; top: 4px; right: 22%; min-width: 16px; height: 16px; padding: 0 4px; \
                    box-sizing: border-box; border-radius: 8px; display: flex; align-items: center; justify-content: center; \
                    background: {t.accent}; color: {t.accent_ink}; font-size: 11px; font-weight: 700;",
            "{text}"
        }
    }
}
