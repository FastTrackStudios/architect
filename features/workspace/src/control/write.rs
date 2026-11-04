//! The writes: one function per action, each over a [`State`], each
//! answering an [`Answer`]. An `Err` names what the call got wrong (an
//! unknown pane, two targets at once); the caller keeps the state it had
//! before the call ([`transact`]).

// Every action takes its arguments by value: one signature for all
// ([`Write`]).
#![allow(clippy::needless_pass_by_value)]

use super::{
    Acted, Answer, At, CloseArgs, DeleteArgs, DockBackArgs, DockPopOutArgs, DockSetArgs, Env,
    FLOAT_AT, FloatArgs, Focus, FocusArgs, Go, GroupSetArgs, LoadArgs, MoveArgs, NavigateArgs,
    OpenArgs, OptionsArgs, PaneArgs, Reason, RenameArgs, ReopenArgs, ResizeArgs, RestoreArgs,
    SaveArgs, SlotArgs, SnapTo, SplitArgs, State, TabSetArgs, To, WindowArgs, WindowSetArgs,
    ZoomArgs, answer, area_root, finish, group_id, neighbour_group, pane_id, window_id,
};
use crate::layout::Direction;
use crate::model::{
    Above, Area, Axis, GroupId, LinkId, Node, Pane, PaneId, Pin, Side, Snap, Split, WindowId,
    Workspace,
};
use crate::ops::Dest;

/// What a write is: a function over the state.
pub type Write<A> = fn(&mut State, &Env, A) -> Result<Answer, String>;

/// Runs `write` so a failure changes nothing: the state is put back as
/// it was when it says no. A call planned at another revision than the
/// layout's (`if_revision`) does not run: it answers `stale`.
///
/// # Errors
///
/// What `write` said.
pub fn transact<A: super::Guarded>(
    st: &mut State,
    env: &Env,
    write: Write<A>,
    args: A,
) -> Result<Answer, String> {
    if args.if_revision().is_some_and(|r| r != st.revision) {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::Stale),
            Acted::default(),
        ));
    }
    let saved = st.clone();
    let out = write(st, env, args);
    if out.is_err() {
        *st = saved;
    }
    out
}

fn err(e: crate::WorkspaceError) -> String {
    e.to_string()
}

/// The pane's location, or why not.
fn locate(w: &Workspace, pane: &PaneId) -> Result<crate::model::Location, String> {
    w.locate(pane)
        .ok_or_else(|| format!("{:?} is in no group", pane.0))
}

