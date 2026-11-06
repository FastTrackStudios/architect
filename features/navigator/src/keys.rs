//! The keyboard map (Notebook Navigator's `src/utils/keyboardShortcuts.ts`).
//!
//! Every key the panes answer is a [`KeyAction`] with a stable id
//! (`pane:move-up`), bound to one or more [`Hotkey`]s a person can change.
//! A [`KeyMap`] is Notebook Navigator's defaults, with Vim's `h` `j` `k`
//! `l` too when the settings say (`vim_keys`), and a person's own over
//! them ([`KeyBinding`]s, kept with the view's state).
//!
//! Architect's panes also open a note with Return (in a tab with ⇧, beside
//! with ⌘) and pin with ⌘⇧P: `list:open`, `list:open-tab`,
//! `list:open-split`, `list:pin`. Renaming is F2 everywhere, as Return
//! opens (Notebook Navigator renames with Return on a Mac).

use std::collections::BTreeMap;

use facet::Facet;

/// What a key does in a pane (`KeyboardShortcutAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyAction {
    PaneMoveUp,
    PaneMoveDown,
    PanePageUp,
    PanePageDown,
    PaneHome,
    PaneEnd,
    PaneRename,
    DeleteSelected,
    NavCollapseOrParent,
    NavExpandOrFocusList,
    NavFocusList,
    ListFocusNavigation,
    ListFocusEditor,
    ListSelectAll,
    ListExtendSelectionUp,
    ListExtendSelectionDown,
    ListManualSortUp,
    ListManualSortDown,
    ListRangeToStart,
    ListRangeToEnd,
    ListOpen,
    ListOpenTab,
    ListOpenSplit,
    ListPin,
    SearchFocusList,
    SearchFocusNavigation,
    SearchClose,
}

/// Where a key is pressed: which actions it can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScope {
    Navigation,
    List,
    Search,
}

impl KeyAction {
    /// Every action, in Notebook Navigator's order, architect's last.
    pub const ALL: [Self; 27] = [
        Self::PaneMoveUp,
        Self::PaneMoveDown,
        Self::PanePageUp,
        Self::PanePageDown,
        Self::PaneHome,
        Self::PaneEnd,
        Self::PaneRename,
        Self::NavCollapseOrParent,
        Self::NavExpandOrFocusList,
        Self::NavFocusList,
        Self::DeleteSelected,
        Self::ListFocusNavigation,
        Self::ListFocusEditor,
        Self::ListSelectAll,
        Self::ListExtendSelectionUp,
        Self::ListExtendSelectionDown,
        Self::ListManualSortUp,
        Self::ListManualSortDown,
        Self::ListRangeToStart,
        Self::ListRangeToEnd,
        Self::SearchFocusList,
        Self::SearchFocusNavigation,
        Self::SearchClose,
        Self::ListOpen,
        Self::ListOpenTab,
        Self::ListOpenSplit,
        Self::ListPin,
    ];

    /// Its stable id.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::PaneMoveUp => "pane:move-up",
            Self::PaneMoveDown => "pane:move-down",
            Self::PanePageUp => "pane:page-up",
            Self::PanePageDown => "pane:page-down",
            Self::PaneHome => "pane:home",
            Self::PaneEnd => "pane:end",
            Self::PaneRename => "pane:rename",
            Self::DeleteSelected => "pane:delete-selected",
            Self::NavCollapseOrParent => "navigation:collapse-or-parent",
            Self::NavExpandOrFocusList => "navigation:expand-or-focus-list",
            Self::NavFocusList => "navigation:focus-list",
            Self::ListFocusNavigation => "list:focus-navigation",
            Self::ListFocusEditor => "list:focus-editor",
            Self::ListSelectAll => "list:select-all",
            Self::ListExtendSelectionUp => "list:extend-selection-up",
            Self::ListExtendSelectionDown => "list:extend-selection-down",
            Self::ListManualSortUp => "list:manual-sort-up",
            Self::ListManualSortDown => "list:manual-sort-down",
            Self::ListRangeToStart => "list:range-to-start",
            Self::ListRangeToEnd => "list:range-to-end",
            Self::ListOpen => "list:open",
            Self::ListOpenTab => "list:open-tab",
            Self::ListOpenSplit => "list:open-split",
            Self::ListPin => "list:pin",
            Self::SearchFocusList => "search:focus-list",
            Self::SearchFocusNavigation => "search:focus-navigation",
            Self::SearchClose => "search:close",
        }
    }

    /// The action with id `id`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id.trim())
    }

    /// Whether it is an action of `scope`: `pane:` in either pane,
    /// `navigation:` in the folders, `list:` in the notes, `search:` in
    /// the search box.
    #[must_use]
    pub fn in_scope(self, scope: KeyScope) -> bool {
        let id = self.id();
        match scope {
            KeyScope::Navigation => id.starts_with("pane:") || id.starts_with("navigation:"),
            KeyScope::List => id.starts_with("pane:") || id.starts_with("list:"),
            KeyScope::Search => id.starts_with("search:"),
        }
    }
}

