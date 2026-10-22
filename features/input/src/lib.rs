//! `architect-input`: the keymap engine.
//!
//! Key chords, a key trie, a modal input processor (mode stack, multi-key
//! sequences, counts, operator + motion, macros, held which-key
//! prefixes), mouse and scroll binding tables, a JSON keymap config with a
//! base + user merge, vim key notation, and the which-key popup model.
//!
//! It binds keys to **action ids**: by default `String`s naming actions in
//! the app's Action List, so a key runs the same action a menu, the
//! palette or an MCP agent runs (see
//! `docs/content/architecture/input.md`). The processor is generic over
//! the id type (`InputProcessor<A = String>`, any [`ActionKey`]).
//!
//! The default feature set is UI-framework-agnostic and wasm-clean: no
//! clock, no file IO, no runtime. Feature `dioxus` adds DOM event
//! conversion and a `use_input_processor` hook.

#![deny(unsafe_code)]

pub mod command;
pub mod config;
pub mod context;
pub mod event;
pub mod key;
pub mod macros;
pub mod mode;
pub mod mouse;
pub mod notation;
pub mod processor;
pub mod scroll;
pub mod sequence;
pub mod trie;
pub mod which_key;

#[cfg(feature = "dioxus")]
pub mod dioxus;

pub use command::{ActionKey, InputArgs, InputCommand};
pub use config::{
    ConfigError, ContextLayerConfig, KeymapConfig, ModeConfig, parse_key_chord, parse_key_sequence,
    parse_leaf_action,
};
pub use context::{InputContext, WhenExpr};
pub use event::{InputEvent, KeyEvent, MouseAction, MouseButton, MouseEvent, ScrollEvent};
pub use key::{KeyChord, KeyCode, Modifiers};
pub use macros::MacroRecorder;
pub use mode::{ModeDefinition, ModeId, ModeStack};
pub use mouse::{MouseBindingTable, MousePattern};
pub use notation::{KeyPlatform, translate_modifiers, translate_sequence};
pub use processor::InputProcessor;
pub use scroll::{ScrollAxis, ScrollBindingTable, ScrollPattern, table_for_surface};
pub use sequence::{SequenceResult, SequenceState};
pub use trie::{KeyTrie, LeafAction, TrieLookup, TrieNode};
pub use which_key::{
    WhichKey, WhichKeyEntry, WhichKeyEntryDef, WhichKeyLabels, WhichKeyNext, WhichKeyTreeDef,
};
