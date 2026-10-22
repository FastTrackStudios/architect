//! The key trie: key sequences to [`LeafAction`]s.
//!
//! Single-key bindings are leaves under the root; multi-key sequences
//! (`g g`, a which-key tree's `z t`) are interior [`TrieNode`]s.

use std::collections::HashMap;

use crate::key::KeyChord;
use crate::mode::ModeId;

/// A trie position: a leaf binding or an interior node.
#[derive(Debug, Clone)]
pub enum KeyTrie<A = String> {
    /// A complete binding.
    Leaf(LeafAction<A>),
    /// A prefix with further bindings below it.
    Node(TrieNode<A>),
}

/// What a whole key sequence resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrieLookup<A = String> {
    /// The sequence is a complete binding.
    Match(LeafAction<A>),
    /// The sequence is a prefix of longer bindings.
    Prefix,
    /// The sequence is bound to nothing.
    Miss,
}

/// An interior trie node.
#[derive(Debug, Clone)]
pub struct TrieNode<A = String> {
    /// The group's name (`goto`, `window`); empty for anonymous prefixes.
    pub name: String,
    /// The node is a sticky sub-mode.
    pub sticky: bool,
    /// Bindings below this prefix, by the next chord.
    pub children: HashMap<KeyChord, KeyTrie<A>>,
    /// Fallthrough action when no child matches.
    pub default: Option<Box<LeafAction<A>>>,
}

/// What a complete binding does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafAction<A = String> {
    /// Run an action.
    Action(A),
    /// Switch the base mode.
    SwitchMode(ModeId),
    /// Push a sub-mode.
    PushMode(ModeId),
    /// Start an operator (`d`), completed by a motion or text object.
    Operator(String),
    /// A motion, composing with a pending operator.
    Motion(String),
    /// A text object, composing with a pending operator.
    TextObject(String),
    /// Run several actions in order.
    Sequence(Vec<A>),
    /// Explicitly unbound: blocks a parent's or default binding.
    Unbind,
}

impl<A> TrieNode<A> {
    /// An empty node.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sticky: false,
            children: HashMap::new(),
            default: None,
        }
    }

    #[must_use]
    pub const fn with_sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }

    #[must_use]
    pub fn with_default(mut self, action: LeafAction<A>) -> Self {
        self.default = Some(Box::new(action));
        self
    }

    /// Bind `action` at `path` below this node, creating intermediate
    /// nodes. A leaf in the way becomes a node.
    pub fn insert(&mut self, path: &[KeyChord], action: LeafAction<A>) {
        let Some((chord, rest)) = path.split_first() else {
            return;
        };
        if rest.is_empty() {
            self.children.insert(chord.clone(), KeyTrie::Leaf(action));
            return;
        }
        let child = self
            .children
            .entry(chord.clone())
            .or_insert_with(|| KeyTrie::Node(Self::new("")));
        if let KeyTrie::Leaf(_) = child {
            *child = KeyTrie::Node(Self::new(""));
        }
        if let KeyTrie::Node(node) = child {
            node.insert(rest, action);
        }
    }

    /// The child under one chord.
    #[must_use]
    pub fn get(&self, chord: &KeyChord) -> Option<&KeyTrie<A>> {
        self.children.get(chord)
    }

    /// The node reached by walking `path` from here, if every step is a node.
    #[must_use]
    pub fn descend(&self, path: &[KeyChord]) -> Option<&Self> {
        path.iter()
            .try_fold(self, |node, chord| match node.get(chord) {
                Some(KeyTrie::Node(next)) => Some(next),
                _ => None,
            })
    }

    /// Merge `other` into this node: its children win on conflict, nodes
    /// on both sides merge recursively, siblings are kept.
    pub fn merge(&mut self, other: Self) {
        for (chord, incoming) in other.children {
            match (self.children.get_mut(&chord), incoming) {
                (Some(KeyTrie::Node(existing)), KeyTrie::Node(incoming)) => {
                    existing.merge(incoming);
                }
                (_, incoming) => {
                    self.children.insert(chord, incoming);
                }
            }
        }
        if other.default.is_some() {
            self.default = other.default;
        }
        if !other.name.is_empty() {
            self.name = other.name;
        }
    }

    /// Every binding below this node as `(keys, leaf)`, in no set order.
    #[must_use]
    pub fn bindings(&self) -> Vec<(Vec<KeyChord>, &LeafAction<A>)> {
        let mut found = Vec::new();
        collect_bindings(self, &mut Vec::new(), &mut found);
        found
    }
}

