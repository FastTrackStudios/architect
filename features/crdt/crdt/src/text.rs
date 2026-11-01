//! Collaborative text: a Loro text bound to an editor, with everyone's
//! carets in it.
//!
//! Every app has text — a note, lyrics, a chart, a comment — and each
//! needs the same three things, which this module is:
//!
//! - **Edits as operations.** The editor reports what it did as
//!   [`TextOp`]s ([`TextBinding::on_local`]); they become Loro text
//!   operations, so concurrent typing merges character by character
//!   (never a whole-string replace).
//! - **Others' edits back, echo-guarded.** What arrives from elsewhere
//!   (a peer's import, a check-out, another part of the app writing the
//!   same text) is queued as operations in the editor's coordinates;
//!   the editor takes them with [`TextBinding::pull`] (or
//!   [`TextBinding::sync_into`] an [`EditorBinding`]). The binding's own
//!   commits carry its origin and are never sent back. Remote operations
//!   still queued when the person types are transformed against what they
//!   typed, so both land where each meant them.
//! - **Carets.** A caret and its selection become Loro stable cursors
//!   ([`TextBinding::caret`], a [`TextCaret`] for the person's presence
//!   state), so they stay on the right character while others type; the
//!   others' are resolved in this editor's coordinates
//!   ([`TextBinding::remote_carets`]).
//!
//! An editor may show text before the bound text (a note's frontmatter):
//! the binding's [`offset`](TextBinding::set_offset) is that prefix's
//! length, added to everything the editor is given and taken off
//! everything it reports.
//!
//! **IME and composition** cross as the editor reports them: while
//! [`composing`](TextBinding::set_composing), nothing remote is handed to
//! the editor (it would break the composition); it all arrives when the
//! composition ends.
//!
//! Positions are Unicode scalar values (`char`s), Loro's unit. An editor
//! working in UTF-8 bytes or UTF-16 translates at its edge (the editor
//! repository's `editor-crdt` does).
//!
//! **Convergence is checked, not hoped for.** The binding keeps the text
//! as the editor shows it; whenever what it hands over would not leave the
//! editor equal to the document (a local edit that raced an import on
//! another thread, a commit that carried the binding's origin but other
//! code's changes), it hands over the one replacement that does instead.

use std::sync::{Arc, Mutex};

use loro::cursor::{Cursor, Side};
use loro::event::{Diff, DiffEvent};
use loro::{ContainerTrait, LoroDoc, LoroText, Subscription, TextDelta};

pub use crate::codec::TextOp;
use crate::presence::{Roster, TextCaret};

/// What an editor implements so a [`TextBinding`] can hand it others' edits.
///
/// Apply `ops` in order, each against the text the previous one left
/// (positions in the editor's own coordinates, prefix included), and do
/// not report them back as the person's own.
pub trait EditorBinding {
    fn apply_remote(&mut self, ops: &[TextOp]);
}

/// A failure binding an edit.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TextError {
    /// The edit touches the prefix the binding does not own (before
    /// [`TextBinding::offset`]).
    #[error("edit at {pos} is inside the {offset}-character prefix the text does not hold")]
    InPrefix { pos: u32, offset: u32 },
    /// Loro refused an operation (a position past the end).
    #[error("text edit refused: {0}")]
    Loro(String),
}

/// Another person's caret, in this editor's coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCaret {
    pub peer: String,
    pub name: String,
    pub color: u32,
    pub anchor: u32,
    pub head: u32,
}

/// One text container bound to one editor. See the module docs.
///
/// Holds the document and its subscription; the subscription's closure
/// holds only the queue (never the document — a Loro subscription that
/// owns its own document deadlocks on drop).
pub struct TextBinding {
    doc: LoroDoc,
    text: LoroText,
    /// The container id, which names this text in carets.
    id: String,
    /// The origin the binding's own commits carry, so their events are
    /// not handed back to the editor.
    origin: String,
    inner: Arc<Mutex<Inner>>,
    _sub: Subscription,
}

