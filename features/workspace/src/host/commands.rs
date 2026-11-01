//! The workspace's commands: everything a tab, a divider or a menu row
//! does, by name and by key, in the app's palette (⌘K), and what ⌘O can
//! open — the views, and the panes already open.

use architect_commands::{Command, OpenIn, OpenItem, Opener, use_command, use_opener};
use dioxus::prelude::*;

use super::{Host, Op, use_host};
use crate::layout::{Direction, area_rects, neighbour};
use crate::model::{Area, Chrome, Rect, Shown, Side};
use crate::ops::{Dest, Target};

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
    let has_sets = !host.screensets.read().sets.is_empty();
    let can_back = active
        .as_ref()
        .and_then(|p| w.pane(p))
        .is_some_and(|p| !p.history.back.is_empty());
    let can_forward = active
        .as_ref()
        .and_then(|p| w.pane(p))
        .is_some_and(|p| !p.history.forward.is_empty());
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
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::Close(p));
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
        move || host.run(Op::Reopen),
    );
    use_command(
        several(
            Command::new("WORKSPACE_NEXT_TAB", "Next tab")
                .group("Tabs")
                .shortcut("⌃Tab"),
        ),
        move || {
            let _ = host.edit(|w| w.cycle_tab(false));
        },
    );
    use_command(
        several(
            Command::new("WORKSPACE_PREVIOUS_TAB", "Previous tab")
                .group("Tabs")
                .shortcut("⌃⇧Tab"),
        ),
        move || {
            let _ = host.edit(|w| w.cycle_tab(true));
        },
    );
    use_command(
        several(Command::new("WORKSPACE_CLOSE_OTHER_TABS", "Close other tabs").group("Tabs")),
        move || {
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::CloseOthers(p));
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_PIN_TAB", "Pin or unpin tab").group("Tabs")),
        move || {
            let pane = host.workspace.peek().active.clone();
            if let Some(p) = pane {
                let pinned = host.workspace.peek().pane(&p).is_some_and(|p| p.pinned);
                host.run(Op::Pin(p, !pinned));
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_GO_BACK", "Go back")
            .group("Tabs")
            .shortcut("⌘["))
        .enabled_if(can_back, "nothing to go back to"),
        move || {
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                let _ = host.edit(|w| w.go(&p, false));
            }
        },
    );
    use_command(
        tab(Command::new("WORKSPACE_GO_FORWARD", "Go forward")
            .group("Tabs")
            .shortcut("⌘]"))
        .enabled_if(can_forward, "nothing to go forward to"),
        move || {
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                let _ = host.edit(|w| w.go(&p, true));
            }
        },
    );

    // Panes.
    for (id, label, side, key) in [
        ("WORKSPACE_SPLIT_RIGHT", "Split right", Side::Right, "⌘\\"),
        ("WORKSPACE_SPLIT_DOWN", "Split down", Side::Bottom, ""),
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
            Direction::Left,
            "⌥⌘←",
        ),
        (
            "WORKSPACE_FOCUS_RIGHT",
            "Focus the pane to the right",
            Direction::Right,
            "⌥⌘→",
        ),
        (
            "WORKSPACE_FOCUS_UP",
            "Focus the pane above",
            Direction::Up,
            "⌥⌘↑",
        ),
        (
            "WORKSPACE_FOCUS_DOWN",
            "Focus the pane below",
            Direction::Down,
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
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::Move(p, Dest::Float(super::FLOAT_AT)));
            }
        },
    );
    use_command(
        tab(
            Command::new("WORKSPACE_MOVE_TO_NEW_WINDOW", "Move tab to a new window").group("Panes"),
        )
        .enabled_if(host.has_windows(), "this app opens no other windows"),
        move || {
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::Move(p, Dest::Window(None)));
            }
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
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::ToMain(p));
            }
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
                host.run(Op::Stacked(g, !on));
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
                    Chrome::Tabs
                } else {
                    Chrome::Bare
                };
                host.run(Op::Chrome(g, next));
            }
        },
    );
    use_command(
        Command::new("WORKSPACE_ARRANGE", "Arrange panes")
            .group("Panes")
            .detail("hold W"),
        move || host.toggle_arrange(),
    );
    use_command(
        Command::new("WORKSPACE_EVERY_PANE", "Show every open pane").group("Panes"),
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
            Side::Left,
        ),
        (
            "WORKSPACE_MOVE_TO_RIGHT_DOCK",
            "Move tab to the right dock",
            Side::Right,
        ),
        (
            "WORKSPACE_MOVE_TO_BOTTOM_DOCK",
            "Move tab to the bottom dock",
            Side::Bottom,
        ),
    ] {
        use_command(tab(Command::new(id, label).group("Docks")), move || {
            let active = host.workspace.peek().active.clone();
            if let Some(p) = active {
                host.run(Op::Move(p, Dest::Dock(side)));
            }
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
    for (id, label, side) in [
        (
            "WORKSPACE_TOGGLE_RIGHT_DOCK",
            "Show or hide the right dock",
            Side::Right,
        ),
        (
            "WORKSPACE_TOGGLE_BOTTOM_DOCK",
            "Show or hide the bottom dock",
            Side::Bottom,
        ),
    ] {
        use_command(Command::new(id, label).group("Docks"), move || {
            let shown = host
                .workspace
                .peek()
                .windows
                .first()
                .and_then(|w| w.docks.get(side))
                .map_or(Shown::Hidden, |d| d.shown);
            let next = if shown == Shown::Open {
                Shown::Collapsed
            } else {
                Shown::Open
            };
            host.run(Op::Dock(side, next));
        });
    }

    // Views: one "Open …" each. The views are fixed for a host's life, so
    // the number of commands (hooks) is too.
    let views: Vec<(String, String)> = host
        .views
        .read()
        .iter()
        .map(|v| (v.kind.0.clone(), v.title.clone()))
        .collect();
    for (kind, title) in views {
        let id = format!(
            "WORKSPACE_OPEN_{}",
            kind.to_uppercase().replace(['.', '-', ' '], "_")
        );
        use_command(
            Command::new(id, format!("Open {title}")).group("Views"),
            move || {
                let _ = host.open(&kind);
            },
        );
    }

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
            move |item, _| host.run(Op::Show(crate::model::PaneId::new(item.id.clone()))),
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
                let _ = target.map_or_else(|| host.open(&item.id), |t| host.open_at(&item.id, t));
            },
        )
    });
    rsx! {
        if has_sets {
            super::screens::ScreensetCommands {}
        }
    }
}

