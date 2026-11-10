//! Without the `presence` feature a pane slot tracks nothing and draws
//! nobody: the same calls as `pane_presence`, doing nothing.

use std::rc::Rc;

use dioxus::prelude::*;

use crate::PaneId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PaneTrack;

pub(super) const fn use_pane_track() -> PaneTrack {
    PaneTrack
}

// The same calls as the real one's, so the pane slot is written once.
#[expect(
    clippy::unused_self,
    reason = "the shape of `pane_presence::PaneTrack`"
)]
impl PaneTrack {
    pub(super) fn mounted(self, _data: Rc<MountedData>) {}
    pub(super) const fn moved(self, _pane: &PaneId, _kind: &str, _title: &str, _x: f64, _y: f64) {}
}

pub(super) const fn pointer_left() {}

#[component]
pub(super) fn PaneCursors(pane: PaneId, kind: String, title: String, track: PaneTrack) -> Element {
    let _ = (pane, kind, title, track);
    rsx! {}
}

#[component]
pub(super) fn TabPeople(pane: PaneId) -> Element {
    let _ = pane;
    rsx! {}
}
