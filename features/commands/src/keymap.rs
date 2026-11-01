//! The keymap a person sees and edits: the app's bindings, the person's
//! changes laid over them, and what each key does where
//! (`docs/content/architecture/actions-everywhere.md` §3, §4, §9).
//!
//! The app's keymap is an `architect_input::KeymapConfig` (its keys,
//! by context) plus the shortcut each command declares. The person's
//! changes are a [`PersonKeymap`]: a list of [`Rebind`]s and their custom
//! actions — plain data, kept with their settings and synced, so their
//! keys follow them. [`Keymap::config`] lays one over the other for the
//! processor; the action list window edits the person's layer through
//! [`Keymap::bind`], [`Keymap::unbind`] and the resets.

use std::collections::HashMap;

use architect_input::{ConfigError, KeyChord, KeyCode, KeymapConfig, Modifiers, parse_key_chord};
use facet::Facet;

use crate::custom::CustomAction;

/// The context of a binding that holds everywhere: the app's own keymap
/// (its `normal` mode), under every view and target.
pub const EVERYWHERE: &str = "";

/// The mode the app's own bindings live in.
const NORMAL: &str = "normal";

/// The action string that removes a binding.
const UNBIND: &str = "unbind";

/// One change a person made: `keys` in `context` runs `action`, or, when
/// `action` is `unbind`, no longer runs what the app bound there.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Rebind {
    /// Where it holds: [`EVERYWHERE`], or a context (`session.arrange`).
    pub context: String,
    /// The key sequence, in keymap notation (`Meta+w`, `g g`).
    pub keys: String,
    /// The action it runs, or `unbind`.
    pub action: String,
}

/// A person's layer over the app's keymap: their rebinds, in the order
/// they made them, and their custom actions. Kept with their settings
/// (synced), so it is the same on every device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct PersonKeymap {
    #[facet(default)]
    pub rebinds: Vec<Rebind>,
    #[facet(default)]
    pub custom: Vec<CustomAction>,
}

impl PersonKeymap {
    /// The layer as JSON, to keep or sync.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_json(&self) -> Result<String, String> {
        facet_json::to_string(self).map_err(|e| e.to_string())
    }

    /// A layer from [`Self::to_json`]'s JSON (an exported keymap too).
    ///
    /// # Errors
    ///
    /// The decoder's message.
    pub fn from_json(json: &str) -> Result<Self, String> {
        facet_json::from_str(json).map_err(|e| e.to_string())
    }
}

/// Whose a binding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// The app ships it.
    App,
    /// The person made it.
    Person,
}

/// A key bound to an action, in a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub context: String,
    /// Keymap notation (`Meta+w`); [`display_keys`] for the glyphs.
    pub keys: String,
    pub action: String,
    pub layer: Layer,
}

/// How a binding being made collides with one that is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clash {
    /// The same keys in the same context: it would replace that one.
    Same,
    /// One sequence starts the other in the same context (`g` and
    /// `g g`): the shorter one would never wait for the longer.
    Prefix,
    /// The same keys in a context around this one: it would win over
    /// that one here.
    Outer,
    /// The same keys in a context inside this one: that one would still
    /// win there.
    Inner,
}

/// A binding a new one would collide with, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub binding: Binding,
    pub clash: Clash,
}

/// The app's keymap with a person's layer over it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    app: KeymapConfig,
    /// What each command's declared shortcut binds (action, keys), where
    /// the app's keymap does not bind the action itself.
    shortcuts: Vec<(String, String)>,
    person: PersonKeymap,
}

impl Keymap {
    /// The app's keymap, with no changes of the person's.
    #[must_use]
    pub fn new(app: KeymapConfig) -> Self {
        Self {
            app,
            shortcuts: Vec::new(),
            person: PersonKeymap::default(),
        }
    }

    #[must_use]
    pub fn with_person(mut self, person: PersonKeymap) -> Self {
        self.person = person;
        self
    }

    #[must_use]
    pub const fn app(&self) -> &KeymapConfig {
        &self.app
    }

    #[must_use]
    pub const fn person(&self) -> &PersonKeymap {
        &self.person
    }

    /// The person's layer changed elsewhere (another device, through the
    /// sync).
    pub fn set_person(&mut self, person: PersonKeymap) {
        self.person = person;
    }

    /// The shortcuts commands declare (`(id, "⌘W")`), bound everywhere
    /// for each action the app's keymap does not bind itself — so a
    /// command's shown shortcut is a binding, and can be changed.
    pub fn set_shortcuts<I, S, T>(&mut self, shortcuts: I)
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: AsRef<str>,
    {
        let explicit = config_bindings(&self.app);
        self.shortcuts = shortcuts
            .into_iter()
            .filter_map(|(id, shortcut)| Some((id.into(), parse_shortcut(shortcut.as_ref())?)))
            .filter(|(id, keys)| {
                !explicit
                    .iter()
                    .any(|(c, k, a)| a == id || (c.is_empty() && k == keys))
            })
            .collect();
    }