/// A modifier a hotkey needs. `Mod` is ⌘ on a Mac and Ctrl elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier {
    Mod,
    Ctrl,
    Shift,
    Alt,
}

/// A key and the modifiers it is pressed with (Obsidian's `Hotkey`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hotkey {
    /// The key as the event names it: `ArrowUp`, `Enter`, `F2`, `a`.
    pub key: String,
    pub mods: Vec<Modifier>,
}

impl Hotkey {
    fn new(key: &str, mods: &[Modifier]) -> Self {
        Self {
            key: key.to_owned(),
            mods: mods.to_vec(),
        }
    }

    /// `Mod+Shift+P`, `ArrowUp`, `k` read (`sanitizeHotkeyEntry`):
    /// modifiers `Mod`, `Ctrl`, `Shift`, `Alt` (any case; `Cmd` is `Mod`,
    /// `Option` is `Alt`), then the key. `None` for no key or a modifier it
    /// does not know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, mods) = match parts.split_last() {
            // `Mod++`: the plus key.
            Some((&"", rest)) if text.trim().ends_with("++") => {
                ("+", rest.get(..rest.len().saturating_sub(1)).unwrap_or(&[]))
            }
            Some((key, rest)) => (*key, rest),
            None => return None,
        };
        if key.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for m in mods {
            let m = match m.to_ascii_lowercase().as_str() {
                "mod" | "cmd" | "command" | "meta" => Modifier::Mod,
                "ctrl" | "control" => Modifier::Ctrl,
                "shift" => Modifier::Shift,
                "alt" | "option" | "opt" => Modifier::Alt,
                _ => return None,
            };
            if !out.contains(&m) {
                out.push(m);
            }
        }
        out.sort();
        Some(Self {
            key: key.to_owned(),
            mods: out,
        })
    }

    /// It written as [`parse`](Self::parse) reads it.
    #[must_use]
    pub fn text(&self) -> String {
        let mut parts: Vec<&str> = self
            .mods
            .iter()
            .map(|m| match m {
                Modifier::Mod => "Mod",
                Modifier::Ctrl => "Ctrl",
                Modifier::Shift => "Shift",
                Modifier::Alt => "Alt",
            })
            .collect();
        parts.push(&self.key);
        parts.join("+")
    }

    /// Whether `e` is it (`hotkeyMatches`): the key in any case, and
    /// exactly its modifiers (`Mod` as ⌘ on a Mac, Ctrl elsewhere).
    #[must_use]
    pub fn matches(&self, e: &KeyEvent, mac: bool) -> bool {
        if !self.key.eq_ignore_ascii_case(&e.key) {
            return false;
        }
        let has = |m: Modifier| self.mods.contains(&m);
        let ctrl = has(Modifier::Ctrl) || (!mac && has(Modifier::Mod));
        let meta = mac && has(Modifier::Mod);
        e.alt == has(Modifier::Alt)
            && e.shift == has(Modifier::Shift)
            && e.ctrl == ctrl
            && e.meta == meta
    }
}

/// A key pressed: its name (`ArrowUp`, `a`) and the modifiers held.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyEvent {
    pub key: String,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl KeyEvent {
    /// `key` with no modifier.
    #[must_use]
    pub fn plain(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            ..Self::default()
        }
    }
}

/// A person's keys for one action, kept with the view's state: its id and
/// its hotkeys as text (`Mod+Shift+P`); none turns the action off.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct KeyBinding {
    pub action: String,
    pub keys: Vec<String>,
}