/// [`TrieNode::bindings`] under `node`, reached by `path`.
fn collect_bindings<'a, A>(
    node: &'a TrieNode<A>,
    path: &mut Vec<KeyChord>,
    found: &mut Vec<(Vec<KeyChord>, &'a LeafAction<A>)>,
) {
    for (chord, child) in &node.children {
        path.push(chord.clone());
        match child {
            KeyTrie::Leaf(leaf) => found.push((path.clone(), leaf)),
            KeyTrie::Node(next) => collect_bindings(next, path, found),
        }
        path.pop();
    }
}

impl<A: Clone> KeyTrie<A> {
    /// An empty root.
    #[must_use]
    pub fn new() -> Self {
        Self::Node(TrieNode::new("root"))
    }

    /// Bind one chord to an action.
    pub fn bind(&mut self, chord: KeyChord, action: A) {
        self.bind_leaf(&[chord], LeafAction::Action(action));
    }

    /// Bind a key sequence to an action.
    pub fn bind_sequence(&mut self, sequence: &[KeyChord], action: A) {
        self.bind_leaf(sequence, LeafAction::Action(action));
    }

    /// Bind one chord to a base-mode switch.
    pub fn bind_mode_switch(&mut self, chord: KeyChord, mode: ModeId) {
        self.bind_leaf(&[chord], LeafAction::SwitchMode(mode));
    }

    /// Bind a key sequence to any leaf. An empty sequence binds nothing.
    pub fn bind_leaf(&mut self, sequence: &[KeyChord], action: LeafAction<A>) {
        if sequence.is_empty() {
            return;
        }
        self.root_mut().insert(sequence, action);
    }

    /// The root node, if this trie is not a bare leaf.
    #[must_use]
    pub const fn root(&self) -> Option<&TrieNode<A>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    /// Resolve a whole key sequence.
    #[must_use]
    pub fn lookup(&self, sequence: &[KeyChord]) -> TrieLookup<A> {
        let (Some(root), Some((last, init))) = (self.root(), sequence.split_last()) else {
            return TrieLookup::Miss;
        };
        match root.descend(init).and_then(|node| node.get(last)) {
            Some(Self::Leaf(action)) => TrieLookup::Match(action.clone()),
            Some(Self::Node(_)) => TrieLookup::Prefix,
            None => TrieLookup::Miss,
        }
    }

    /// The root node; a bare-leaf trie is replaced by an empty root first.
    fn root_mut(&mut self) -> &mut TrieNode<A> {
        match self {
            Self::Node(node) => node,
            Self::Leaf(_) => {
                *self = Self::new();
                self.root_mut()
            }
        }
    }
}