/// `WORKSPACE_OPEN`.
///
/// # Errors
///
/// An unknown view, a single view given another id, or a target that is
/// not there.
pub fn open(st: &mut State, env: &Env, a: OpenArgs) -> Result<Answer, String> {
    let before = st.clone();
    let spec = env
        .view(&a.view)
        .ok_or_else(|| format!("no view {:?} (WORKSPACE_VIEWS lists them)", a.view))?
        .clone();
    let w = &mut st.workspace;
    let id = match (spec.single, a.id) {
        (true, Some(id)) if id != spec.kind => {
            return Err(format!(
                "{:?} is a single view: its pane's id is {:?}",
                spec.kind, spec.kind
            ));
        }
        (true, _) => PaneId::new(spec.kind.clone()),
        (false, Some(id)) => PaneId::new(id),
        (false, None) => PaneId::new(format!("{}-{}", spec.kind, w.alloc())),
    };
    if w.pane(&id).is_some() {
        let focus = Focus::of(a.focus, &id);
        return Ok(finish(
            st,
            env,
            &before,
            focus,
            Acted::pane(&id),
            Reason::AlreadyOpen,
        ));
    }
    let mut pane = Pane::new(
        id.0.clone(),
        spec.kind.clone(),
        a.title.unwrap_or(spec.title),
    );
    pane.state = a.state.unwrap_or_default();
    let dest = match a.at {
        At::Replace { pane: old } => {
            let old = pane_id(w, &old)?;
            let l = locate(w, &old)?;
            let pinned = w.pane(&old).is_some_and(|p| p.pinned);
            w.panes.push(pane);
            let g = w
                .group_mut(l.group)
                .ok_or_else(|| format!("no group {}", l.group.0))?;
            g.tabs
                .insert(l.index.saturating_add(1).min(g.tabs.len()), id.clone());
            g.current = l
                .index
                .saturating_add(1)
                .min(g.tabs.len().saturating_sub(1));
            if !pinned {
                w.close(&old).map_err(err)?;
            }
            w.repair();
            return Ok(finish(
                st,
                env,
                &before,
                Focus::of(a.focus, &id),
                Acted::pane(&id),
                Reason::Unchanged,
            ));
        }
        At::Beside { pane: near } => {
            let near = pane_id(w, &near)?;
            let l = locate(w, &near)?;
            Dest::Group {
                group: l.group,
                index: Some(l.index.saturating_add(1)),
            }
        }
        At::Group { group, index } => Dest::Group {
            group: group_id(w, group)?,
            index,
        },
        At::Split { pane: near, side } => {
            let near = pane_id(w, &near)?;
            Dest::Edge {
                group: locate(w, &near)?.group,
                side: side.into(),
            }
        }
        At::Window { bounds } => Dest::Window(bounds),
        At::Dock { side } => Dest::Dock(side.into()),
        At::Float { rect } => Dest::Float(rect.unwrap_or(FLOAT_AT)),
    };
    w.panes.push(pane);
    w.place(id.clone(), dest).map_err(err)?;
    w.repair();
    Ok(finish(
        st,
        env,
        &before,
        Focus::of(a.focus, &id),
        Acted::pane(&id),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_MOVE`.
///
/// # Errors
///
/// A pane or destination that is not there.
pub fn move_pane(st: &mut State, env: &Env, a: MoveArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    let here = locate(w, &pane)?;
    let only = w.group(here.group).is_some_and(|g| g.tabs.len() == 1);
    let dest = match a.to {
        To::Group { group, index } => Dest::Group {
            group: group_id(w, group)?,
            index,
        },
        To::Beside { pane: near } => {
            let near = pane_id(w, &near)?;
            if near == pane {
                return Ok(answer(
                    st,
                    env,
                    false,
                    Some(Reason::Unchanged),
                    Acted::pane(&pane),
                ));
            }
            let l = locate(w, &near)?;
            Dest::Group {
                group: l.group,
                index: Some(l.index.saturating_add(1)),
            }
        }
        To::Edge { group, side } => Dest::Edge {
            group: group_id(w, group)?,
            side: side.into(),
        },
        To::Split { pane: near, side } => {
            let near = pane_id(w, &near)?;
            Dest::Edge {
                group: locate(w, &near)?.group,
                side: side.into(),
            }
        }
        To::Window { id: Some(id), .. } => {
            let window = window_id(w, &id)?;
            if window == WindowId::main() {
                w.to_main_window(&pane).map_err(err)?;
                return Ok(finish(
                    st,
                    env,
                    &before,
                    Focus::of(a.focus, &pane),
                    Acted::pane(&pane),
                    Reason::Unchanged,
                ));
            }
            let group = w
                .window(&window)
                .and_then(crate::model::Window::first_group)
                .map(|g| g.id)
                .ok_or_else(|| format!("window {id:?} has no group to move into"))?;
            Dest::Group { group, index: None }
        }
        To::Window { id: None, bounds } => Dest::Window(bounds),
        To::Dock { side } => Dest::Dock(side.into()),
        To::Float { rect } => Dest::Float(rect.unwrap_or(FLOAT_AT)),
    };
    // A lone tab split off its own group, or floated from its own float,
    // would leave the layout as it was under a new group id.
    let lone = match dest {
        Dest::Edge { group, .. } => only && group == here.group,
        Dest::Float(_) => only && matches!(here.area, Area::Float(_)),
        _ => false,
    };
    if lone {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::OnlyTab),
            Acted::pane(&pane),
        ));
    }
    w.move_pane(&pane, dest).map_err(err)?;
    Ok(finish(
        st,
        env,
        &before,
        Focus::of(a.focus, &pane),
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_SPLIT`.
///
/// # Errors
///
/// A pane that is not there.
pub fn split(st: &mut State, env: &Env, a: SplitArgs) -> Result<Answer, String> {
    move_pane(
        st,
        env,
        MoveArgs {
            if_revision: a.if_revision,
            to: super::To::Split {
                pane: a.pane.clone(),
                side: a.side,
            },
            pane: a.pane,
            focus: a.focus,
        },
    )
}

/// `WORKSPACE_FOCUS`.
///
/// # Errors
///
/// Neither a pane nor `from` and `direction`, or a pane that is not there.
pub fn focus(st: &mut State, env: &Env, a: FocusArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = match (a.pane, a.from, a.direction) {
        (Some(p), None, None) => pane_id(w, &p)?,
        (None, Some(from), Some(direction)) => {
            let from = pane_id(w, &from)?;
            let to = neighbour_group(w, &from, direction.into())?
                .and_then(|g| w.group(g))
                .and_then(|g| g.current_tab().cloned());
            match to {
                Some(p) => p,
                None => {
                    return Ok(answer(
                        st,
                        env,
                        false,
                        Some(Reason::NoNeighbor),
                        Acted::pane(&from),
                    ));
                }
            }
        }
        _ => return Err("give `pane`, or `from` and `direction`".to_owned()),
    };
    w.activate(&pane).map_err(err)?;
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        Acted::pane(&pane),
        Reason::AlreadyFocused,
    ))
}

/// `WORKSPACE_ZOOM`.
///
/// # Errors
///
/// A pane that is not there, or not in a window's center.
pub fn zoom(st: &mut State, env: &Env, a: ZoomArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    let l = locate(w, &pane)?;
    if l.area != Area::Center {
        return Err(format!(
            "{:?} is not in a window's center: only a center group zooms",
            a.pane
        ));
    }
    let window = w.windows.get(l.window).ok_or("no such window")?;
    let acted = Acted {
        pane: Some(pane),
        group: Some(l.group),
        ..Acted::default()
    };
    if !matches!(window.center, Some(Node::Split(_))) {
        return Ok(answer(st, env, false, Some(Reason::SingleGroup), acted));
    }
    let now = window.zoomed == Some(l.group);
    let on = a.mode.apply(now);
    let same = if on {
        Reason::AlreadyZoomed
    } else {
        Reason::AlreadyUnzoomed
    };
    w.set_zoomed(l.group, on).map_err(err)?;
    Ok(finish(st, env, &before, Focus::Keep, acted, same))
}

/// The child indices from `node` down to the group `group`.
fn path_to(node: &Node, group: GroupId) -> Option<Vec<usize>> {
    match node {
        Node::Group(g) => (g.id == group).then(Vec::new),
        Node::Split(s) => s.children.iter().enumerate().find_map(|(i, c)| {
            let mut rest = path_to(&c.node, group)?;
            rest.insert(0, i);
            Some(rest)
        }),
    }
}

/// The split reached by following `path` from `node`.
fn split_at<'a>(node: &'a Node, path: &[usize]) -> Option<&'a Split> {
    let Node::Split(s) = node else {
        return None;
    };
    match path.split_first() {
        None => Some(s),
        Some((&i, rest)) => split_at(&s.children.get(i)?.node, rest),
    }
}

