//! The input state machine: events in, [`InputCommand`]s out.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::command::{ActionKey, InputArgs, InputCommand};
use crate::config::{
    ConfigError, KeymapConfig, mode_definition_from_config, parse_key_sequence, parse_leaf_action,
    parse_mouse_pattern, parse_scroll_pattern, parse_when_expr,
};
use crate::context::{InputContext, WhenExpr};
use crate::control::{ControlBindings, parse_midi_pattern};
use crate::event::{InputEvent, KeyEvent, MidiEvent, MouseEvent, OscEvent, ScrollEvent};
use crate::key::{KeyChord, KeyCode, Modifiers};
use crate::macros::MacroRecorder;
use crate::mode::{ModeDefinition, ModeId, ModeStack};
use crate::mouse::MouseBindingTable;
use crate::scroll::ScrollBindingTable;
use crate::sequence::DEFAULT_TIMEOUT;
use crate::trie::{KeyTrie, LeafAction, TrieLookup, TrieNode};
use crate::which_key::{self, WhichKey, WhichKeyLabels};

/// Where a chord came from: typed by the user, or replayed from a macro
/// (which has no physical keydown/keyup and must not be re-recorded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Typed,
    Playback,
}

/// After `q` or `@`, the next key names a macro register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterPrompt {
    Record,
    Play,
}

/// Whether `q` and `@` drive vim macros.
///
/// [`MacroKeys::Vim`] (the default): an unbound `q<reg>` records, `q`
/// stops, `@<reg>` / `@@` plays, so the key after an unbound `q` or `@`
/// names a register and runs nothing else. [`MacroKeys::Off`]: `q` and
/// `@` are ordinary keys, unbound ones come back
/// [`InputCommand::Unhandled`], and nothing is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacroKeys {
    #[default]
    Vim,
    Off,
}

/// A key held down as a modifier, waiting to see what comes next.
#[derive(Debug, Clone)]
struct Hold {
    chord: KeyChord,
    /// Another input arrived while it was down: it was a modifier, not a
    /// tap.
    used: bool,
}

/// What a key going up did, from [`InputProcessor::release`].
#[derive(Debug, Clone, PartialEq)]
pub struct Released<A = String> {
    /// [`InputCommand::Unhold`] when a held-letter key went up, and the
    /// tap's own commands when nothing came while it was down.
    pub commands: Vec<InputCommand<A>>,
    /// Hide a lingering which-key overlay (see
    /// [`InputProcessor::notify_key_release`]).
    pub hide_which_key: bool,
}

/// What a held key does while it is down: the gestures it enables, for
/// its cursor and the which-key hint ("Z: drag — zoom to box · click —
/// zoom in").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldTool<A = String> {
    pub key: KeyChord,
    /// Each gesture (`z+Left.Drag`) with its action, the most specific
    /// context's first.
    pub gestures: Vec<(String, A)>,
}

/// A pending sequence's timer, from
/// [`InputProcessor::sequence_timeout`].
///
/// Arm a timer of [`after`](Self::after) and hand the ticket back to
/// [`InputProcessor::expire_sequence`] when it fires. A ticket goes stale
/// when the processor takes another key, the sequence times out, or the
/// config reloads; a stale ticket expires nothing, so a host needs no
/// epoch counter of its own and never has to cancel a timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceTimeout {
    after: Duration,
    generation: u64,
}

impl SequenceTimeout {
    /// How long to wait before expiring the sequence.
    #[must_use]
    pub const fn after(&self) -> Duration {
        self.after
    }
}

