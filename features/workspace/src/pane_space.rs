//! Cursors that come with the pane (feature `presence`).
//!
//! Every pane has a space the others' pointers are placed in. With no code
//! in its view, that is the pane's own box ([`ViewSpace::fractions`]): a
//! pointer two thirds across is drawn two thirds across for everyone,
//! whatever size their pane is. A view whose content scrolls or zooms
//! declares its own space ([`ViewSpace::mapped`]): each axis either a
//! number that grows with the pixels (a time, a bar) or a row of lanes by
//! key (tracks, rows), so a pointer stays on the same bar of the same track
//! for a person zoomed and scrolled differently.
//!
//! No Dioxus here: [`marks`] turns a [`Roster`] into the [`PresenceMarks`]
//! a pane draws, and [`PresenceBoard`] is what the Action List's
//! `PRESENCE_*` actions and the host share, so an extension in any runtime
//! (process, WASI, a page, Luau) declares a space, publishes a place and
//! reads who is here by running actions.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use crdt::presence::{Coord, Place, Roster};
use facet::Facet;

use crate::presence::{PointerMark, PresenceMarks, WashMark};

/// The default space of a pane: its view kind and what it shows.
///
/// `subject` is what makes two people's panes the same place (a note's
/// path, a setlist's id); a pane with none uses its title.
#[must_use]
pub fn pane_space(kind: &str, subject: &str) -> String {
    format!("pane:{kind}:{subject}")
}

/// A pane's content box in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// One axis of a declared space.
///
/// With no `keys` it is a number: `at_zero` is the value at the pane's
/// edge (the scroll position) and `per_px` how much it grows per pixel
/// (the zoom). With `keys` it is lanes: each key a lane `lane_px` tall
/// (or wide), the first starting at `first_px`; the place carries the
/// lane's key and how far into the lane the pointer is (`<coord>.f`).
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct Axis {
    /// The coordinate's name in the space (`time`, `track`).
    pub coord: String,
    #[facet(default)]
    pub at_zero: f64,
    #[facet(default)]
    pub per_px: f64,
    #[facet(default)]
    pub keys: Vec<String>,
    #[facet(default)]
    pub first_px: f64,
    #[facet(default)]
    pub lane_px: f64,
}

impl Axis {
    /// A numeric axis: `at_zero` at pixel 0, growing `per_px` a pixel.
    #[must_use]
    pub fn linear(coord: impl Into<String>, at_zero: f64, per_px: f64) -> Self {
        Self {
            coord: coord.into(),
            at_zero,
            per_px,
            ..Self::default()
        }
    }

    /// Lanes by key, the first at `first_px`, each `lane_px`.
    #[must_use]
    pub fn lanes(
        coord: impl Into<String>,
        keys: impl IntoIterator<Item = impl Into<String>>,
        first_px: f64,
        lane_px: f64,
    ) -> Self {
        Self {
            coord: coord.into(),
            keys: keys.into_iter().map(Into::into).collect(),
            first_px,
            lane_px,
            ..Self::default()
        }
    }

    fn fraction_name(&self) -> String {
        format!("{}.f", self.coord)
    }

    /// Pixel `px` as this axis's coordinates; `None` off its lanes or
    /// when the axis cannot be read (no zoom, no lane height).
    fn write(&self, px: f64, place: Place) -> Option<Place> {
        if self.keys.is_empty() {
            if self.per_px == 0.0 {
                return None;
            }
            return Some(place.with(self.coord.clone(), px.mul_add(self.per_px, self.at_zero)));
        }
        if self.lane_px <= 0.0 {
            return None;
        }
        let along = (px - self.first_px) / self.lane_px;
        if along < 0.0 {
            return None;
        }
        let key = self.keys.iter().nth(lane_index(along)?)?;
        Some(
            place
                .with(self.coord.clone(), key.as_str())
                .with(self.fraction_name(), along.fract()),
        )
    }

    /// Where `place` is on this axis, in pixels.
    fn read(&self, place: &Place) -> Option<f64> {
        if self.keys.is_empty() {
            if self.per_px == 0.0 {
                return None;
            }
            return Some((place.num(&self.coord)? - self.at_zero) / self.per_px);
        }
        let key = place.key(&self.coord)?;
        let index = self.keys.iter().position(|k| k == key)?;
        let into = place.num(&self.fraction_name()).unwrap_or(0.5);
        let lanes = f64::from(u32::try_from(index).ok()?);
        Some((lanes + into).mul_add(self.lane_px, self.first_px))
    }
}

