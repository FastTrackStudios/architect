//! Vim-style key notation (`<C-s>`, `gg`, `<S-Tab>`) to keymap notation
//! (`Ctrl+s`, `g g`, `Shift+Tab`).
//!
//! Keybind files written by hand use the compact vim form; the keymap
//! ([`crate::parse_key_sequence`]) reads the spelled-out one. On macOS the
//! "primary" modifier `C-` means Cmd (`Meta`), so a file written once
//! binds the platform's usual shortcut; `CC-` is always the Control key.

/// Which platform's modifier conventions to translate for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPlatform {
    /// `C-` is Cmd (`Meta`); `C-` after `M-` (or the reverse) is Control.
    Mac,
    /// `C-` is Control, `M-` is Meta.
    Other,
}

impl KeyPlatform {
    /// The platform this crate was compiled for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Other
        }
    }
}

/// [`translate_sequence_for`] on the [`KeyPlatform::current`] platform.
///
/// `"a"` → `"a"`, `"gg"` → `"g g"`, `"<C-s>"` → `"Ctrl+s"`,
/// `"<C-S-a>"` → `"Ctrl+Shift+a"`, `"g<C-a>"` → `"g Ctrl+a"`.
#[must_use]
pub fn translate_sequence(seq: &str) -> String {
    translate_sequence_for(seq, KeyPlatform::current())
}

/// Translate a vim-notation key sequence into keymap notation: each bare
/// character is one chord (lower-cased), each `<...>` group is one chord.
#[must_use]
pub fn translate_sequence_for(seq: &str, platform: KeyPlatform) -> String {
    let mut chords = Vec::new();
    let mut chars = seq.chars();
    while let Some(c) = chars.next() {
        if c == '<' {
            let group: String = chars.by_ref().take_while(|&ch| ch != '>').collect();
            chords.push(translate_group(&group, platform));
        } else {
            chords.push(c.to_lowercase().to_string());
        }
    }
    chords.join(" ")
}

/// One `<...>` group, without its brackets: a special key name (`space`,
/// `esc`), or `-`-separated modifiers then a key (`C-S-a`).
fn translate_group(content: &str, platform: KeyPlatform) -> String {
    if let Some(special) = special_key(&content.to_lowercase()) {
        return special.to_owned();
    }
    let mut parts: Vec<&str> = content.split('-').collect();
    let key_token = parts.pop().unwrap_or_default().to_lowercase();
    let key = special_key(&key_token).map_or(key_token, str::to_owned);
    let mut modifiers: Vec<&'static str> = Vec::new();
    for part in parts {
        let Some(m) = group_modifier(&part.to_uppercase(), platform, &modifiers) else {
            continue;
        };
        if !modifiers.contains(&m) {
            modifiers.push(m);
        }
    }
    if modifiers.is_empty() {
        return key;
    }
    format!("{}+{key}", modifiers.join("+"))
}

/// The keymap modifier one vim modifier letter means, given the ones
/// already read (on a Mac the second "primary" modifier is Control).
fn group_modifier(
    letter: &str,
    platform: KeyPlatform,
    so_far: &[&'static str],
) -> Option<&'static str> {
    let primary = || match platform {
        KeyPlatform::Mac if so_far.contains(&"Meta") => "Ctrl",
        KeyPlatform::Mac => "Meta",
        KeyPlatform::Other => "Ctrl",
    };
    let command = || match platform {
        KeyPlatform::Mac if so_far.contains(&"Meta") => "Ctrl",
        _ => "Meta",
    };
    match letter {
        "C" | "CTRL" | "CONTROL" => Some(primary()),
        "CC" | "RAWCTRL" => Some("Ctrl"),
        "S" | "SHIFT" => Some("Shift"),
        "A" | "ALT" | "OPT" | "OPTION" => Some("Alt"),
        "M" | "META" | "CMD" | "COMMAND" | "WIN" | "SUPER" | "D" => Some(command()),
        _ => None,
    }
}

