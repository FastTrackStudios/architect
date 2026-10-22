//! The keymap config: a serde model of modes and bindings, its parsers,
//! and the base + user merge.
//!
//! Bindings are strings both ways: a key sequence (`g g`, `Ctrl+s`,
//! `Shift+Tab`) to an action string. An action string is an action id
//! (`file.save`, an Action List id), or one of `mode:<id>`,
//! `operator:<op>`, `motion:<m>`, `textobj:<o>`, `unbind`.
//!
//! The crate reads no files: the app reads the bytes (a file, a bundled
//! asset, a fetch) and hands them to [`KeymapConfig::from_json_str`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::context::WhenExpr;
use crate::event::{MouseAction, MouseButton};
use crate::key::{KeyChord, KeyCode, Modifiers};
use crate::mode::{ModeDefinition, ModeId};
use crate::mouse::MousePattern;
use crate::scroll::{ScrollAxis, ScrollPattern};
use crate::trie::LeafAction;

/// Why a config could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to decode config: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid key sequence: {0}")]
    InvalidKeySequence(String),
    #[error("invalid key chord: {0}")]
    InvalidKeyChord(String),
    #[error("invalid mouse pattern: {0}")]
    InvalidMousePattern(String),
    #[error("invalid scroll pattern: {0}")]
    InvalidScrollPattern(String),
}

/// A whole keymap, as written in a config file. Every map is keyed by mode
/// name (`scroll` may also be keyed by surface, see
/// [`crate::table_for_surface`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapConfig {
    /// Mode definitions by mode id.
    #[serde(default)]
    pub modes: HashMap<String, ModeConfig>,
    /// Key sequence to action string, per mode.
    #[serde(default)]
    pub keymap: HashMap<String, HashMap<String, String>>,
    /// Conditional binding layers, per mode, checked before `keymap`.
    #[serde(default)]
    pub keymap_context: HashMap<String, Vec<ContextLayerConfig>>,
    /// Mouse pattern (`Ctrl+Left.DoubleClick`) to action id, per mode.
    #[serde(default)]
    pub mouse: HashMap<String, HashMap<String, String>>,
    /// Scroll pattern (`Shift+Scroll`) to action id, per mode or surface.
    #[serde(default)]
    pub scroll: HashMap<String, HashMap<String, String>>,
}

/// A mode as written in a config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeConfig {
    /// Status-line name; the upper-cased id when empty.
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub passthrough_text: bool,
    #[serde(default)]
    pub sticky: bool,
}

/// A binding layer live only while its `when` clause holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextLayerConfig {
    /// A [`WhenExpr`] source (`tab:performance`, `context:editor`).
    pub when: String,
    #[serde(default)]
    pub bindings: HashMap<String, String>,
}

impl KeymapConfig {
    /// Decode a JSON keymap.
    pub fn from_json_str(source: &str) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(source)?)
    }

    /// Merge a user config over a base:
    ///
    /// - user mode definitions replace the base's, per mode;
    /// - in each mode, the user owns every prefix root it binds into: base
    ///   bindings whose first chord the user also starts a binding with are
    ///   dropped, so a user `f` is a clean leaf rather than a prefix still
    ///   waiting on the base's `f f`, and a user `o s` does not inherit the
    ///   base's other `o *` entries;
    /// - a user value of `unbind` removes that sequence;
    /// - user context layers replace the base's, per mode;
    /// - mouse and scroll bindings merge per pattern, the user's winning.
    #[must_use]
    pub fn merge(base: Self, user: Self) -> Self {
        let mut merged = base;
        merged.modes.extend(user.modes);
        for (mode, user_bindings) in user.keymap {
            let mode_map = merged.keymap.entry(mode).or_default();
            overlay_bindings(mode_map, user_bindings);
        }
        merged.keymap_context.extend(user.keymap_context);
        merge_tables(&mut merged.mouse, user.mouse);
        merge_tables(&mut merged.scroll, user.scroll);
        merged
    }
}

