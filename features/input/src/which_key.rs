//! Which-key: named prefix trees, and the popup model of what can follow
//! a typed prefix.
//!
//! A [`WhichKeyTreeDef`] is how a keybind file writes a group of
//! sequences under one prefix (`z` "Zoom" → `t` "Toggle tracks"). It
//! becomes plain keymap bindings ([`bindings`]) plus the labels the keymap
//! itself does not keep ([`labels`]). While a prefix is pending,
//! [`crate::InputProcessor::which_key`] turns the trie below it into a
//! [`WhichKey`] for the app to render however it likes.

use std::collections::HashMap;

use facet::Facet;

use crate::key::KeyChord;
use crate::notation::translate_sequence;
use crate::trie::{KeyTrie, LeafAction, TrieNode};

/// Labels by the key sequence exactly as the keymap binds it.
pub type WhichKeyLabels = HashMap<Vec<KeyChord>, String>;

/// A named prefix tree, as a keybind file writes it.
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyTreeDef {
    /// The prefix that opens the tree, in vim notation (`z`, `<S-m>`).
    pub prefix: String,
    /// The popup title for the prefix.
    pub label: String,
    /// What can follow the prefix.
    pub entries: Vec<WhichKeyEntryDef>,
    /// Children match with or without the prefix's modifiers held, so a
    /// tree opened by a shifted chord can be walked with Shift still down.
    #[facet(skip_serializing_if = Option::is_none)]
    pub case_insensitive: Option<bool>,
}

/// One key in a tree: a leaf with an `action`, or a branch with
/// `children`.
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyEntryDef {
    /// The key under the current prefix, in vim notation.
    pub key: String,
    pub label: String,
    /// The action id of a leaf.
    #[facet(skip_serializing_if = Option::is_none)]
    pub action: Option<String>,
    /// The entries of a branch.
    #[facet(skip_serializing_if = Option::is_none)]
    #[allow(clippy::use_self, reason = "the Facet derive cannot see `Self`")]
    pub children: Option<Vec<WhichKeyEntryDef>>,
}

/// Every leaf of `trees` as a keymap binding: space-separated key
/// sequence (keymap notation) to action id. Entries with neither an
/// action nor children bind nothing.
#[must_use]
pub fn bindings(trees: &[WhichKeyTreeDef]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    walk_trees(trees, &mut |seq, entry| {
        let is_branch = entry.children.as_deref().is_some_and(|c| !c.is_empty());
        if let (false, Some(action)) = (is_branch, &entry.action) {
            out.insert(seq.to_owned(), action.clone());
        }
    });
    out
}

/// The labels of `trees`' prefixes, groups and leaves, keyed by the key
/// sequence the keymap binds (`z` → "Zoom", `z t` → "Toggle tracks").
#[must_use]
pub fn labels(trees: &[WhichKeyTreeDef]) -> WhichKeyLabels {
    let mut out = WhichKeyLabels::new();
    let mut put = |seq: &str, label: &str| {
        if let Ok(chords) = crate::parse_key_sequence(seq) {
            out.insert(chords, label.to_owned());
        }
    };
    for tree in trees {
        put(&translate_sequence(&tree.prefix), &tree.label);
    }
    walk_trees(trees, &mut |seq, entry| put(seq, &entry.label));
    out
}

/// Visit every entry of `trees` under each key sequence it is bound at
/// (one per case-insensitive variant).
fn walk_trees(trees: &[WhichKeyTreeDef], visit: &mut dyn FnMut(&str, &WhichKeyEntryDef)) {
    for tree in trees {
        let prefix = translate_sequence(&tree.prefix);
        let style = VariantStyle {
            case_insensitive: tree.case_insensitive.unwrap_or(false),
            anchor_mods: anchor_modifiers(&prefix),
        };
        walk_entries(&prefix, &tree.entries, &style, visit);
    }
}

/// How a tree's keys expand into trie keys.
struct VariantStyle {
    case_insensitive: bool,
    /// The modifiers of the prefix's first chord.
    anchor_mods: Vec<&'static str>,
}