    /// The app's keymap with the declared shortcuts in it.
    fn base(&self) -> KeymapConfig {
        let mut base = self.app.clone();
        let normal = base.keymap.entry(NORMAL.to_owned()).or_default();
        for (id, keys) in &self.shortcuts {
            normal.entry(keys.clone()).or_insert_with(|| id.clone());
        }
        base
    }

    /// The person's layer as a config, to merge over the base.
    fn person_config(&self) -> KeymapConfig {
        let mut config = KeymapConfig::default();
        for r in &self.person.rebinds {
            let map = if r.context.is_empty() {
                config.keymap.entry(NORMAL.to_owned()).or_default()
            } else {
                &mut config.contexts.entry(r.context.clone()).or_default().keys
            };
            map.insert(r.keys.clone(), r.action.clone());
        }
        config
    }

    /// The keymap the processor runs: the app's, the declared shortcuts,
    /// the person's layer over both.
    #[must_use]
    pub fn config(&self) -> KeymapConfig {
        KeymapConfig::merge(self.base(), self.person_config())
    }

    /// Every key binding in force, in every context.
    #[must_use]
    pub fn bindings(&self) -> Vec<Binding> {
        let mut out: Vec<Binding> = config_bindings(&self.config())
            .into_iter()
            .map(|(context, keys, action)| {
                let layer = if self.person_binds(&context, &keys) {
                    Layer::Person
                } else {
                    Layer::App
                };
                Binding {
                    context,
                    keys,
                    action,
                    layer,
                }
            })
            .collect();
        out.sort_by(|a, b| a.context.cmp(&b.context).then_with(|| a.keys.cmp(&b.keys)));
        out
    }

    fn person_binds(&self, context: &str, keys: &str) -> bool {
        self.person
            .rebinds
            .iter()
            .any(|r| r.context == context && r.keys == keys && r.action != UNBIND)
    }

    /// The bindings of `action`, everywhere first.
    #[must_use]
    pub fn bindings_of(&self, action: &str) -> Vec<Binding> {
        self.bindings()
            .into_iter()
            .filter(|b| b.action == action)
            .collect()
    }

    /// What `keys` does in each context that binds them.
    #[must_use]
    pub fn what_does(&self, keys: &str) -> Vec<Binding> {
        let Ok(keys) = normalize_keys(keys) else {
            return Vec::new();
        };
        self.bindings()
            .into_iter()
            .filter(|b| b.keys == keys)
            .collect()
    }

    /// The shortcut to show for `action` (`⌘W`): its binding everywhere,
    /// else its first in any context. `None` when nothing binds it.
    #[must_use]
    pub fn shortcut(&self, action: &str) -> Option<String> {
        let bindings = self.bindings_of(action);
        bindings
            .iter()
            .find(|b| b.context.is_empty())
            .or_else(|| bindings.first())
            .map(|b| display_keys(&b.keys))
    }

