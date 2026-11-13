//! An open note's text area against its live document (`app::org`
//! `LiveNote`): what the browser half needs that needs no browser, tested
//! natively.
//!
//! The text area is not a controlled input: its value is set once, and
//! later only by [`caret_after`]-preserving writes when the document
//! changed, never while an IME composes. Typing is read back from it as
//! a [`Splice`] against the text it last showed, and [`LiveText`] hands
//! that splice to Loro at the **version the text area showed**
//! ([`merge_at`]): Loro merges it with whatever the document received
//! since, as it merges any two peers' edits. So a remote edit that lands
//! between a keystroke and its handler, or several during an IME
//! composition, is kept, and the person's typing and deleting stay where
//! they were made. Loro is the only merge; there is no second one here.
//!
//! Positions of a [`Splice`] are in chars (Unicode scalar values), as the
//! document's text counts them; a caret is in UTF-16 code units, as the
//! DOM's selection counts it.

use loro::{ExportMode, Frontiers, LoroDoc, LoroText};

/// One contiguous change: `removed` chars at `start` replaced by `added`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub start: usize,
    pub removed: usize,
    pub added: String,
}

/// The one splice that turns `old` into `new` (their common start and
/// end kept); `None` when they are the same.
#[must_use]
pub fn splice(old: &str, new: &str) -> Option<Splice> {
    if old == new {
        return None;
    }
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(old.len().min(new.len()).saturating_sub(start))
        .take_while(|(a, b)| a == b)
        .count();
    Some(Splice {
        start,
        removed: old.len().saturating_sub(start).saturating_sub(tail),
        added: new
            .get(start..new.len().saturating_sub(tail))
            .unwrap_or_default()
            .iter()
            .collect(),
    })
}

/// `user` into `text`.
fn edit(text: &LoroText, user: &Splice) -> Result<(), String> {
    if user.removed > 0 {
        text.delete(user.start, user.removed)
            .map_err(|e| format!("the edit did not apply: {e}"))?;
    }
    if !user.added.is_empty() {
        text.insert(user.start, &user.added)
            .map_err(|e| format!("the edit did not apply: {e}"))?;
    }
    Ok(())
}

/// `user`, a splice of the text `container` read at version `at`, into
/// `doc`, merged by Loro with whatever `doc` received after `at`.
///
/// At `at` still, it is applied to `doc` as it is (a local edit, which
/// `doc`'s subscribers send on): `Ok(None)`. Otherwise it is made on a
/// fork of `doc` at `at` and the fork's change imported into `doc`, which
/// merges it as it would a peer's: `Ok(Some(update))`, the update to send
/// on, as an import is not a local edit. The fork writes as `doc`'s own
/// peer when it holds all of `doc`'s own changes (nothing else writes to
/// `doc` meanwhile: this is synchronous), so a long session does not
/// collect a peer per merge.
///
/// # Errors
///
/// `at` is not in `doc`'s history, or the splice is out of the text's
/// range at `at`: the typing is not in the document, said rather than lost.
pub fn merge_at(
    doc: &LoroDoc,
    container: &str,
    at: &Frontiers,
    user: &Splice,
) -> Result<Option<Vec<u8>>, String> {
    if doc.state_frontiers() == *at {
        edit(&doc.get_text(container), user)?;
        doc.commit();
        return Ok(None);
    }
    let fork = doc
        .fork_at(at)
        .map_err(|e| format!("the version the text showed: {e}"))?;
    let peer = doc.peer_id();
    let own = |d: &LoroDoc| d.oplog_vv().get(&peer).copied();
    if own(&fork) == own(doc) {
        fork.set_peer_id(peer)
            .map_err(|e| format!("the edit did not apply: {e}"))?;
    }
    let before = doc.oplog_vv();
    edit(&fork.get_text(container), user)?;
    fork.commit();
    let update = fork
        .export(ExportMode::updates(&before))
        .map_err(|e| format!("the edit did not apply: {e}"))?;
    doc.import(&update)
        .map_err(|e| format!("the edit did not apply: {e}"))?;
    Ok(Some(update))
}

/// What came of reading the text area back ([`LiveText::typed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Not attached yet: the document does not hold the note, so typing
    /// is not taken (the text area is read-only until then).
    NotLive,
    /// The text area reads what it showed.
    Nothing,
    /// In the document; `relay` is the update to send on when it was
    /// merged rather than applied ([`merge_at`]).
    Merged { relay: Option<Vec<u8>> },
}

/// A text area's side of a live note body.
///
/// The document, the text the text area last showed and the document's
/// version then, and whether the note's session has attached (the
/// document holds the node's copy).
pub struct LiveText {
    doc: LoroDoc,
    container: String,
    shown: String,
    at: Frontiers,
    live: bool,
}

