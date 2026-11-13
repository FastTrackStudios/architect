//! A key press as the chords the commands' keymap reads (`⇧⌘K`), in the
//! order to try them: what `app/keys.rs` hears from the document's
//! `keydown`. Pure, so it is tested here.

/// A key press: the event's `key` and its modifiers.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)] // a key event's own flags
pub struct Press<'a> {
    pub key: &'a str,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// The chords `press` is tried as, first match wins; none for a key that
/// stays the page's (plain typing).
///
/// The app's bindings name ⌘ (the Mac's command key). On a Mac or an
/// iPhone (`apple`) a press is its own chord. Elsewhere Ctrl is the
/// platform's command key, so a press with Ctrl and no Meta is tried as
/// `⌃…` (a binding that names Control) and then as `⌘…`: Ctrl+K opens
/// what ⌘K does.
#[must_use]
pub fn chords(press: Press<'_>, apple: bool) -> Vec<String> {
    let key = press.key;
    let named = match key {
        " " => "Space".to_owned(),
        "Tab" => "Tab".to_owned(),
        "Enter" => "↵".to_owned(),
        "Escape" => "Esc".to_owned(),
        "Backspace" => "⌫".to_owned(),
        "ArrowLeft" => "←".to_owned(),
        "ArrowRight" => "→".to_owned(),
        "ArrowUp" => "↑".to_owned(),
        "ArrowDown" => "↓".to_owned(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k if k.starts_with('F') && k.len() <= 3 => k.to_owned(),
        _ => return Vec::new(),
    };
    if !(press.ctrl || press.meta) && named != "Esc" {
        return Vec::new();
    }
    // ⇧ is in the character itself for a symbol (`?`), as on a desk.
    let shifted_char = key.chars().count() == 1 && !key.chars().all(char::is_alphabetic);
    let chord = |ctrl: bool, meta: bool| {
        let mut out = String::new();
        if ctrl {
            out.push('⌃');
        }
        if press.alt {
            out.push('⌥');
        }
        if press.shift && !shifted_char {
            out.push('⇧');
        }
        if meta {
            out.push('⌘');
        }
        out.push_str(&named);
        out
    };
    let mut out = vec![chord(press.ctrl, press.meta)];
    if !apple && press.ctrl && !press.meta {
        out.push(chord(false, true));
    }
    out
}

/// Whether a browser's platform is Apple's (macOS, iOS, iPadOS), from
/// `navigator.userAgentData.platform` or else `navigator.platform`.
#[must_use]
pub fn is_apple(platform: &str) -> bool {
    let p = platform.to_ascii_lowercase();
    ["mac", "iphone", "ipad", "ipod", "ios"]
        .iter()
        .any(|a| p.contains(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: &str) -> Press<'_> {
        Press {
            key,
            ..Press::default()
        }
    }

    #[test]
    fn ctrl_is_the_command_key_off_apple_platforms() {
        let ctrl_k = Press {
            ctrl: true,
            ..press("k")
        };
        assert_eq!(chords(ctrl_k, false), ["⌃K", "⌘K"]);
        // On a Mac, Control stays Control.
        assert_eq!(chords(ctrl_k, true), ["⌃K"]);
        let cmd_k = Press {
            meta: true,
            ..press("k")
        };
        assert_eq!(chords(cmd_k, true), ["⌘K"]);
        assert_eq!(chords(cmd_k, false), ["⌘K"]);
        let ctrl_comma = Press {
            ctrl: true,
            ..press(",")
        };
        assert_eq!(chords(ctrl_comma, false), ["⌃,", "⌘,"]);
        let ctrl_shift_p = Press {
            ctrl: true,
            shift: true,
            ..press("P")
        };
        assert_eq!(chords(ctrl_shift_p, false), ["⌃⇧P", "⇧⌘P"]);
    }

    #[test]
    fn plain_keys_stay_the_pages_and_escape_is_always_heard() {
        assert!(chords(press("k"), false).is_empty());
        assert!(chords(press("Enter"), true).is_empty());
        assert_eq!(chords(press("Escape"), false), ["Esc"]);
        assert!(
            chords(
                Press {
                    ctrl: true,
                    ..press("Shift")
                },
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn apple_platforms_are_recognised() {
        for p in ["MacIntel", "macOS", "iPhone", "iPad"] {
            assert!(is_apple(p), "{p}");
        }
        for p in ["Linux x86_64", "Win32", "Windows", "Android", ""] {
            assert!(!is_apple(p), "{p}");
        }
    }
}