/// The input state machine.
///
/// Feed it events with [`process`](Self::process); it answers with the
/// commands they produce. It owns no clock: when
/// [`sequence_timeout`](Self::sequence_timeout) gives a ticket the host
/// arms a timer of [`SequenceTimeout::after`] and calls
/// [`expire_sequence`](Self::expire_sequence) when it fires. Hosts that
/// see key releases call [`release`](Self::release) (or
/// [`notify_key_release`](Self::notify_key_release)), which powers
/// hold-to-keep-prefix (sticky which-key prefixes) and held-letter tools.
///
/// # Held-letter tools
///
/// A key a pointer or wheel pattern names as held (`Z+Left.Drag`) is a
/// modifier while it is down. Its press runs nothing: it comes back as
/// [`InputCommand::Hold`], and the pointer gestures that follow match
/// the patterns that name it. Its release says what it was, by what
/// arrived in between, not by a timer: if another input came (a gesture,
/// another key) it was a modifier and the release gives
/// [`InputCommand::Unhold`]; if nothing did, it was a tap and runs
/// whatever the key is bound to, after the `Unhold`.
pub struct InputProcessor<A = String> {
    modes: ModeStack<A>,
    keymaps: HashMap<ModeId, KeyTrie<A>>,
    context_keymaps: HashMap<ModeId, Vec<(WhenExpr, KeyTrie<A>)>>,
    mouse_bindings: HashMap<ModeId, MouseBindingTable<A>>,
    scroll_bindings: HashMap<ModeId, ScrollBindingTable<A>>,
    /// Each named context's keys, mouse and wheel bindings.
    context_keys: HashMap<String, KeyTrie<A>>,
    context_mouse: HashMap<String, MouseBindingTable<A>>,
    context_scroll: HashMap<String, ScrollBindingTable<A>>,
    /// MIDI and OSC, in every mode.
    controls: ControlBindings<A>,
    /// The held-letter key down now, if any.
    holding: Option<Hold>,
    sequence: Vec<KeyChord>,
    timeout: Duration,
    count_prefix: Option<u32>,
    pending_operator: Option<String>,
    macro_recorder: MacroRecorder,
    register_prompt: Option<RegisterPrompt>,
    is_playing_back_macro: bool,
    /// Chords between keydown and keyup.
    held_keys: HashSet<KeyChord>,
    /// The anchor (first chord of `sequence`) was released since the
    /// sequence began: tells a vim double-tap (`g g`) from a held prefix
    /// (hold `g`, tap `e`), which look the same by chord content alone.
    anchor_released_during_sequence: bool,
    /// While set, a match rewinds the sequence to this anchor instead of
    /// clearing it. Cleared on the anchor's keyup.
    sticky_anchor: Option<KeyChord>,
    /// Leaf actions fired during the current sticky sequence, so releasing
    /// the anchor can tell "done" (hide the overlay) from "exploring".
    sticky_actions_fired: u32,
    macro_keys: MacroKeys,
    /// Bumped on every key event, timeout and reload: a
    /// [`SequenceTimeout`] from an older generation is stale.
    generation: u64,
}