struct Inner {
    offset: u32,
    composing: bool,
    /// Others' edits not yet handed over, in text coordinates, each
    /// against the text the previous one left — starting from `shown`.
    pending: Vec<TextOp>,
    /// The text as the editor shows it (without the prefix).
    shown: Vec<char>,
}

impl TextBinding {
    /// Bind `text`, a text container of `doc`. Seed the editor with
    /// [`TextBinding::shown`].
    #[must_use]
    pub fn new(doc: &LoroDoc, text: LoroText) -> Self {
        let id = text.id().to_string();
        let origin = format!("text-binding:{}", uuid::Uuid::new_v4());
        let inner = Arc::new(Mutex::new(Inner {
            offset: 0,
            composing: false,
            pending: Vec::new(),
            shown: text.to_string().chars().collect(),
        }));
        let queue = inner.clone();
        let own = origin.clone();
        let sub = doc.subscribe(
            &text.id(),
            Arc::new(move |event: DiffEvent<'_>| {
                if event.origin == own {
                    return;
                }
                let mut ops = Vec::new();
                for diff in &event.events {
                    if let Diff::Text(deltas) = &diff.diff {
                        ops.extend(delta_ops(deltas));
                    }
                }
                if !ops.is_empty() {
                    architect::lock(&queue).pending.extend(ops);
                }
            }),
        );
        Self {
            doc: doc.clone(),
            text,
            id,
            origin,
            inner,
            _sub: sub,
        }
    }

    /// The same binding with an `offset`-character prefix before the text.
    #[must_use]
    pub fn with_offset(self, offset: u32) -> Self {
        self.set_offset(offset);
        self
    }

    /// The prefix's length changed (the frontmatter was edited).
    pub fn set_offset(&self, offset: u32) {
        architect::lock(&self.inner).offset = offset;
    }

    /// The prefix's length.
    #[must_use]
    pub fn offset(&self) -> u32 {
        architect::lock(&self.inner).offset
    }

    /// The container id: what names this text in a [`TextCaret`].
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The bound text container.
    #[must_use]
    pub const fn text(&self) -> &LoroText {
        &self.text
    }

    /// The text as the editor shows it (without the prefix): what to seed
    /// the editor with.
    #[must_use]
    pub fn shown(&self) -> String {
        architect::lock(&self.inner).shown.iter().collect()
    }

    /// A composition (IME) started or ended. While composing, others'
    /// edits wait.
    pub fn set_composing(&self, composing: bool) {
        architect::lock(&self.inner).composing = composing;
    }

    /// The person edited: `ops`, in the editor's coordinates, each against
    /// the text the previous one left. Committed to the document at once.
    pub fn on_local(&self, ops: &[TextOp]) -> Result<(), TextError> {
        if ops.is_empty() {
            return Ok(());
        }
        let mut inner = architect::lock(&self.inner);
        let local = ops
            .iter()
            .map(|op| unshift(op, inner.offset))
            .collect::<Result<Vec<_>, _>>()?;
        apply_chars(&mut inner.shown, &local);
        // Others' edits the editor has not seen yet: what the person typed
        // goes where they meant it in the document as it is now, and those
        // edits go where their authors meant them in the editor as it is now.
        let pending = std::mem::take(&mut inner.pending);
        let (into_doc, for_editor) = transform(&local, &pending);
        inner.pending = for_editor;
        // Committed under the queue's lock: the event for this commit is
        // ours (skipped, without locking), and any import racing it on
        // another thread waits to queue until the transform above stands.
        let applied = self.apply_to_doc(&into_doc);
        self.doc.set_next_commit_origin(&self.origin);
        self.doc.commit();
        drop(inner);
        applied
    }

    fn apply_to_doc(&self, ops: &[TextOp]) -> Result<(), TextError> {
        let fit = |n: u32| usize::try_from(n).map_err(|e| TextError::Loro(e.to_string()));
        for op in ops {
            match op {
                TextOp::Insert { pos, text } => self.text.insert(fit(*pos)?, text),
                TextOp::Delete { pos, len } => self.text.delete(fit(*pos)?, fit(*len)?),
            }
            .map_err(|e| TextError::Loro(e.to_string()))?;
        }
        Ok(())
    }

    /// Others' edits for the editor, in its coordinates, each against the
    /// text the previous one left; empty while composing. After applying
    /// them the editor shows exactly the document's text.
    #[must_use]
    pub fn pull(&self) -> Vec<TextOp> {
        let mut inner = architect::lock(&self.inner);
        if inner.composing {
            return Vec::new();
        }
        let pending = std::mem::take(&mut inner.pending);
        let mut after = inner.shown.clone();
        apply_chars(&mut after, &pending);
        let now: Vec<char> = self.text.to_string().chars().collect();
        let ops = if after == now {
            pending
        } else {
            tracing::debug!(
                text = %self.id,
                "text binding: the editor would not match the document; replacing the difference"
            );
            replace_difference(&inner.shown, &now)
        };
        inner.shown = now;
        let offset = inner.offset;
        drop(inner);
        ops.into_iter().map(|op| shift(op, offset)).collect()
    }

    /// [`pull`](Self::pull) into `editor`. Returns how many operations it
    /// was handed.
    pub fn sync_into(&self, editor: &mut impl EditorBinding) -> usize {
        let ops = self.pull();
        if !ops.is_empty() {
            editor.apply_remote(&ops);
        }
        ops.len()
    }

    /// A caret (`head`) and its selection's other end (`anchor`), in the
    /// editor's coordinates, as stable cursors for the person's presence
    /// state. `None` where the text has no cursor to give (a position past
    /// its end). A position in the prefix is the text's start.
    #[must_use]
    pub fn caret(&self, anchor: u32, head: u32) -> Option<TextCaret> {
        let inner = architect::lock(&self.inner);
        let at = |p: u32| {
            let p = map_forward(p.saturating_sub(inner.offset), &inner.pending);
            self.text
                .get_cursor(usize::try_from(p).ok()?, Side::Left)
                .map(|c| c.encode())
        };
        Some(TextCaret {
            text: self.id.clone(),
            anchor: at(anchor)?,
            head: at(head)?,
        })
    }

    /// Where a caret is now, in the editor's coordinates (anchor, head).
    #[must_use]
    pub fn resolve(&self, caret: &TextCaret) -> Option<(u32, u32)> {
        if caret.text != self.id {
            return None;
        }
        let inner = architect::lock(&self.inner);
        let at = |bytes: &[u8]| {
            let cursor = Cursor::decode(bytes).ok()?;
            let pos = self.doc.get_cursor_pos(&cursor).ok()?.current.pos;
            let pos = map_back(u32::try_from(pos).ok()?, &inner.pending);
            Some(pos.saturating_add(inner.offset))
        };
        Some((at(&caret.anchor)?, at(&caret.head)?))
    }

    /// Everyone's caret in this text, in the editor's coordinates.
    #[must_use]
    pub fn remote_carets(&self, roster: &Roster) -> Vec<RemoteCaret> {
        roster
            .carets_in(&self.id)
            .filter_map(|(peer, state, caret)| {
                let (anchor, head) = self.resolve(caret)?;
                Some(RemoteCaret {
                    peer: peer.to_owned(),
                    name: state.name.clone(),
                    color: state.color,
                    anchor,
                    head,
                })
            })
            .collect()
    }
}