/// `WORKSPACE_RESIZE`.
///
/// # Errors
///
/// None or more than one of its forms, or a target that is not there.
pub fn resize(st: &mut State, env: &Env, a: ResizeArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let mut acted = Acted::default();
    let by_path = match &a.path {
        Some(path) => {
            let window = a.window.as_deref().unwrap_or("main");
            Some(
                crate::tree::split_at_path(w, window, path)
                    .ok_or_else(|| format!("no split at {path:?} in {window:?}"))?
                    .0,
            )
        }
        None => None,
    };
    let split = match (a.split, by_path) {
        (Some(_), Some(_)) => return Err("give `split` or `path`, not both".to_owned()),
        (one, other) => one.or(other),
    };
    match (split, a.dock, a.pane) {
        (Some(split), None, None) => {
            let id = crate::model::SplitId(split);
            match (a.shares, a.even) {
                (Some(shares), false) => w.set_shares(id, &shares).map_err(err)?,
                (None, true) => w.even(id).map_err(err)?,
                _ => return Err("with `split`, give `shares` or `even: true`".to_owned()),
            }
        }
        (None, Some(dock), None) => {
            let size = a.size.ok_or("with `dock`, give `size`")?;
            w.set_dock_size(dock.into(), size).map_err(err)?;
        }
        (None, None, Some(pane)) => {
            let pane = pane_id(w, &pane)?;
            let direction = a.direction.ok_or("with `pane`, give `direction`")?;
            acted = Acted::pane(&pane);
            let amount = a.amount.unwrap_or(0.1).clamp(0.0, 1.0);
            let Some((split, shares)) = grown(w, &pane, direction.into(), amount)? else {
                return Ok(answer(st, env, false, Some(Reason::NoNeighbor), acted));
            };
            w.set_shares(split, &shares).map_err(err)?;
        }
        _ => {
            return Err(
                "give one of: `split` or `path` (with `shares` or `even`), `dock` (with `size`), `pane` (with `direction`)"
                    .to_owned(),
            );
        }
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// The nearest split around `pane`'s group along `direction`'s axis with a
/// child beyond it that way, and its shares with the pane's side grown by
/// `amount` (the neighbour keeping a little).
fn grown(
    w: &Workspace,
    pane: &PaneId,
    direction: Direction,
    amount: f32,
) -> Result<Option<(crate::model::SplitId, Vec<f32>)>, String> {
    let l = locate(w, pane)?;
    let Some((_, _, root)) = area_root(w, pane) else {
        return Ok(None);
    };
    let Some(path) = path_to(root, l.group) else {
        return Ok(None);
    };
    let (axis, forward) = match direction {
        Direction::Left => (Axis::Row, false),
        Direction::Right => (Axis::Row, true),
        Direction::Up => (Axis::Column, false),
        Direction::Down => (Axis::Column, true),
    };
    for depth in (0..path.len()).rev() {
        let Some(split) = split_at(root, path.get(..depth).unwrap_or_default()) else {
            continue;
        };
        let Some(&i) = path.get(depth) else {
            continue;
        };
        let j = if forward {
            i.checked_add(1)
        } else {
            i.checked_sub(1)
        };
        let Some(j) = j.filter(|&j| split.axis == axis && j < split.children.len()) else {
            continue;
        };
        let mut shares: Vec<f32> = split.children.iter().map(|c| c.share).collect();
        let theirs = shares.get(j).copied().unwrap_or(0.0);
        let take = amount.min((theirs - 0.05).max(0.0));
        if let Some(s) = shares.get_mut(j) {
            *s -= take;
        }
        if let Some(s) = shares.get_mut(i) {
            *s += take;
        }
        return Ok(Some((split.id, shares)));
    }
    Ok(None)
}

/// `WORKSPACE_GROUP_SET`.
///
/// # Errors
///
/// A group that is not there, or a `current` that is not one of its tabs.
pub fn group_set(st: &mut State, env: &Env, a: GroupSetArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let group = group_id(w, a.group)?;
    if let Some(stacked) = a.stacked {
        w.set_stacked(group, stacked).map_err(err)?;
    }
    if let Some(chrome) = a.chrome {
        w.set_chrome(group, chrome.into()).map_err(err)?;
    }
    if let Some(current) = a.current {
        let pane = PaneId::new(current.clone());
        let g = w.group_mut(group).ok_or("no such group")?;
        let at = g
            .tabs
            .iter()
            .position(|t| *t == pane)
            .ok_or_else(|| format!("{current:?} is not a tab of group {}", group.0))?;
        g.current = at;
    }
    let acted = Acted {
        group: Some(group),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_TAB_SET`.
///
/// # Errors
///
/// A pane that is not there.
pub fn tab_set(st: &mut State, env: &Env, a: TabSetArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    if let Some(pinned) = a.pinned {
        w.set_pinned(&pane, pinned).map_err(err)?;
    }
    if let Some(link) = a.link {
        let link = (!link.is_empty()).then_some(LinkId(link));
        w.set_link(&pane, link).map_err(err)?;
    }
    if let Some(title) = a.title {
        if let Some(p) = w.pane_mut(&pane) {
            p.title = title;
        }
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_NAVIGATE`.
///
/// # Errors
///
/// A pane that is not there, or neither `go` nor `state` (or both).
pub fn navigate(st: &mut State, env: &Env, a: NavigateArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    match (a.go, a.state) {
        (Some(go), None) => match w.go(&pane, go == Go::Forward) {
            Ok(()) => {}
            Err(crate::WorkspaceError::NoHistory) => {
                return Ok(answer(
                    st,
                    env,
                    false,
                    Some(Reason::NoHistory),
                    Acted::pane(&pane),
                ));
            }
            Err(e) => return Err(err(e)),
        },
        (None, Some(state)) => {
            let same = w.pane(&pane).is_some_and(|p| p.state == state);
            if !same {
                w.navigate(&pane, state).map_err(err)?;
            }
        }
        _ => return Err("give `go` or `state`".to_owned()),
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_DOCK_SET`.
///
/// # Errors
///
/// Neither `shown` nor `size`.
pub fn dock_set(st: &mut State, env: &Env, a: DockSetArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    if a.shown.is_none() && a.size.is_none() {
        return Err("give `shown`, `size` or both".to_owned());
    }
    let side: Side = a.side.into();
    if let Some(shown) = a.shown {
        w.set_dock_shown(side, shown.into()).map_err(err)?;
    }
    if let Some(size) = a.size {
        w.set_dock_size(side, size).map_err(err)?;
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        Acted::default(),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_DOCK_POP_OUT`.
///
/// # Errors
///
/// None the caller can mend: a dock that cannot pop out answers why.
pub fn dock_pop_out(st: &mut State, env: &Env, a: DockPopOutArgs) -> Result<Answer, String> {
    let before = st.clone();
    let side: Side = a.side.into();
    let w = &mut st.workspace;
    let out = w
        .dock_window(side)
        .and_then(|i| w.windows.get(i))
        .map(|win| win.id.clone());
    let acted = Acted {
        window: out.clone(),
        ..Acted::default()
    };
    if out.is_some() {
        return Ok(answer(st, env, false, Some(Reason::AlreadyOut), acted));
    }
    if !env.windows {
        return Ok(answer(st, env, false, Some(Reason::NoWindows), acted));
    }
    if w.dock(side).is_none_or(|d| d.root.is_none()) {
        return Ok(answer(st, env, false, Some(Reason::DockEmpty), acted));
    }
    let bounds = a.bounds.or_else(|| env.dock_bounds(side));
    let id = w.pop_out_dock(side, bounds).map_err(err)?;
    let acted = Acted {
        window: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_DOCK_BACK`.
///
/// # Errors
///
/// None the caller can mend.
pub fn dock_back(st: &mut State, env: &Env, a: DockBackArgs) -> Result<Answer, String> {
    let before = st.clone();
    let side: Side = a.side.into();
    let w = &mut st.workspace;
    if w.dock_window(side).is_none() {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::DockNotOut),
            Acted::default(),
        ));
    }
    w.dock_back(side).map_err(err)?;
    let acted = Acted {
        window: Some(WindowId::main()),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_FLOAT`.
///
/// # Errors
///
/// A pane that is not there.
pub fn float(st: &mut State, env: &Env, a: FloatArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    let floating = matches!(locate(w, &pane)?.area, Area::Float(_));
    if floating {
        if let Some(rect) = a.rect {
            w.set_float_rect(&pane, rect).map_err(err)?;
        }
    } else {
        w.move_pane(&pane, Dest::Float(a.rect.unwrap_or(FLOAT_AT)))
            .map_err(err)?;
    }
    if let Some(snap) = a.snap {
        let width = a.panel_width.unwrap_or(360.0);
        let snap = match snap {
            SnapTo::None => None,
            SnapTo::Maximised => Some(Snap::Maximised),
            SnapTo::LeftHalf => Some(Snap::Half(Side::Left)),
            SnapTo::RightHalf => Some(Snap::Half(Side::Right)),
            SnapTo::LeftPanel => Some(Snap::EdgeDock {
                side: Side::Left,
                width,
            }),
            SnapTo::RightPanel => Some(Snap::EdgeDock {
                side: Side::Right,
                width,
            }),
        };
        w.snap(&pane, snap).map_err(err)?;
    }
    if let Some(pin) = a.pin {
        let above = if a.above_all {
            Above::All
        } else {
            Above::Window
        };
        let pin = pin.corner().map(|corner| Pin { corner, above });
        w.pin(&pane, pin).map_err(err)?;
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::of(a.focus, &pane),
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_MINIMISE`.
///
/// # Errors
///
/// A pane that is not there, or not floating.
pub fn minimise(st: &mut State, env: &Env, a: PaneArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    w.minimise(&pane).map_err(err)?;
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_RESTORE`.
///
/// # Errors
///
/// A pane that is not there, or not floating.
pub fn restore(st: &mut State, env: &Env, a: RestoreArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    let float = w.float_of(&pane).map_err(err)?;
    if !float.minimised && !a.focus {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::NotMinimised),
            Acted::pane(&pane),
        ));
    }
    float.minimised = false;
    Ok(finish(
        st,
        env,
        &before,
        Focus::of(a.focus, &pane),
        Acted::pane(&pane),
        Reason::NotMinimised,
    ))
}

/// `WORKSPACE_WINDOW_SET`.
///
/// # Errors
///
/// A window that is not there, or bounds for the main window.
pub fn window_set(st: &mut State, env: &Env, a: WindowSetArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let window = window_id(w, &a.window)?;
    if let Some(bounds) = a.bounds {
        if window == WindowId::main() {
            return Err("the platform places the main window: it takes no bounds".to_owned());
        }
        w.set_window_bounds(&window, bounds).map_err(err)?;
    }
    if let Some(maximized) = a.maximized {
        if let Some(win) = w.windows.iter_mut().find(|win| win.id == window) {
            win.maximized = maximized;
        }
    }
    let acted = Acted {
        window: Some(window),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_REOPEN`.
///
/// # Errors
///
/// None the caller can mend.
pub fn reopen(st: &mut State, env: &Env, a: ReopenArgs) -> Result<Answer, String> {
    let before = st.clone();
    let reopened = st.workspace.reopen().map_err(err)?;
    let Some(pane) = reopened else {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::NothingClosed),
            Acted::default(),
        ));
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::of(a.focus, &pane),
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_CLOSE`: every pane named, or none of them.
///
/// # Errors
///
/// A pane that is not there (then nothing is closed).
pub fn close(st: &mut State, env: &Env, a: CloseArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let panes = a
        .panes
        .iter()
        .map(|p| pane_id(w, p))
        .collect::<Result<Vec<_>, _>>()?;
    for pane in &panes {
        if w.pane(pane).is_some() {
            w.close(pane).map_err(err)?;
        }
    }
    let acted = panes.first().map(Acted::pane).unwrap_or_default();
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_CLOSE_OTHERS`: every other unpinned tab in the pane's group.
///
/// # Errors
///
/// A pane that is not there.
pub fn close_others(st: &mut State, env: &Env, a: PaneArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let pane = pane_id(w, &a.pane)?;
    w.close_others(&pane).map_err(err)?;
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        Acted::pane(&pane),
        Reason::Unchanged,
    ))
}

/// `WORKSPACE_CLOSE_WINDOW`: a pop-out closes, its tabs coming back to the
/// main window.
///
/// # Errors
///
/// A window that is not there, or the main window.
pub fn close_window(st: &mut State, env: &Env, a: WindowArgs) -> Result<Answer, String> {
    let before = st.clone();
    let w = &mut st.workspace;
    let window = window_id(w, &a.window)?;
    w.close_window(&window).map_err(err)?;
    let acted = Acted {
        window: Some(window),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::Keep,
        acted,
        Reason::Unchanged,
    ))
}

/// The workspace becomes `layout` (a screenset's), its ids still never
/// handed out twice.
fn show(w: &mut Workspace, mut layout: Workspace) {
    layout.next_id = layout.next_id.max(w.next_id);
    layout.aliases = std::mem::take(&mut w.aliases);
    *w = layout;
}

/// The set `id` or `name` names (either, not both).
fn set_id(st: &State, id: Option<&str>, name: Option<&str>) -> Result<String, String> {
    match (id, name) {
        (Some(id), None) => st
            .screensets
            .get(id)
            .map(|s| s.id.clone())
            .ok_or_else(|| format!("no workspace {id:?} (WORKSPACES_LIST lists them)")),
        (None, Some(name)) => st
            .screensets
            .named(name)
            .map(|s| s.id.clone())
            .ok_or_else(|| format!("no workspace named {name:?}")),
        _ => Err("give `id` or `name`".to_owned()),
    }
}

/// `WORKSPACES_LOAD`: a saved layout becomes the live one (for this class
/// of screen). With the option on, the live layout is saved into the
/// active set first.
///
/// # Errors
///
/// None or more than one of `id`, `name`, `slot`, or no such set.
pub fn load(st: &mut State, env: &Env, a: LoadArgs) -> Result<Answer, String> {
    let before = st.clone();
    let id = match (a.slot, a.id.as_deref(), a.name.as_deref()) {
        (Some(n), None, None) => st
            .screensets
            .in_slot(n)
            .map(|s| s.id.clone())
            .ok_or_else(|| format!("no workspace is in slot {n}"))?,
        (None, id, name) => set_id(st, id, name)?,
        _ => return Err("give one of `id`, `name`, `slot`".to_owned()),
    };
    if st.screensets.options.save_before_load && st.screensets.active.is_some() {
        let live = st.workspace.clone();
        st.screensets.set_working(st.form, live);
        let _ = st.screensets.save(st.form);
    }
    let layout = st
        .screensets
        .recall(&id, st.form)
        .ok_or_else(|| format!("workspace {id:?} has no layout"))?;
    show(&mut st.workspace, layout);
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_SAVE`: with no `name`, the live layout into the active
/// set; with one, over the set of that name, or as a new set (Obsidian's
/// save). The set saved becomes active.
///
/// # Errors
///
/// A blank name.
pub fn save(st: &mut State, env: &Env, a: SaveArgs) -> Result<Answer, String> {
    let before = st.clone();
    let live = st.workspace.clone();
    let id = match a.name {
        Some(name) if name.trim().is_empty() => {
            return Err("a workspace needs a name".to_owned());
        }
        Some(name) => {
            if let Some(id) = st.screensets.named(&name).map(|s| s.id.clone())
                && let Some(refused) = builtin_from_outside(st, env, &id)
            {
                return Ok(refused);
            }
            st.screensets.set_working(st.form, live);
            st.screensets.save_named(&name)
        }
        None => {
            let Some(id) = st.screensets.active.clone() else {
                return Ok(answer(
                    st,
                    env,
                    false,
                    Some(Reason::NoScreenset),
                    Acted::default(),
                ));
            };
            if let Some(refused) = builtin_from_outside(st, env, &id) {
                return Ok(refused);
            }
            st.screensets.set_working(st.form, live);
            let _ = st.screensets.save(st.form);
            id
        }
    };
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// A save over built-in set `id` by a caller from outside the app (an
/// agent, a script run as someone): refused, `builtin`. What the app
/// ships is the person's to change, not a caller's: it clones it first
/// (`WORKSPACES_CLONE`). `None` when the save may go ahead.
fn builtin_from_outside(st: &State, env: &Env, id: &str) -> Option<Answer> {
    let builtin = st.screensets.get(id).is_some_and(|s| s.builtin);
    (builtin && st.actor != super::APP).then(|| {
        let acted = Acted {
            screenset: Some(id.to_owned()),
            ..Acted::default()
        };
        answer(st, env, false, Some(Reason::Builtin), acted)
    })
}

/// `WORKSPACES_CLONE`: a copy of set `name` (by name or id).
///
/// Not built in and in no slot, named `as` (`<name> copy` unless given).
/// It becomes the active set, the live layout staying as it is, so what
/// is saved next goes to the copy (herdr's `layout.clone`).
///
/// # Errors
///
/// No such set, a blank name, or a name another set has.
pub fn clone_set(st: &mut State, env: &Env, a: super::CloneArgs) -> Result<Answer, String> {
    let before = st.clone();
    let from = st
        .screensets
        .named(&a.name)
        .or_else(|| st.screensets.get(&a.name))
        .cloned()
        .ok_or_else(|| {
            format!(
                "no workspace named {:?} (WORKSPACES_LIST lists them)",
                a.name
            )
        })?;
    let name = a
        .as_name
        .map_or_else(|| format!("{} copy", from.name), |n| n.trim().to_owned());
    if name.is_empty() {
        return Err("a workspace needs a name".to_owned());
    }
    if st.screensets.named(&name).is_some() {
        return Err(format!("another workspace is named {name:?}: give `as`"));
    }
    let id = st
        .screensets
        .clone_set(&from.id, &name)
        .ok_or("no such workspace")?;
    // The live layout stays as it is (what was being changed is kept),
    // now the copy's working one.
    st.screensets.active = Some(id.clone());
    let live = st.workspace.clone();
    st.screensets.set_working(st.form, live);
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_RESTORE_BUILTINS`, the escape hatch.
///
/// Every set the app ships back as it ships it ([`Env::shipped`]), a
/// changed one kept as a copy (`<name> (edited)`) first, and the first of
/// them (by slot) shown as the live layout.
///
/// # Errors
///
/// None the caller can mend.
pub fn restore_builtins(
    st: &mut State,
    env: &Env,
    _: super::RevisionArgs,
) -> Result<Answer, String> {
    let before = st.clone();
    if env.shipped.is_empty() {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::NoScreenset),
            Acted::default(),
        ));
    }
    let first = st.screensets.restore_builtins(&env.shipped);
    if let Some(id) = &first
        && let Some(layout) = st.screensets.recall(id, st.form)
    {
        show(&mut st.workspace, layout);
    }
    let acted = Acted {
        screenset: first,
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_RENAME`.
///
/// # Errors
///
/// No such set, a blank name, or one another set has.
pub fn rename(st: &mut State, env: &Env, a: RenameArgs) -> Result<Answer, String> {
    let before = st.clone();
    let id = set_id(st, Some(&a.id), None)?;
    let name = a.name.trim();
    if name.is_empty() {
        return Err("a workspace needs a name".to_owned());
    }
    if st.screensets.named(name).is_some_and(|s| s.id != id) {
        return Err(format!("another workspace is named {name:?}"));
    }
    st.screensets.rename(&id, name);
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_SET_SLOT`: a set into slot 1–9 (the set there gives it
/// up), or out of any.
///
/// # Errors
///
/// No such set, or a slot past 9.
pub fn set_slot(st: &mut State, env: &Env, a: SlotArgs) -> Result<Answer, String> {
    let before = st.clone();
    let id = set_id(st, Some(&a.id), None)?;
    if a.slot.is_some_and(|n| !(1..=9).contains(&n)) {
        return Err("a slot is 1 to 9".to_owned());
    }
    st.screensets.set_slot(&id, a.slot);
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_OPTIONS`.
///
/// # Errors
///
/// Never.
pub fn set_options(st: &mut State, env: &Env, a: OptionsArgs) -> Result<Answer, String> {
    let before = st.clone();
    let options = &mut st.screensets.options;
    if let Some(on) = a.save_before_load {
        options.save_before_load = on;
    }
    if let Some(on) = a.slot_keys {
        options.slot_keys = on;
    }
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        Acted::default(),
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_RESET`: back to the active screenset's saved layout.
///
/// # Errors
///
/// None the caller can mend.
pub fn reset(st: &mut State, env: &Env, _: super::RevisionArgs) -> Result<Answer, String> {
    let before = st.clone();
    let Some(id) = st.screensets.active.clone() else {
        return Ok(answer(
            st,
            env,
            false,
            Some(Reason::NoScreenset),
            Acted::default(),
        ));
    };
    if let Some(layout) = st.screensets.reset(st.form) {
        show(&mut st.workspace, layout);
    }
    let acted = Acted {
        screenset: Some(id),
        ..Acted::default()
    };
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}

/// `WORKSPACES_DELETE`: a set the person made.
///
/// # Errors
///
/// No such screenset.
pub fn delete(st: &mut State, env: &Env, a: DeleteArgs) -> Result<Answer, String> {
    let before = st.clone();
    let set = st
        .screensets
        .get(&a.id)
        .ok_or_else(|| format!("no workspace {:?}", a.id))?;
    let acted = Acted {
        screenset: Some(a.id.clone()),
        ..Acted::default()
    };
    if set.builtin {
        return Ok(answer(st, env, false, Some(Reason::Builtin), acted));
    }
    st.screensets.delete(&a.id);
    Ok(finish(
        st,
        env,
        &before,
        Focus::AsIs,
        acted,
        Reason::Unchanged,
    ))
}