/// Lay a user's bindings for one mode over the base's (see
/// [`KeymapConfig::merge`]).
fn overlay_bindings(base: &mut HashMap<String, String>, user: HashMap<String, String>) {
    let user_roots: HashSet<&str> = user.keys().filter_map(|seq| first_chord(seq)).collect();
    base.retain(|seq, _| first_chord(seq).is_none_or(|root| !user_roots.contains(root)));
    for (seq, action) in user {
        if is_unbind(&action) {
            base.remove(&seq);
        } else {
            base.insert(seq, action);
        }
    }
}

/// The first chord of a space-separated key sequence.
fn first_chord(seq: &str) -> Option<&str> {
    seq.split(' ').next().filter(|first| !first.is_empty())
}

fn is_unbind(action: &str) -> bool {
    action.trim().eq_ignore_ascii_case("unbind")
}

/// Merge pattern tables per mode, the user's entries winning.
fn merge_tables(
    base: &mut HashMap<String, HashMap<String, String>>,
    user: HashMap<String, HashMap<String, String>>,
) {
    for (mode, bindings) in user {
        base.entry(mode).or_default().extend(bindings);
    }
}

/// A [`ModeDefinition`] from its config entry.
#[must_use]
pub fn mode_definition_from_config<A>(id: &str, cfg: &ModeConfig) -> ModeDefinition<A> {
    let display_name = if cfg.display_name.is_empty() {
        id.to_uppercase()
    } else {
        cfg.display_name.clone()
    };
    ModeDefinition::new(ModeId::new(id), display_name)
        .with_passthrough_text(cfg.passthrough_text)
        .with_sticky(cfg.sticky)
}

/// Parse a whitespace-separated key sequence: `g g`, `Ctrl+x Ctrl+s`.
pub fn parse_key_sequence(input: &str) -> Result<Vec<KeyChord>, ConfigError> {
    let chords = input
        .split_whitespace()
        .map(parse_key_chord)
        .collect::<Result<Vec<_>, _>>()?;
    if chords.is_empty() {
        return Err(ConfigError::InvalidKeySequence(input.to_owned()));
    }
    Ok(chords)
}

/// Parse one chord: `+`-separated modifiers then a key (`Ctrl+Shift+s`,
/// `Escape`, `F5`, `Space`). A lone `+` is the plus key.
pub fn parse_key_chord(input: &str) -> Result<KeyChord, ConfigError> {
    if input == "+" {
        return Ok(KeyChord::plain(KeyCode::Character("+".into())));
    }
    let invalid = || ConfigError::InvalidKeyChord(input.to_owned());
    let (modifiers, key_token) = split_modifiers(input).ok_or_else(invalid)?;
    Ok(KeyChord::new(parse_key_code(key_token)?, modifiers))
}

/// Split `Ctrl+Alt+x` into its modifiers and the last token. `None` when a
/// modifier name is unknown.
fn split_modifiers(input: &str) -> Option<(Modifiers, &str)> {
    let mut parts: Vec<&str> = input.split('+').collect();
    let last = parts.pop()?.trim();
    let modifiers = parts
        .into_iter()
        .try_fold(Modifiers::NONE, |held, name| with_modifier(held, name))?;
    Some((modifiers, last))
}

/// `held` with the modifier `name` added; `None` for an unknown name.
fn with_modifier(held: Modifiers, name: &str) -> Option<Modifiers> {
    let mut held = held;
    match name.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => held.ctrl = true,
        "alt" | "option" => held.alt = true,
        "shift" => held.shift = true,
        "cmd" | "meta" | "super" => held.meta = true,
        _ => return None,
    }
    Some(held)
}

