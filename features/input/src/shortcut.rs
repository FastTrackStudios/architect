//! Shortcuts as people write them: the Mac glyphs commands show (`⌘W`,
//! `⇧⌘P`, `⌘K ⌘S`) and keymap notation (`Meta+Shift+p`), read into the one
//! notation bindings are compared in, and shown back in glyphs.

use crate::{ConfigError, KeyChord, KeyCode, Modifiers, parse_key_chord};

/// `keys` in the one notation bindings are compared in: each chord as
/// `KeyChord` prints it, one space between. Glyph shortcuts (`⌘W`) are
/// read too.
///
/// # Errors
///
/// A chord that is not one.
pub fn normalize_keys(keys: &str) -> Result<String, ConfigError> {
    let chords = keys
        .split_whitespace()
        .map(parse_token)
        .collect::<Result<Vec<KeyChord>, ConfigError>>()?;
    if chords.is_empty() {
        return Err(ConfigError::InvalidKeySequence(keys.to_owned()));
    }
    Ok(chords
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" "))
}

/// One chord, in glyphs or keymap notation. A token that starts with a
/// glyph modifier is the glyph form's or nothing.
pub fn parse_token(token: &str) -> Result<KeyChord, ConfigError> {
    let glyph_led = token.starts_with(['⌃', '⌥', '⇧', '⌘']);
    match parse_glyph_chord(token) {
        Some(chord) => Ok(chord),
        None if glyph_led => Err(ConfigError::InvalidKeyChord(token.to_owned())),
        None => parse_key_chord(token),
    }
}

/// A shortcut as commands show it (`⌘W`, `⇧⌘P`, `⌘K ⌘S`), or in keymap
/// notation (`Shift+G`), as keymap notation. `None` when it is neither.
#[must_use]
pub fn parse_shortcut(shortcut: &str) -> Option<String> {
    normalize_keys(shortcut).ok()
}

/// Keys in keymap notation as commands show them: `Meta+Shift+p` is
/// `⇧⌘P`, `g g` is `G G`. Text that is not a key sequence is shown as is.
#[must_use]
pub fn display_keys(keys: &str) -> String {
    keys.split_whitespace()
        .map(|token| {
            parse_key_chord(token).map_or_else(|_| token.to_owned(), |chord| display_chord(&chord))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One chord with the Mac glyphs, modifiers in the Mac order: `⌃⌥⇧⌘K`.
#[must_use]
pub fn display_chord(chord: &KeyChord) -> String {
    let m = chord.modifiers;
    let mut out = String::new();
    for (held, glyph) in [(m.ctrl, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.meta, '⌘')] {
        if held {
            out.push(glyph);
        }
    }
    match &chord.key {
        KeyCode::Character(c) if c == " " => out.push_str("Space"),
        KeyCode::Character(c) => out.push_str(&c.to_uppercase()),
        KeyCode::Enter => out.push('↵'),
        KeyCode::Escape => out.push_str("Esc"),
        KeyCode::Tab => out.push_str("Tab"),
        KeyCode::Backspace => out.push('⌫'),
        KeyCode::Delete => out.push_str("Del"),
        KeyCode::ArrowUp => out.push('↑'),
        KeyCode::ArrowDown => out.push('↓'),
        KeyCode::ArrowLeft => out.push('←'),
        KeyCode::ArrowRight => out.push('→'),
        KeyCode::F(n) => {
            out.push('F');
            out.push_str(&n.to_string());
        }
    }
    out
}

/// A chord in glyphs (`⇧⌘P`, `⌃Tab`, `Esc`, `G`): what [`display_chord`]
/// prints. `None` when the token is not in that form (so it is read as
/// keymap notation instead).
#[must_use]
pub fn parse_glyph_chord(token: &str) -> Option<KeyChord> {
    let mut modifiers = Modifiers::NONE;
    let mut rest = token;
    let mut glyphs = false;
    while let Some(c) = rest.chars().next() {
        match c {
            '⌃' => modifiers.ctrl = true,
            '⌥' => modifiers.alt = true,
            '⇧' => modifiers.shift = true,
            '⌘' => modifiers.meta = true,
            _ => break,
        }
        glyphs = true;
        rest = rest.get(c.len_utf8()..)?;
    }
    let key = match rest {
        "↵" => KeyCode::Enter,
        "Esc" => KeyCode::Escape,
        "⌫" => KeyCode::Backspace,
        "Del" => KeyCode::Delete,
        "↑" => KeyCode::ArrowUp,
        "↓" => KeyCode::ArrowDown,
        "←" => KeyCode::ArrowLeft,
        "→" => KeyCode::ArrowRight,
        "Space" if glyphs => KeyCode::Character(" ".into()),
        "Tab" if glyphs => KeyCode::Tab,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                // `+` and `.` alone are keys either way; a lone letter is
                // the glyph form's (keymap notation reads it the same).
                (Some(c), None) => KeyCode::char(c),
                _ if glyphs => {
                    let n = other.strip_prefix('F')?.parse::<u8>().ok()?;
                    KeyCode::F(n)
                }
                _ => return None,
            }
        }
    };
    Some(KeyChord::new(key, modifiers))
}