impl<A: ActionKey> InputProcessor<A> {
    /// A processor in Normal mode with no bindings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            modes: ModeStack::new(ModeId::normal()),
            keymaps: HashMap::new(),
            context_keymaps: HashMap::new(),
            mouse_bindings: HashMap::new(),
            scroll_bindings: HashMap::new(),
            context_keys: HashMap::new(),
            context_mouse: HashMap::new(),
            context_scroll: HashMap::new(),
            controls: ControlBindings::new(),
            holding: None,
            sequence: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            count_prefix: None,
            pending_operator: None,
            macro_recorder: MacroRecorder::new(),
            register_prompt: None,
            is_playing_back_macro: false,
            held_keys: HashSet::new(),
            anchor_released_during_sequence: false,
            sticky_anchor: None,
            sticky_actions_fired: 0,
            macro_keys: MacroKeys::Vim,
            generation: 0,
        }
    }

    /// This processor with `q` / `@` macro handling set to `keys`.
    #[must_use]
    pub const fn with_macro_keys(mut self, keys: MacroKeys) -> Self {
        self.set_macro_keys(keys);
        self
    }

    /// Turn vim macro keys on or off. Turning them off drops a half-typed
    /// `q` / `@` and stops a recording.
    pub const fn set_macro_keys(&mut self, keys: MacroKeys) {
        self.macro_keys = keys;
        if matches!(keys, MacroKeys::Off) {
            self.register_prompt = None;
            self.macro_recorder.stop_recording();
        }
    }

    #[must_use]
    pub const fn macro_keys(&self) -> MacroKeys {
        self.macro_keys
    }

    /// A processor built from a config. A `normal` mode is defined if the
    /// config has none.
    pub fn from_config(config: KeymapConfig) -> Result<Self, ConfigError> {
        let mut proc = Self::new();
        if !config.modes.contains_key(ModeId::NORMAL) {
            proc.add_mode(ModeDefinition::new(ModeId::normal(), "NORMAL"));
        }
        for (name, mode) in config.modes {
            proc.add_mode(mode_definition_from_config(&name, &mode));
        }
        for (name, bindings) in config.keymap {
            proc.set_keymap(ModeId::new(name), trie_from_bindings(&bindings)?);
        }
        for (name, layers) in config.keymap_context {
            let parsed = layers
                .into_iter()
                .map(|layer| {
                    Ok((
                        parse_when_expr(&layer.when),
                        trie_from_bindings(&layer.bindings)?,
                    ))
                })
                .collect::<Result<Vec<_>, ConfigError>>()?;
            proc.context_keymaps.insert(ModeId::new(name), parsed);
        }
        for (name, bindings) in config.mouse {
            proc.mouse_bindings
                .insert(ModeId::new(name), mouse_table(&bindings)?);
        }
        for (name, bindings) in config.scroll {
            proc.scroll_bindings
                .insert(ModeId::new(name), scroll_table(&bindings)?);
        }
        for (name, context) in config.contexts {
            proc.context_keys
                .insert(name.clone(), trie_from_bindings(&context.keys)?);
            proc.context_mouse
                .insert(name.clone(), mouse_table(&context.mouse)?);
            proc.context_scroll
                .insert(name, scroll_table(&context.scroll)?);
        }
        for (pattern, action) in &config.midi {
            proc.controls
                .bind_midi(parse_midi_pattern(pattern)?, A::from(action.clone()));
        }
        for (address, action) in &config.osc {
            proc.controls
                .bind_osc(address.trim(), A::from(action.clone()));
        }
        Ok(proc)
    }

    /// Replace all state with a processor built from `config`, keeping
    /// the [`MacroKeys`] setting. On error the processor is unchanged.
    pub fn reload_config(&mut self, config: KeymapConfig) -> Result<(), ConfigError> {
        let fresh = Self::from_config(config)?.with_macro_keys(self.macro_keys);
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            ..fresh
        };
        Ok(())
    }

    pub fn add_mode(&mut self, def: ModeDefinition<A>) {
        self.modes.add_mode(def);
    }

    pub fn set_keymap(&mut self, mode: ModeId, trie: KeyTrie<A>) {
        self.keymaps.insert(mode, trie);
    }

    /// Set context `name`'s keys (see [`KeymapConfig::contexts`]).
    pub fn set_context_keymap(&mut self, name: impl Into<String>, trie: KeyTrie<A>) {
        self.context_keys.insert(name.into(), trie);
    }

    /// Set context `name`'s mouse bindings.
    pub fn set_context_mouse(&mut self, name: impl Into<String>, table: MouseBindingTable<A>) {
        self.context_mouse.insert(name.into(), table);
    }

    /// Set context `name`'s wheel bindings.
    pub fn set_context_scroll(&mut self, name: impl Into<String>, table: ScrollBindingTable<A>) {
        self.context_scroll.insert(name.into(), table);
    }

    /// Set the MIDI and OSC bindings.
    pub fn set_controls(&mut self, controls: ControlBindings<A>) {
        self.controls = controls;
    }

    /// Every context's keys, by name.
    #[must_use]
    pub const fn context_tries(&self) -> &HashMap<String, KeyTrie<A>> {
        &self.context_keys
    }

    /// The MIDI and OSC bindings.
    #[must_use]
    pub const fn controls(&self) -> &ControlBindings<A> {
        &self.controls
    }

    /// Add a conditional layer to `mode`, checked (in insertion order)
    /// before its base keymap.
    pub fn add_context_keymap(&mut self, mode: ModeId, when: WhenExpr, trie: KeyTrie<A>) {
        self.context_keymaps
            .entry(mode)
            .or_default()
            .push((when, trie));
    }

    pub const fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    #[must_use]
    pub fn current_mode(&self) -> &ModeId {
        self.modes.current()
    }

    /// The typed prefix in compact form (`g`, `C-xC-s`), or `None` when no
    /// sequence is pending.
    #[must_use]
    pub fn pending_display(&self) -> Option<String> {
        if self.sequence.is_empty() {
            return None;
        }
        Some(self.sequence.iter().map(KeyChord::short_label).collect())
    }

    /// The chords of the pending sequence (empty when none is).
    #[must_use]
    pub fn pending_keys(&self) -> &[KeyChord] {
        &self.sequence
    }

    /// Whether a sequence is pending, so the host should arm the timeout.
    #[must_use]
    pub const fn needs_timeout(&self) -> bool {
        !self.sequence.is_empty()
    }

    /// The timer to arm for the pending sequence, or `None` when nothing
    /// is pending. Ask after each key; the same ticket comes back until
    /// the next key, so a host may skip arming one it already armed.
    #[must_use]
    pub const fn sequence_timeout(&self) -> Option<SequenceTimeout> {
        if self.sequence.is_empty() {
            return None;
        }
        Some(SequenceTimeout {
            after: self.timeout,
            generation: self.generation,
        })
    }

    /// `timeout`'s timer fired: expire the sequence as
    /// [`timeout_expired`](Self::timeout_expired) does, unless the ticket
    /// is stale (a key arrived since), in which case nothing happens.
    pub fn expire_sequence(&mut self, timeout: SequenceTimeout) -> Vec<InputCommand<A>> {
        if timeout.generation != self.generation {
            return Vec::new();
        }
        self.timeout_expired()
    }

    /// Whether the pending sequence's anchor is still physically held.
    ///
    /// A host driving the timeout from a periodic tick should not expire a
    /// held which-key prefix: it would flicker on every tick as OS
    /// auto-repeat re-enters it.
    #[must_use]
    pub fn is_anchor_held(&self) -> bool {
        self.sequence
            .first()
            .is_some_and(|anchor| self.held_keys.contains(anchor))
    }

    #[must_use]
    pub const fn timeout_duration(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn mode_stack(&self) -> &ModeStack<A> {
        &self.modes
    }

    /// Every mode's base keymap.
    #[must_use]
    pub const fn keymaps(&self) -> &HashMap<ModeId, KeyTrie<A>> {
        &self.keymaps
    }

    /// Every mode's conditional layers.
    #[must_use]
    pub const fn context_keymaps(&self) -> &HashMap<ModeId, Vec<(WhenExpr, KeyTrie<A>)>> {
        &self.context_keymaps
    }

    #[must_use]
    pub const fn mouse_bindings(&self) -> &HashMap<ModeId, MouseBindingTable<A>> {
        &self.mouse_bindings
    }

    #[must_use]
    pub const fn scroll_bindings(&self) -> &HashMap<ModeId, ScrollBindingTable<A>> {
        &self.scroll_bindings
    }

    #[must_use]
    pub const fn is_recording_macro(&self) -> bool {
        self.macro_recorder.is_recording()
    }

    /// What can follow the pending sequence in the current mode, for a
    /// which-key popup: the base keymap and every layer live in `ctx`,
    /// labelled from `labels` (see [`crate::which_key::labels`]). `None`
    /// when nothing is pending.
    #[must_use]
    pub fn which_key(&self, ctx: &InputContext, labels: &WhichKeyLabels) -> Option<WhichKey<A>> {
        if self.sequence.is_empty() {
            return None;
        }
        let nodes: Vec<&TrieNode<A>> = self
            .live_tries(self.modes.current(), ctx)
            .filter_map(|trie| trie.root()?.descend(&self.sequence))
            .collect();
        which_key::menu(&nodes, &self.sequence, labels)
    }

    /// The host saw `chord` released.
    ///
    /// A release matches by key, not modifiers: letting go of Ctrl before
    /// `g` reports a plain `g`, and that still releases a held `Ctrl+g`.
    ///
    /// Returns `true` when the host should hide a lingering which-key
    /// overlay: a held (sticky) prefix that fired at least one action has
    /// just been let go. A bare hold-and-release returns `false`, so the
    /// overlay keeps guiding the user.
    pub fn notify_key_release(&mut self, chord: &KeyChord) -> bool {
        let released = |held: &KeyChord| held.key == chord.key;
        self.held_keys.retain(|held| !released(held));
        if self.sequence.first().is_some_and(released) {
            self.anchor_released_during_sequence = true;
        }
        if !self.sticky_anchor.as_ref().is_some_and(released) {
            return false;
        }
        let fired = self.sticky_actions_fired > 0;
        self.sticky_anchor = None;
        self.sticky_actions_fired = 0;
        self.sequence.clear();
        self.count_prefix = None;
        self.pending_operator = None;
        self.register_prompt = None;
        fired
    }

    /// The held-letter key down now, if any.
    #[must_use]
    pub fn holding(&self) -> Option<&KeyChord> {
        self.holding.as_ref().map(|h| &h.chord)
    }

    /// What the held key does in `ctx` (its gestures, for the cursor and
    /// the which-key hint); `None` when no key is held.
    #[must_use]
    pub fn held_tool(&self, ctx: &InputContext) -> Option<HeldTool<A>> {
        let key = self.holding.as_ref()?.chord.clone();
        let mut gestures: Vec<(String, A)> = Vec::new();
        let mut add = |pattern: String, action: &A| {
            if !gestures.iter().any(|(p, _)| *p == pattern) {
                gestures.push((pattern, action.clone()));
            }
        };
        for table in self.live_mouse_tables(ctx) {
            for (pattern, when, action) in table.bindings() {
                if pattern.held.as_ref() == Some(&key.key) && when.evaluate(ctx) {
                    add(pattern.to_string(), action);
                }
            }
        }
        for table in self.live_scroll_tables(ctx) {
            for (pattern, when, action) in table.bindings() {
                if pattern.held.as_ref() == Some(&key.key) && when.evaluate(ctx) {
                    add(pattern.to_string(), action);
                }
            }
        }
        Some(HeldTool { key, gestures })
    }

    /// The host saw `chord` go up: [`notify_key_release`](Self::notify_key_release),
    /// and the end of a held-letter key (see the type's docs). Hosts that
    /// see keyups call this rather than `notify_key_release`.
    #[must_use = "what the release did: an Unhold, a tap's commands"]
    pub fn release(&mut self, chord: &KeyChord, ctx: &InputContext) -> Released<A> {
        let hide_which_key = self.notify_key_release(chord);
        let mut commands = Vec::new();
        if let Some(hold) = self.holding.take_if(|hold| hold.chord.key == chord.key) {
            commands.push(InputCommand::Unhold(hold.chord.clone()));
            if !hold.used {
                // Nothing came while it was down: it was a tap.
                self.generation = self.generation.wrapping_add(1);
                let event = InputEvent::Key(KeyEvent {
                    key: hold.chord.key.clone(),
                    modifiers: hold.chord.modifiers,
                });
                commands.extend(self.process_key_chord(hold.chord, event, ctx, Origin::Typed));
                self.held_keys.retain(|held| held.key != chord.key);
            }
        }
        Released {
            commands,
            hide_which_key,
        }
    }

    /// Process one event in `ctx`.
    pub fn process(&mut self, event: InputEvent, ctx: &InputContext) -> Vec<InputCommand<A>> {
        match event {
            InputEvent::Key(ref key) => {
                self.generation = self.generation.wrapping_add(1);
                let chord = KeyChord::new(key.key.clone(), key.modifiers);
                if let Some(commands) = self.hold_key(&chord, ctx) {
                    return commands;
                }
                self.process_key_chord(chord, event, ctx, Origin::Typed)
            }
            InputEvent::Mouse(ref mouse) => self.process_mouse_event(mouse, ctx),
            InputEvent::Scroll(ref scroll) => self.process_scroll_event(scroll, ctx),
            InputEvent::Midi(midi) => self.process_midi_event(midi),
            InputEvent::Osc(ref osc) => self.process_osc_event(osc),
            InputEvent::FocusGained | InputEvent::FocusLost => {
                vec![InputCommand::Unhandled(event)]
            }
        }
    }

    /// A key while a held-letter key is down, or the press of one: `Some`
    /// when the key was taken (a held key's press, or its auto-repeat).
    fn hold_key(&mut self, chord: &KeyChord, ctx: &InputContext) -> Option<Vec<InputCommand<A>>> {
        if let Some(hold) = &mut self.holding {
            if hold.chord.key == chord.key {
                return Some(Vec::new());
            }
            // Another key came: the held one was a modifier for it.
            hold.used = true;
            return None;
        }
        let idle = self.sequence.is_empty()
            && self.register_prompt.is_none()
            && chord.modifiers == Modifiers::NONE;
        if !idle || !self.hold_keys(ctx).contains(&chord.key) {
            return None;
        }
        self.held_keys.insert(chord.clone());
        self.holding = Some(Hold {
            chord: chord.clone(),
            used: false,
        });
        Some(vec![InputCommand::Hold(chord.clone())])
    }

    /// Every key a live pointer or wheel pattern holds as a modifier.
    fn hold_keys(&self, ctx: &InputContext) -> HashSet<KeyCode> {
        let mouse = self
            .live_mouse_tables(ctx)
            .flat_map(|t| t.bindings().iter().filter_map(|(p, _, _)| p.held.clone()));
        let scroll = self
            .live_scroll_tables(ctx)
            .flat_map(|t| t.bindings().iter().filter_map(|(p, _, _)| p.held.clone()));
        mouse.chain(scroll).collect()
    }

    /// Whether the current mode takes plain keys as text (no context
    /// bindings while typing).
    fn typing(&self) -> bool {
        self.modes
            .definition(self.modes.current())
            .is_some_and(|def| def.passthrough_text)
    }

    /// The live contexts' names, innermost first; none while typing.
    fn live_contexts<'a>(&self, ctx: &'a InputContext) -> impl Iterator<Item = &'a String> {
        let contexts = if self.typing() {
            &[][..]
        } else {
            ctx.contexts()
        };
        contexts.iter().rev()
    }

    /// The mouse tables live in `ctx`: the contexts' innermost first, then
    /// the mode's.
    fn live_mouse_tables<'a>(
        &'a self,
        ctx: &'a InputContext,
    ) -> impl Iterator<Item = &'a MouseBindingTable<A>> {
        self.live_contexts(ctx)
            .filter_map(|name| self.context_mouse.get(name))
            .chain(self.mouse_bindings.get(self.modes.current()))
    }

    /// The wheel tables live in `ctx`, as [`Self::live_mouse_tables`].
    fn live_scroll_tables<'a>(
        &'a self,
        ctx: &'a InputContext,
    ) -> impl Iterator<Item = &'a ScrollBindingTable<A>> {
        self.live_contexts(ctx)
            .filter_map(|name| self.context_scroll.get(name))
            .chain(self.scroll_bindings.get(self.modes.current()))
    }

    /// The held key, marked used: an input arrived while it was down.
    fn use_held(&mut self) -> Option<KeyCode> {
        let hold = self.holding.as_mut()?;
        hold.used = true;
        Some(hold.chord.key.clone())
    }

    /// The pending sequence timed out: it is dropped, and its first chord
    /// comes back as [`InputCommand::Unhandled`].
    pub fn timeout_expired(&mut self) -> Vec<InputCommand<A>> {
        self.generation = self.generation.wrapping_add(1);
        let Some(first) = self.sequence.first().cloned() else {
            return Vec::new();
        };
        self.sequence.clear();
        self.count_prefix = None;
        vec![InputCommand::Unhandled(InputEvent::Key(KeyEvent {
            key: first.key,
            modifiers: first.modifiers,
        }))]
    }

    fn process_mouse_event(
        &mut self,
        event: &MouseEvent,
        ctx: &InputContext,
    ) -> Vec<InputCommand<A>> {
        let held = self.use_held();
        let action = self
            .live_mouse_tables(ctx)
            .find_map(|table| table.match_held(event, held.as_ref(), ctx));
        vec![gesture(action, InputEvent::Mouse(event.clone()))]
    }

    fn process_scroll_event(
        &mut self,
        event: &ScrollEvent,
        ctx: &InputContext,
    ) -> Vec<InputCommand<A>> {
        let held = self.use_held();
        let action = self
            .live_scroll_tables(ctx)
            .find_map(|table| table.match_held(event, held.as_ref(), ctx));
        vec![gesture(action, InputEvent::Scroll(event.clone()))]
    }

    fn process_midi_event(&self, event: MidiEvent) -> Vec<InputCommand<A>> {
        let action = self.controls.match_midi(&event);
        vec![gesture(action, InputEvent::Midi(event))]
    }

    fn process_osc_event(&self, event: &OscEvent) -> Vec<InputCommand<A>> {
        let action = self.controls.match_osc(event);
        vec![gesture(action, InputEvent::Osc(event.clone()))]
    }

    fn process_key_chord(
        &mut self,
        chord: KeyChord,
        original_event: InputEvent,
        ctx: &InputContext,
        origin: Origin,
    ) -> Vec<InputCommand<A>> {
        if chord == KeyChord::plain(KeyCode::Escape) {
            return self.press_escape(&chord, ctx);
        }
        if origin == Origin::Typed {
            if let Some(commands) = self.handle_macro_controls(&chord, ctx) {
                return commands;
            }
            if self.handle_count_prefix(&chord, ctx) {
                return Vec::new();
            }
            if self.macro_recorder.is_recording() && !self.is_playing_back_macro {
                self.macro_recorder.record(chord.clone());
            }
            if self.swallow_auto_repeat(&chord) {
                return Vec::new();
            }
        }
        self.advance_sequence(chord, original_event, ctx, origin)
    }

    /// Escape: an idle Escape binding runs; otherwise Escape cancels.
    fn press_escape(&mut self, chord: &KeyChord, ctx: &InputContext) -> Vec<InputCommand<A>> {
        if self.sequence.is_empty() {
            let mode = self.modes.current().clone();
            if let TrieLookup::Match(action) =
                self.lookup_in_mode(&mode, ctx, std::slice::from_ref(chord))
            {
                return self.execute_leaf_action(action);
            }
        }
        self.handle_escape()
    }

    /// Track the chord as held; `true` when it is an OS auto-repeat inside
    /// a sequence and must be dropped.
    ///
    /// Inside a which-key sequence a repeat would re-enter the prefix (and
    /// re-flash the overlay), match a same-key `z z`, or toggle a leaf 30
    /// times a second under a sticky prefix. Outside one, repeats pass so
    /// held cursor movement keeps working.
    fn swallow_auto_repeat(&mut self, chord: &KeyChord) -> bool {
        let in_sequence = !self.sequence.is_empty() || self.sticky_anchor.is_some();
        let repeat = !self.held_keys.insert(chord.clone());
        repeat && in_sequence
    }

    fn advance_sequence(
        &mut self,
        chord: KeyChord,
        original_event: InputEvent,
        ctx: &InputContext,
        origin: Origin,
    ) -> Vec<InputCommand<A>> {
        if self.sequence.is_empty() {
            self.anchor_released_during_sequence = false;
        }
        self.sequence.push(chord);
        let mode = self.modes.current().clone();
        match self.lookup_in_mode(&mode, ctx, &self.sequence) {
            TrieLookup::Match(action) => {
                self.settle_after_match(origin);
                self.execute_leaf_action(action)
            }
            TrieLookup::Prefix => vec![InputCommand::Pending {
                display: self.pending_display().unwrap_or_default(),
            }],
            TrieLookup::Miss => {
                self.reset_sequence();
                self.handle_unmatched(original_event, &mode)
            }
        }
    }

    /// After a match: rewind to the anchor if it is still held (a sticky
    /// prefix, so the next key resolves as a sibling), else clear.
    fn settle_after_match(&mut self, origin: Origin) {
        let keep = self
            .sequence
            .first()
            .filter(|anchor| {
                self.sequence.len() > 1
                    && origin == Origin::Typed
                    && self.held_keys.contains(*anchor)
                    && !self.anchor_released_during_sequence
            })
            .cloned();
        self.sequence.clear();
        if let Some(anchor) = keep {
            self.sequence.push(anchor.clone());
            self.sticky_anchor = Some(anchor);
            self.sticky_actions_fired = self.sticky_actions_fired.saturating_add(1);
        }
    }

    fn reset_sequence(&mut self) {
        self.sequence.clear();
        self.sticky_anchor = None;
        self.sticky_actions_fired = 0;
        self.anchor_released_during_sequence = false;
        self.count_prefix = None;
        self.pending_operator = None;
    }

    /// The tries live in `ctx`, in the order they win: the mode's `when`
    /// layers, then the live contexts innermost first, then the mode's
    /// base keymap.
    fn live_tries<'a>(
        &'a self,
        mode: &ModeId,
        ctx: &'a InputContext,
    ) -> impl Iterator<Item = &'a KeyTrie<A>> {
        let layers = self
            .context_keymaps
            .get(mode)
            .into_iter()
            .flatten()
            .filter(move |(when, _)| when.evaluate(ctx))
            .map(|(_, trie)| trie);
        let contexts = self
            .live_contexts(ctx)
            .filter_map(|name| self.context_keys.get(name));
        layers.chain(contexts).chain(self.keymaps.get(mode))
    }

    /// The first match among the live tries; a prefix anywhere otherwise.
    fn lookup_in_mode(
        &self,
        mode: &ModeId,
        ctx: &InputContext,
        keys: &[KeyChord],
    ) -> TrieLookup<A> {
        let mut has_prefix = false;
        for trie in self.live_tries(mode, ctx) {
            match trie.lookup(keys) {
                TrieLookup::Match(action) => return TrieLookup::Match(action),
                TrieLookup::Prefix => has_prefix = true,
                TrieLookup::Miss => {}
            }
        }
        if has_prefix {
            TrieLookup::Prefix
        } else {
            TrieLookup::Miss
        }
    }

    fn is_bound(&self, chord: &KeyChord, ctx: &InputContext) -> bool {
        let mode = self.modes.current();
        self.lookup_in_mode(mode, ctx, std::slice::from_ref(chord)) != TrieLookup::Miss
    }

    /// Accumulate a vim count (`3` in `3j`); `true` when the digit was
    /// taken. A digit the keymap binds is the binding's, not a count.
    fn handle_count_prefix(&mut self, chord: &KeyChord, ctx: &InputContext) -> bool {
        if !self.sequence.is_empty() || chord.modifiers != Modifiers::NONE {
            return false;
        }
        let Some(digit) = chord.key.single_char().and_then(|c| c.to_digit(10)) else {
            return false;
        };
        if (digit == 0 && self.count_prefix.is_none()) || self.is_bound(chord, ctx) {
            return false;
        }
        self.count_prefix = Some(
            self.count_prefix
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit),
        );
        true
    }

    /// `q<reg>` records, `q` stops, `@<reg>` / `@@` plays. `Some` when the
    /// chord was a macro control.
    ///
    /// These are vim's defaults, not a claim on the keymap: a keymap that
    /// binds `q` or `@` keeps them, and macros go without that key. They
    /// fire only as standalone keys, so `g q` in a which-key prefix
    /// reaches its leaf.
    fn handle_macro_controls(
        &mut self,
        chord: &KeyChord,
        ctx: &InputContext,
    ) -> Option<Vec<InputCommand<A>>> {
        if self.macro_keys == MacroKeys::Off || chord.modifiers != Modifiers::NONE {
            return None;
        }
        let ch = chord.key.single_char()?;
        if let Some(prompt) = self.register_prompt.take() {
            return Some(self.answer_register_prompt(prompt, ch, ctx));
        }
        if !self.sequence.is_empty() || self.is_bound(chord, ctx) {
            return None;
        }
        match ch {
            'q' if self.macro_recorder.is_recording() => self.macro_recorder.stop_recording(),
            'q' => self.register_prompt = Some(RegisterPrompt::Record),
            '@' => self.register_prompt = Some(RegisterPrompt::Play),
            _ => return None,
        }
        Some(Vec::new())
    }

    fn answer_register_prompt(
        &mut self,
        prompt: RegisterPrompt,
        register: char,
        ctx: &InputContext,
    ) -> Vec<InputCommand<A>> {
        match prompt {
            RegisterPrompt::Record => {
                if register.is_ascii_lowercase() {
                    self.macro_recorder.start_recording(register);
                }
                self.count_prefix = None;
                Vec::new()
            }
            RegisterPrompt::Play if register == '@' => self.play_macro(None, ctx),
            RegisterPrompt::Play if register.is_ascii_lowercase() => {
                self.play_macro(Some(register), ctx)
            }
            RegisterPrompt::Play => Vec::new(),
        }
    }

    /// Replay a register (`None`: the last one played), count times.
    fn play_macro(&mut self, register: Option<char>, ctx: &InputContext) -> Vec<InputCommand<A>> {
        if self.is_playing_back_macro {
            return Vec::new();
        }
        let repeat = self.count_prefix.take().unwrap_or(1);
        let recorded = match register {
            Some(reg) => self.macro_recorder.play(reg),
            None => self.macro_recorder.play_last(),
        };
        let Some(recorded) = recorded else {
            return Vec::new();
        };
        self.is_playing_back_macro = true;
        let mut out = Vec::new();
        for chord in std::iter::repeat_n(&recorded, usize::try_from(repeat).unwrap_or(1)).flatten()
        {
            let event = InputEvent::Key(KeyEvent {
                key: chord.key.clone(),
                modifiers: chord.modifiers,
            });
            out.extend(self.process_key_chord(chord.clone(), event, ctx, Origin::Playback));
        }
        self.is_playing_back_macro = false;
        out
    }

    /// Escape with nothing bound to it: cancel the sequence, else pop a
    /// sub-mode, else return to Normal.
    fn handle_escape(&mut self) -> Vec<InputCommand<A>> {
        self.register_prompt = None;
        let commands = if !self.sequence.is_empty() {
            self.sequence.clear();
            Vec::new()
        } else if self.modes.depth() > 1 {
            self.modes.pop()
        } else if self.modes.current() != &ModeId::normal() {
            self.modes.switch_base(ModeId::normal())
        } else {
            return Vec::new();
        };
        self.count_prefix = None;
        self.pending_operator = None;
        commands
    }

    /// An unbound key: text in a passthrough mode, else unhandled.
    fn handle_unmatched(&self, original_event: InputEvent, mode: &ModeId) -> Vec<InputCommand<A>> {
        let passthrough = self
            .modes
            .definition(mode)
            .is_some_and(|def| def.passthrough_text);
        if passthrough
            && let InputEvent::Key(ref key) = original_event
            && let KeyCode::Character(ref text) = key.key
            && key.modifiers == Modifiers::NONE
        {
            return vec![InputCommand::InsertText(text.clone())];
        }
        vec![InputCommand::Unhandled(original_event)]
    }

    fn execute_leaf_action(&mut self, action: LeafAction<A>) -> Vec<InputCommand<A>> {
        match action {
            LeafAction::Action(id) => vec![self.action_command(id)],
            LeafAction::SwitchMode(mode) => {
                self.clear_composition();
                prepend(
                    InputCommand::SwitchMode(mode.clone()),
                    self.modes.switch_base(mode),
                )
            }
            LeafAction::PushMode(mode) => {
                self.clear_composition();
                prepend(InputCommand::PushMode(mode.clone()), self.modes.push(mode))
            }
            LeafAction::Operator(op) => {
                self.pending_operator = Some(op);
                Vec::new()
            }
            LeafAction::Motion(motion) => self.complete_operator(Completion::Motion(motion)),
            LeafAction::TextObject(object) => {
                self.complete_operator(Completion::TextObject(object))
            }
            LeafAction::Sequence(actions) => {
                let count = self.count_prefix.take();
                actions
                    .into_iter()
                    .map(|action| with_count(action, count))
                    .collect()
            }
            LeafAction::Unbind => Vec::new(),
        }
    }

    /// A motion or text object after a pending operator becomes
    /// `operator.<op>` with the composed args; without one it does nothing.
    fn complete_operator(&mut self, completion: Completion) -> Vec<InputCommand<A>> {
        let Some(operator) = self.pending_operator.take() else {
            return Vec::new();
        };
        let (motion, text_object) = match completion {
            Completion::Motion(m) => (Some(m), None),
            Completion::TextObject(o) => (None, Some(o)),
        };
        vec![InputCommand::ActionWithArgs {
            action: A::from(format!("operator.{operator}")),
            args: InputArgs {
                count: self.count_prefix.take(),
                operator: Some(operator),
                motion,
                text_object,
                register: None,
            },
        }]
    }

    fn action_command(&mut self, action: A) -> InputCommand<A> {
        with_count(action, self.count_prefix.take())
    }

    fn clear_composition(&mut self) {
        self.count_prefix = None;
        self.pending_operator = None;
    }
}

