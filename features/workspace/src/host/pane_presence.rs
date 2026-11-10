//! Cursors that come with the pane (feature `presence`): every pane
//! publishes this person's pointer in its space and draws everyone
//! else's, with no code in its view.
//!
//! The app says who is here: it provides a [`PaneRoom`] over the presence
//! channel of what it has open ([`use_pane_room`]). A view that scrolls or
//! zooms declares its space ([`use_pane_space`]); one that only needs the
//! others to land on the same thing names its subject
//! ([`use_pane_subject`]). An extension in another runtime does the same
//! with the `CURSORS_*` actions ([`crate::pane_space::register`]).
//!
//! ```ignore
//! let room = use_pane_room(|| PaneRoom::new(here, roster, board, crdt::presence::clock_ms));
//! // …and every pane of a `WorkspaceView` under it has cursors.
//! ```

use std::rc::Rc;
use std::time::Duration;

use crdt::presence::{Here, Place, PresenceSink, Roster};
use dioxus::prelude::*;

use super::presence::PresenceMarksLayer;
use super::use_host;
use crate::PaneId;
use crate::pane_space::{self, PresenceBoard, Size, ViewSpace};
use crate::presence::{PresenceMarks, css_color};

/// A measured pane is trusted this long before it is measured again
/// while the pointer moves over it (a split dragged, a dock opened).
const MEASURE_EVERY_MS: f64 = 250.0;

/// The people in what the app has open, as the panes see them: the
/// roster to draw, and where this person's pointer and "looking at" go.
/// Provided as context by [`use_pane_room`].
#[derive(Clone)]
pub struct PaneRoom {
    roster: Signal<Roster>,
    board: PresenceBoard,
    send: Rc<dyn Fn(Option<Place>)>,
    look: Rc<dyn Fn(Option<String>)>,
    clock: Rc<dyn Fn() -> f64>,
}

impl PaneRoom {
    /// A room over `here` (this person on a presence channel) and
    /// `roster` (everyone else on it). `board` is what the `CURSORS_*`
    /// actions were registered over; `clock` the local clock in
    /// milliseconds (`crdt::presence::clock_ms`).
    pub fn new<S: PresenceSink + Clone + 'static>(
        here: Here<S>,
        roster: Signal<Roster>,
        board: PresenceBoard,
        clock: impl Fn() -> f64 + 'static,
    ) -> Self {
        let clock: Rc<dyn Fn() -> f64> = Rc::new(clock);
        let (pointing, at) = (here.clone(), Rc::clone(&clock));
        let send = Rc::new(move |place: Option<Place>| match place {
            Some(place) => pointing.pointer(place, at()),
            None => pointing.no_pointer(),
        });
        let at = Rc::clone(&clock);
        let look = Rc::new(move |space: Option<String>| {
            here.edit_state(at(), |state| state.looking_at = space);
        });
        Self {
            roster,
            board,
            send,
            look,
            clock,
        }
    }

    /// What the actions and the panes share.
    #[must_use]
    pub const fn board(&self) -> &PresenceBoard {
        &self.board
    }

    /// Everyone else. Reactive when read.
    #[must_use]
    pub const fn roster(&self) -> Signal<Roster> {
        self.roster
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }
}

/// Provide `make`'s room to every pane under the caller, and keep the
/// actions' side current: what `CURSORS_HERE` published is sent, and
/// `CURSORS_ROSTER` answers from the roster as it is.
pub fn use_pane_room(make: impl FnOnce() -> PaneRoom) -> PaneRoom {
    let room = use_context_provider(make);
    let driven = room.clone();
    use_future(move || {
        let room = driven.clone();
        async move {
            loop {
                futures_timer::Delay::new(Duration::from_millis(33)).await;
                for place in room.board.take_published() {
                    (room.send)(place);
                }
                room.board.saw(&room.roster.peek());
            }
        }
    });
    room
}

/// The calling view's pane has `space`: call it on every render with the
/// view's scroll and zoom (the same space again costs nothing). Outside a
/// room it does nothing, so a view never checks.
pub fn use_pane_space(space: ViewSpace) {
    let pane = super::use_pane().id;
    let Some(room) = try_use_context::<PaneRoom>() else {
        return;
    };
    room.board.declare(&pane.0, space);
    let board = room.board;
    use_drop(move || board.undeclare(&pane.0));
}

/// The calling view shows `subject` (a note's path, a setlist's id): two
/// people's panes of one view kind with one subject are one place.
/// Without it the pane's title is its subject.
pub fn use_pane_subject(subject: &str) {
    let pane = super::use_pane().id;
    let kind = use_host()
        .workspace
        .read()
        .pane(&pane)
        .map(|p| p.view.0.clone())
        .unwrap_or_default();
    use_pane_space(ViewSpace::fractions(pane_space::pane_space(&kind, subject)));
}

/// The space of pane `pane`: what its view declared, else its own box
/// named by its view kind and title.
fn space_of(room: &PaneRoom, pane: &PaneId, kind: &str, title: &str) -> ViewSpace {
    room.board
        .declared(&pane.0)
        .unwrap_or_else(|| ViewSpace::fractions(pane_space::pane_space(kind, title)))
}

#[derive(Clone, Copy, PartialEq)]
struct Measured {
    x: f64,
    y: f64,
    size: Size,
    at_ms: f64,
}

/// A pane slot's own measure of itself, and the pointer over it.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct PaneTrack {
    mounted: Signal<Option<Rc<MountedData>>>,
    rect: Signal<Option<Measured>>,
}