impl LiveText {
    /// A text area showing `shown` (the note as read) over `doc`, not
    /// attached yet.
    #[must_use]
    pub fn new(doc: LoroDoc, container: impl Into<String>, shown: impl Into<String>) -> Self {
        let at = doc.state_frontiers();
        Self {
            doc,
            container: container.into(),
            shown: shown.into(),
            at,
            live: false,
        }
    }

    /// The body as the document holds it.
    #[must_use]
    pub fn body(&self) -> String {
        self.doc.get_text(self.container.as_str()).to_string()
    }

    /// Whether typing is taken ([`Self::attach`] was called).
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.live
    }

    /// The session attached: typing is taken from now on. What the text
    /// area must show now (anything typed into it before is dropped: it
    /// was read-only).
    pub fn attach(&mut self) -> String {
        self.live = true;
        self.show()
    }

    /// The text area reads `typed`: what changed since it last showed,
    /// into the document at the version it showed. Then it must show
    /// [`Self::show`]'s text.
    ///
    /// # Errors
    ///
    /// The edit did not apply ([`merge_at`]).
    pub fn typed(&mut self, typed: &str) -> Result<Typed, String> {
        if !self.live {
            return Ok(Typed::NotLive);
        }
        let Some(user) = splice(&self.shown, typed) else {
            return Ok(Typed::Nothing);
        };
        let relay = merge_at(&self.doc, &self.container, &self.at, &user);
        // Shown or not, the text area is brought to the document next.
        let relay = relay?;
        Ok(Typed::Merged { relay })
    }

    /// The document's body, which the text area shows from now on.
    pub fn show(&mut self) -> String {
        let body = self.body();
        self.shown.clone_from(&body);
        self.at = self.doc.state_frontiers();
        body
    }
}

