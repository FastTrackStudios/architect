//! Keys, modifiers and chords: the units a keymap is made of.

use std::fmt;

/// A keyboard key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// A printable character key, lower-cased. Space is `" "`.
    Character(String),
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    /// Function key `F<n>`.
    F(u8),
}

impl KeyCode {
    /// A character key from one `char`, lower-cased the way keymaps store it.
    #[must_use]
    pub fn char(c: char) -> Self {
        Self::Character(c.to_lowercase().collect())
    }

    /// The single character of a [`KeyCode::Character`], if it is exactly one.
    #[must_use]
    pub fn single_char(&self) -> Option<char> {
        let Self::Character(text) = self else {
            return None;
        };
        let mut chars = text.chars();
        let first = chars.next()?;
        chars.next().is_none().then_some(first)
    }

    /// The short name the pending-sequence display uses (`Esc`, `BS`, `Up`).
    fn short_name(&self) -> String {
        match self {
            Self::Character(c) => c.clone(),
            Self::ArrowUp => "Up".into(),
            Self::ArrowDown => "Down".into(),
            Self::ArrowLeft => "Left".into(),
            Self::ArrowRight => "Right".into(),
            Self::Enter => "Enter".into(),
            Self::Escape => "Esc".into(),
            Self::Tab => "Tab".into(),
            Self::Backspace => "BS".into(),
            Self::Delete => "Del".into(),
            Self::F(n) => format!("F{n}"),
        }
    }

    /// The name [`crate::parse_key_chord`] reads back (`Escape`, `Space`).
    fn config_name(&self) -> String {
        match self {
            Self::Character(c) if c == " " => "Space".into(),
            Self::Character(c) if c == "+" => "+".into(),
            Self::Escape => "Escape".into(),
            Self::Backspace => "Backspace".into(),
            Self::Delete => "Delete".into(),
            other => other.short_name(),
        }
    }
}

/// Modifier key state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    /// Only Ctrl held.
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };

    /// Only Shift held.
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };

    /// Only Alt held.
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    /// Only Meta (Cmd / Super) held.
    pub const META: Self = Self {
        meta: true,
        ..Self::NONE
    };

    /// The held modifiers paired with a name each, in display order.
    const fn named(self) -> [(bool, &'static str); 4] {
        [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, "Meta"),
        ]
    }
}

/// One key press with its modifiers: the key of every trie edge and
/// binding table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

impl KeyChord {
    #[must_use]
    pub const fn new(key: KeyCode, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// A chord with no modifiers.
    #[must_use]
    pub const fn plain(key: KeyCode) -> Self {
        Self::new(key, Modifiers::NONE)
    }

    /// The compact form the processor's pending display uses: `C-g`, `S-Tab`.
    #[must_use]
    pub fn short_label(&self) -> String {
        let m = self.modifiers;
        let prefix: String = [
            (m.ctrl, "C-"),
            (m.alt, "A-"),
            (m.shift, "S-"),
            (m.meta, "M-"),
        ]
        .into_iter()
        .filter_map(|(held, tag)| held.then_some(tag))
        .collect();
        format!("{prefix}{}", self.key.short_name())
    }
}

/// `Ctrl+Shift+s`, `g`, `Space`: the notation [`crate::parse_key_chord`]
/// parses, so a chord printed and parsed again is the same chord.
impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in self.modifiers.named() {
            if held {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&self.key.config_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_key_chord;

    #[test]
    fn a_printed_chord_parses_back_to_itself() {
        for chord in [
            KeyChord::plain(KeyCode::char('g')),
            KeyChord::new(KeyCode::char('s'), Modifiers::CTRL),
            KeyChord::new(
                KeyCode::Tab,
                Modifiers {
                    shift: true,
                    alt: true,
                    ..Modifiers::NONE
                },
            ),
            KeyChord::plain(KeyCode::char(' ')),
            KeyChord::plain(KeyCode::Escape),
            KeyChord::new(KeyCode::F(5), Modifiers::META),
        ] {
            assert_eq!(parse_key_chord(&chord.to_string()).unwrap(), chord);
        }
    }

    #[test]
    fn short_label_is_the_compact_vim_form() {
        let chord = KeyChord::new(KeyCode::char('g'), Modifiers::CTRL);
        assert_eq!(chord.short_label(), "C-g");
        assert_eq!(KeyChord::plain(KeyCode::Escape).short_label(), "Esc");
    }
}