/// Every action's hotkeys (`KeyboardShortcutConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap(BTreeMap<KeyAction, Vec<Hotkey>>);

impl Default for KeyMap {
    /// Notebook Navigator's defaults (`createDefaultKeyboardShortcuts`),
    /// rename on F2, and architect's open and pin keys.
    fn default() -> Self {
        use KeyAction as A;
        use Modifier::{Mod, Shift};
        let k = Hotkey::new;
        let map = [
            (A::PaneMoveUp, vec![k("ArrowUp", &[])]),
            (A::PaneMoveDown, vec![k("ArrowDown", &[])]),
            (A::PanePageUp, vec![k("PageUp", &[])]),
            (A::PanePageDown, vec![k("PageDown", &[])]),
            (A::PaneHome, vec![k("Home", &[])]),
            (A::PaneEnd, vec![k("End", &[])]),
            (A::PaneRename, vec![k("F2", &[])]),
            (A::NavCollapseOrParent, vec![k("ArrowLeft", &[])]),
            (A::NavExpandOrFocusList, vec![k("ArrowRight", &[])]),
            (A::NavFocusList, vec![k("Tab", &[])]),
            (
                A::DeleteSelected,
                vec![k("Delete", &[]), k("Backspace", &[])],
            ),
            (
                A::ListFocusNavigation,
                vec![k("ArrowLeft", &[]), k("Tab", &[Shift])],
            ),
            (
                A::ListFocusEditor,
                vec![k("ArrowRight", &[]), k("Tab", &[])],
            ),
            (A::ListSelectAll, vec![k("a", &[Mod])]),
            (A::ListExtendSelectionUp, vec![k("ArrowUp", &[Shift])]),
            (A::ListExtendSelectionDown, vec![k("ArrowDown", &[Shift])]),
            (A::ListManualSortUp, vec![k("ArrowUp", &[Mod])]),
            (A::ListManualSortDown, vec![k("ArrowDown", &[Mod])]),
            (A::ListRangeToStart, vec![k("Home", &[Shift])]),
            (A::ListRangeToEnd, vec![k("End", &[Shift])]),
            (A::ListOpen, vec![k("Enter", &[])]),
            (A::ListOpenTab, vec![k("Enter", &[Shift])]),
            (A::ListOpenSplit, vec![k("Enter", &[Mod])]),
            (A::ListPin, vec![k("p", &[Mod, Shift])]),
            (A::SearchFocusList, vec![k("Tab", &[]), k("Enter", &[])]),
            (A::SearchFocusNavigation, vec![k("Tab", &[Shift])]),
            (A::SearchClose, vec![k("Escape", &[])]),
        ];
        Self(map.into_iter().collect())
    }
}

impl KeyMap {
    /// The defaults with Vim's keys too: `k` `j` up and down, `h` `l` out
    /// and in, `g`/`G` (⇧G) to the top and the end.
    #[must_use]
    pub fn vim() -> Self {
        use KeyAction as A;
        let mut m = Self::default();
        let k = Hotkey::new;
        for (a, h) in [
            (A::PaneMoveUp, k("k", &[])),
            (A::PaneMoveDown, k("j", &[])),
            (A::PaneHome, k("g", &[])),
            (A::PaneEnd, k("G", &[Modifier::Shift])),
            (A::NavCollapseOrParent, k("h", &[])),
            (A::NavExpandOrFocusList, k("l", &[])),
            (A::ListFocusNavigation, k("h", &[])),
            (A::ListFocusEditor, k("l", &[])),
            (A::ListExtendSelectionUp, k("K", &[Modifier::Shift])),
            (A::ListExtendSelectionDown, k("J", &[Modifier::Shift])),
        ] {
            m.0.entry(a).or_default().push(h);
        }
        m
    }

    /// The defaults (or Vim's, with `vim`) with a person's `bindings` over
    /// them (`sanitizeKeyboardShortcuts`): ids it does not know and
    /// hotkeys that do not read are left out.
    #[must_use]
    pub fn new(vim: bool, bindings: &[KeyBinding]) -> Self {
        let mut m = if vim { Self::vim() } else { Self::default() };
        for b in bindings {
            if let Some(a) = KeyAction::from_id(&b.action) {
                m.0.insert(a, b.keys.iter().filter_map(|k| Hotkey::parse(k)).collect());
            }
        }
        m
    }

