//! A numbered [`Tree`] as `com.canonical.dbusmenu` describes its items:
//! each node's properties (label, enabled, the tick, the shortcut), for
//! the Linux bar (`linux_menu`). Pure, so it is tested wherever the crate
//! builds.

use crate::menu::{Kind, Tree};

/// A property's value, as dbusmenu types it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prop {
    Str(String),
    Bool(bool),
    Int(i32),
    /// A shortcut: chords of key names (`[["Control", "Shift", "t"]]`).
    Keys(Vec<Vec<String>>),
}

/// `node`'s properties. Defaults are left out (dbusmenu's own: enabled,
/// visible, a standard item), as the spec asks.
#[must_use]
pub fn props(node: &Tree) -> Vec<(&'static str, Prop)> {
    let submenu = ("children-display", Prop::Str("submenu".to_owned()));
    match &node.kind {
        Kind::Root => vec![submenu],
        Kind::Menu { title } => vec![("label", Prop::Str(escape_mnemonic(title))), submenu],
        Kind::Row(row) => {
            let mut out = vec![("label", Prop::Str(escape_mnemonic(&row.label)))];
            if !row.enabled {
                out.push(("enabled", Prop::Bool(false)));
            }
            if let Some(on) = row.checked {
                out.push(("toggle-type", Prop::Str("checkmark".to_owned())));
                out.push(("toggle-state", Prop::Int(i32::from(on))));
            }
            if let Some(keys) = keys(&row.shortcut) {
                out.push(("shortcut", Prop::Keys(keys)));
            }
            out
        }
        Kind::Separator => vec![("type", Prop::Str("separator".to_owned()))],
        // Not drawn on Linux: `without_platform` leaves these out.
        Kind::Standard(_) => vec![("visible", Prop::Bool(false))],
    }
}

/// A label with its underscores doubled: dbusmenu reads one `_` as the
/// mnemonic's mark, and commands have none.
fn escape_mnemonic(label: &str) -> String {
    label.replace('_', "__")
}

/// A shortcut as commands show it (`⇧⌘T`) as dbusmenu's key names
/// (`[["Shift", "Super", "t"]]`): ⌘ is Super, the key that is held for it.
/// `None` when it has none, or one with a key dbusmenu cannot name.
#[must_use]
pub fn keys(shortcut: &str) -> Option<Vec<Vec<String>>> {
    if shortcut.is_empty() {
        return None;
    }
    let mut chord = Vec::new();
    let mut rest = shortcut;
    loop {
        let mut chars = rest.chars();
        let name = match chars.next() {
            Some('⌃') => "Control",
            Some('⌥') => "Alt",
            Some('⇧') => "Shift",
            Some('⌘') => "Super",
            _ => break,
        };
        chord.push(name.to_owned());
        rest = chars.as_str();
    }
    let key = match rest {
        "Tab" => "Tab".to_owned(),
        "Space" => "space".to_owned(),
        "Esc" => "Escape".to_owned(),
        "⌫" => "BackSpace".to_owned(),
        "↵" | "Return" => "Return".to_owned(),
        "←" => "Left".to_owned(),
        "→" => "Right".to_owned(),
        "↑" => "Up".to_owned(),
        "↓" => "Down".to_owned(),
        "[" => "bracketleft".to_owned(),
        "]" => "bracketright".to_owned(),
        "\\" => "backslash".to_owned(),
        "," => "comma".to_owned(),
        "." => "period".to_owned(),
        "/" => "slash".to_owned(),
        ";" => "semicolon".to_owned(),
        "'" => "apostrophe".to_owned(),
        "-" => "minus".to_owned(),
        "=" => "equal".to_owned(),
        "`" => "grave".to_owned(),
        key => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
                _ => function_key(key)?,
            }
        }
    };
    chord.push(key);
    Some(vec![chord])
}

fn function_key(key: &str) -> Option<String> {
    let n: u8 = key.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&n).then(|| key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Prop, keys, props};
    use crate::menu::{Item, Resolved, Tree};
    use architect_palette::Command;

    #[test]
    fn a_shortcut_is_named_as_dbusmenu_names_keys() {
        let k = |chords: &[&str]| Some(vec![chords.iter().map(|s| (*s).to_owned()).collect()]);
        assert_eq!(keys("⇧⌘T"), k(&["Shift", "Super", "t"]));
        assert_eq!(keys("⌃Tab"), k(&["Control", "Tab"]));
        assert_eq!(keys("⌥⌘←"), k(&["Alt", "Super", "Left"]));
        assert_eq!(keys("⌘,"), k(&["Super", "comma"]));
        assert_eq!(keys("F5"), k(&["F5"]));
        assert_eq!(keys(""), None);
        assert_eq!(keys("⌘Hyper"), None);
    }

    #[test]
    fn a_row_says_its_label_state_and_tick() {
        let menus = vec![Resolved {
            title: "View".into(),
            items: vec![
                Item::Command(Command::new("VIEW_GRID", "Show_grid").checked(true)),
                Item::Command(Command::new("VIEW_SAVE", "Save").disabled("nothing open")),
            ],
        }];
        let tree = Tree::new(&menus);
        let grid = tree.find(2).expect("numbered");
        assert_eq!(
            props(grid),
            vec![
                ("label", Prop::Str("Show__grid".into())),
                ("toggle-type", Prop::Str("checkmark".into())),
                ("toggle-state", Prop::Int(1)),
            ]
        );
        let save = tree.find(3).expect("numbered");
        assert!(props(save).contains(&("enabled", Prop::Bool(false))));
        assert!(!props(save).iter().any(|(k, _)| *k == "toggle-type"));
    }
}