/// Keymap names for vim special-key names.
fn special_key(name: &str) -> Option<&'static str> {
    const NUMPAD: [(&str, &str); 10] = [
        ("kp_0", "kp_0"),
        ("kp_1", "kp_1"),
        ("kp_2", "kp_2"),
        ("kp_3", "kp_3"),
        ("kp_4", "kp_4"),
        ("kp_5", "kp_5"),
        ("kp_6", "kp_6"),
        ("kp_7", "kp_7"),
        ("kp_8", "kp_8"),
        ("kp_9", "kp_9"),
    ];
    const FUNCTION: [&str; 12] = [
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    ];
    let named = match name {
        "space" | "spc" => "Space",
        "tab" => "Tab",
        "enter" | "return" | "ret" | "cr" => "Enter",
        "esc" | "escape" => "Escape",
        "backspace" | "bs" => "Backspace",
        "delete" | "del" => "Delete",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "home",
        "end" => "end",
        "pageup" | "pgup" => "pageup",
        "pagedown" | "pgdn" => "pagedown",
        "insert" | "ins" => "insert",
        "plus" | "equals" | "equal" => "equals",
        "minus" => "minus",
        "kp_+" | "kp_plus" => "kp_plus",
        "kp_-" | "kp_minus" => "kp_minus",
        "kp_*" | "kp_multiply" => "kp_multiply",
        "kp_/" | "kp_divide" => "kp_divide",
        "kp_." | "kp_period" => "kp_period",
        "kp_enter" => "kp_enter",
        other => {
            return NUMPAD
                .iter()
                .find(|(from, _)| *from == other)
                .map(|(_, to)| *to)
                .or_else(|| FUNCTION.iter().copied().find(|f| *f == other));
        }
    };
    Some(named)
}

/// [`translate_modifiers_for`] on the [`KeyPlatform::current`] platform.
#[must_use]
pub fn translate_modifiers(mods: &str) -> String {
    translate_modifiers_for(mods, KeyPlatform::current())
}

/// A bare modifier string (`""`, `<C->`, `<C-S->`) as a keymap modifier
/// prefix (`""`, `"Ctrl"`, `"Ctrl+Shift"`), e.g. for a wheel binding.
#[must_use]
pub fn translate_modifiers_for(mods: &str, platform: KeyPlatform) -> String {
    let trimmed = mods.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed);
    inner
        .split('-')
        .filter_map(|part| match part.to_uppercase().as_str() {
            "C" | "CTRL" if platform == KeyPlatform::Mac => Some("Meta"),
            "C" | "CTRL" | "CC" | "RAWCTRL" => Some("Ctrl"),
            "S" | "SHIFT" => Some("Shift"),
            "A" | "ALT" => Some("Alt"),
            "M" | "META" | "CMD" | "D" => Some("Meta"),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn other(seq: &str) -> String {
        translate_sequence_for(seq, KeyPlatform::Other)
    }

    #[test]
    fn translate_simple_and_multi() {
        assert_eq!(other("a"), "a");
        assert_eq!(other("gg"), "g g");
        assert_eq!(other("G"), "g");
    }

    #[test]
    fn translate_modifiers_in_groups() {
        assert_eq!(other("<C-s>"), "Ctrl+s");
        assert_eq!(other("<S-Tab>"), "Shift+Tab");
        assert_eq!(other("<C-S-a>"), "Ctrl+Shift+a");
        assert_eq!(other("g<C-a>"), "g Ctrl+a");
        assert_eq!(other("<space>"), "Space");
        assert_eq!(other("<C-f5>"), "Ctrl+f5");
    }

    #[test]
    fn on_a_mac_the_primary_modifier_is_cmd() {
        let mac = |seq| translate_sequence_for(seq, KeyPlatform::Mac);
        assert_eq!(mac("<C-s>"), "Meta+s");
        assert_eq!(mac("<M-C-s>"), "Meta+Ctrl+s");
        assert_eq!(mac("<CC-s>"), "Ctrl+s");
    }

    #[test]
    fn translated_sequences_parse() {
        for seq in ["gg", "<C-S-a>", "<space>", "<esc>", "z<C-t>", "<kp_5>"] {
            assert!(crate::parse_key_sequence(&other(seq)).is_ok(), "{seq}");
        }
    }

    #[test]
    fn modifier_strings() {
        assert_eq!(translate_modifiers_for("", KeyPlatform::Other), "");
        assert_eq!(translate_modifiers_for("<>", KeyPlatform::Other), "");
        assert_eq!(
            translate_modifiers_for("<C-S->", KeyPlatform::Other),
            "Ctrl+Shift"
        );
        assert_eq!(translate_modifiers_for("<C->", KeyPlatform::Mac), "Meta");
    }
}