    /// `action`'s hotkeys.
    #[must_use]
    pub fn keys(&self, action: KeyAction) -> &[Hotkey] {
        self.0.get(&action).map_or(&[], Vec::as_slice)
    }

    /// The action `e` is in `scope` (`matchesShortcut`, each action in
    /// [`KeyAction::ALL`]'s order).
    #[must_use]
    pub fn action(&self, scope: KeyScope, e: &KeyEvent, mac: bool) -> Option<KeyAction> {
        KeyAction::ALL
            .into_iter()
            .filter(|a| a.in_scope(scope))
            .find(|a| self.keys(*a).iter().any(|h| h.matches(e, mac)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(key: &str, shift: bool, meta: bool) -> KeyEvent {
        KeyEvent {
            key: key.to_owned(),
            shift,
            meta,
            ..KeyEvent::default()
        }
    }

    #[test]
    fn the_defaults_answer_by_pane_and_mod_is_command_on_a_mac() {
        let m = KeyMap::default();
        let left = ev("ArrowLeft", false, false);
        assert_eq!(
            m.action(KeyScope::Navigation, &left, true),
            Some(KeyAction::NavCollapseOrParent)
        );
        assert_eq!(
            m.action(KeyScope::List, &left, true),
            Some(KeyAction::ListFocusNavigation)
        );
        let select_all = ev("A", false, true);
        assert_eq!(
            m.action(KeyScope::List, &select_all, true),
            Some(KeyAction::ListSelectAll)
        );
        assert_eq!(
            m.action(KeyScope::List, &select_all, false),
            None,
            "Ctrl elsewhere"
        );
        let ctrl_a = KeyEvent {
            key: "a".to_owned(),
            ctrl: true,
            ..KeyEvent::default()
        };
        assert_eq!(
            m.action(KeyScope::List, &ctrl_a, false),
            Some(KeyAction::ListSelectAll)
        );
        assert_eq!(
            m.action(KeyScope::Navigation, &ev("j", false, false), true),
            None
        );
        assert_eq!(
            m.action(KeyScope::Search, &ev("Escape", false, false), true),
            Some(KeyAction::SearchClose)
        );
    }

    #[test]
    fn vim_keys_and_a_persons_own_are_over_the_defaults() {
        let m = KeyMap::new(true, &[]);
        assert_eq!(
            m.action(KeyScope::List, &ev("j", false, false), true),
            Some(KeyAction::PaneMoveDown)
        );
        assert_eq!(
            m.action(KeyScope::Navigation, &ev("l", false, false), true),
            Some(KeyAction::NavExpandOrFocusList)
        );
        let own = [
            KeyBinding {
                action: "pane:rename".to_owned(),
                keys: vec!["Mod+r".to_owned(), "Enter".to_owned()],
            },
            KeyBinding {
                action: "pane:delete-selected".to_owned(),
                keys: vec![],
            },
            KeyBinding {
                action: "no:such".to_owned(),
                keys: vec!["x".to_owned()],
            },
        ];
        let m = KeyMap::new(false, &own);
        assert_eq!(
            m.action(KeyScope::Navigation, &ev("r", false, true), true),
            Some(KeyAction::PaneRename)
        );
        assert_eq!(
            m.action(KeyScope::List, &ev("Delete", false, false), true),
            None,
            "off"
        );
        assert_eq!(m.keys(KeyAction::PaneRename).len(), 2);
    }

    #[test]
    fn hotkeys_read_and_write_back() {
        let h = Hotkey::parse("cmd + shift + P").unwrap_or_else(|| Hotkey::new("", &[]));
        assert_eq!(h.mods, [Modifier::Mod, Modifier::Shift]);
        assert_eq!(h.text(), "Mod+Shift+P");
        assert_eq!(Hotkey::parse("Mod++").map(|h| h.key), Some("+".to_owned()));
        assert_eq!(Hotkey::parse("Hyper+x"), None);
        assert_eq!(Hotkey::parse(""), None);
        assert_eq!(KeyAction::from_id("list:pin"), Some(KeyAction::ListPin));
        let ids: std::collections::BTreeSet<&str> = KeyAction::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), KeyAction::ALL.len(), "stable, distinct ids");
    }
}