// ── operations ──────────────────────────────────────────────────────────

fn chars(s: &str) -> u32 {
    u32::try_from(s.chars().count()).unwrap_or(u32::MAX)
}

/// A Loro text delta as operations, each against the text the previous
/// one left.
fn delta_ops(deltas: &[TextDelta]) -> Vec<TextOp> {
    let mut ops = Vec::new();
    let mut pos: u32 = 0;
    for delta in deltas {
        match delta {
            TextDelta::Retain { retain, .. } => {
                pos = pos.saturating_add(u32::try_from(*retain).unwrap_or(u32::MAX));
            }
            TextDelta::Insert { insert, .. } => {
                ops.push(TextOp::Insert {
                    pos,
                    text: insert.clone(),
                });
                pos = pos.saturating_add(chars(insert));
            }
            TextDelta::Delete { delete } => ops.push(TextOp::Delete {
                pos,
                len: u32::try_from(*delete).unwrap_or(u32::MAX),
            }),
        }
    }
    ops
}

fn unshift(op: &TextOp, offset: u32) -> Result<TextOp, TextError> {
    let pos = match op {
        TextOp::Insert { pos, .. } | TextOp::Delete { pos, .. } => *pos,
    };
    let inside = pos
        .checked_sub(offset)
        .ok_or(TextError::InPrefix { pos, offset })?;
    Ok(match op {
        TextOp::Insert { text, .. } => TextOp::Insert {
            pos: inside,
            text: text.clone(),
        },
        TextOp::Delete { len, .. } => TextOp::Delete {
            pos: inside,
            len: *len,
        },
    })
}