    /// The shortcut to show for every bound action, as
    /// [`shortcut`](Self::shortcut) gives it, computed once.
    #[must_use]
    pub fn shortcuts(&self) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        // Everywhere sorts first: its binding is the one shown.
        for b in self.bindings() {
            out.entry(b.action).or_insert_with(|| display_keys(&b.keys));
        }
        out
    }

    /// What binding `keys` in `context` would collide with (other than
    /// `action`'s own binding there).
    #[must_use]
    pub fn conflicts(&self, action: &str, context: &str, keys: &str) -> Vec<Conflict> {
        let Ok(keys) = normalize_keys(keys) else {
            return Vec::new();
        };
        self.bindings()
            .into_iter()
            .filter(|b| !(b.action == action && b.context == context && b.keys == keys))
            .filter_map(|b| {
                let clash = if b.context == context {
                    if b.keys == keys {
                        Clash::Same
                    } else if starts(&b.keys, &keys) || starts(&keys, &b.keys) {
                        Clash::Prefix
                    } else {
                        return None;
                    }
                } else if b.keys != keys {
                    return None;
                } else if encloses(&b.context, context) {
                    Clash::Outer
                } else if encloses(context, &b.context) {
                    Clash::Inner
                } else {
                    return None;
                };
                Some(Conflict { binding: b, clash })
            })
            .collect()
    }

    /// Binds `keys` in `context` to `action`, replacing what the person
    /// bound there before. The app's binding there gives way to it.
    ///
    /// # Errors
    ///
    /// `keys` is not a key sequence.
    pub fn bind(&mut self, action: &str, context: &str, keys: &str) -> Result<(), ConfigError> {
        let keys = normalize_keys(keys)?;
        self.drop_rebinds(context, &keys);
        self.person.rebinds.push(Rebind {
            context: context.to_owned(),
            keys,
            action: action.to_owned(),
        });
        Ok(())
    }

    /// Removes the binding of `keys` in `context`: the person's, and the
    /// app's (as an `unbind` in the person's layer).
    ///
    /// # Errors
    ///
    /// `keys` is not a key sequence.
    pub fn unbind(&mut self, context: &str, keys: &str) -> Result<(), ConfigError> {
        let keys = normalize_keys(keys)?;
        self.drop_rebinds(context, &keys);
        let app_binds = config_bindings(&self.base())
            .iter()
            .any(|(c, k, _)| c == context && *k == keys);
        if app_binds {
            self.person.rebinds.push(Rebind {
                context: context.to_owned(),
                keys,
                action: UNBIND.to_owned(),
            });
        }
        Ok(())
    }

    fn drop_rebinds(&mut self, context: &str, keys: &str) {
        self.person
            .rebinds
            .retain(|r| !(r.context == context && r.keys == keys));
    }

    /// Whether the person changed `action`'s bindings.
    #[must_use]
    pub fn is_changed(&self, action: &str) -> bool {
        let app = config_bindings(&self.base());
        self.person.rebinds.iter().any(|r| {
            r.action == action
                || (r.action == UNBIND
                    && app
                        .iter()
                        .any(|(c, k, a)| *c == r.context && *k == r.keys && a == action))
        })
    }

    /// Puts `action`'s bindings back to the app's: the person's bindings
    /// of it go, and so do their removals of the app's.
    pub fn reset(&mut self, action: &str) {
        let app = config_bindings(&self.base());
        self.person.rebinds.retain(|r| {
            let unbinds_it = r.action == UNBIND
                && app
                    .iter()
                    .any(|(c, k, a)| *c == r.context && *k == r.keys && a == action);
            r.action != action && !unbinds_it
        });
    }

    /// Puts every binding in `context` back to the app's.
    pub fn reset_context(&mut self, context: &str) {
        self.person.rebinds.retain(|r| r.context != context);
    }

    /// Puts every binding back to the app's. Custom actions stay.
    pub fn reset_all(&mut self) {
        self.person.rebinds.clear();
    }

    /// The person's custom actions.
    #[must_use]
    pub fn custom(&self) -> &[CustomAction] {
        &self.person.custom
    }

    /// Adds `action`, or replaces the one with its id.
    pub fn save_custom(&mut self, action: CustomAction) {
        match self.person.custom.iter_mut().find(|c| c.id == action.id) {
            Some(slot) => *slot = action,
            None => self.person.custom.push(action),
        }
    }

    /// Removes custom action `id`, and the person's bindings of it.
    pub fn remove_custom(&mut self, id: &str) {
        self.person.custom.retain(|c| c.id != id);
        self.person.rebinds.retain(|r| r.action != id);
    }
}

/// Every key binding of a config as (context, keys, action): the
/// `normal` mode's everywhere, then each context's.
fn config_bindings(config: &KeymapConfig) -> Vec<(String, String, String)> {
    let normal = config
        .keymap
        .get(NORMAL)
        .into_iter()
        .flatten()
        .map(|(k, a)| (String::new(), k, a));
    let contexts = config
        .contexts
        .iter()
        .flat_map(|(c, cfg)| cfg.keys.iter().map(move |(k, a)| (c.clone(), k, a)));
    normal
        .chain(contexts)
        .filter(|(_, _, a)| a.as_str() != UNBIND)
        .filter_map(|(c, k, a)| Some((c, normalize_keys(k).ok()?, a.clone())))
        .collect()
}

/// Whether sequence `long` starts with every chord of `short`.
fn starts(long: &str, short: &str) -> bool {
    long.len() > short.len()
        && long
            .strip_prefix(short)
            .is_some_and(|rest| rest.starts_with(' '))
}

/// Whether context `outer` encloses `inner`: everywhere encloses every
/// context, `session` encloses `session.arrange`.
fn encloses(outer: &str, inner: &str) -> bool {
    outer.is_empty()
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('.'))
}

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
fn parse_token(token: &str) -> Result<KeyChord, ConfigError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shown_shortcut_and_keymap_notation_are_one_binding() {
        assert_eq!(parse_shortcut("⌘W").as_deref(), Some("Meta+w"));
        assert_eq!(parse_shortcut("⇧⌘P").as_deref(), Some("Shift+Meta+p"));
        assert_eq!(
            parse_shortcut("Ctrl+Shift+Tab").as_deref(),
            Some("Ctrl+Shift+Tab")
        );
        assert_eq!(parse_shortcut("⌘K ⌘S").as_deref(), Some("Meta+k Meta+s"));
        assert_eq!(parse_shortcut("Esc").as_deref(), Some("Escape"));
        assert_eq!(display_keys("Shift+Meta+p"), "⇧⌘P");
        assert_eq!(display_keys("g g"), "G G");
        for shown in ["⌘W", "⌃⇧Tab", "⌥⌘←", "F5", "⌘,", "Esc", "⌘Space"] {
            let keys = parse_shortcut(shown).unwrap();
            assert_eq!(display_keys(&keys), shown, "{keys}");
        }
        assert_eq!(parse_shortcut("⌘"), None);
    }
}
