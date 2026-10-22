//! A keystroke accumulator for multi-key sequences, with a timeout.
//!
//! [`SequenceState`] buffers chords and walks a [`TrieNode`] one step at
//! a time, reporting a match, a pending prefix or a miss;
//! [`SequenceState::timeout_expired`] reports a stale sequence.
//!
//! The clock is the caller's: every call that needs the time takes `now`
//! (time since any epoch the app picks, monotonic), so the accumulator
//! runs the same on wasm, where `std::time::Instant` is unavailable, and
//! in tests.

use std::time::Duration;

use crate::key::KeyChord;
use crate::trie::{KeyTrie, LeafAction, TrieNode};

/// What one chord did to the sequence.
#[derive(Debug)]
pub enum SequenceResult<'a, A = String> {
    /// The sequence completed a binding.
    Matched(LeafAction<A>),
    /// More keys are needed; feed the next chord against this node.
    Pending(&'a TrieNode<A>),
    /// The chord matched nothing at this position.
    NoMatch,
}

/// The default time a pending sequence waits for its next key.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// Buffers chords and walks a trie.
#[derive(Debug)]
pub struct SequenceState {
    pending_keys: Vec<KeyChord>,
    /// When the first chord of the pending sequence arrived.
    started_at: Option<Duration>,
    timeout: Duration,
}

impl SequenceState {
    /// An empty accumulator with [`DEFAULT_TIMEOUT`].
    #[must_use]
    pub const fn new() -> Self {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    /// An empty accumulator with a custom timeout.
    #[must_use]
    pub const fn with_timeout(timeout: Duration) -> Self {
        Self {
            pending_keys: Vec::new(),
            started_at: None,
            timeout,
        }
    }

    /// Feed `chord` at `now`, looking it up in `current`: the root when
    /// starting a sequence, else the node of the previous
    /// [`SequenceResult::Pending`].
    pub fn feed<'a, A: Clone>(
        &mut self,
        chord: KeyChord,
        current: &'a TrieNode<A>,
        now: Duration,
    ) -> SequenceResult<'a, A> {
        if self.pending_keys.is_empty() {
            self.started_at = Some(now);
        }
        match current.get(&chord) {
            Some(KeyTrie::Leaf(action)) => {
                self.reset();
                SequenceResult::Matched(action.clone())
            }
            Some(KeyTrie::Node(node)) => {
                self.pending_keys.push(chord);
                SequenceResult::Pending(node)
            }
            None => {
                self.reset();
                SequenceResult::NoMatch
            }
        }
    }

    /// The buffered chords if the pending sequence has waited at least the
    /// timeout by `now` (clearing it), else `None`.
    pub fn timeout_expired(&mut self, now: Duration) -> Option<Vec<KeyChord>> {
        let started = self.started_at?;
        if now.saturating_sub(started) < self.timeout {
            return None;
        }
        self.started_at = None;
        Some(std::mem::take(&mut self.pending_keys))
    }

    /// Whether a sequence is in progress.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        !self.pending_keys.is_empty()
    }

    /// The buffered chords.
    #[must_use]
    pub fn pending_keys(&self) -> &[KeyChord] {
        &self.pending_keys
    }

    /// The buffered chords for a status line: `g d`, `<C-g>`.
    #[must_use]
    pub fn pending_display(&self) -> String {
        self.pending_keys
            .iter()
            .map(vim_label)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Drop the buffered sequence.
    pub fn reset(&mut self) {
        self.pending_keys.clear();
        self.started_at = None;
    }
}

impl Default for SequenceState {
    fn default() -> Self {
        Self::new()
    }
}

