//! Frames of a group of runtime views shown together (#891).
//!
//! Studio's arrangement and mixer are process extensions on one engine,
//! each pane sending its own frames. A selection made in one must show
//! in the other in the same window frame, as it does when one program
//! draws both. A change carried by an engine value already does (every
//! pane reads it when it composes); a change of layout (a strip opening
//! wide, a row's tint) is each pane's own frame, and they arrive apart.
//!
//! So views join a named group and say which edit each frame shows: an
//! element of the view carries [`GROUP_ATTR`] (the group's name) and
//! [`EPOCH_ATTR`] (a number that grows with each edit of the shared
//! state, the engine's revision for instance). The host holds a member's
//! frame of epoch `e` until every member of the group has sent `e` or
//! later, then applies them in one turn, so they are drawn in one frame.
//! A member that does not answer within [`DEADLINE`] is not waited for:
//! a slow or stuck pane costs the others a frame's delay, never more.
//! A frame without an epoch, or of a view in no group, is applied at once.
//!
//! The rule is the host's and the carrier's attributes are DOM edits, so
//! it is the same for a process, a WASI component and a page.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use futures::channel::oneshot;

/// The attribute that names the group a view's frames are shown with.
pub const GROUP_ATTR: &str = "data-architect-group";

/// The attribute that says which edit of the shared state a view's frame
/// shows: a number that grows with each edit.
pub const EPOCH_ATTR: &str = "data-architect-epoch";

/// The longest a member's frame waits for the rest of its group.
pub const DEADLINE: Duration = Duration::from_millis(50);

/// One window's groups (the remote views of a window share it).
#[derive(Clone, Default)]
pub struct Barrier(Rc<RefCell<HashMap<String, Group>>>);

#[derive(Default)]
struct Group {
    /// Each member (a mounted view, by its pane number): the newest epoch
    /// it has sent, if any.
    members: HashMap<u64, Option<u64>>,
    /// Frames held: the epoch each waits for, and what releases it.
    held: Vec<(u64, oneshot::Sender<()>)>,
}

impl Group {
    /// Every member has sent `epoch` or later.
    fn reached(&self, epoch: u64) -> bool {
        self.members.values().all(|e| e.is_some_and(|e| e >= epoch))
    }

    /// Lets go of the frames whose epoch every member has reached.
    fn release(&mut self) {
        let held = std::mem::take(&mut self.held);
        for (epoch, go) in held {
            if self.reached(epoch) {
                let _ = go.send(());
            } else {
                self.held.push((epoch, go));
            }
        }
    }
}

thread_local! {
    static WINDOW: Barrier = Barrier::default();
}

impl Barrier {
    /// The barrier of the views this thread (a window's) mounts.
    #[must_use]
    pub fn here() -> Self {
        WINDOW.with(Clone::clone)
    }

    /// View `pane` is a member of `group` from now on (once: again does
    /// nothing).
    pub fn join(&self, group: &str, pane: u64) {
        let mut groups = self.0.borrow_mut();
        groups
            .entry(group.to_owned())
            .or_default()
            .members
            .entry(pane)
            .or_insert(None);
    }

    /// View `pane` leaves `group` (unmounted): frames waiting only for it
    /// go.
    pub fn leave(&self, group: &str, pane: u64) {
        let mut groups = self.0.borrow_mut();
        if let Some(g) = groups.get_mut(group) {
            g.members.remove(&pane);
            g.release();
            if g.members.is_empty() {
                groups.remove(group);
            }
        }
    }

    /// View `pane` of `group` has a frame of `epoch`: ready when every
    /// member has reached it, or after `deadline`. Its arrival may let
    /// other members' frames go first.
    // The window's thread's, as the views' `Mount` tasks that call it.
    #[allow(clippy::future_not_send)]
    pub async fn arrive(&self, group: &str, pane: u64, epoch: u64, deadline: Duration) {
        let waiting = {
            let mut groups = self.0.borrow_mut();
            let g = groups.entry(group.to_owned()).or_default();
            let newest = g.members.entry(pane).or_insert(None);
            *newest = Some(newest.map_or(epoch, |e| e.max(epoch)));
            g.release();
            if g.reached(epoch) {
                None
            } else {
                let (go, gone) = oneshot::channel();
                g.held.push((epoch, go));
                Some(gone)
            }
        };
        if let Some(gone) = waiting {
            use futures::FutureExt as _;
            futures::select! {
                _ = gone.fuse() => {},
                () = futures_timer::Delay::new(deadline).fuse() => {},
            }
        }
    }
}