/// The lane a distance (in lanes) falls in.
fn lane_index(along: f64) -> Option<usize> {
    // A pane is never millions of lanes tall; past that it is not a lane.
    if !(0.0..1_000_000.0).contains(&along) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be in 0..1e6 just above"
    )]
    Some(along.floor() as usize)
}

/// How a pane's pixels and a space's coordinates meet.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewSpace {
    /// The pane's own box: `x` and `y` as fractions of it.
    Fractions { space: String },
    /// A space the view declared, one [`Axis`] across and one down.
    Mapped { space: String, x: Axis, y: Axis },
}

impl ViewSpace {
    #[must_use]
    pub fn fractions(space: impl Into<String>) -> Self {
        Self::Fractions {
            space: space.into(),
        }
    }

    #[must_use]
    pub fn mapped(space: impl Into<String>, x: Axis, y: Axis) -> Self {
        Self::Mapped {
            space: space.into(),
            x,
            y,
        }
    }

    /// The space's name: what a [`Place`] in it carries.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Fractions { space } | Self::Mapped { space, .. } => space,
        }
    }

    /// The pixel `(x, y)` of a pane `size` big, as a place in this space.
    /// `None` outside what the space covers (off the lanes, an empty pane).
    #[must_use]
    pub fn to_place(&self, x: f64, y: f64, size: Size) -> Option<Place> {
        match self {
            Self::Fractions { space } => {
                if size.w <= 0.0 || size.h <= 0.0 {
                    return None;
                }
                Some(
                    Place::new(space.clone())
                        .with("x", x / size.w)
                        .with("y", y / size.h),
                )
            }
            Self::Mapped { space, x: ax, y: ay } => {
                let place = ax.write(x, Place::new(space.clone()))?;
                ay.write(y, place)
            }
        }
    }

    /// Where `place` is in a pane `size` big; `None` for another space's
    /// place, or one this pane does not show (a track it has not got).
    #[must_use]
    pub fn from_place(&self, place: &Place, size: Size) -> Option<(f64, f64)> {
        if place.space != self.name() {
            return None;
        }
        match self {
            Self::Fractions { .. } => Some((place.num("x")? * size.w, place.num("y")? * size.h)),
            Self::Mapped { x, y, .. } => Some((x.read(place)?, y.read(place)?)),
        }
    }
}

/// What a pane draws of the others at `local_ms`: every pointer whose
/// place is in `space` and inside the pane, and every selection in it
/// that names a range (a wash from one corner to the other).
#[must_use]
pub fn marks(roster: &Roster, space: &ViewSpace, size: Size, local_ms: f64) -> PresenceMarks {
    let inside = |(x, y): (f64, f64)| {
        ((0.0..=size.w).contains(&x) && (0.0..=size.h).contains(&y)).then_some((x, y))
    };
    let pointers = roster
        .pointers(local_ms, |place| {
            space.from_place(place, size).and_then(inside)
        })
        .into_iter()
        .map(|p| PointerMark {
            peer: p.peer,
            name: p.name,
            color: p.color,
            x: p.x,
            y: p.y,
        })
        .collect();
    let washes = roster
        .selections_in(space.name())
        .filter_map(|(peer, state, selection)| {
            let (from, to) = selection.range.as_ref()?;
            let corner = |at: &BTreeMap<String, Coord>| {
                space.from_place(
                    &Place {
                        space: space.name().to_owned(),
                        at: at.clone(),
                    },
                    size,
                )
            };
            let (ax, ay) = corner(from)?;
            let (bx, by) = corner(to)?;
            Some(WashMark {
                peer: peer.to_owned(),
                color: state.color,
                x: ax.min(bx),
                y: ay.min(by),
                w: (ax - bx).abs(),
                h: (ay - by).abs(),
            })
        })
        .collect();
    PresenceMarks {
        pointers,
        washes,
        playheads: Vec::new(),
    }
}

/// The people whose `looking_at` is `space`: `(id, name, colour)`, in id
/// order. What a pane's tab shows.
#[must_use]
pub fn looking_at(roster: &Roster, space: &str) -> Vec<(String, String, u32)> {
    roster
        .here()
        .filter(|(_, state)| state.looking_at.as_deref() == Some(space))
        .map(|(id, state)| (id.to_owned(), state.name.clone(), state.color))
        .collect()
}

// ── the board: what the actions and the host share ──────────────────────

/// One named coordinate, as an action carries it: a number or a key.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct CoordArg {
    pub name: String,
    #[facet(default)]
    pub num: Option<f64>,
    #[facet(default)]
    pub key: Option<String>,
}

/// A place as an action carries it.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct PlaceArg {
    pub space: String,
    #[facet(default)]
    pub at: Vec<CoordArg>,
}

