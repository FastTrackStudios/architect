//! An open note's text area against its live document (`app::org`
//! `LiveNote`): what the browser half needs that needs no browser, tested
//! natively.
//!
//! The text area is not a controlled input: its value is set once, and
//! later only by [`caret_after`]-preserving writes when the document
//! changed, never while an IME composes. Typing is read back from it as
//! a [`Splice`] against the text it last showed, moved past whatever the
//! document received meanwhile ([`rebase`]), so a remote edit that lands
//! between a keystroke and its handler is kept, not typed over.
//!
//! Positions of a [`Splice`] are in chars (Unicode scalar values), as the
//! document's text counts them; a caret is in UTF-16 code units, as the
//! DOM's selection counts it.

/// One contiguous change: `removed` chars at `start` replaced by `added`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub start: usize,
    pub removed: usize,
    pub added: String,
}

impl Splice {
    fn added_len(&self) -> usize {
        self.added.chars().count()
    }

    const fn end(&self) -> usize {
        self.start.saturating_add(self.removed)
    }
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

/// `user`'s splice, made before `remote` changed the text, as splices
/// for the changed text.
///
/// They apply in order. Text only `remote` changed stays as `remote` left
/// it; the user's insertion lands where they typed it.
#[must_use]
pub fn rebase(user: &Splice, remote: Option<&Splice>) -> Vec<Splice> {
    let Some(remote) = remote else {
        return vec![user.clone()];
    };
    let shift = |at: usize| {
        at.saturating_add(remote.added_len())
            .saturating_sub(remote.removed)
    };
    // Wholly before the remote change (an insertion at its start too).
    if user.end() <= remote.start {
        return vec![user.clone()];
    }
    // Wholly after it.
    if user.start >= remote.end() {
        return vec![Splice {
            start: shift(user.start),
            removed: user.removed,
            added: user.added.clone(),
        }];
    }
    // Overlapping: delete what the user deleted outside the remote's
    // change, then insert where the user typed.
    let mut ops = Vec::new();
    let after_from = user.start.max(remote.end());
    if user.end() > after_from {
        ops.push(Splice {
            start: shift(after_from),
            removed: user.end().saturating_sub(after_from),
            added: String::new(),
        });
    }
    let at = if user.start < remote.start {
        let before = remote.start.min(user.end()).saturating_sub(user.start);
        if before > 0 {
            ops.push(Splice {
                start: user.start,
                removed: before,
                added: String::new(),
            });
        }
        user.start
    } else {
        remote.start.saturating_add(remote.added_len())
    };
    if !user.added.is_empty() {
        ops.push(Splice {
            start: at,
            removed: 0,
            added: user.added.clone(),
        });
    }
    ops
}

/// `text` with `ops` applied in order.
#[must_use]
pub fn apply(text: &str, ops: &[Splice]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for op in ops {
        let start = op.start.min(chars.len());
        let end = op.end().min(chars.len());
        chars.splice(start..end, op.added.chars());
    }
    chars.into_iter().collect()
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

    /// The user typed `typed` in a text area showing `shown`, while the
    /// document went from `shown` to `doc`: what the document reads after.
    fn merged(shown: &str, typed: &str, doc: &str) -> String {
        let user = splice(shown, typed).unwrap();
        apply(doc, &rebase(&user, splice(shown, doc).as_ref()))
    }

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

    #[test]
    fn typing_with_nothing_remote_is_the_typing() {
        assert_eq!(merged("abc", "abXc", "abc"), "abXc");
    }

    /// The bug a controlled text area had: a remote edit landed after the
    /// keystroke, and diffing the document against the text area undid it.
    #[test]
    fn a_remote_edit_that_landed_meanwhile_is_kept() {
        // Remote appended a line; the user typed at the start.
        assert_eq!(
            merged("one\ntwo\n", ">one\ntwo\n", "one\ntwo\nthree\n"),
            ">one\ntwo\nthree\n"
        );
        // Remote inserted at the start; the user typed at the end.
        assert_eq!(
            merged("one\ntwo\n", "one\ntwo\n!", "zero\none\ntwo\n"),
            "zero\none\ntwo\n!"
        );
        // Remote deleted a word before the user's.
        assert_eq!(merged("a bb c", "a bb cd", "a c"), "a cd");
    }

    #[test]
    fn overlapping_edits_keep_both_sides() {
        // Both deleted around the same word; the user also typed.
        let out = merged("one two three", "one 2 three", "one TWO three");
        assert!(out.contains("TWO"), "{out}");
        assert!(out.contains('2'), "{out}");
        assert!(out.starts_with("one ") && out.ends_with(" three"), "{out}");
        // A user's delete across the remote's change removes only text
        // the remote did not touch.
        let out = merged("abcdef", "af", "abXef");
        assert_eq!(out, "aXf");
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