/// Where a caret at `offset` (UTF-16 units) in `old` belongs once the text
/// reads `new`: before the change it stays, after it it moves with the
/// text, inside it it goes to the change's end.
#[must_use]
pub fn caret_after(offset: u32, old: &str, new: &str) -> u32 {
    let old: Vec<u16> = old.encode_utf16().collect();
    let new: Vec<u16> = new.encode_utf16().collect();
    let start = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(old.len().min(new.len()).saturating_sub(start))
        .take_while(|(a, b)| a == b)
        .count();
    let removed_end = old.len().saturating_sub(tail);
    let added_end = new.len().saturating_sub(tail);
    let at = usize::try_from(offset).unwrap_or(usize::MAX).min(old.len());
    let moved = if at <= start {
        at
    } else if at >= removed_end {
        at.saturating_sub(removed_end).saturating_add(added_end)
    } else {
        added_end
    };
    u32::try_from(moved).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_splice_is_the_changed_middle() {
        assert_eq!(splice("same", "same"), None);
        assert_eq!(
            splice("hello world", "hello brave world"),
            Some(Splice {
                start: 6,
                removed: 0,
                added: "brave ".into()
            })
        );
        assert_eq!(
            splice("aXXb", "ab"),
            Some(Splice {
                start: 1,
                removed: 2,
                added: String::new()
            })
        );
        // Chars, not bytes.
        assert_eq!(splice("é1", "é2").unwrap().start, 1);
    }

    const BODY: &str = "body";

    /// Two peers on one note: the browser's replica (`LiveText` over it,
    /// attached, showing `start`) and another's.
    fn two_peers(start: &str) -> (LoroDoc, LoroDoc, LiveText) {
        let other = LoroDoc::new();
        other.get_text(BODY).insert(0, start).unwrap();
        other.commit();
        let replica = LoroDoc::new();
        replica
            .import(&other.export(ExportMode::all_updates()).unwrap())
            .unwrap();
        let mut text = LiveText::new(replica.clone(), BODY, start);
        assert_eq!(text.attach(), start);
        (replica, other, text)
    }

    /// `other` edits (each `(at, removed, added)`, one commit each), and
    /// the replica receives them, as the session imports them.
    fn remote(replica: &LoroDoc, other: &LoroDoc, edits: &[(usize, usize, &str)]) {
        for (at, removed, added) in edits {
            let before = other.oplog_vv();
            edit(
                &other.get_text(BODY),
                &Splice {
                    start: *at,
                    removed: *removed,
                    added: (*added).to_owned(),
                },
            )
            .unwrap();
            other.commit();
            replica
                .import(&other.export(ExportMode::updates(&before)).unwrap())
                .unwrap();
        }
    }

    /// The person's typing (the text area now reads `typed`) into the
    /// replica; what the replica reads after, checked against `other`
    /// once the relayed update reaches it.
    fn type_in(replica: &LoroDoc, other: &LoroDoc, text: &mut LiveText, typed: &str) -> String {
        let before = replica.oplog_vv();
        let relay = match text.typed(typed).unwrap() {
            Typed::Merged { relay } => relay,
            other => panic!("not merged: {other:?}"),
        };
        // What reaches the other peer: the relay, or the local commit.
        let update = relay.unwrap_or_else(|| replica.export(ExportMode::updates(&before)).unwrap());
        other.import(&update).unwrap();
        let shown = text.show();
        assert_eq!(shown, other.get_text(BODY).to_string(), "the peers differ");
        // A merge writes as the replica's own peer: no peer per merge.
        let peers = [replica.peer_id(), other.peer_id()];
        for peer in replica.oplog_vv().keys() {
            assert!(
                peers.contains(peer),
                "a peer per merge: {:?}",
                replica.oplog_vv()
            );
        }
        shown
    }

    #[test]
    fn typing_with_nothing_remote_is_the_typing() {
        let (replica, other, mut text) = two_peers("abc");
        assert_eq!(type_in(&replica, &other, &mut text, "abXc"), "abXc");
        assert_eq!(text.typed("abXc").unwrap(), Typed::Nothing);
    }

    /// #628 M1: remote changes in two places since the text area showed
    /// the note, and the person typed between them. Diffing two strings
    /// made the remote change one span and moved the typing to its end.
    #[test]
    fn typing_between_two_remote_changes_stays_where_it_was_made() {
        let (replica, other, mut text) = two_peers("A middle Z");
        remote(&replica, &other, &[(0, 1, "a"), (9, 1, "z")]);
        assert_eq!(
            type_in(&replica, &other, &mut text, "A miXddle Z"),
            "a miXddle z"
        );
        // A remote insert before repeated characters.
        let (replica, other, mut text) = two_peers("aa");
        remote(&replica, &other, &[(0, 0, "a")]);
        assert_eq!(type_in(&replica, &other, &mut text, "aXa"), "aaXa");
    }

    /// #628 M1: the person's delete inside a span the remote changed
    /// around was dropped.
    #[test]
    fn a_delete_is_kept_when_the_text_around_it_changed() {
        let (replica, other, mut text) = two_peers("A middle Z");
        remote(&replica, &other, &[(0, 1, "a"), (9, 1, "z")]);
        assert_eq!(type_in(&replica, &other, &mut text, "A Z"), "a z");
        // A remote insert inside the span the person deleted is kept;
        // only what the person saw is gone.
        let (replica, other, mut text) = two_peers("abcdef");
        remote(&replica, &other, &[(3, 0, "X")]);
        assert_eq!(type_in(&replica, &other, &mut text, "af"), "aXf");
        // Both deleted the same word: deleted once, nothing else lost.
        let (replica, other, mut text) = two_peers("one two four");
        remote(&replica, &other, &[(4, 4, "")]);
        assert_eq!(type_in(&replica, &other, &mut text, "one four"), "one four");
    }

    /// #628 M1: while an IME composes, the text area is not touched, so
    /// remote edits pile up; the composed text lands where it was
    /// composed, and every remote edit is kept.
    #[test]
    fn a_composition_lands_where_it_was_composed() {
        let (replica, other, mut text) = two_peers("Hello world");
        // During the composition: three remote edits, several imports.
        remote(
            &replica,
            &other,
            &[(0, 0, "Big "), (15, 0, "!"), (10, 1, "W")],
        );
        assert_eq!(other.get_text(BODY).to_string(), "Big Hello World!");
        // Composition end: the text area reads the shown text with the
        // composed characters after "Hello".
        assert_eq!(
            type_in(&replica, &other, &mut text, "Hello日本語 world"),
            "Big Hello日本語 World!"
        );
    }

    /// Before the session attaches, the document does not hold the note:
    /// typing is not taken (it would add the note a second time).
    #[test]
    fn nothing_is_taken_before_the_session_attaches() {
        let doc = LoroDoc::new();
        let mut text = LiveText::new(doc, BODY, "The note.");
        assert_eq!(text.typed(">The note.").unwrap(), Typed::NotLive);
        assert_eq!(text.body(), "");
        assert!(!text.is_live());
    }

    /// An edit that cannot apply is said, not lost silently.
    #[test]
    fn an_edit_that_does_not_apply_is_an_error() {
        let doc = LoroDoc::new();
        let out = merge_at(
            &doc,
            BODY,
            &doc.state_frontiers(),
            &Splice {
                start: 5,
                removed: 0,
                added: "x".into(),
            },
        );
        assert!(out.is_err(), "{out:?}");
    }

    #[test]
    fn the_caret_stays_with_its_text() {
        // A change after the caret: it does not move.
        assert_eq!(caret_after(2, "abcd", "abcdXYZ"), 2);
        // A change before it: it moves by the change.
        assert_eq!(caret_after(3, "abcd", "XYabcd"), 5);
        assert_eq!(caret_after(4, "abcd", "cd"), 2);
        // Inside a replaced span: to the replacement's end.
        assert_eq!(caret_after(2, "abcd", "aZd"), 2);
        // UTF-16: an emoji is two units.
        assert_eq!(caret_after(1, "ab", "😀ab"), 3);
        // Past the end is clamped; text added after the caret leaves it.
        assert_eq!(caret_after(99, "ab", "abc"), 2);
        assert_eq!(caret_after(99, "abc", "a"), 1);
    }
}