fn walk_entries(
    prefix: &str,
    entries: &[WhichKeyEntryDef],
    style: &VariantStyle,
    visit: &mut dyn FnMut(&str, &WhichKeyEntryDef),
) {
    for entry in entries {
        for variant in key_variants(&entry.key, style) {
            let seq = format!("{prefix} {variant}");
            visit(&seq, entry);
            if let Some(children) = entry.children.as_deref() {
                walk_entries(&seq, children, style, visit);
            }
        }
    }
}

/// The trie keys one entry key is bound at. A bare letter in a
/// case-insensitive tree is also bound with the anchor's modifiers held.
fn key_variants(key: &str, style: &VariantStyle) -> Vec<String> {
    let translated = translate_sequence(key);
    let mut chars = key.chars();
    let is_bare_letter =
        matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_alphabetic());
    if !(style.case_insensitive && is_bare_letter) {
        return vec![translated];
    }
    let lower = translated.to_lowercase();
    let shift = style.anchor_mods.contains(&"Shift");
    let others: Vec<&str> = style
        .anchor_mods
        .iter()
        .copied()
        .filter(|m| *m != "Shift")
        .collect();
    let mut out = vec![lower.clone()];
    if shift {
        out.push(format!("Shift+{lower}"));
    }
    if !others.is_empty() {
        out.push(format!("{}+{lower}", others.join("+")));
        if shift {
            out.push(format!("{}+Shift+{lower}", others.join("+")));
        }
    }
    out
}

/// The modifiers of a translated prefix's first chord.
fn anchor_modifiers(translated_prefix: &str) -> Vec<&'static str> {
    let first = translated_prefix.split(' ').next().unwrap_or_default();
    ["Ctrl", "Alt", "Meta", "Shift"]
        .into_iter()
        .filter(|tag| first.contains(&format!("{tag}+")))
        .collect()
}

/// The which-key popup: what can follow the typed prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhichKey<A = String> {
    /// The prefix's label, when the labels name it.
    pub title: Option<String>,
    /// The chords typed so far.
    pub typed: Vec<KeyChord>,
    /// The next keys, sorted by their printed chord.
    pub entries: Vec<WhichKeyEntry<A>>,
}

/// One next key in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyEntry<A = String> {
    /// The key to press next. Print it with `Display` (`Ctrl+t`).
    pub chord: KeyChord,
    /// The label for this key: from the labels, else a group's own name.
    /// `None` when neither names it (show the action id).
    pub label: Option<String>,
    /// What pressing it does.
    pub next: WhichKeyNext<A>,
}

/// What a which-key entry leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhichKeyNext<A = String> {
    /// A further group of keys.
    Group,
    /// A complete binding.
    Leaf(LeafAction<A>),
}

impl<A> WhichKeyEntry<A> {
    /// Whether the entry opens a further group.
    #[must_use]
    pub const fn is_group(&self) -> bool {
        matches!(self.next, WhichKeyNext::Group)
    }

    /// The action a leaf runs, if it runs one action.
    #[must_use]
    pub const fn action(&self) -> Option<&A> {
        match &self.next {
            WhichKeyNext::Leaf(LeafAction::Action(action)) => Some(action),
            _ => None,
        }
    }
}

/// The popup for `typed`, from the trie nodes reached by it (earlier
/// nodes win a key both bind). `None` when no node has anything below.
#[must_use]
pub fn menu<A: Clone>(
    nodes: &[&TrieNode<A>],
    typed: &[KeyChord],
    labels: &WhichKeyLabels,
) -> Option<WhichKey<A>> {
    let mut seen: HashMap<&KeyChord, &KeyTrie<A>> = HashMap::new();
    for (chord, child) in nodes.iter().flat_map(|node| &node.children) {
        seen.entry(chord).or_insert(child);
    }
    if seen.is_empty() {
        return None;
    }
    let mut entries: Vec<WhichKeyEntry<A>> = seen
        .into_iter()
        .map(|(chord, child)| entry_for(typed, chord, child, labels))
        .collect();
    entries.sort_by_cached_key(|entry| entry.chord.to_string());
    Some(WhichKey {
        title: labels.get(typed).cloned(),
        typed: typed.to_vec(),
        entries,
    })
}