pub(super) fn use_pane_track() -> PaneTrack {
    PaneTrack {
        mounted: use_signal(|| None),
        rect: use_signal(|| None),
    }
}

impl PaneTrack {
    pub(super) fn mounted(self, data: Rc<MountedData>) {
        let mut mounted = self.mounted;
        mounted.set(Some(data));
        self.measure(0.0);
    }

    fn measure(self, now_ms: f64) {
        let Some(mounted) = self.mounted.peek().clone() else {
            return;
        };
        let mut rect = self.rect;
        spawn(async move {
            if let Ok(r) = mounted.get_client_rect().await {
                let measured = Measured {
                    x: r.origin.x,
                    y: r.origin.y,
                    size: Size {
                        w: r.size.width,
                        h: r.size.height,
                    },
                    at_ms: now_ms,
                };
                // Measured before its first layout a pane has no size:
                // that is not a measure, and the next tick asks again.
                if measured.size.w > 0.0 && measured.size.h > 0.0 && *rect.peek() != Some(measured)
                {
                    rect.set(Some(measured));
                }
            }
        });
    }

    fn size(self) -> Option<Size> {
        (self.rect)().map(|m| m.size)
    }

    /// The pointer moved to `(x, y)` (window points) over pane `pane`.
    pub(super) fn moved(self, pane: &PaneId, kind: &str, title: &str, x: f64, y: f64) {
        let Some(room) = try_consume_context::<PaneRoom>() else {
            return;
        };
        let now = room.now();
        let measured = *self.rect.peek();
        if measured.is_none_or(|m| now - m.at_ms > MEASURE_EVERY_MS) {
            self.measure(now);
        }
        let Some(m) = measured else {
            return;
        };
        let place = space_of(&room, pane, kind, title).to_place(x - m.x, y - m.y, m.size);
        (room.send)(place);
    }
}

/// The pointer left a pane.
pub(super) fn pointer_left() {
    if let Some(room) = try_consume_context::<PaneRoom>() {
        (room.send)(None);
    }
}

/// The others, over pane `pane`: nothing outside a room.
#[component]
pub(super) fn PaneCursors(pane: PaneId, kind: String, title: String, track: PaneTrack) -> Element {
    if try_use_context::<PaneRoom>().is_none() {
        return rsx! {};
    }
    rsx! { Cursors { pane, kind, title, track } }
}

#[component]
fn Cursors(pane: PaneId, kind: String, title: String, track: PaneTrack) -> Element {
    let room = use_context::<PaneRoom>();
    let host = use_host();
    // What the loop below reads: a slot may be handed another pane.
    let mut shown = use_signal(|| (pane.clone(), kind.clone(), title.clone()));
    if *shown.peek() != (pane.clone(), kind.clone(), title.clone()) {
        shown.set((pane.clone(), kind.clone(), title.clone()));
    }
    let mut frame = use_signal(|| 0_u64);
    let ticking = room.clone();
    use_future(move || {
        let room = ticking.clone();
        async move {
            let mut spaces = room.board.spaces_revision();
            let mut ticks = 0_u32;
            loop {
                let now = room.now();
                let moving = room.roster.peek().moving(now);
                let revision = room.board.spaces_revision();
                if moving || revision != spaces {
                    spaces = revision;
                    frame.with_mut(|f| *f = f.wrapping_add(1));
                }
                // The pane this person is in is what they look at.
                let (pane, kind, title) = shown.peek().clone();
                if host.workspace.peek().active.as_ref() == Some(&pane) {
                    let space = space_of(&room, &pane, &kind, &title);
                    (room.look)(Some(space.name().to_owned()));
                }
                // Somebody is here: keep the pane's size current, so
                // their pointer is right after a split moved.
                ticks = ticks.wrapping_add(1);
                let unmeasured = track.rect.peek().is_none();
                if (unmeasured || ticks.is_multiple_of(20)) && !room.roster.peek().peers.is_empty()
                {
                    track.measure(now);
                }
                let wait = if moving { 16 } else { 50 };
                futures_timer::Delay::new(Duration::from_millis(wait)).await;
            }
        }
    });
    let _ = frame();
    let roster = room.roster.read();
    let space = space_of(&room, &pane, &kind, &title);
    let marks = track.size().map_or_else(PresenceMarks::default, |size| {
        pane_space::marks(&roster, &space, size, room.now())
    });
    rsx! { PresenceMarksLayer { marks } }
}

/// On a pane's tab: a dot for each person looking at that pane.
#[component]
pub(super) fn TabPeople(pane: PaneId) -> Element {
    if try_use_context::<PaneRoom>().is_none() {
        return rsx! {};
    }
    rsx! { People { pane } }
}

#[component]
fn People(pane: PaneId) -> Element {
    let room = use_context::<PaneRoom>();
    let host = use_host();
    let (kind, title) = host
        .workspace
        .read()
        .pane(&pane)
        .map(|p| (p.view.0.clone(), p.title.clone()))
        .unwrap_or_default();
    let space = space_of(&room, &pane, &kind, &title);
    let people = pane_space::looking_at(&room.roster.read(), space.name());
    if people.is_empty() {
        return rsx! {};
    }
    rsx! {
        span {
            "data-tab-people": "{pane.0}",
            style: "position:relative; flex:0 0 auto; display:flex; align-items:center; gap:2px;",
            for (peer, name, color) in people {
                span {
                    key: "{peer}",
                    title: "{name}",
                    "aria-label": "{name} is here",
                    style: "width:8px; height:8px; border-radius:4px; background:{css_color(color, 1.0)};",
                }
            }
        }
    }
}