fn parse_key_code(token: &str) -> Result<KeyCode, ConfigError> {
    let t = token.trim();
    let mut chars = t.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(KeyCode::char(c));
    }
    let lower = t.to_ascii_lowercase();
    let code = match lower.as_str() {
        "comma" => KeyCode::Character(",".into()),
        "period" | "dot" => KeyCode::Character(".".into()),
        "space" => KeyCode::Character(" ".into()),
        "equals" | "equal" => KeyCode::Character("=".into()),
        "minus" | "dash" => KeyCode::Character("-".into()),
        "enter" | "return" | "kp_enter" => KeyCode::Enter,
        "escape" | "esc" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "backspace" | "bs" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "up" | "arrowup" => KeyCode::ArrowUp,
        "down" | "arrowdown" => KeyCode::ArrowDown,
        "left" | "arrowleft" => KeyCode::ArrowLeft,
        "right" | "arrowright" => KeyCode::ArrowRight,
        other if other.starts_with("kp_") => KeyCode::Character(other.to_owned()),
        other => {
            let n = other
                .strip_prefix('f')
                .and_then(|n| n.parse::<u8>().ok())
                .ok_or_else(|| ConfigError::InvalidKeyChord(token.to_owned()))?;
            KeyCode::F(n)
        }
    };
    Ok(code)
}

/// Parse an action string (see the module docs) into a leaf.
#[must_use]
pub fn parse_leaf_action<A: From<String>>(input: &str) -> LeafAction<A> {
    let tagged = |prefix: &str| {
        input
            .strip_prefix(prefix)
            .map(|rest| rest.trim().to_owned())
    };
    if let Some(mode) = tagged("mode:") {
        return LeafAction::SwitchMode(ModeId::new(mode));
    }
    if let Some(op) = tagged("operator:") {
        return LeafAction::Operator(op);
    }
    if let Some(motion) = tagged("motion:") {
        return LeafAction::Motion(motion);
    }
    if let Some(text_object) = tagged("textobj:") {
        return LeafAction::TextObject(text_object);
    }
    if is_unbind(input) {
        return LeafAction::Unbind;
    }
    LeafAction::Action(A::from(input.trim().to_owned()))
}

/// Parse a when-clause; see [`WhenExpr::parse`].
#[must_use]
pub fn parse_when_expr(input: &str) -> WhenExpr {
    WhenExpr::parse(input)
}

/// Parse a mouse pattern: `[Mods+]Button.Action`, e.g.
/// `Ctrl+Left.DoubleClick`.
pub fn parse_mouse_pattern(input: &str) -> Result<MousePattern, ConfigError> {
    let invalid = || ConfigError::InvalidMousePattern(input.to_owned());
    let (lhs, rhs) = input.split_once('.').ok_or_else(invalid)?;
    let (modifiers, button) = split_modifiers(lhs).ok_or_else(invalid)?;
    let button = parse_mouse_button(button).ok_or_else(invalid)?;
    let action = parse_mouse_action(rhs.trim()).ok_or_else(invalid)?;
    Ok(MousePattern::new(button, action, modifiers))
}

/// Parse a scroll pattern: `[Mods+]Axis`, the axis one of `Scroll`
/// (any), `ScrollX`, `ScrollY` (and their aliases).
pub fn parse_scroll_pattern(input: &str) -> Result<ScrollPattern, ConfigError> {
    let invalid = || ConfigError::InvalidScrollPattern(input.to_owned());
    let (modifiers, axis) = split_modifiers(input).ok_or_else(invalid)?;
    let axis = match axis.to_ascii_lowercase().as_str() {
        "scroll" | "wheel" => ScrollAxis::Any,
        "scrollx" | "wheelx" | "hscroll" | "horizontal" => ScrollAxis::Horizontal,
        "scrolly" | "wheely" | "vscroll" | "vertical" => ScrollAxis::Vertical,
        _ => return Err(invalid()),
    };
    Ok(ScrollPattern::new(axis, modifiers))
}