fn entry_for<A: Clone>(
    typed: &[KeyChord],
    chord: &KeyChord,
    child: &KeyTrie<A>,
    labels: &WhichKeyLabels,
) -> WhichKeyEntry<A> {
    let path: Vec<KeyChord> = typed
        .iter()
        .chain(std::iter::once(chord))
        .cloned()
        .collect();
    let label = labels.get(&path).cloned();
    let (label, next) = match child {
        KeyTrie::Node(node) => (
            label.or_else(|| Some(node.name.clone()).filter(|n| !n.is_empty())),
            WhichKeyNext::Group,
        ),
        KeyTrie::Leaf(leaf) => (label, WhichKeyNext::Leaf(leaf.clone())),
    };
    WhichKeyEntry {
        chord: chord.clone(),
        label,
        next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_key_sequence;

    fn leaf(key: &str, label: &str, action: &str) -> WhichKeyEntryDef {
        WhichKeyEntryDef {
            key: key.into(),
            label: label.into(),
            action: Some(action.into()),
            children: None,
        }
    }

    fn visibility_tree() -> WhichKeyTreeDef {
        WhichKeyTreeDef {
            prefix: "v".into(),
            label: "Visibility".into(),
            case_insensitive: None,
            entries: vec![
                leaf("d", "Drums", "vis:drums"),
                WhichKeyEntryDef {
                    key: "e".into(),
                    label: "EQ".into(),
                    action: None,
                    children: Some(vec![leaf("r", "Rescue", "fx:rescue")]),
                },
            ],
        }
    }

    #[test]
    fn which_key_tree_flattens() {
        let flat = bindings(&[visibility_tree()]);
        assert_eq!(flat.get("v d").map(String::as_str), Some("vis:drums"));
        assert_eq!(flat.get("v e r").map(String::as_str), Some("fx:rescue"));
        assert_eq!(flat.get("v"), None);
        assert_eq!(flat.get("v e"), None, "a branch binds nothing itself");
    }

    #[test]
    fn labels_name_the_prefix_groups_and_leaves() {
        let names = labels(&[visibility_tree()]);
        let at = |seq: &str| names.get(&parse_key_sequence(seq).unwrap()).cloned();
        assert_eq!(at("v").as_deref(), Some("Visibility"));
        assert_eq!(at("v e").as_deref(), Some("EQ"));
        assert_eq!(at("v e r").as_deref(), Some("Rescue"));
    }

    #[test]
    fn a_case_insensitive_tree_binds_with_the_anchor_modifiers_too() {
        let tree = WhichKeyTreeDef {
            prefix: "<S-m>".into(),
            label: "Markers".into(),
            case_insensitive: Some(true),
            entries: vec![leaf("a", "Add", "marker.add")],
        };
        let flat = bindings(&[tree]);
        assert_eq!(
            flat.get("Shift+m a").map(String::as_str),
            Some("marker.add")
        );
        assert_eq!(
            flat.get("Shift+m Shift+a").map(String::as_str),
            Some("marker.add")
        );
    }

    #[test]
    fn the_menu_lists_next_keys_with_labels_and_kinds() {
        let tree = visibility_tree();
        let mut keys: KeyTrie = KeyTrie::new();
        for (seq, action) in bindings(std::slice::from_ref(&tree)) {
            keys.bind_sequence(&parse_key_sequence(&seq).unwrap(), action);
        }
        let typed = parse_key_sequence("v").unwrap();
        let node = keys.root().unwrap().descend(&typed).unwrap();

        let popup = menu(&[node], &typed, &labels(&[tree])).unwrap();

        assert_eq!(popup.title.as_deref(), Some("Visibility"));
        let summary: Vec<(String, Option<String>, bool)> = popup
            .entries
            .iter()
            .map(|e| (e.chord.to_string(), e.label.clone(), e.is_group()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("d".into(), Some("Drums".into()), false),
                ("e".into(), Some("EQ".into()), true),
            ]
        );
        assert_eq!(
            popup
                .entries
                .first()
                .and_then(WhichKeyEntry::action)
                .map(String::as_str),
            Some("vis:drums")
        );
    }
}
