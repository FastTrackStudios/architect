//! Where each group sits, from the shares: for the host to draw and
//! hit-test, and for moving the focus to the group in a direction.

use crate::model::{Axis, GroupId, Node, Rect};

/// A group and the rectangle it gets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub group: GroupId,
    pub rect: Rect,
}

/// A direction to move the focus in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Every group in a tree laid out in `rect`, by the splits' shares.
#[must_use]
pub fn area_rects(node: &Node, rect: Rect) -> Vec<Placed> {
    let mut out = Vec::new();
    place(node, rect, &mut out);
    out
}

fn place(node: &Node, rect: Rect, out: &mut Vec<Placed>) {
    match node {
        Node::Group(group) => out.push(Placed {
            group: group.id,
            rect,
        }),
        Node::Split(split) => {
            let mut at = 0.0_f32;
            for child in &split.children {
                let part = match split.axis {
                    Axis::Row => {
                        let w = rect.w * child.share;
                        Rect::new(rect.x + at, rect.y, w, rect.h)
                    }
                    Axis::Column => {
                        let h = rect.h * child.share;
                        Rect::new(rect.x, rect.y + at, rect.w, h)
                    }
                };
                at += match split.axis {
                    Axis::Row => part.w,
                    Axis::Column => part.h,
                };
                place(&child.node, part, out);
            }
        }
    }
}

/// The group next to `from` in `direction`: the nearest one beyond that
/// edge that overlaps it across, the most overlap breaking a tie.
#[must_use]
pub fn neighbour(placed: &[Placed], from: GroupId, direction: Direction) -> Option<GroupId> {
    let here = placed.iter().find(|p| p.group == from)?.rect;
    let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
    let eps = 0.5;
    placed
        .iter()
        .filter(|p| p.group != from)
        .filter_map(|p| {
            let r = p.rect;
            let (gap, across) = match direction {
                Direction::Left => (
                    here.x - r.right(),
                    overlap(here.y, here.bottom(), r.y, r.bottom()),
                ),
                Direction::Right => (
                    r.x - here.right(),
                    overlap(here.y, here.bottom(), r.y, r.bottom()),
                ),
                Direction::Up => (
                    here.y - r.bottom(),
                    overlap(here.x, here.right(), r.x, r.right()),
                ),
                Direction::Down => (
                    r.y - here.bottom(),
                    overlap(here.x, here.right(), r.x, r.right()),
                ),
            };
            (gap >= -eps && across > 0.0).then_some((gap, across, p.group))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(b.1.total_cmp(&a.1)))
        .map(|(_, _, g)| g)
}