impl<A: Clone> Default for KeyTrie<A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{KeyCode, Modifiers};

    fn chord(c: &str) -> KeyChord {
        KeyChord::plain(KeyCode::Character(c.to_owned()))
    }

    fn ctrl_chord(c: &str) -> KeyChord {
        KeyChord::new(KeyCode::Character(c.to_owned()), Modifiers::CTRL)
    }

    fn act(id: &str) -> LeafAction {
        LeafAction::Action(id.to_owned())
    }

    fn is_action(trie: Option<&KeyTrie>, want: &str) -> bool {
        matches!(trie, Some(KeyTrie::Leaf(LeafAction::Action(id))) if id == want)
    }

    #[test]
    fn test_trie_single_key_lookup_returns_leaf() {
        let mut root = TrieNode::new("root");
        root.insert(&[chord("j")], act("move.down"));
        assert!(is_action(root.get(&chord("j")), "move.down"));
    }

    #[test]
    fn test_trie_two_key_sequence_first_returns_node_second_returns_leaf() {
        let mut root = TrieNode::new("root");
        root.insert(&[chord("g"), chord("g")], act("goto.top"));
        let Some(KeyTrie::Node(node)) = root.get(&chord("g")) else {
            panic!("expected Node for first key of sequence");
        };
        assert!(is_action(node.get(&chord("g")), "goto.top"));
    }

    #[test]
    fn test_trie_merge_override_behavior() {
        let mut base = TrieNode::new("base");
        base.insert(&[chord("j")], act("move.down"));
        base.insert(&[chord("k")], act("move.up"));
        let mut overlay = TrieNode::new("overlay");
        overlay.insert(&[chord("j")], act("custom.down"));

        base.merge(overlay);

        assert!(is_action(base.get(&chord("j")), "custom.down"));
        assert!(is_action(base.get(&chord("k")), "move.up"));
    }

    #[test]
    fn test_trie_sticky_node_flag_preserved() {
        let sticky_node = TrieNode::new("goto").with_sticky(true);
        let mut root: TrieNode = TrieNode::new("root");
        root.children.insert(chord("g"), KeyTrie::Node(sticky_node));
        let Some(KeyTrie::Node(n)) = root.get(&chord("g")) else {
            panic!("expected Node");
        };
        assert!(n.sticky);
        assert_eq!(n.name, "goto");
    }

    #[test]
    fn test_trie_missing_key_returns_none() {
        let root: TrieNode = TrieNode::new("root");
        assert!(root.get(&chord("x")).is_none());
    }

    #[test]
    fn test_trie_insert_with_modifiers() {
        let mut root = TrieNode::new("root");
        root.insert(&[ctrl_chord("s")], act("file.save"));
        assert!(root.get(&ctrl_chord("s")).is_some());
        assert!(root.get(&chord("s")).is_none());
    }

    #[test]
    fn test_trie_leaf_action_variants() {
        let mut root: TrieNode = TrieNode::new("root");
        root.insert(&[chord("i")], LeafAction::SwitchMode(ModeId::insert()));
        root.insert(&[chord("d")], LeafAction::Operator("delete".into()));
        root.insert(&[chord("w")], LeafAction::Motion("word".into()));
        root.insert(
            &[chord("q")],
            LeafAction::Sequence(vec!["save".into(), "quit".into()]),
        );
        assert!(matches!(
            root.get(&chord("i")),
            Some(KeyTrie::Leaf(LeafAction::SwitchMode(_)))
        ));
        assert!(matches!(
            root.get(&chord("d")),
            Some(KeyTrie::Leaf(LeafAction::Operator(s))) if s == "delete"
        ));
        assert!(matches!(
            root.get(&chord("w")),
            Some(KeyTrie::Leaf(LeafAction::Motion(s))) if s == "word"
        ));
        assert!(matches!(
            root.get(&chord("q")),
            Some(KeyTrie::Leaf(LeafAction::Sequence(v))) if v.len() == 2
        ));
    }

    #[test]
    fn test_trie_merge_deep_nodes() {
        let mut base = TrieNode::new("root");
        base.insert(&[chord("g"), chord("g")], act("goto.top"));
        let mut overlay = TrieNode::new("root");
        overlay.insert(&[chord("g"), chord("d")], act("goto.definition"));

        base.merge(overlay);

        let Some(KeyTrie::Node(g_node)) = base.get(&chord("g")) else {
            panic!("expected Node for 'g'");
        };
        assert!(is_action(g_node.get(&chord("g")), "goto.top"));
        assert!(is_action(g_node.get(&chord("d")), "goto.definition"));
    }

    #[test]
    fn test_trie_default_fallthrough() {
        let node: TrieNode = TrieNode::new("fallthrough").with_default(act("default.action"));
        assert_eq!(node.default.as_deref(), Some(&act("default.action")));
    }

    #[test]
    fn test_trie_unbind_variant() {
        let mut root: TrieNode = TrieNode::new("root");
        root.insert(&[chord("q")], LeafAction::Unbind);
        assert!(matches!(
            root.get(&chord("q")),
            Some(KeyTrie::Leaf(LeafAction::Unbind))
        ));
    }

    #[test]
    fn lookup_reports_match_prefix_and_miss() {
        let mut trie: KeyTrie = KeyTrie::new();
        trie.bind_sequence(&[chord("g"), chord("g")], "top".into());
        trie.bind(chord("j"), "down".into());
        assert_eq!(trie.lookup(&[chord("j")]), TrieLookup::Match(act("down")));
        assert_eq!(trie.lookup(&[chord("g")]), TrieLookup::Prefix);
        assert_eq!(
            trie.lookup(&[chord("g"), chord("g")]),
            TrieLookup::Match(act("top"))
        );
        assert_eq!(trie.lookup(&[chord("j"), chord("j")]), TrieLookup::Miss);
        assert_eq!(trie.lookup(&[]), TrieLookup::Miss);
    }

    #[test]
    fn bindings_lists_every_leaf_with_its_keys() {
        let mut trie: KeyTrie = KeyTrie::new();
        trie.bind_sequence(&[chord("z"), chord("t")], "zoom.tracks".into());
        trie.bind(chord("j"), "down".into());
        let mut found: Vec<String> = trie
            .root()
            .unwrap()
            .bindings()
            .into_iter()
            .map(|(keys, leaf)| {
                let keys: Vec<String> = keys.iter().map(ToString::to_string).collect();
                format!("{} -> {leaf:?}", keys.join(" "))
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                "j -> Action(\"down\")".to_owned(),
                "z t -> Action(\"zoom.tracks\")".to_owned()
            ]
        );
    }
}
