//! Vim-style macro registers: record chords into `a`..`z`, play them back.

use crate::key::KeyChord;

const REGISTERS: usize = 26;

/// Recorded chord sequences, one per register `a`..`z`.
#[derive(Debug, Clone)]
pub struct MacroRecorder {
    registers: [Vec<KeyChord>; REGISTERS],
    recording: Option<usize>,
    last_played: Option<usize>,
}

impl Default for MacroRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MacroRecorder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            registers: std::array::from_fn(|_| Vec::new()),
            recording: None,
            last_played: None,
        }
    }

    /// Start recording into `register`, clearing it. `false` when the
    /// register is not a letter.
    pub fn start_recording(&mut self, register: char) -> bool {
        let Some(idx) = register_index(register) else {
            return false;
        };
        self.recording = Some(idx);
        if let Some(slot) = self.registers.get_mut(idx) {
            slot.clear();
        }
        true
    }

    pub const fn stop_recording(&mut self) {
        self.recording = None;
    }

    /// Append `chord` to the register being recorded, if any.
    pub fn record(&mut self, chord: KeyChord) {
        if let Some(slot) = self.recording.and_then(|idx| self.registers.get_mut(idx)) {
            slot.push(chord);
        }
    }

    /// The chords in `register`, remembered as the last one played.
    pub fn play(&mut self, register: char) -> Option<Vec<KeyChord>> {
        let idx = register_index(register)?;
        self.last_played = Some(idx);
        self.registers.get(idx).cloned()
    }

    /// The chords of the register played last (`@@`).
    #[must_use]
    pub fn play_last(&self) -> Option<Vec<KeyChord>> {
        self.registers.get(self.last_played?).cloned()
    }

    #[must_use]
    pub const fn is_recording(&self) -> bool {
        self.recording.is_some()
    }
}

/// `a`..`z` (either case) as `0..26`.
fn register_index(register: char) -> Option<usize> {
    let lower = register.to_ascii_lowercase();
    if !lower.is_ascii_lowercase() {
        return None;
    }
    ('a'..='z').position(|c| c == lower)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::KeyCode;

    fn chord(ch: char) -> KeyChord {
        KeyChord::plain(KeyCode::char(ch))
    }

    #[test]
    fn test_record_and_play_register() {
        let mut recorder = MacroRecorder::new();
        assert!(recorder.start_recording('a'));
        recorder.record(chord('j'));
        recorder.record(chord('j'));
        recorder.stop_recording();

        let seq = recorder.play('a').expect("register should exist");
        assert_eq!(seq.len(), 2);
    }

    #[test]
    fn test_invalid_register_rejected() {
        let mut recorder = MacroRecorder::new();
        assert!(!recorder.start_recording('1'));
        assert!(recorder.play('1').is_none());
    }

    #[test]
    fn test_play_last_register() {
        let mut recorder = MacroRecorder::new();
        recorder.start_recording('b');
        recorder.record(chord('k'));
        recorder.stop_recording();
        let _ = recorder.play('b');
        let seq = recorder.play_last().expect("has last");
        assert_eq!(seq.len(), 1);
    }
}