impl From<&Place> for PlaceArg {
    fn from(place: &Place) -> Self {
        Self {
            space: place.space.clone(),
            at: place
                .at
                .iter()
                .map(|(name, coord)| match coord {
                    Coord::Num(v) => CoordArg {
                        name: name.clone(),
                        num: Some(*v),
                        key: None,
                    },
                    Coord::Key(k) => CoordArg {
                        name: name.clone(),
                        num: None,
                        key: Some(k.clone()),
                    },
                })
                .collect(),
        }
    }
}

impl From<&PlaceArg> for Place {
    fn from(arg: &PlaceArg) -> Self {
        let mut place = Self::new(arg.space.clone());
        for coord in &arg.at {
            if let Some(key) = &coord.key {
                place = place.with(coord.name.clone(), key.as_str());
            } else if let Some(num) = coord.num {
                place = place.with(coord.name.clone(), num);
            }
        }
        place
    }
}

/// One person, as `PRESENCE_ROSTER` tells them.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct PersonHere {
    pub peer: String,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    pub looking_at: Option<String>,
    /// Where their pointer is, if it is anywhere.
    pub pointer: Option<PlaceArg>,
}

/// Everyone else, as `PRESENCE_ROSTER` answers.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct RosterAnswer {
    /// Goes up whenever the people or their places change: ask again
    /// when it differs.
    pub revision: u64,
    pub people: Vec<PersonHere>,
}

/// The people of `roster` as an action tells them, optionally only those
/// looking at or pointing in `space`.
#[must_use]
pub fn people(roster: &Roster, space: Option<&str>) -> Vec<PersonHere> {
    roster
        .peers
        .iter()
        .filter_map(|(id, peer)| {
            let state = peer.state.as_ref()?;
            let pointer = peer.trail.latest();
            let shown = space.is_none_or(|s| {
                state.looking_at.as_deref() == Some(s) || pointer.is_some_and(|p| p.space == s)
            });
            shown.then(|| PersonHere {
                peer: id.clone(),
                name: state.name.clone(),
                color: format!("#{:06x}", state.color & 0x00ff_ffff),
                looking_at: state.looking_at.clone(),
                pointer: pointer.map(PlaceArg::from),
            })
        })
        .collect()
}

#[derive(Default)]
struct Board {
    /// The spaces views declared, by pane id.
    spaces: BTreeMap<String, ViewSpace>,
    /// Moves on whenever `spaces` changes.
    spaces_revision: u64,
    /// The last roster the host saw.
    people: Vec<PersonHere>,
    people_revision: u64,
    /// Places an action published, waiting for the host to send them:
    /// `None` withdraws the pointer.
    outbox: Vec<Option<Place>>,
}

/// What the `PRESENCE_*` actions and the host that draws the panes share:
/// the spaces views declared (by action or by hook), the roster as last
/// seen, and the places an action asked to publish. Cloning shares it.
#[derive(Clone, Default)]
pub struct PresenceBoard(Arc<Mutex<Board>>);

impl PresenceBoard {
    fn lock(&self) -> std::sync::MutexGuard<'_, Board> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Pane `pane`'s view declared `space` (again: it replaces the last).
    pub fn declare(&self, pane: &str, space: ViewSpace) {
        let mut board = self.lock();
        if board.spaces.get(pane) != Some(&space) {
            board.spaces.insert(pane.to_owned(), space);
            board.spaces_revision += 1;
        }
    }

    /// Pane `pane` is back to its default space.
    pub fn undeclare(&self, pane: &str) {
        let mut board = self.lock();
        if board.spaces.remove(pane).is_some() {
            board.spaces_revision += 1;
        }
    }

    /// The space pane `pane`'s view declared, if it did.
    #[must_use]
    pub fn declared(&self, pane: &str) -> Option<ViewSpace> {
        self.lock().spaces.get(pane).cloned()
    }

    /// Moves on whenever a declared space changes.
    #[must_use]
    pub fn spaces_revision(&self) -> u64 {
        self.lock().spaces_revision
    }

    /// The host saw `roster`: what `PRESENCE_ROSTER` answers from now.
    pub fn saw(&self, roster: &Roster) {
        let people = people(roster, None);
        let mut board = self.lock();
        if board.people != people {
            board.people = people;
            board.people_revision += 1;
        }
    }

    /// Everyone else as last seen, optionally those in `space` only.
    #[must_use]
    pub fn roster(&self, space: Option<&str>) -> RosterAnswer {
        let board = self.lock();
        RosterAnswer {
            revision: board.people_revision,
            people: board
                .people
                .iter()
                .filter(|p| {
                    space.is_none_or(|s| {
                        p.looking_at.as_deref() == Some(s)
                            || p.pointer.as_ref().is_some_and(|at| at.space == s)
                    })
                })
                .cloned()
                .collect(),
        }
    }