fn shift(op: TextOp, offset: u32) -> TextOp {
    match op {
        TextOp::Insert { pos, text } => TextOp::Insert {
            pos: pos.saturating_add(offset),
            text,
        },
        TextOp::Delete { pos, len } => TextOp::Delete {
            pos: pos.saturating_add(offset),
            len,
        },
    }
}

/// Apply `ops` to `text`, clamped (an operation past the end applies to
/// what there is).
fn apply_chars(text: &mut Vec<char>, ops: &[TextOp]) {
    let fit = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    for op in ops {
        match op {
            TextOp::Insert { pos, text: s } => {
                let at = fit(*pos).min(text.len());
                text.splice(at..at, s.chars());
            }
            TextOp::Delete { pos, len } => {
                let from = fit(*pos).min(text.len());
                let to = from.saturating_add(fit(*len)).min(text.len());
                text.drain(from..to);
            }
        }
    }
}

/// The one replacement (common prefix and suffix kept) that turns `from`
/// into `to`.
fn replace_difference(from: &[char], to: &[char]) -> Vec<TextOp> {
    let prefix = from
        .iter()
        .zip(to.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let from_rest = from.get(prefix..).unwrap_or_default();
    let to_rest = to.get(prefix..).unwrap_or_default();
    let suffix = from_rest
        .iter()
        .rev()
        .zip(to_rest.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let removed = from_rest.len().saturating_sub(suffix);
    let added: String = to_rest
        .get(..to_rest.len().saturating_sub(suffix))
        .unwrap_or_default()
        .iter()
        .collect();
    let pos = u32::try_from(prefix).unwrap_or(u32::MAX);
    let mut ops = Vec::new();
    if removed > 0 {
        ops.push(TextOp::Delete {
            pos,
            len: u32::try_from(removed).unwrap_or(u32::MAX),
        });
    }
    if !added.is_empty() {
        ops.push(TextOp::Insert { pos, text: added });
    }
    ops
}

/// A position in the text before `ops`, in the text after them. Inside a
/// deletion it lands at the deletion's start; text inserted exactly at it
/// goes after it (the caret stays left of it).
fn map_forward(mut p: u32, ops: &[TextOp]) -> u32 {
    for op in ops {
        match op {
            TextOp::Insert { pos, text } => {
                if *pos < p {
                    p = p.saturating_add(chars(text));
                }
            }
            TextOp::Delete { pos, len } => {
                if p > *pos {
                    p = p.saturating_sub((*len).min(p.saturating_sub(*pos)));
                }
            }
        }
    }
    p
}

/// A position in the text after `ops`, in the text before them (inside
/// an insertion: its start).
fn map_back(mut p: u32, ops: &[TextOp]) -> u32 {
    for op in ops.iter().rev() {
        match op {
            TextOp::Insert { pos, text } => {
                let n = chars(text);
                if p >= pos.saturating_add(n) {
                    p = p.saturating_sub(n);
                } else if p > *pos {
                    p = *pos;
                }
            }
            TextOp::Delete { pos, len } => {
                if p > *pos {
                    p = p.saturating_add(*len);
                }
            }
        }
    }
    p
}

/// Transform two operation sequences made against the same text:
/// `(a', b')` such that `a` then `b'` and `b` then `a'` leave the same
/// text. Where both insert at one position, `a`'s text goes first.
fn transform(a: &[TextOp], b: &[TextOp]) -> (Vec<TextOp>, Vec<TextOp>) {
    match (a, b) {
        ([], _) | (_, []) => (a.to_vec(), b.to_vec()),
        ([x], [y]) => transform_one(x, y),
        // One against many: through `b` one at a time.
        ([_], [y, rest @ ..]) => {
            let (a2, y2) = transform(a, std::slice::from_ref(y));
            let (a3, rest2) = transform(&a2, rest);
            (a3, concat(y2, rest2))
        }
        // Many against any: `a` one at a time.
        ([x, rest @ ..], _) => {
            let (x2, b2) = transform(std::slice::from_ref(x), b);
            let (rest2, b3) = transform(rest, &b2);
            (concat(x2, rest2), b3)
        }
    }
}

fn concat(mut a: Vec<TextOp>, b: Vec<TextOp>) -> Vec<TextOp> {
    a.extend(b);
    a
}

fn ins(pos: u32, text: &str) -> TextOp {
    TextOp::Insert {
        pos,
        text: text.to_owned(),
    }
}

fn del(pos: u32, len: u32) -> Vec<TextOp> {
    if len == 0 {
        Vec::new()
    } else {
        vec![TextOp::Delete { pos, len }]
    }
}

/// [`transform`] for one operation each.
fn transform_one(a: &TextOp, b: &TextOp) -> (Vec<TextOp>, Vec<TextOp>) {
    match (a, b) {
        (
            TextOp::Insert {
                pos: a_pos,
                text: a_text,
            },
            TextOp::Insert {
                pos: b_pos,
                text: b_text,
            },
        ) => {
            if a_pos <= b_pos {
                let b2 = ins(b_pos.saturating_add(chars(a_text)), b_text);
                (vec![a.clone()], vec![b2])
            } else {
                let a2 = ins(a_pos.saturating_add(chars(b_text)), a_text);
                (vec![a2], vec![b.clone()])
            }
        }
        (TextOp::Insert { .. }, TextOp::Delete { .. }) => {
            let (b2, a2) = delete_against_insert(b, a);
            (a2, b2)
        }
        (TextOp::Delete { .. }, TextOp::Insert { .. }) => delete_against_insert(a, b),
        (
            TextOp::Delete {
                pos: a_pos,
                len: a_len,
            },
            TextOp::Delete {
                pos: b_pos,
                len: b_len,
            },
        ) => {
            let (a_pos, a_len, b_pos, b_len) = (*a_pos, *a_len, *b_pos, *b_len);
            let (a_end, b_end) = (a_pos.saturating_add(a_len), b_pos.saturating_add(b_len));
            let overlap = a_end.min(b_end).saturating_sub(a_pos.max(b_pos));
            // What of `b` lies before `a`, and of `a` before `b`.
            let b_before = b_end.min(a_pos).saturating_sub(b_pos);
            let a_before = a_end.min(b_pos).saturating_sub(a_pos);
            (
                del(
                    a_pos.saturating_sub(b_before),
                    a_len.saturating_sub(overlap),
                ),
                del(
                    b_pos.saturating_sub(a_before),
                    b_len.saturating_sub(overlap),
                ),
            )
        }
    }
}

/// A delete against an insert: `(delete', insert')`. An insert inside the
/// deleted range survives, and the delete goes round it.
fn delete_against_insert(delete: &TextOp, insert: &TextOp) -> (Vec<TextOp>, Vec<TextOp>) {
    let (
        TextOp::Delete { pos: from, len },
        TextOp::Insert {
            pos: at,
            text: inserted,
        },
    ) = (delete, insert)
    else {
        return (vec![delete.clone()], vec![insert.clone()]);
    };
    let (from, len, at, width) = (*from, *len, *at, chars(inserted));
    let end = from.saturating_add(len);
    if at <= from {
        (del(from.saturating_add(width), len), vec![insert.clone()])
    } else if at >= end {
        (
            vec![delete.clone()],
            vec![ins(at.saturating_sub(len), inserted)],
        )
    } else {
        let before = at.saturating_sub(from);
        (
            concat(
                del(from, before),
                del(from.saturating_add(width), len.saturating_sub(before)),
            ),
            vec![ins(from, inserted)],
        )
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn run(text: &str, ops: &[TextOp]) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        apply_chars(&mut chars, ops);
        chars.into_iter().collect()
    }

    /// A random op valid against a text of `len` characters.
    fn op(len: u32) -> impl Strategy<Value = TextOp> {
        let insert = (0..=len, "[a-zé]{1,3}").prop_map(|(pos, text)| TextOp::Insert { pos, text });
        if len == 0 {
            return insert.boxed();
        }
        let delete = (0..len)
            .prop_flat_map(move |pos| (Just(pos), 1..=len.saturating_sub(pos)))
            .prop_map(|(pos, len)| TextOp::Delete { pos, len });
        prop_oneof![insert, delete].boxed()
    }

    /// A text and two sequences of up to three ops each made against it.
    fn case() -> impl Strategy<Value = (String, Vec<TextOp>, Vec<TextOp>)> {
        "[a-z]{0,8}".prop_flat_map(|base| {
            let seq = |base: String| {
                let n = u32::try_from(base.chars().count()).unwrap();
                proptest::collection::vec(op(n), 0..3).prop_map(move |ops| {
                    // Each op is made against the text the previous one left:
                    // regenerate positions against that text by clamping.
                    let mut text: Vec<char> = base.chars().collect();
                    let mut out = Vec::new();
                    for op in ops {
                        let len = u32::try_from(text.len()).unwrap();
                        let op = match op {
                            TextOp::Insert { pos, text: s } => TextOp::Insert {
                                pos: pos.min(len),
                                text: s,
                            },
                            TextOp::Delete { pos, len: l } if pos < len => TextOp::Delete {
                                pos,
                                len: l.min(len.saturating_sub(pos)),
                            },
                            TextOp::Delete { .. } => continue,
                        };
                        apply_chars(&mut text, std::slice::from_ref(&op));
                        out.push(op);
                    }
                    out
                })
            };
            (Just(base.clone()), seq(base.clone()), seq(base))
        })
    }

    proptest! {
        #[test]
        fn both_orders_leave_the_same_text((base, a, b) in case()) {
            let (a2, b2) = transform(&a, &b);
            let a_then_b = run(&run(&base, &a), &b2);
            let b_then_a = run(&run(&base, &b), &a2);
            prop_assert_eq!(a_then_b, b_then_a);
        }
    }

    #[test]
    fn an_insert_inside_a_deletion_survives_it() {
        let a = [TextOp::Delete { pos: 1, len: 4 }];
        let b = [ins(3, "XY")];
        let (a2, b2) = transform(&a, &b);
        assert_eq!(run(&run("abcdefg", &a), &b2), "aXYfg");
        assert_eq!(run(&run("abcdefg", &b), &a2), "aXYfg");
    }

    #[test]
    fn a_delta_becomes_operations_in_sequence() {
        let deltas = vec![
            TextDelta::Retain {
                retain: 2,
                attributes: None,
            },
            TextDelta::Insert {
                insert: "XY".into(),
                attributes: None,
            },
            TextDelta::Delete { delete: 1 },
        ];
        let ops = delta_ops(&deltas);
        assert_eq!(ops, vec![ins(2, "XY"), TextOp::Delete { pos: 4, len: 1 }]);
        assert_eq!(run("abcd", &ops), "abXYd");
    }

    #[test]
    fn positions_map_through_operations_and_back() {
        let ops = [ins(2, "XY"), TextOp::Delete { pos: 8, len: 2 }];
        // "abcdefgh" → "abXYcdefgh" → "abXYcdef": "f" moves from 5 to 7
        assert_eq!(map_forward(5, &ops), 7);
        assert_eq!(map_back(7, &ops), 5);
        assert_eq!(
            map_forward(2, &ops),
            2,
            "a caret stays left of an insert at it"
        );
        assert_eq!(map_back(3, &ops), 2, "inside an insert: its start");
    }

    #[test]
    fn the_difference_is_one_replacement() {
        let from: Vec<char> = "hello world".chars().collect();
        let to: Vec<char> = "hello brave world".chars().collect();
        let ops = replace_difference(&from, &to);
        assert_eq!(ops, vec![ins(6, "brave ")]);
        assert_eq!(run("hello world", &ops), "hello brave world");
    }

    fn bound(text: &str) -> (LoroDoc, TextBinding) {
        let doc = LoroDoc::new();
        let t = doc.get_text("body");
        t.insert(0, text).unwrap();
        doc.commit();
        let binding = TextBinding::new(&doc, t);
        (doc, binding)
    }

    #[test]
    fn local_edits_are_not_handed_back() {
        let (doc, binding) = bound("hello");
        binding.on_local(&[ins(5, " world")]).unwrap();
        assert_eq!(doc.get_text("body").to_string(), "hello world");
        assert!(binding.pull().is_empty(), "no echo");
        assert_eq!(binding.shown(), "hello world");
    }

    #[test]
    fn other_writes_arrive_as_operations_and_the_prefix_is_kept_out() {
        let (doc, binding) = bound("hello");
        let binding = binding.with_offset(4); // "---\n" before the body
        let t = doc.get_text("body");
        t.insert(0, ">> ").unwrap();
        doc.commit();
        assert_eq!(binding.pull(), vec![ins(4, ">> ")]);
        assert_eq!(
            binding.on_local(&[ins(1, "x")]),
            Err(TextError::InPrefix { pos: 1, offset: 4 })
        );
    }

    #[test]
    fn typing_over_queued_edits_lands_where_each_meant() {
        let (doc, binding) = bound("one three");
        // Someone else writes "two " before "three"; not pulled yet.
        doc.get_text("body").insert(4, "two ").unwrap();
        doc.commit();
        // Meanwhile the person, seeing "one three", types "!" at the end.
        binding.on_local(&[ins(9, "!")]).unwrap();
        assert_eq!(doc.get_text("body").to_string(), "one two three!");
        let mut editor = "one three!".to_owned();
        let ops = binding.pull();
        editor = run(&editor, &ops);
        assert_eq!(editor, "one two three!");
    }

    #[test]
    fn nothing_arrives_while_composing() {
        let (doc, binding) = bound("abc");
        binding.set_composing(true);
        doc.get_text("body").insert(0, "x").unwrap();
        doc.commit();
        assert!(binding.pull().is_empty());
        binding.set_composing(false);
        assert_eq!(binding.pull(), vec![ins(0, "x")]);
    }

    #[test]
    fn a_caret_follows_its_character() {
        let (doc, binding) = bound("hello world");
        let caret = binding.caret(6, 11).unwrap(); // "world" selected
        doc.get_text("body").insert(0, ">> ").unwrap();
        doc.commit();
        let _ = binding.pull();
        assert_eq!(binding.resolve(&caret), Some((9, 14)));
    }
}
