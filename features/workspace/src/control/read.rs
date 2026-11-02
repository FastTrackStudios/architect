//! The reads: what a caller looks at before it writes.

// Every action takes its arguments by value: one signature for all.
#![allow(clippy::needless_pass_by_value)]

use facet::Facet;

use super::{Env, NeighborArgs, PaneArgs, State, ViewSpec, neighbour_group, pane_id};
use crate::guide::Arrangement;
use crate::model::{Area, Rect, Side};
use crate::screenset::FormClass;

/// Where a pane is, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum PlaceName {
    Center,
    LeftDock,
    RightDock,
    BottomDock,
    Float,
}

/// `WORKSPACE_PANE_GET`: one pane, and where it is.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct PaneInfo {
    pub pane: String,
    pub view: String,
    pub title: String,
    /// The view's own state (its JSON, opaque here).
    pub state: String,
    pub pinned: bool,
    #[facet(default)]
    pub link: Option<String>,
    pub window: String,
    pub place: PlaceName,
    pub group: u64,
    /// Its index among its group's tabs.
    pub index: usize,
    /// Its group shows it.
    pub showing: bool,
    /// It is the active pane.
    pub active: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Its float's rectangle (window points), when it floats.
    #[facet(default)]
    pub float_rect: Option<Rect>,
    #[facet(default)]
    pub minimised: bool,
}

/// `WORKSPACE_NEIGHBOR`: the pane showing next to one, toward a direction.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct NeighborInfo {
    /// The neighbouring group's tab on show; unset when nothing lies that
    /// way (an edge, or a float).
    #[facet(default)]
    pub neighbor: Option<String>,
    #[facet(default)]
    pub group: Option<u64>,
}

/// `WORKSPACE_VIEWS`: what can be opened.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ViewsAnswer {
    pub views: Vec<ViewSpec>,
}

/// A class of screen, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "snake_case")]
pub enum ClassName {
    Desk,
    TabletLandscape,
    TabletPortrait,
    PhoneLandscape,
    PhonePortrait,
}

impl From<FormClass> for ClassName {
    fn from(c: FormClass) -> Self {
        match c {
            FormClass::Desk => Self::Desk,
            FormClass::TabletLandscape => Self::TabletLandscape,
            FormClass::TabletPortrait => Self::TabletPortrait,
            FormClass::PhoneLandscape => Self::PhoneLandscape,
            FormClass::PhonePortrait => Self::PhonePortrait,
        }
    }
}

/// One screenset, as `SCREENSET_LIST` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ScreensetInfo {
    pub id: String,
    pub name: String,
    #[facet(default)]
    pub slot: Option<u8>,
    /// Shipped by the app: changed and reset, never deleted.
    pub builtin: bool,
    /// The classes of screen it has a layout for.
    pub classes: Vec<ClassName>,
}

/// `SCREENSET_LIST`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ScreensetsAnswer {
    pub sets: Vec<ScreensetInfo>,
    #[facet(default)]
    pub active: Option<String>,
    /// The live layout differs from the active set's saved one.
    pub dirty: bool,
    /// The class of screen layouts are recalled for here.
    pub class: ClassName,
}

/// `WORKSPACE_LAYOUT`.
///
/// # Errors
///
/// Never.
pub fn layout(st: &State, _env: &Env, (): ()) -> Result<Arrangement, String> {
    Ok(Arrangement::of(&st.workspace))
}

/// `WORKSPACE_PANE_GET`.
///
/// # Errors
///
/// A pane that is not there.
pub fn pane_get(st: &State, _env: &Env, a: PaneArgs) -> Result<PaneInfo, String> {
    let w = &st.workspace;
    let id = pane_id(w, &a.pane)?;
    let pane = w.pane(&id).ok_or("no such pane")?;
    let at = w.locate(&id).ok_or("the pane is in no group")?;
    let window = w.windows.get(at.window).ok_or("no such window")?;
    let float = match at.area {
        Area::Float(index) => window.floats.get(index),
        _ => None,
    };
    let place = match at.area {
        Area::Center => PlaceName::Center,
        Area::Dock(Side::Left) => PlaceName::LeftDock,
        Area::Dock(Side::Right) => PlaceName::RightDock,
        Area::Dock(_) => PlaceName::BottomDock,
        Area::Float(_) => PlaceName::Float,
    };
    Ok(PaneInfo {
        pane: id.0.clone(),
        view: pane.view.0.clone(),
        title: pane.title.clone(),
        state: pane.state.clone(),
        pinned: pane.pinned,
        link: pane.link.as_ref().map(|l| l.0.clone()),
        window: window.id.0.clone(),
        place,
        group: at.group.0,
        index: at.index,
        showing: w.group(at.group).is_some_and(|g| g.current == at.index),
        active: w.active.as_ref() == Some(&id),
        can_go_back: !pane.history.back.is_empty(),
        can_go_forward: !pane.history.forward.is_empty(),
        float_rect: float.map(|f| f.rect),
        minimised: float.is_some_and(|f| f.minimised),
    })
}

/// `WORKSPACE_NEIGHBOR`.
///
/// # Errors
///
/// A pane that is not there.
pub fn neighbor(st: &State, _env: &Env, a: NeighborArgs) -> Result<NeighborInfo, String> {
    let w = &st.workspace;
    let id = pane_id(w, &a.pane)?;
    let group = neighbour_group(w, &id, a.direction.into())?;
    Ok(NeighborInfo {
        neighbor: group
            .and_then(|g| w.group(g))
            .and_then(|g| g.current_tab())
            .map(|p| p.0.clone()),
        group: group.map(|g| g.0),
    })
}

/// `WORKSPACE_VIEWS`.
///
/// # Errors
///
/// Never.
pub fn views(_st: &State, env: &Env, (): ()) -> Result<ViewsAnswer, String> {
    Ok(ViewsAnswer {
        views: env.views.clone(),
    })
}

/// `SCREENSET_LIST`.
///
/// # Errors
///
/// Never.
pub fn screensets(st: &State, _env: &Env, (): ()) -> Result<ScreensetsAnswer, String> {
    let s = &st.screensets;
    Ok(ScreensetsAnswer {
        sets: s
            .sets
            .iter()
            .map(|set| ScreensetInfo {
                id: set.id.clone(),
                name: set.name.clone(),
                slot: set.slot,
                builtin: set.builtin,
                classes: set.layouts.iter().map(|l| l.class.into()).collect(),
            })
            .collect(),
        active: s.active.clone(),
        dirty: s
            .active
            .as_deref()
            .and_then(|id| s.get(id))
            .and_then(|set| set.layout_for(st.form))
            .is_some_and(|saved| !saved.same_arrangement(&st.workspace)),
        class: st.form.into(),
    })
}