    /// An action publishes this person's pointer at `place` (`None`: no
    /// pointer). The host sends it on its next tick.
    pub fn publish(&self, place: Option<Place>) {
        self.lock().outbox.push(place);
    }

    /// What actions published since the last call, oldest first.
    #[must_use]
    pub fn take_published(&self) -> Vec<Option<Place>> {
        std::mem::take(&mut self.lock().outbox)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use crdt::presence::{Here, PeerState, PresenceSink, Selection};
    use loro::LoroValue;

    use super::*;

    /// A presence channel two people share: a map.
    #[derive(Clone, Default)]
    struct Channel(Rc<RefCell<HashMap<String, LoroValue>>>);

    impl PresenceSink for Channel {
        fn set(&self, key: &str, value: LoroValue) {
            self.0.borrow_mut().insert(key.to_owned(), value);
        }
        fn delete(&self, key: &str) {
            self.0.borrow_mut().remove(key);
        }
    }

    fn roster_of(channel: &Channel, me: &str, now: f64) -> Roster {
        let mut roster = Roster::default();
        roster.sync(me, &channel.0.borrow(), now);
        roster
    }

    const BIG: Size = Size { w: 1000.0, h: 600.0 };
    const SMALL: Size = Size { w: 500.0, h: 300.0 };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_pointer_in_a_plain_pane_lands_at_the_same_fraction_of_a_smaller_one() {
        let channel = Channel::default();
        let space = ViewSpace::fractions(pane_space("vault.note", "Songs/Night Drive.md"));
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        // Ann's view has no presence code: the pane publishes for it.
        ann.pointer(space.to_place(750.0, 150.0, BIG).unwrap(), 0.0);

        let roster = roster_of(&channel, "bob", 0.0);
        let marks = marks(&roster, &space, SMALL, 1000.0);
        assert_eq!(marks.pointers.len(), 1);
        let p = &marks.pointers[0];
        assert_eq!(p.name, "Ann");
        assert!(close(p.x, 375.0) && close(p.y, 75.0), "{p:?}");
    }

    #[test]
    fn a_pointer_in_another_pane_is_not_drawn_here() {
        let channel = Channel::default();
        let theirs = ViewSpace::fractions(pane_space("vault.note", "A.md"));
        let mine = ViewSpace::fractions(pane_space("vault.note", "B.md"));
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.pointer(theirs.to_place(10.0, 10.0, BIG).unwrap(), 0.0);
        let roster = roster_of(&channel, "bob", 0.0);
        assert!(marks(&roster, &mine, BIG, 1000.0).is_empty());
        assert!(!marks(&roster, &theirs, BIG, 1000.0).is_empty());
    }

    #[test]
    fn your_own_pointer_is_not_drawn_back_to_you() {
        let channel = Channel::default();
        let space = ViewSpace::fractions("pane:x:y");
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.pointer(space.to_place(10.0, 10.0, BIG).unwrap(), 0.0);
        let roster = roster_of(&channel, "ann", 0.0);
        assert!(marks(&roster, &space, BIG, 1000.0).is_empty());
    }

    /// An arrangement: time across, tracks down.
    fn arrange(scroll_s: f64, s_per_px: f64, first_px: f64, lane_px: f64) -> ViewSpace {
        ViewSpace::mapped(
            "session.arrange",
            Axis::linear("time", scroll_s, s_per_px),
            Axis::lanes("track", ["Kick", "Snare", "Bass"], first_px, lane_px),
        )
    }

    #[test]
    fn a_declared_space_keeps_a_pointer_on_the_same_bar_and_track_at_another_zoom() {
        let channel = Channel::default();
        // Ann: scrolled to 0 s, 10 px a second, 40 px lanes.
        let anns = arrange(0.0, 0.1, 0.0, 40.0);
        // Bob: scrolled to 8 s, 50 px a second, 100 px lanes under a 20 px ruler.
        let bobs = arrange(8.0, 0.02, 20.0, 100.0);
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        // 12.5 s, a quarter of the way down Snare.
        let place = anns.to_place(125.0, 50.0, BIG).unwrap();
        assert!(close(place.num("time").unwrap(), 12.5));
        assert_eq!(place.key("track"), Some("Snare"));
        ann.pointer(place, 0.0);

        let roster = roster_of(&channel, "bob", 0.0);
        let marks = marks(&roster, &bobs, BIG, 1000.0);
        let p = &marks.pointers[0];
        // (12.5 - 8) s * 50 px; ruler + one lane + a quarter of a lane.
        assert!(close(p.x, 225.0) && close(p.y, 145.0), "{p:?}");
    }

    #[test]
    fn a_pointer_scrolled_out_of_view_or_on_a_missing_track_is_not_drawn() {
        let channel = Channel::default();
        let anns = arrange(0.0, 0.1, 0.0, 40.0);
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.pointer(anns.to_place(125.0, 50.0, BIG).unwrap(), 0.0);
        let roster = roster_of(&channel, "bob", 0.0);
        // Bob is scrolled past it.
        let far = arrange(100.0, 0.1, 0.0, 40.0);
        assert!(marks(&roster, &far, BIG, 1000.0).pointers.is_empty());
        // Bob's session has no Snare.
        let other = ViewSpace::mapped(
            "session.arrange",
            Axis::linear("time", 0.0, 0.1),
            Axis::lanes("track", ["Kick"], 0.0, 40.0),
        );
        assert!(marks(&roster, &other, BIG, 1000.0).pointers.is_empty());
    }

    #[test]
    fn off_the_lanes_there_is_no_place() {
        let space = arrange(0.0, 0.1, 20.0, 40.0);
        assert!(space.to_place(10.0, 5.0, BIG).is_none(), "the ruler");
        assert!(space.to_place(10.0, 500.0, BIG).is_none(), "under the last track");
        assert!(ViewSpace::fractions("p").to_place(1.0, 1.0, Size::default()).is_none());
    }

    #[test]
    fn a_range_selection_is_a_wash_in_the_others_pixels() {
        let channel = Channel::default();
        let space = arrange(0.0, 0.1, 0.0, 40.0);
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        let from = space.to_place(100.0, 0.0, BIG).unwrap().at;
        let to = space.to_place(200.0, 60.0, BIG).unwrap().at;
        ann.edit_state(0.0, |s| {
            s.selections = vec![Selection {
                space: "session.arrange".into(),
                keys: Vec::new(),
                range: Some((from, to)),
            }];
        });
        ann.flush(10_000.0);
        let roster = roster_of(&channel, "bob", 0.0);
        let zoomed = arrange(0.0, 0.05, 0.0, 80.0);
        let marks = marks(&roster, &zoomed, BIG, 1000.0);
        assert_eq!(marks.washes.len(), 1);
        let w = &marks.washes[0];
        assert!(close(w.x, 200.0) && close(w.w, 200.0), "{w:?}");
        assert!(close(w.y, 0.0) && close(w.h, 120.0), "{w:?}");
    }

    #[test]
    fn a_tab_shows_who_looks_at_its_pane() {
        let channel = Channel::default();
        let space = pane_space("studio.setlist", "Christmas Eve");
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.edit_state(0.0, |s| s.looking_at = Some(space.clone()));
        ann.flush(10_000.0);
        let _cy = Here::new(channel.clone(), "cy", PeerState::new("cy", "Cy"));
        let roster = roster_of(&channel, "bob", 0.0);
        let names: Vec<String> = looking_at(&roster, &space)
            .into_iter()
            .map(|(_, name, _)| name)
            .collect();
        assert_eq!(names, ["Ann"]);
    }

    #[test]
    fn the_board_carries_spaces_people_and_published_places() {
        let board = PresenceBoard::default();
        assert!(board.declared("p1").is_none());
        let space = arrange(0.0, 0.1, 0.0, 40.0);
        board.declare("p1", space.clone());
        let after_first = board.spaces_revision();
        board.declare("p1", space.clone());
        assert_eq!(board.spaces_revision(), after_first, "the same again is no change");
        assert_eq!(board.declared("p1"), Some(space));
        board.undeclare("p1");
        assert!(board.declared("p1").is_none());

        let channel = Channel::default();
        let ann = Here::new(channel.clone(), "ann", PeerState::new("ann", "Ann"));
        ann.pointer(Place::new("session.arrange").with("time", 3.0).with("track", "Kick"), 0.0);
        board.saw(&roster_of(&channel, "bob", 0.0));
        let all = board.roster(None);
        assert_eq!(all.people.len(), 1);
        let pointer = all.people[0].pointer.clone().unwrap();
        assert_eq!(Place::from(&pointer).key("track"), Some("Kick"));
        assert!(board.roster(Some("elsewhere")).people.is_empty());
        assert_eq!(board.roster(Some("session.arrange")).people.len(), 1);

        board.publish(Some(Place::new("s").with("x", 1.0)));
        board.publish(None);
        let sent = board.take_published();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].is_none());
        assert!(board.take_published().is_empty());
    }
}
