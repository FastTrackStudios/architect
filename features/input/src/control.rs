//! MIDI and OSC bindings: a control surface's buttons, faders and
//! addresses bound to actions, like keys.
//!
//! The binding kinds live here; the sources do not. The app plugs in
//! whatever hears MIDI or OSC (a port, a socket, a plugin host) and hands
//! each message to [`crate::InputProcessor::process`] as
//! [`InputEvent::Midi`](crate::InputEvent::Midi) or
//! [`InputEvent::Osc`](crate::InputEvent::Osc). A match comes back as
//! [`InputCommand::Gesture`](crate::InputCommand::Gesture) carrying the
//! message, so a fader's value reaches the action.

use std::fmt;

use crate::config::ConfigError;
use crate::event::{MidiEvent, MidiMessage, OscEvent};

/// Which kind of MIDI message a pattern names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiKind {
    /// A note going down (a note-on with a velocity above 0).
    Note,
    /// A control change, at any value.
    Cc,
    /// A program change.
    Program,
}

/// A MIDI message a binding matches: `Note60`, `CC64 ch1`, `PC5`, `CC`
/// (any controller), `Note ch10` (any note on channel 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MidiPattern {
    pub kind: MidiKind,
    /// The note, controller or program; `None` for any.
    pub number: Option<u8>,
    /// The channel (1–16); `None` for any.
    pub channel: Option<u8>,
}

impl MidiPattern {
    /// Whether `event` is this message.
    #[must_use]
    pub fn matches(&self, event: &MidiEvent) -> bool {
        let number = match (self.kind, event.message) {
            (MidiKind::Note, MidiMessage::NoteOn { note, velocity }) if velocity > 0 => note,
            (MidiKind::Cc, MidiMessage::ControlChange { control, .. }) => control,
            (MidiKind::Program, MidiMessage::ProgramChange { program }) => program,
            _ => return false,
        };
        self.number.is_none_or(|n| n == number) && self.channel.is_none_or(|c| c == event.channel)
    }
}

/// `CC64 ch1`: what [`parse_midi_pattern`] reads.
impl fmt::Display for MidiPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            MidiKind::Note => "Note",
            MidiKind::Cc => "CC",
            MidiKind::Program => "PC",
        })?;
        if let Some(n) = self.number {
            write!(f, "{n}")?;
        }
        if let Some(c) = self.channel {
            write!(f, " ch{c}")?;
        }
        Ok(())
    }
}

/// Parse a MIDI pattern: a kind (`Note`, `CC`, `PC`), an optional number
/// (0–127), and an optional ` ch<1–16>`.
pub fn parse_midi_pattern(input: &str) -> Result<MidiPattern, ConfigError> {
    let invalid = || ConfigError::InvalidMidiPattern(input.to_owned());
    let mut words = input.split_whitespace();
    let head = words.next().ok_or_else(invalid)?;
    let channel = match words.next() {
        None => None,
        Some(word) => {
            let lower = word.to_ascii_lowercase();
            let n: u8 = lower
                .strip_prefix("ch")
                .and_then(|n| n.parse().ok())
                .filter(|n| (1..=16).contains(n))
                .ok_or_else(invalid)?;
            Some(n)
        }
    };
    if words.next().is_some() {
        return Err(invalid());
    }
    let split = head
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(head.len());
    let (kind, number) = head.split_at_checked(split).ok_or_else(invalid)?;
    let kind = match kind.to_ascii_lowercase().as_str() {
        "note" => MidiKind::Note,
        "cc" => MidiKind::Cc,
        "pc" | "program" => MidiKind::Program,
        _ => return Err(invalid()),
    };
    let number = if number.is_empty() {
        None
    } else {
        Some(
            number
                .parse::<u8>()
                .ok()
                .filter(|n| *n <= 127)
                .ok_or_else(invalid)?,
        )
    };
    Ok(MidiPattern {
        kind,
        number,
        channel,
    })
}

/// The MIDI and OSC bindings: the first match wins.
#[derive(Debug, Clone)]
pub struct ControlBindings<A = String> {
    midi: Vec<(MidiPattern, A)>,
    osc: Vec<(String, A)>,
}

impl<A> Default for ControlBindings<A> {
    fn default() -> Self {
        Self {
            midi: Vec::new(),
            osc: Vec::new(),
        }
    }
}

impl<A: Clone> ControlBindings<A> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind_midi(&mut self, pattern: MidiPattern, action: A) {
        self.midi.push((pattern, action));
    }

    /// Binds an OSC address (`/transport/play`), matched exactly.
    pub fn bind_osc(&mut self, address: impl Into<String>, action: A) {
        self.osc.push((address.into(), action));
    }

    #[must_use]
    pub fn midi(&self) -> &[(MidiPattern, A)] {
        &self.midi
    }

    #[must_use]
    pub fn osc(&self) -> &[(String, A)] {
        &self.osc
    }

    /// The action `event` is bound to.
    #[must_use]
    pub fn match_midi(&self, event: &MidiEvent) -> Option<A> {
        self.midi
            .iter()
            .find(|(p, _)| p.matches(event))
            .map(|(_, a)| a.clone())
    }

    /// The action `event`'s address is bound to.
    #[must_use]
    pub fn match_osc(&self, event: &OscEvent) -> Option<A> {
        self.osc
            .iter()
            .find(|(address, _)| *address == event.address)
            .map(|(_, a)| a.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_printed_pattern_parses_back_to_itself() {
        for source in ["Note60", "CC64 ch1", "PC5", "CC", "Note ch10"] {
            let pattern = parse_midi_pattern(source).unwrap();
            assert_eq!(pattern.to_string(), source);
        }
        assert!(parse_midi_pattern("CC200").is_err());
        assert!(parse_midi_pattern("CC64 ch17").is_err());
        assert!(parse_midi_pattern("Pitch").is_err());
    }

    #[test]
    fn a_note_pattern_hears_a_note_going_down_on_its_channel() {
        let pattern = parse_midi_pattern("Note60 ch1").unwrap();
        let on = |channel, velocity| MidiEvent {
            channel,
            message: MidiMessage::NoteOn { note: 60, velocity },
        };
        assert!(pattern.matches(&on(1, 100)));
        assert!(!pattern.matches(&on(2, 100)), "another channel");
        assert!(!pattern.matches(&on(1, 0)), "a note-on at 0 is a note-off");
    }
}