/// The group and the epoch a frame's edits set, if they set both or
/// either (an attribute set on any element of the view).
#[must_use]
pub fn of_frame(edits: &[crate::wire::Edit]) -> (Option<String>, Option<u64>) {
    use crate::wire::{Edit, Value};
    let (mut group, mut epoch) = (None, None);
    for edit in edits {
        if let Edit::SetAttribute { name, value, .. } = edit {
            let text = match value {
                Value::Text(t) => Some(t.clone()),
                Value::Int(i) => Some(i.to_string()),
                Value::Float(f) => Some(f.to_string()),
                _ => None,
            };
            if name == GROUP_ATTR {
                group = text.filter(|t| !t.is_empty());
            } else if name == EPOCH_ATTR {
                epoch = text.and_then(|t| t.trim().parse::<u64>().ok());
            }
        }
    }
    (group, epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt as _;

    fn when_ready(pool: &mut LocalPool, b: &Barrier, pane: u64, epoch: u64) -> Rc<RefCell<bool>> {
        let ready = Rc::new(RefCell::new(false));
        let (b, r) = (b.clone(), Rc::clone(&ready));
        pool.spawner()
            .spawn_local(async move {
                b.arrive("studio", pane, epoch, Duration::from_secs(5))
                    .await;
                *r.borrow_mut() = true;
            })
            .unwrap();
        pool.run_until_stalled();
        ready
    }

    #[test]
    fn a_frame_waits_for_the_rest_of_its_group_then_both_go() {
        let b = Barrier::default();
        b.join("studio", 1);
        b.join("studio", 2);
        let mut pool = LocalPool::new();
        let arrangement = when_ready(&mut pool, &b, 1, 7);
        assert!(
            !*arrangement.borrow(),
            "applied before the mixer drew edit 7"
        );
        let mixer = when_ready(&mut pool, &b, 2, 7);
        pool.run_until_stalled();
        assert!(*arrangement.borrow() && *mixer.borrow());
    }

    #[test]
    fn a_later_frame_releases_an_earlier_one_and_a_member_gone_holds_none() {
        let b = Barrier::default();
        b.join("studio", 1);
        b.join("studio", 2);
        let mut pool = LocalPool::new();
        // The mixer draws edit 9 first: held, the arrangement has drawn none.
        let mixer = when_ready(&mut pool, &b, 2, 9);
        assert!(!*mixer.borrow());
        // The arrangement draws 10: the mixer's 9 goes; its own 10 waits
        // for the mixer.
        let arrangement = when_ready(&mut pool, &b, 1, 10);
        assert!(*mixer.borrow());
        assert!(!*arrangement.borrow());
        // The mixer closes: nothing is waited for.
        b.leave("studio", 2);
        pool.run_until_stalled();
        assert!(*arrangement.borrow());
    }

    #[test]
    fn a_member_that_never_answers_costs_the_deadline_alone() {
        let b = Barrier::default();
        b.join("studio", 1);
        b.join("studio", 2);
        let started = std::time::Instant::now();
        futures::executor::block_on(b.arrive("studio", 1, 3, Duration::from_millis(30)));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(30) && waited < Duration::from_secs(1),
            "{waited:?}"
        );
    }

    #[test]
    fn group_and_epoch_are_read_from_a_frames_attributes() {
        use crate::wire::{Edit, Value};
        let set = |name: &str, value: Value| Edit::SetAttribute {
            name: name.to_owned(),
            namespace: None,
            value,
        };
        let edits = vec![
            set("class", Value::Text("x".into())),
            set(GROUP_ATTR, Value::Text("studio".into())),
            set(EPOCH_ATTR, Value::Text("42".into())),
        ];
        assert_eq!(of_frame(&edits), (Some("studio".to_owned()), Some(42)));
        assert_eq!(of_frame(&[set(EPOCH_ATTR, Value::Int(5))]), (None, Some(5)));
    }
}
