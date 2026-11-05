//! Input event types: keyboard, mouse, scroll and focus, and the MIDI and
//! OSC messages an app's control surfaces hand in.

use crate::key::{KeyCode, Modifiers};

/// Top-level input event.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Scroll(ScrollEvent),
    /// A MIDI message from a source the app plugged in (a control surface).
    Midi(MidiEvent),
    /// An OSC message from a source the app plugged in.
    Osc(OscEvent),
    FocusGained,
    FocusLost,
}

/// A keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What happened with the mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Click,
    DoubleClick,
    /// A press, a move and a release: [`MouseEvent::from`] is where it
    /// started, `x`/`y` where it ended.
    Drag,
    /// A press held in place.
    LongPress,
}

/// A mouse event.
#[derive(Debug, Clone, PartialEq)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub action: MouseAction,
    pub x: f64,
    pub y: f64,
    pub modifiers: Modifiers,
    /// Where a [`MouseAction::Drag`] started; `None` for any other gesture.
    pub from: Option<(f64, f64)>,
}

impl MouseEvent {
    /// A gesture at `(x, y)`.
    #[must_use]
    pub const fn new(
        button: MouseButton,
        action: MouseAction,
        (x, y): (f64, f64),
        modifiers: Modifiers,
    ) -> Self {
        Self {
            button,
            action,
            x,
            y,
            modifiers,
            from: None,
        }
    }

    /// A drag from `from` to `to`.
    #[must_use]
    pub const fn drag(
        button: MouseButton,
        from: (f64, f64),
        to: (f64, f64),
        modifiers: Modifiers,
    ) -> Self {
        Self {
            button,
            action: MouseAction::Drag,
            x: to.0,
            y: to.1,
            modifiers,
            from: Some(from),
        }
    }

    /// How far a drag went, `(dx, dy)`; `(0, 0)` for anything else.
    #[must_use]
    pub fn delta(&self) -> (f64, f64) {
        self.from
            .map_or((0.0, 0.0), |(fx, fy)| (self.x - fx, self.y - fy))
    }
}

/// A scroll event.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollEvent {
    pub delta_x: f64,
    pub delta_y: f64,
    pub modifiers: Modifiers,
}

/// A MIDI message on a channel (1–16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    pub channel: u8,
    pub message: MidiMessage,
}

/// The MIDI messages a binding can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
        velocity: u8,
    },
    ControlChange {
        control: u8,
        value: u8,
    },
    ProgramChange {
        program: u8,
    },
    /// A pitch bend, `0..=16383` (8192 the centre): a Mackie Control
    /// fader's position.
    PitchBend {
        value: u16,
    },
}

/// An OSC message: its address and its numeric arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct OscEvent {
    pub address: String,
    pub args: Vec<f32>,
}