fn parse_mouse_button(token: &str) -> Option<MouseButton> {
    match token.to_ascii_lowercase().as_str() {
        "left" => Some(MouseButton::Left),
        "right" => Some(MouseButton::Right),
        "middle" => Some(MouseButton::Middle),
        _ => None,
    }
}

fn parse_mouse_action(token: &str) -> Option<MouseAction> {
    match token.to_ascii_lowercase().as_str() {
        "press" | "down" => Some(MouseAction::Press),
        "release" | "up" => Some(MouseAction::Release),
        "click" => Some(MouseAction::Click),
        "doubleclick" | "double_click" | "double-click" => Some(MouseAction::DoubleClick),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::InputCommand;
    use crate::context::InputContext;
    use crate::event::{InputEvent, KeyEvent, MouseEvent, ScrollEvent};
    use crate::processor::InputProcessor;

    const SAMPLE: &str = r#"{
        "modes": {
            "normal": { "display_name": "NORMAL" },
            "insert": { "display_name": "INSERT", "passthrough_text": true }
        },
        "keymap": {
            "normal": {
                "j": "cursor.down",
                "g g": "cursor.top",
                "i": "mode:insert",
                "d": "operator:delete",
                "w": "motion:word"
            }
        },
        "keymap_context": {
            "normal": [
                { "when": "tab:performance", "bindings": { "j": "perf.next_song" } }
            ]
        },
        "mouse": {
            "normal": {
                "Left.Click": "mouse.primary",
                "Ctrl+Left.DoubleClick": "mouse.ctrl_double"
            }
        },
        "scroll": {
            "normal": {
                "Ctrl+Scroll": "view.zoom",
                "Shift+ScrollX": "view.hscroll"
            }
        }
    }"#;

    fn one_mode(mode: &str, seq: &str, action: &str) -> KeymapConfig {
        let mut cfg = KeymapConfig::default();
        cfg.keymap
            .entry(mode.into())
            .or_default()
            .insert(seq.into(), action.into());
        cfg
    }

    #[test]
    fn test_decode_json_and_modes() {
        let cfg = KeymapConfig::from_json_str(SAMPLE).unwrap();
        assert!(cfg.modes.contains_key("normal"));
        assert!(cfg.modes.contains_key("insert"));
    }

    #[test]
    fn test_parse_key_sequence_multi_chord() {
        assert_eq!(parse_key_sequence("g g").unwrap().len(), 2);
        assert!(parse_key_sequence("  ").is_err());
    }

    #[test]
    fn parse_key_chord_reads_modifiers_names_and_function_keys() {
        let chord = parse_key_chord("Ctrl+Shift+S").unwrap();
        assert_eq!(chord.key, KeyCode::char('s'));
        assert!(chord.modifiers.ctrl && chord.modifiers.shift);
        assert_eq!(parse_key_chord("F12").unwrap().key, KeyCode::F(12));
        assert_eq!(parse_key_chord("space").unwrap().key, KeyCode::char(' '));
        assert_eq!(parse_key_chord("+").unwrap().key, KeyCode::char('+'));
        assert!(parse_key_chord("Hyper+x").is_err());
        assert!(parse_key_chord("fnord").is_err());
    }

    #[test]
    fn parse_leaf_action_reads_every_prefix() {
        assert_eq!(
            parse_leaf_action::<String>("mode:insert"),
            LeafAction::SwitchMode(ModeId::insert())
        );
        assert_eq!(
            parse_leaf_action::<String>("operator:delete"),
            LeafAction::Operator("delete".into())
        );
        assert_eq!(
            parse_leaf_action::<String>("textobj:word"),
            LeafAction::TextObject("word".into())
        );
        assert_eq!(parse_leaf_action::<String>("Unbind"), LeafAction::Unbind);
        assert_eq!(
            parse_leaf_action::<String>(" FILE_SAVE "),
            LeafAction::Action("FILE_SAVE".into())
        );
    }

    #[test]
    fn test_parse_mouse_pattern_with_modifiers() {
        let pat = parse_mouse_pattern("Ctrl+Left.DoubleClick").unwrap();
        assert_eq!(pat.button, MouseButton::Left);
        assert_eq!(pat.action, MouseAction::DoubleClick);
        assert!(pat.modifiers.ctrl);
        assert!(parse_mouse_pattern("Left").is_err());
    }

    #[test]
    fn test_parse_scroll_pattern_with_modifiers() {
        let pat = parse_scroll_pattern("Ctrl+ScrollX").unwrap();
        assert_eq!(pat.axis, ScrollAxis::Horizontal);
        assert!(pat.modifiers.ctrl);
        assert!(parse_scroll_pattern("Ctrl+Sideways").is_err());
    }

    #[test]
    fn test_round_trip_from_config_process_flow() {
        let cfg = KeymapConfig::from_json_str(SAMPLE).unwrap();
        let mut processor: InputProcessor = InputProcessor::from_config(cfg).unwrap();
        let mut ctx = InputContext::new();
        ctx.set_var("tab", "performance");

        let cmds = processor.process(
            InputEvent::Key(KeyEvent {
                key: KeyCode::char('j'),
                modifiers: Modifiers::NONE,
            }),
            &ctx,
        );
        assert_eq!(
            cmds.first()
                .and_then(InputCommand::action)
                .map(String::as_str),
            Some("perf.next_song"),
            "the context layer wins"
        );

        let cmds = processor.process(
            InputEvent::Mouse(MouseEvent {
                button: MouseButton::Left,
                action: MouseAction::Click,
                x: 10.0,
                y: 20.0,
                modifiers: Modifiers::NONE,
            }),
            &ctx,
        );
        assert_eq!(
            cmds.first()
                .and_then(InputCommand::action)
                .map(String::as_str),
            Some("mouse.primary")
        );

        let cmds = processor.process(
            InputEvent::Scroll(ScrollEvent {
                delta_x: 0.0,
                delta_y: -8.0,
                modifiers: Modifiers::CTRL,
            }),
            &ctx,
        );
        assert_eq!(
            cmds.first()
                .and_then(InputCommand::action)
                .map(String::as_str),
            Some("view.zoom")
        );
    }

    #[test]
    fn test_merge_user_override_replaces_binding() {
        let merged = KeymapConfig::merge(
            one_mode("normal", "j", "cursor.down"),
            one_mode("normal", "j", "cursor.custom_down"),
        );
        assert_eq!(merged.keymap["normal"]["j"], "cursor.custom_down");
    }

    #[test]
    fn test_merge_unbind_removes_base_binding() {
        let merged = KeymapConfig::merge(
            one_mode("normal", "j", "cursor.down"),
            one_mode("normal", "j", "unbind"),
        );
        assert!(!merged.keymap["normal"].contains_key("j"));
    }

    #[test]
    fn test_merge_user_adds_new_binding() {
        let merged = KeymapConfig::merge(
            KeymapConfig::default(),
            one_mode("normal", "k", "cursor.up"),
        );
        assert_eq!(merged.keymap["normal"]["k"], "cursor.up");
    }

    #[test]
    fn a_user_prefix_owns_its_whole_subtree() {
        let mut base = one_mode("normal", "o x", "options.x");
        base.keymap.get_mut("normal").unwrap().extend([
            ("o y".into(), "options.y".into()),
            ("g g".into(), "top".into()),
        ]);
        let merged = KeymapConfig::merge(base, one_mode("normal", "o s", "options.s"));
        let normal = &merged.keymap["normal"];
        assert_eq!(normal.get("o s").map(String::as_str), Some("options.s"));
        assert!(!normal.contains_key("o x") && !normal.contains_key("o y"));
        assert_eq!(
            normal.get("g g").map(String::as_str),
            Some("top"),
            "siblings stay"
        );
    }
}