/// Moves the active pane beside its group, on `side`.
fn split(host: Host, side: Side) {
    let w = host.workspace.peek();
    let Some(pane) = w.active.clone() else {
        return;
    };
    let Some(loc) = w.locate(&pane) else {
        return;
    };
    drop(w);
    host.run(Op::Move(
        pane,
        Dest::Edge {
            group: loc.group,
            side,
        },
    ));
}

/// Moves the focus to the group beside the active one, in the center.
fn focus(host: Host, direction: Direction) {
    let w = host.workspace.peek();
    let Some(from) = w.active_group() else {
        return;
    };
    let Some(center) = w.windows.first().and_then(|win| win.center.clone()) else {
        return;
    };
    let placed = area_rects(&center, Rect::new(0.0, 0.0, 1000.0, 1000.0));
    let Some(to) = neighbour(&placed, from, direction) else {
        return;
    };
    let pane = w.group(to).and_then(|g| g.current_tab().cloned());
    drop(w);
    if let Some(pane) = pane {
        host.run(Op::Show(pane));
    }
}

/// Moves the active pane into the center's first group.
fn to_center(host: Host) {
    let w = host.workspace.peek();
    let Some(pane) = w.active.clone() else {
        return;
    };
    let Some(group) = w
        .windows
        .first()
        .and_then(|win| win.center.as_ref())
        .and_then(crate::model::Node::first_group)
        .map(|g| g.id)
    else {
        return;
    };
    drop(w);
    host.run(Op::Move(pane, Dest::Group { group, index: None }));
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