/// `g`, or `<C-g>` when modifiers are held.
fn vim_label(chord: &KeyChord) -> String {
    let label = chord.short_label();
    if chord.modifiers == crate::key::Modifiers::NONE {
        label
    } else {
        format!("<{label}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{KeyCode, Modifiers};

    const T0: Duration = Duration::ZERO;

    fn chord(c: &str) -> KeyChord {
        KeyChord::plain(KeyCode::Character(c.to_owned()))
    }

    fn act(id: &str) -> LeafAction {
        LeafAction::Action(id.to_owned())
    }

    fn make_test_trie() -> TrieNode {
        let mut root = TrieNode::new("root");
        root.insert(&[chord("j")], act("move.down"));
        root.insert(&[chord("g"), chord("g")], act("goto.top"));
        root.insert(&[chord("g"), chord("d")], act("goto.definition"));
        root
    }

    fn matched(result: &SequenceResult<'_>, want: &str) -> bool {
        matches!(result, SequenceResult::Matched(LeafAction::Action(id)) if id == want)
    }

    #[test]
    fn test_sequence_single_key_match_returns_matched() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        assert!(matched(&state.feed(chord("j"), &root, T0), "move.down"));
        assert!(!state.is_pending());
    }

    #[test]
    fn test_sequence_two_key_first_pending_second_matched() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        let SequenceResult::Pending(node) = state.feed(chord("g"), &root, T0) else {
            panic!("expected Pending");
        };
        assert!(state.is_pending());
        assert_eq!(state.pending_display(), "g");
        assert_eq!(state.pending_keys(), &[chord("g")]);

        assert!(matched(&state.feed(chord("g"), node, T0), "goto.top"));
        assert!(!state.is_pending());
    }

    #[test]
    fn test_sequence_invalid_key_after_pending_returns_no_match() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        let SequenceResult::Pending(node) = state.feed(chord("g"), &root, T0) else {
            panic!("expected Pending");
        };
        assert!(matches!(
            state.feed(chord("x"), node, T0),
            SequenceResult::NoMatch
        ));
        assert!(!state.is_pending());
    }

    #[test]
    fn test_sequence_timeout_on_pending_returns_accumulated_keys() {
        let root = make_test_trie();
        let mut state = SequenceState::with_timeout(Duration::from_millis(500));
        let start = Duration::from_secs(10);
        assert!(matches!(
            state.feed(chord("g"), &root, start),
            SequenceResult::Pending(_)
        ));

        let later = start.saturating_add(Duration::from_millis(500));
        assert_eq!(state.timeout_expired(later), Some(vec![chord("g")]));
        assert!(!state.is_pending());
    }

    #[test]
    fn test_sequence_no_timeout_when_not_pending() {
        let mut state = SequenceState::new();
        assert!(state.timeout_expired(Duration::from_secs(100)).is_none());
    }

    #[test]
    fn test_sequence_no_timeout_when_not_expired() {
        let root = make_test_trie();
        let mut state = SequenceState::with_timeout(Duration::from_secs(60));
        let _ = state.feed(chord("g"), &root, T0);
        assert!(state.timeout_expired(Duration::from_secs(59)).is_none());
        assert!(state.is_pending());
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_expire() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        let _ = state.feed(chord("g"), &root, Duration::from_secs(5));
        assert!(state.timeout_expired(Duration::from_secs(1)).is_none());
    }

    #[test]
    fn test_sequence_reset_clears_state() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        let _ = state.feed(chord("g"), &root, T0);
        state.reset();
        assert!(!state.is_pending());
        assert_eq!(state.pending_display(), "");
    }

    #[test]
    fn test_sequence_pending_display_multiple_keys() {
        let mut root = TrieNode::new("root");
        root.insert(&[chord("g"), chord("d"), chord("x")], act("some.action"));
        let mut state = SequenceState::new();
        let SequenceResult::Pending(node1) = state.feed(chord("g"), &root, T0) else {
            panic!("expected Pending");
        };
        assert!(matches!(
            state.feed(chord("d"), node1, T0),
            SequenceResult::Pending(_)
        ));
        assert_eq!(state.pending_display(), "g d");
    }

    #[test]
    fn test_sequence_pending_display_with_modifiers() {
        let ctrl_g = KeyChord::new(KeyCode::Character("g".into()), Modifiers::CTRL);
        let mut root = TrieNode::new("root");
        root.insert(&[ctrl_g.clone(), chord("d")], act("some.action"));
        let mut state = SequenceState::new();
        let _ = state.feed(ctrl_g, &root, T0);
        assert_eq!(state.pending_display(), "<C-g>");
    }

    #[test]
    fn test_sequence_matched_resets_for_next_sequence() {
        let root = make_test_trie();
        let mut state = SequenceState::new();
        assert!(matches!(
            state.feed(chord("j"), &root, T0),
            SequenceResult::Matched(_)
        ));
        assert_eq!(state.pending_display(), "");
        assert!(matches!(
            state.feed(chord("g"), &root, T0),
            SequenceResult::Pending(_)
        ));
        assert_eq!(state.pending_display(), "g");
    }
}
