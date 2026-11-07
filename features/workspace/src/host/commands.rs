//! The workspace's commands: everything a tab, a divider or a menu row
//! does, by name and by key, in the app's palette (⌘K), and what ⌘O can
//! open — the views, and the panes already open.
//!
//! Each is its workspace action ([`crate::control`]) run on the active
//! pane's behalf with `focus: true`: a key, ⌘K and a model over the
//! Action List run one code path.

use architect_commands::{Command, OpenIn, OpenItem, Opener, use_command, use_opener};
use dioxus::prelude::*;

use super::{Host, use_host};
use crate::control::{
    self, At, CloseArgs, DockBackArgs, DockPopOutArgs, DockSetArgs, DockShown, DockSide, FloatArgs,
    FocusArgs, Go, GroupSetArgs, MoveArgs, NavigateArgs, OpenArgs, PaneArgs, PaneSide, ReopenArgs,
    SplitArgs, TabChrome, TabSetArgs, To, Toward, ZoomArgs,
};
use crate::model::{Area, Chrome, GroupId, PaneId, Shown, Side};
use crate::ops::Target;

/// Registers the workspace's commands while the host is mounted (a
/// child of [`super::WorkspaceView`], so it has the host).
#[component]
pub(super) fn WorkspaceCommands() -> Element {
    let host = use_host();
    let w = host.workspace.read();
    let active = w.active.clone();
    let loc = active.as_ref().and_then(|p| w.locate(p));
    let group = loc.and_then(|l| w.group(l.group)).cloned();
    let tabs = group.as_ref().map_or(0, |g| g.tabs.len());
    let stacked = group.as_ref().is_some_and(|g| g.stacked);
    let closed = !w.closed.is_empty();
    let docked = loc.is_some_and(|l| matches!(l.area, Area::Dock(_)));
    let popped = loc.is_some_and(|l| l.window > 0);
    let split_center = loc.is_some_and(|l| {
        l.area == Area::Center
            && w.windows
                .get(l.window)
                .is_some_and(|win| matches!(win.center, Some(crate::model::Node::Split(_))))
    });
    let can_back = active
        .as_ref()
        .and_then(|p| w.pane(p))
        .is_some_and(|p| !p.history.back.is_empty());
    let can_forward = active
        .as_ref()
        .and_then(|p| w.pane(p))
        .is_some_and(|p| !p.history.forward.is_empty());
    // Each dock: in the main window with groups to pop out, or out.
    let dock_state = |side: Side| {
        let out = w.dock_window(side).is_some();
        let full = w
            .windows
            .first()
            .and_then(|m| m.docks.get(side))
            .is_some_and(|d| d.root.is_some());
        (full, out)
    };
    let docks = [
        dock_state(Side::Left),
        dock_state(Side::Right),
        dock_state(Side::Bottom),
    ];
    drop(w);

    let none = "no tab is active";
    let tab = |c: Command| {
        if active.is_some() {
            c
        } else {
            c.disabled(none)
        }
    };
    let several = |c: Command| {
        if tabs > 1 {
            c
        } else {
            c.disabled("it is the only tab in its pane")
        }
    };

    use_command(
        tab(Command::new("WORKSPACE_CLOSE_TAB", "Close tab")
            .group("Tabs")
            .shortcut("⌘W")),
        move || {
            if let Some(p) = active_pane(host) {
                host.write(
                    control::close,
                    CloseArgs {
                        if_revision: None,
                        panes: vec![p.0.clone()],
                    },
                );
                let mut sheet = host.sheet;
                if sheet.peek().as_ref() == Some(&p) {
                    sheet.set(None);
                }
            }
        },
    );
    use_command(
        if closed {
            Command::new("WORKSPACE_REOPEN_TAB", "Reopen closed tab")
        } else {
            Command::new("WORKSPACE_REOPEN_TAB", "Reopen closed tab").disabled("nothing was closed")
        }
        .group("Tabs")
        .shortcut("⇧⌘T"),
        move || {
            host.write(
                control::reopen,
                ReopenArgs {
                    if_revision: None,
                    focus: true,
                },
            );
        },
    );
    use_command(
        several(
            Command::new("WORKSPACE_NEXT_TAB", "Next tab")
                .group("Tabs")
                .shortcut("⌃Tab"),
        ),
        move || cycle(host, false),
    );
    use_command(
        several(
            Command::new("WORKSPACE_PREVIOUS_TAB", "Previous tab")
                .group("Tabs")
                .shortcut("⌃⇧Tab"),
        ),
        move || cycle(host, true),
    );
    use_command(
        several(
            Command::new("WORKSPACE_CLOSE_OTHER_TABS", "Close other tabs")
                .group("Tabs")
                .destructive(),
        ),
        move || {
            if let Some(p) = active_pane(host) {
                host.write(
                    control::close_others,
                    PaneArgs {
                        if_revision: None,
                        pane: p.0,
                    },
                );
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_PIN_TAB", "Pin or unpin tab").group("Tabs")),
        move || {
            if let Some(p) = active_pane(host) {
                let pinned = host.workspace.peek().pane(&p).is_some_and(|p| p.pinned);
                host.write(
                    control::tab_set,
                    TabSetArgs {
                        if_revision: None,
                        pane: p.0,
                        pinned: Some(!pinned),
                        link: None,
                        title: None,
                    },
                );
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_GO_BACK", "Go back")
            .group("Tabs")
            .shortcut("⌘["))
        .enabled_if(can_back, "nothing to go back to"),
        move || go(host, Go::Back),
    );
    use_command(
        tab(Command::new("WORKSPACE_GO_FORWARD", "Go forward")
            .group("Tabs")
            .shortcut("⌘]"))
        .enabled_if(can_forward, "nothing to go forward to"),
        move || go(host, Go::Forward),
    );

    // Panes.
    for (id, label, side, key) in [
        (
            "WORKSPACE_SPLIT_RIGHT",
            "Split right",
            PaneSide::Right,
            "⌘\\",
        ),
        ("WORKSPACE_SPLIT_DOWN", "Split down", PaneSide::Bottom, ""),
    ] {
        use_command(
            several(Command::new(id, label).group("Panes").shortcut(key)),
            move || split(host, side),
        );
    }
    for (id, label, direction, key) in [
        (
            "WORKSPACE_FOCUS_LEFT",
            "Focus the pane to the left",
            Toward::Left,
            "⌥⌘←",
        ),
        (
            "WORKSPACE_FOCUS_RIGHT",
            "Focus the pane to the right",
            Toward::Right,
            "⌥⌘→",
        ),
        (
            "WORKSPACE_FOCUS_UP",
            "Focus the pane above",
            Toward::Up,
            "⌥⌘↑",
        ),
        (
            "WORKSPACE_FOCUS_DOWN",
            "Focus the pane below",
            Toward::Down,
            "⌥⌘↓",
        ),
    ] {
        use_command(
            tab(Command::new(id, label).group("Panes").shortcut(key)),
            move || focus(host, direction),
        );
    }
    use_command(
        tab(Command::new("WORKSPACE_FLOAT_TAB", "Float tab").group("Panes")),
        move || {
            if let Some(p) = active_pane(host) {
                host.write(
                    control::float,
                    FloatArgs {
                        if_revision: None,
                        pane: p.0,
                        rect: None,
                        snap: None,
                        panel_width: None,
                        pin: None,
                        above_all: false,
                        focus: true,
                    },
                );
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_ZOOM_PANE", "Zoom the pane").group("Panes"))
            .enabled_if(split_center, "its window shows one group"),
        move || {
            if let Some(p) = active_pane(host) {
                host.write(
                    control::zoom,
                    ZoomArgs {
                        if_revision: None,
                        pane: p.0,
                        mode: control::Mode::Toggle,
                    },
                );
            }
        },
    );
    use_command(
        tab(
            Command::new("WORKSPACE_MOVE_TO_NEW_WINDOW", "Move tab to a new window").group("Panes"),
        )
        .enabled_if(host.has_windows(), "this app opens no other windows"),
        move || {
            move_active(
                host,
                To::Window {
                    id: None,
                    bounds: None,
                },
            );
        },
    );
    use_command(
        tab(Command::new(
            "WORKSPACE_MOVE_TO_MAIN_WINDOW",
            "Move tab to the main window",
        )
        .group("Panes"))
        .enabled_if(popped, "it is in the main window"),
        move || {
            move_active(
                host,
                To::Window {
                    id: Some("main".to_owned()),
                    bounds: None,
                },
            );
        },
    );
    use_command(
        tab(Command::new(
            "WORKSPACE_STACK_TABS",
            if stacked {
                "Unstack tabs"
            } else {
                "Stack tabs"
            },
        )
        .group("Panes")),
        move || {
            let g = host.workspace.peek().active_group();
            if let Some(g) = g {
                let on = host.workspace.peek().group(g).is_some_and(|g| g.stacked);
                host.write(
                    control::group_set,
                    GroupSetArgs {
                        if_revision: None,
                        group: g.0,
                        stacked: Some(!on),
                        chrome: None,
                        current: None,
                    },
                );
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_TAB_BAR", "Show or hide the tab bar").group("Panes")),
        move || {
            let g = host.workspace.peek().active_group();
            if let Some(g) = g {
                let chrome = host
                    .workspace
                    .peek()
                    .group(g)
                    .map_or(Chrome::Tabs, |g| g.chrome);
                let next = if chrome == Chrome::Bare {
                    TabChrome::Tabs
                } else {
                    TabChrome::Bare
                };
                host.write(
                    control::group_set,
                    GroupSetArgs {
                        if_revision: None,
                        group: g.0,
                        stacked: None,
                        chrome: Some(next),
                        current: None,
                    },
                );
            }
        },
    );
    use_command(
        Command::new("WORKSPACE_ARRANGE", "Arrange panes")
            .group("Panes")
            .detail("hold W")
            .read_only(),
        move || host.toggle_arrange(),
    );
    use_command(
        Command::new("WORKSPACE_EVERY_PANE", "Show every open pane")
            .group("Panes")
            .read_only(),
        move || {
            if (host.phone)() {
                host.show_switcher();
            } else {
                let group = host.workspace.peek().active_group();
                if let Some(g) = group {
                    host.show_menu(super::Menu::Tabs(g));
                }
            }
        },
    );

    // Docks.
    for (id, label, side) in [
        (
            "WORKSPACE_MOVE_TO_LEFT_DOCK",
            "Move tab to the left dock",
            DockSide::Left,
        ),
        (
            "WORKSPACE_MOVE_TO_RIGHT_DOCK",
            "Move tab to the right dock",
            DockSide::Right,
        ),
        (
            "WORKSPACE_MOVE_TO_BOTTOM_DOCK",
            "Move tab to the bottom dock",
            DockSide::Bottom,
        ),
    ] {
        use_command(tab(Command::new(id, label).group("Docks")), move || {
            move_active(host, To::Dock { side });
        });
    }
    use_command(
        if docked {
            Command::new("WORKSPACE_MOVE_TO_CENTER", "Move tab to the center")
        } else {
            Command::new("WORKSPACE_MOVE_TO_CENTER", "Move tab to the center")
                .disabled("it is in the center")
        }
        .group("Docks"),
        move || to_center(host),
    );
    for (id, label, side, out) in [
        (
            "WORKSPACE_TOGGLE_RIGHT_DOCK",
            "Show or hide the right dock",
            DockSide::Right,
            docks[1].1,
        ),
        (
            "WORKSPACE_TOGGLE_BOTTOM_DOCK",
            "Show or hide the bottom dock",
            DockSide::Bottom,
            docks[2].1,
        ),
    ] {
        let command = Command::new(id, label).group("Docks");
        let command = if out {
            command.disabled("it is in its own window")
        } else {
            command
        };
        use_command(command, move || {
            let shown = host
                .workspace
                .peek()
                .dock(side.into())
                .map_or(Shown::Hidden, |d| d.shown);
            let next = if shown == Shown::Open {
                DockShown::Collapsed
            } else {
                DockShown::Open
            };
            host.write(
                control::dock_set,
                DockSetArgs {
                    if_revision: None,
                    side,
                    shown: Some(next),
                    size: None,
                },
            );
        });
    }

    // Back to the saved layout (the manage sheet's Reset layout): every
    // pop-out docked back, every dock at its saved size.
    let resettable = host.screensets.read().is_resettable(*host.form.read());
    use_command(
        Command::new("WORKSPACE_RESET_LAYOUT", "Reset layout")
            .group("Workspaces")
            .enabled_if(resettable, "the layout is as it was saved"),
        move || host.write(control::reset, control::RevisionArgs::default()),
    );

    // A dock into a window of its own, and back: one pair per side. Which
    // of the two is enabled says where the dock is (for a scenario to
    // check, as for a person).
    for ((pop_id, pop_label, back_id, back_label, side), (full, out)) in [
        (
            "WORKSPACE_POP_OUT_LEFT_DOCK",
            "Move the sidebar to its own window",
            "WORKSPACE_DOCK_BACK_LEFT_DOCK",
            "Dock the sidebar back",
            DockSide::Left,
        ),
        (
            "WORKSPACE_POP_OUT_RIGHT_DOCK",
            "Move the right sidebar to its own window",
            "WORKSPACE_DOCK_BACK_RIGHT_DOCK",
            "Dock the right sidebar back",
            DockSide::Right,
        ),
        (
            "WORKSPACE_POP_OUT_BOTTOM_DOCK",
            "Move the bottom panel to its own window",
            "WORKSPACE_DOCK_BACK_BOTTOM_DOCK",
            "Dock the bottom panel back",
            DockSide::Bottom,
        ),
    ]
    .into_iter()
    .zip(docks)
    {
        let pop = Command::new(pop_id, pop_label).group("Docks");
        let pop = if out {
            pop.disabled("it is in its own window")
        } else if !full {
            pop.disabled("the dock is empty")
        } else {
            pop.enabled_if(host.has_windows(), "this app opens no other windows")
        };
        use_command(pop, move || {
            host.write(
                control::dock_pop_out,
                DockPopOutArgs {
                    if_revision: None,
                    side,
                    bounds: None,
                },
            );
        });
        let back = Command::new(back_id, back_label).group("Docks");
        use_command(
            back.enabled_if(out, "it is in the main window"),
            move || {
                host.write(
                    control::dock_back,
                    DockBackArgs {
                        if_revision: None,
                        side,
                    },
                );
            },
        );
    }

    // Views: one "Open …" each, a component of its own, so views that
    // come and go (an extension's, a person's new toolbar) add and take
    // away their commands without changing this one's hooks.
    let views: Vec<(String, String)> = host
        .views
        .read()
        .iter()
        .map(|v| (v.kind.0.clone(), v.title.clone()))
        .collect();

    // ⌘O: the open panes, and the views.
    use_opener(move || {
        Opener::new(
            "tab",
            "Open tabs",
            move |_| {
                let w = host.workspace.peek();
                w.panes
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let place = match w.locate(&p.id).map(|l| l.area) {
                            Some(Area::Center) => "Open",
                            Some(Area::Dock(Side::Left)) => "Left dock",
                            Some(Area::Dock(Side::Right)) => "Right dock",
                            Some(Area::Dock(_)) => "Bottom dock",
                            Some(Area::Float(_)) => "Floating",
                            None => "",
                        };
                        let recency = w.recent.iter().position(|r| r == &p.id).map_or(0, |r| {
                            1000u64.saturating_sub(u64::try_from(r).unwrap_or(1000))
                        });
                        let _ = i;
                        OpenItem::new("tab", p.id.0.clone(), host.title(p))
                            .detail(place)
                            .recency(recency)
                    })
                    .collect()
            },
            move |item, _| {
                host.write(
                    control::focus,
                    FocusArgs {
                        if_revision: None,
                        pane: Some(item.id.clone()),
                        from: None,
                        direction: None,
                    },
                );
            },
        )
    });
    use_opener(move || {
        Opener::new(
            "view",
            "Views",
            move |_| {
                host.views
                    .peek()
                    .iter()
                    .map(|v| {
                        OpenItem::new("view", v.kind.0.clone(), v.title.clone()).detail("View")
                    })
                    .collect()
            },
            move |item, open_in| {
                let target = match open_in {
                    OpenIn::Here => None,
                    OpenIn::Tab => Some(Target::Tab),
                    OpenIn::Split => Some(Target::Split(Side::Right)),
                };
                open_view(host, &item.id, target);
            },
        )
    });
    rsx! {
        super::screens::ScreensetCommands {}
        for (kind, title) in views {
            OpenViewCommand { key: "{kind}", kind, title }
        }
    }
}

/// A view's "Open …" command, while it is one of the host's.
#[component]
fn OpenViewCommand(kind: String, title: String) -> Element {
    let host = use_host();
    let id = format!(
        "WORKSPACE_OPEN_{}",
        kind.to_uppercase().replace(['.', '-', ' '], "_")
    );
    use_command(
        Command::new(id, format!("Open {title}")).group("Views"),
        move || open_view(host, &kind, None),
    );
    rsx! {}
}

/// The active pane, the target a command acts for.
fn active_pane(host: Host) -> Option<PaneId> {
    host.workspace.peek().active.clone()
}

/// Moves the active pane `to`, the focus going with it.
fn move_active(host: Host, to: To) {
    if let Some(p) = active_pane(host) {
        host.write(
            control::move_pane,
            MoveArgs {
                if_revision: None,
                pane: p.0,
                to,
                focus: true,
            },
        );
    }
}

/// Moves the active pane into a new group beside its own, on `side`; the
/// new group grows in from that edge.
fn split(host: Host, side: PaneSide) {
    let Some(pane) = active_pane(host) else {
        return;
    };
    let answer = host.act(|st, env| {
        control::transact(
            st,
            env,
            control::split,
            SplitArgs {
                if_revision: None,
                pane: pane.0,
                side,
                focus: true,
            },
        )
    });
    if let Ok(a) = answer
        && a.changed
        && let Some(g) = a.group
    {
        let mut grow_size = host.grow_size;
        grow_size.set(None);
        host.grow_in(GroupId(g));
    }
}

/// Moves the focus to the group beside the active pane's.
fn focus(host: Host, direction: Toward) {
    if let Some(p) = active_pane(host) {
        host.write(
            control::focus,
            FocusArgs {
                if_revision: None,
                pane: None,
                from: Some(p.0),
                direction: Some(direction),
            },
        );
    }
}

/// The next (or, `back`, the previous) tab of the active group, round.
fn cycle(host: Host, back: bool) {
    let next = {
        let w = host.workspace.peek();
        w.active_group().and_then(|g| w.group(g)).and_then(|g| {
            let n = g.tabs.len();
            let i = if back {
                g.current
                    .checked_sub(1)
                    .unwrap_or_else(|| n.saturating_sub(1))
            } else {
                g.current.saturating_add(1).checked_rem(n)?
            };
            g.tabs.get(i).cloned()
        })
    };
    if let Some(p) = next {
        host.write(
            control::focus,
            FocusArgs {
                if_revision: None,
                pane: Some(p.0),
                from: None,
                direction: None,
            },
        );
    }
}

/// Back or forward through the active pane's history.
fn go(host: Host, go: Go) {
    if let Some(p) = active_pane(host) {
        host.write(
            control::navigate,
            NavigateArgs {
                if_revision: None,
                pane: p.0,
                go: Some(go),
                state: None,
            },
        );
    }
}

/// Moves the active pane into the center's first group.
fn to_center(host: Host) {
    let group = host
        .workspace
        .peek()
        .windows
        .first()
        .and_then(|win| win.center.as_ref())
        .and_then(crate::model::Node::first_group)
        .map(|g| g.id.0);
    if let Some(group) = group {
        move_active(host, To::Group { group, index: None });
    }
}

/// Opens a view of `kind` at `target` (where it lives when unset), next
/// to the active pane, the focus going to it. A view of the main area
/// opens beside the document on show when the active pane is a dock's
/// (#442: a page took the sidebar's place), else in the center.
fn open_view(host: Host, kind: &str, target: Option<Target>) {
    let Some(home) = target.or_else(|| host.views.peek().get(kind).map(|v| v.home)) else {
        return;
    };
    let (active, first) = {
        let w = host.workspace.peek();
        let docked = |p: &crate::model::PaneId| {
            w.locate(p)
                .is_some_and(|l| matches!(l.area, crate::model::Area::Dock(_)))
        };
        let active = match w.active.clone() {
            Some(p) if docked(&p) && !matches!(home, Target::Dock(_)) => {
                w.current_document().cloned()
            }
            other => other,
        };
        let first = w.active_group().filter(|_| active.is_some()).or_else(|| {
            w.windows
                .first()?
                .center
                .as_ref()?
                .first_group()
                .map(|g| g.id)
        });
        (active, first)
    };
    let in_group = || {
        first.map(|g| At::Group {
            group: g.0,
            index: None,
        })
    };
    let at = match (home, active) {
        (Target::Here, Some(p)) => Some(At::Replace { pane: p.0 }),
        (Target::Tab, Some(p)) => Some(At::Beside { pane: p.0 }),
        (Target::Split(side), Some(p)) => Some(At::Split {
            pane: p.0,
            side: match side {
                Side::Left => PaneSide::Left,
                Side::Right => PaneSide::Right,
                Side::Top => PaneSide::Top,
                Side::Bottom => PaneSide::Bottom,
            },
        }),
        (Target::Here | Target::Tab | Target::Split(_), None) => in_group(),
        (Target::Window(bounds), _) => Some(At::Window { bounds }),
        (Target::Dock(side), _) => Some(At::Dock {
            side: match side {
                Side::Left | Side::Top => DockSide::Left,
                Side::Right => DockSide::Right,
                Side::Bottom => DockSide::Bottom,
            },
        }),
        (Target::Float(rect), _) => Some(At::Float { rect: Some(rect) }),
    };
    if let Some(at) = at {
        host.write(
            control::open,
            OpenArgs {
                if_revision: None,
                view: kind.to_owned(),
                id: None,
                state: None,
                title: None,
                at,
                focus: true,
            },
        );
    }
}

/// `enabled_if` on a command: disabled with `why` unless `ok`.
trait EnabledIf {
    fn enabled_if(self, ok: bool, why: &str) -> Self;
}

impl EnabledIf for Command {
    fn enabled_if(self, ok: bool, why: &str) -> Self {
        if ok || !self.enabled {
            self
        } else {
            self.disabled(why)
        }
    }
}