/// What completed an operator.
enum Completion {
    Motion(String),
    TextObject(String),
}

impl<A: ActionKey> Default for InputProcessor<A> {
    fn default() -> Self {
        Self::new()
    }
}

/// `action`, carrying `count` when there is one.
fn with_count<A>(action: A, count: Option<u32>) -> InputCommand<A> {
    match count {
        Some(count) => InputCommand::ActionWithArgs {
            action,
            args: InputArgs {
                count: Some(count),
                ..InputArgs::default()
            },
        },
        None => InputCommand::Action(action),
    }
}

/// A pointer, wheel or control match: the action with the input that
/// triggered it; no match is the input, unhandled.
fn gesture<A>(action: Option<A>, event: InputEvent) -> InputCommand<A> {
    match action {
        Some(action) => InputCommand::Gesture { action, event },
        None => InputCommand::Unhandled(event),
    }
}

fn prepend<A>(first: InputCommand<A>, mut rest: Vec<InputCommand<A>>) -> Vec<InputCommand<A>> {
    rest.insert(0, first);
    rest
}

fn trie_from_bindings<A: ActionKey>(
    bindings: &HashMap<String, String>,
) -> Result<KeyTrie<A>, ConfigError> {
    let mut trie = KeyTrie::new();
    for (seq, action) in bindings {
        trie.bind_leaf(&parse_key_sequence(seq)?, parse_leaf_action(action));
    }
    Ok(trie)
}

fn mouse_table<A: ActionKey>(
    bindings: &HashMap<String, String>,
) -> Result<MouseBindingTable<A>, ConfigError> {
    let mut table = MouseBindingTable::new();
    for (pattern, action) in bindings {
        table.insert(
            parse_mouse_pattern(pattern)?,
            WhenExpr::True,
            A::from(action.clone()),
        );
    }
    Ok(table)
}

fn scroll_table<A: ActionKey>(
    bindings: &HashMap<String, String>,
) -> Result<ScrollBindingTable<A>, ConfigError> {
    let mut table = ScrollBindingTable::new();
    for (pattern, action) in bindings {
        table.insert(
            parse_scroll_pattern(pattern)?,
            WhenExpr::True,
            A::from(action.clone()),
        );
    }
    Ok(table)
}
