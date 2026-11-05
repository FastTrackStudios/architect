//! Control surfaces: a device's controls mapped to an app's targets.
//!
//! Faders, knobs, buttons, encoders, displays and LEDs, with banks,
//! feedback to the device, touch and value takeover (#311;
//! `docs/content/architecture/input.md`, "Control surfaces").
//!
//! - **Targets** ([`Target`], [`Targets`]): what an app lets a surface
//!   move — Studio's track volumes and FX parameters, the lab's metronome
//!   tempo. Each kind has a value range, a number of instances (eight
//!   tracks), and reads, writes and (optionally) observes its values.
//! - **A surface** ([`SurfaceDesc`]): its controls ([`ControlDesc`]),
//!   each with the message the device sends ([`Address`]), what it is sent
//!   back, its touch sensor, the target kind it moves and, for a strip
//!   control, its strip; and how many strips a bank is.
//! - **The engine** ([`Surface`]): turns what the device sends into
//!   target writes, and target values into feedback — motor faders, LEDs,
//!   scribble strips. Bank switching shifts which instances the strips
//!   hit; a touched fader is not moved under the person's finger; a knob
//!   with [`Takeover::Pickup`] writes only once it has reached the
//!   target's value, so a jump never happens.
//!
//! Pure logic over [`PortMessage`]s: who opens the port and sends what it
//! returns is the app's (`architect-extensions`' surfaces).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::control::{MidiKind, MidiPattern, parse_midi_pattern};
use crate::event::{MidiEvent, MidiMessage};
use crate::ports::{OscArg, OscMessage, PortMessage, midi_bytes, midi_event};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Targets ──────────────────────────────────────────────────────────

/// A kind of target an app offers: its range and how many there are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetKind {
    /// `<app or feature>.<what>` (`lab.metronome.tempo`, `track.volume`).
    pub id: String,
    pub title: String,
    pub min: f64,
    pub max: f64,
    /// What one encoder detent moves it (none: a hundredth of its range).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// How many there are (a track's per track): banks page through them.
    pub count: u32,
}

impl TargetKind {
    /// `value` as `0..=1` of its range.
    #[must_use]
    pub fn normalize(&self, value: f64) -> f64 {
        if self.max > self.min {
            ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// `0..=1` of its range as a value.
    #[must_use]
    pub fn value(&self, normal: f64) -> f64 {
        normal
            .clamp(0.0, 1.0)
            .mul_add(self.max - self.min, self.min)
    }

    /// One encoder detent.
    #[must_use]
    pub fn detent(&self) -> f64 {
        self.step.unwrap_or((self.max - self.min) / 100.0)
    }
}

/// Where a target says its value changed (not by the surface): the
/// instance and its value.
pub type TargetSink = Arc<dyn Fn(u32, f64) + Send + Sync>;

/// One kind of target an app offers surfaces.
pub trait Target: Send + Sync {
    fn kind(&self) -> TargetKind;

    /// Instance `index`'s value now (none: there is no such one).
    fn read(&self, index: u32) -> Option<f64>;

    /// Sets instance `index` to `value` (within its range).
    ///
    /// # Errors
    ///
    /// The app would not.
    fn write(&self, index: u32, value: f64) -> Result<(), String>;

    /// Instance `index`'s name, for a scribble strip (a track's).
    fn label(&self, index: u32) -> Option<String> {
        let _ = index;
        None
    }

    /// A fader moving instance `index` was touched or let go (automation's
    /// touch mode).
    fn touch(&self, index: u32, touched: bool) {
        let _ = (index, touched);
    }

    /// Tells `changed` each value that changes from elsewhere, from now
    /// on. None: the app says so itself ([`Targets::changed`]).
    fn observe(&self, changed: TargetSink) {
        let _ = changed;
    }
}

type Listener = Arc<dyn Fn(&str, u32, f64) + Send + Sync>;

#[derive(Default)]
struct TargetState {
    kinds: BTreeMap<String, Arc<dyn Target>>,
    listeners: Vec<Listener>,
}

/// The targets an app offers, by kind. Cheap to clone; shared.
#[derive(Clone, Default)]
pub struct Targets(Arc<Mutex<TargetState>>);

impl fmt::Debug for Targets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Targets")
            .field(&lock(&self.0).kinds.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Targets {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offers `target`, in place of one of its kind's id.
    pub fn register(&self, target: impl Target + 'static) {
        let target: Arc<dyn Target> = Arc::new(target);
        let id = target.kind().id;
        // Weakly: the target, kept here, holds this.
        let me = Arc::downgrade(&self.0);
        let kind = id.clone();
        target.observe(Arc::new(move |index, value| {
            if let Some(me) = me.upgrade() {
                Self(me).changed(&kind, index, value);
            }
        }));
        lock(&self.0).kinds.insert(id, target);
    }

    /// Takes kind `id` off: whether it was offered.
    // Whether it was is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn unregister(&self, id: &str) -> bool {
        lock(&self.0).kinds.remove(id).is_some()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn Target>> {
        lock(&self.0).kinds.get(id).cloned()
    }

    /// Every kind offered.
    #[must_use]
    pub fn kinds(&self) -> Vec<TargetKind> {
        let all: Vec<_> = lock(&self.0).kinds.values().cloned().collect();
        all.iter().map(|t| t.kind()).collect()
    }

    /// Instance `index` of kind `id` changed to `value` from elsewhere:
    /// tells whoever listens (the surfaces, to move their faders).
    pub fn changed(&self, id: &str, index: u32, value: f64) {
        let listeners = lock(&self.0).listeners.clone();
        for l in listeners {
            l(id, index, value);
        }
    }

    /// Hears every [`Self::changed`].
    pub fn listen(&self, listener: impl Fn(&str, u32, f64) + Send + Sync + 'static) {
        lock(&self.0).listeners.push(Arc::new(listener));
    }
}

// ── A surface's description ──────────────────────────────────────────

/// What a control is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    /// Absolute, maybe motorised (feedback moves it), maybe touch-sensing.
    #[default]
    Fader,
    /// Absolute, not motorised: [`Takeover`] says how it catches up.
    Knob,
    /// A press: a toggle of its target (min and max), or a bank switch.
    Button,
    /// Relative: each message a number of detents up or down.
    Encoder,
    /// Text: a scribble strip (feedback only).
    Display,
    /// On or off (feedback only).
    Led,
}

/// How an absolute control that is not where its target is takes over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Takeover {
    /// The target jumps to the control.
    #[default]
    Jump,
    /// Nothing is written until the control reaches the target's value
    /// (or crosses it); then it follows.
    Pickup,
}

/// What a button mapped to no target does with the bank.
pub const BANK_NEXT: &str = "bank.next";
/// The previous bank.
pub const BANK_PREV: &str = "bank.prev";

/// One control of a surface.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlDesc {
    pub id: String,
    pub kind: ControlKind,
    /// The strip it is on (`0..bank_size`): its target's instance is the
    /// bank's first plus this. None: [`Self::index`], whatever the bank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strip: Option<u32>,
    /// Its target's instance when it is on no strip (none: 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// What the device sends ([`Address`]): `CC7 ch1`, `PB ch1`,
    /// `Note16 ch1`, `/strip/1/fader`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// What it is sent back: none, its input for a fader, knob, encoder
    /// and button; a display's `sysex:F0 … {text} F7` or an OSC address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    /// Its touch sensor, a note (`Note104 ch1`): on while touched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touch: Option<String>,
    /// The target kind it moves, or [`BANK_NEXT`] / [`BANK_PREV`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub takeover: Takeover,
    /// A display's characters (none: 7, a Mackie strip's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
}

/// A control surface, as an extension describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceDesc {
    pub id: String,
    pub title: String,
    /// The port it is on: `midi:<name>` or `osc:<host>:<port>`.
    pub port: String,
    /// An OSC surface's local port, where the device sends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<u16>,
    /// Strips a bank is (none: one more than the highest strip).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank_size: Option<u32>,
    pub controls: Vec<ControlDesc>,
}

/// A message a control is heard or answered by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Address {
    /// A MIDI message: a note, a controller or a channel's pitch bend.
    Midi(MidiPattern),
    /// An OSC address: its first argument the value.
    Osc(String),
    /// A sysex whose `{text}` is a display's text: hex bytes around it.
    Sysex { before: Vec<u8>, after: Vec<u8> },
}

impl Address {
    /// `CC7 ch1`, `PB ch1`, `Note16 ch1`, `/strip/1/fader`, or
    /// `sysex:F0 00 00 66 14 12 00 {text} F7`.
    ///
    /// # Errors
    ///
    /// It is none of them; a MIDI one without a number (but `PB`) or a
    /// channel.
    // `{text}` is the template's placeholder, not a format argument.
    #[allow(clippy::literal_string_with_formatting_args)]
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.starts_with('/') {
            return Ok(Self::Osc(text.to_owned()));
        }
        if let Some(rest) = text.strip_prefix("sysex:") {
            let (before, after) = rest
                .split_once("{text}")
                .ok_or_else(|| format!("{text:?}: a sysex display needs {{text}}"))?;
            let hex = |s: &str| {
                s.split_whitespace()
                    .map(|b| u8::from_str_radix(b, 16).map_err(|e| format!("{text:?}: {b}: {e}")))
                    .collect::<Result<Vec<u8>, String>>()
            };
            return Ok(Self::Sysex {
                before: hex(before)?,
                after: hex(after)?,
            });
        }
        let p = parse_midi_pattern(text).map_err(|e| e.to_string())?;
        if p.channel.is_none() || (p.number.is_none() && p.kind != MidiKind::PitchBend) {
            return Err(format!(
                "{text:?}: a control's message names its number and channel"
            ));
        }
        if p.kind == MidiKind::Program {
            return Err(format!("{text:?}: a program change is not a control"));
        }
        Ok(Self::Midi(p))
    }

    /// The value `message` gives this address, `0..=1` (an encoder's: its
    /// detents, signed): none when it is not this address's.
    fn hears(&self, message: &PortMessage, relative: bool) -> Option<f64> {
        match (self, message) {
            (Self::Osc(a), PortMessage::Osc(m)) if *a == m.address => {
                Some(m.args.first().and_then(OscArg::number).unwrap_or(1.0))
            }
            (Self::Midi(p), PortMessage::Midi(bytes)) => {
                let MidiEvent { channel, message } = midi_event(bytes)?;
                if p.channel.is_some_and(|c| c != channel) {
                    return None;
                }
                match (p.kind, message) {
                    (MidiKind::Cc, MidiMessage::ControlChange { control, value })
                        if p.number == Some(control) =>
                    {
                        Some(if relative {
                            // Signed bit: 1–63 up, 65–127 down.
                            if value < 64 {
                                f64::from(value)
                            } else {
                                -f64::from(value.saturating_sub(64))
                            }
                        } else {
                            f64::from(value) / 127.0
                        })
                    }
                    (MidiKind::Note, MidiMessage::NoteOn { note, velocity })
                        if p.number == Some(note) =>
                    {
                        Some(if velocity > 0 { 1.0 } else { 0.0 })
                    }
                    (MidiKind::Note, MidiMessage::NoteOff { note, .. })
                        if p.number == Some(note) =>
                    {
                        Some(0.0)
                    }
                    (MidiKind::PitchBend, MidiMessage::PitchBend { value }) => {
                        Some(f64::from(value) / 16383.0)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The message saying `normal` (`0..=1`) at this address.
    fn says(&self, normal: f64) -> Option<PortMessage> {
        let normal = normal.clamp(0.0, 1.0);
        match self {
            // Rounded within 0..=127 and 0..=16383: the casts hold.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::as_conversions
            )]
            Self::Midi(p) => {
                let channel = p.channel.unwrap_or(1);
                let seven = (normal * 127.0).round() as u8;
                let message = match p.kind {
                    MidiKind::Cc => MidiMessage::ControlChange {
                        control: p.number?,
                        value: seven,
                    },
                    MidiKind::Note => MidiMessage::NoteOn {
                        note: p.number?,
                        velocity: seven,
                    },
                    MidiKind::PitchBend => MidiMessage::PitchBend {
                        value: (normal * 16383.0).round() as u16,
                    },
                    MidiKind::Program => return None,
                };
                Some(PortMessage::Midi(midi_bytes(&MidiEvent {
                    channel,
                    message,
                })))
            }
            #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
            Self::Osc(a) => Some(PortMessage::Osc(OscMessage::new(
                a.clone(),
                vec![OscArg::Float(normal as f32)],
            ))),
            Self::Sysex { .. } => None,
        }
    }

    /// The message showing `text` at this address (a display).
    fn shows(&self, text: &str, width: usize) -> Option<PortMessage> {
        let mut t: String = text
            .chars()
            .map(|c| {
                if c.is_ascii() && !c.is_ascii_control() {
                    c
                } else {
                    '?'
                }
            })
            .take(width)
            .collect();
        while t.len() < width {
            t.push(' ');
        }
        match self {
            Self::Osc(a) => Some(PortMessage::Osc(OscMessage::new(
                a.clone(),
                vec![OscArg::String(t.trim_end().to_owned())],
            ))),
            Self::Sysex { before, after } => {
                let mut bytes = before.clone();
                bytes.extend(t.bytes());
                bytes.extend(after);
                Some(PortMessage::Midi(bytes))
            }
            Self::Midi(_) => None,
        }
    }
}

// ── The engine ───────────────────────────────────────────────────────

struct Control {
    desc: ControlDesc,
    input: Option<Address>,
    feedback: Option<Address>,
    touch: Option<Address>,
}

/// A surface, running: its bank, what is touched, which knobs have
/// picked up their targets.
pub struct Surface {
    desc: SurfaceDesc,
    controls: Vec<Control>,
    bank_size: u32,
    bank: u32,
    touched: BTreeSet<usize>,
    picked: BTreeSet<usize>,
    last: BTreeMap<usize, f64>,
}

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Surface")
            .field("id", &self.desc.id)
            .field("bank", &self.bank)
            .field("bank_size", &self.bank_size)
            .finish_non_exhaustive()
    }
}

/// Close enough for a pickup to catch: a fiftieth of the range.
const CATCH: f64 = 0.02;

impl Surface {
    /// `desc`, running, on bank 0.
    ///
    /// # Errors
    ///
    /// A control's address does not parse, or a display or an LED has no
    /// feedback.
    pub fn new(desc: SurfaceDesc) -> Result<Self, String> {
        let parse = |what: &str, c: &ControlDesc, s: &Option<String>| {
            s.as_deref()
                .map(Address::parse)
                .transpose()
                .map_err(|e| format!("control {:?} {what}: {e}", c.id))
        };
        let mut controls = Vec::new();
        let mut ids = BTreeSet::new();
        for c in &desc.controls {
            if c.id.is_empty() || !ids.insert(c.id.as_str()) {
                return Err(format!("control {:?}: an id of its own", c.id));
            }
            let input = parse("input", c, &c.input)?;
            let feedback = parse("feedback", c, &c.feedback)?.or_else(|| match c.kind {
                ControlKind::Display | ControlKind::Led => None,
                _ => input.clone(),
            });
            if matches!(c.kind, ControlKind::Display | ControlKind::Led) && feedback.is_none() {
                return Err(format!("control {:?}: a {:?} needs feedback", c.id, c.kind));
            }
            controls.push(Control {
                desc: c.clone(),
                input,
                feedback,
                touch: parse("touch", c, &c.touch)?,
            });
        }
        let bank_size = desc.bank_size.unwrap_or_else(|| {
            desc.controls
                .iter()
                .filter_map(|c| c.strip)
                .max()
                .map_or(1, |s| s.saturating_add(1))
        });
        Ok(Self {
            desc,
            controls,
            bank_size: bank_size.max(1),
            bank: 0,
            touched: BTreeSet::new(),
            picked: BTreeSet::new(),
            last: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn desc(&self) -> &SurfaceDesc {
        &self.desc
    }

    #[must_use]
    pub const fn bank(&self) -> u32 {
        self.bank
    }

    #[must_use]
    pub const fn bank_size(&self) -> u32 {
        self.bank_size
    }

    /// How many banks its strips' targets have: the most instances of a
    /// kind a strip moves, a bank at a time.
    #[must_use]
    pub fn banks(&self, targets: &Targets) -> u32 {
        self.controls
            .iter()
            .filter(|c| c.desc.strip.is_some())
            .filter_map(|c| c.desc.target.as_deref().and_then(|t| targets.get(t)))
            .map(|t| t.kind().count.div_ceil(self.bank_size))
            .max()
            .unwrap_or(1)
            .max(1)
    }

    /// Control `i`'s target and instance on this bank.
    fn mapped(&self, targets: &Targets, i: usize) -> Option<(Arc<dyn Target>, TargetKind, u32)> {
        let c = &self.controls.get(i)?.desc;
        let target = targets.get(c.target.as_deref()?)?;
        let kind = target.kind();
        let index = match c.strip {
            Some(s) => self.bank.checked_mul(self.bank_size)?.checked_add(s)?,
            None => c.index.unwrap_or(0),
        };
        (index < kind.count).then_some((target, kind, index))
    }

    /// What control `i` should show now.
    fn feedback(&self, targets: &Targets, i: usize) -> Option<PortMessage> {
        let c = self.controls.get(i)?;
        let address = c.feedback.as_ref()?;
        let mapped = self.mapped(targets, i);
        if c.desc.kind == ControlKind::Display {
            let width = usize::try_from(c.desc.width.unwrap_or(7)).unwrap_or(7);
            let text = mapped.map_or_else(String::new, |(t, _, index)| {
                t.label(index)
                    .or_else(|| t.read(index).map(|v| format!("{v:.0}")))
                    .unwrap_or_default()
            });
            return address.shows(&text, width);
        }
        let normal = mapped.map_or(0.0, |(t, kind, index)| {
            kind.normalize(t.read(index).unwrap_or(kind.min))
        });
        let normal = match c.desc.kind {
            ControlKind::Button | ControlKind::Led => {
                if normal >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            _ => normal,
        };
        address.says(normal)
    }

    /// Everything it shows, as it is now: what to send the device when it
    /// starts, or on a new bank.
    #[must_use]
    pub fn refresh(&self, targets: &Targets) -> Vec<PortMessage> {
        (0..self.controls.len())
            .filter_map(|i| self.feedback(targets, i))
            .collect()
    }

    /// Bank `bank` (within its banks): its feedback, every control's.
    pub fn set_bank(&mut self, targets: &Targets, bank: u32) -> Vec<PortMessage> {
        self.bank = bank.min(self.banks(targets).saturating_sub(1));
        self.picked.clear();
        self.last.clear();
        self.refresh(targets)
    }

    /// Instance `index` of kind `kind` is now `value` (changed elsewhere):
    /// the feedback for the controls on it, but a touched fader's.
    pub fn changed(&mut self, targets: &Targets, kind: &str, index: u32) -> Vec<PortMessage> {
        let on: Vec<usize> = (0..self.controls.len())
            .filter(|i| {
                self.mapped(targets, *i)
                    .is_some_and(|(_, k, at)| k.id == kind && at == index)
            })
            .collect();
        let mut out = Vec::new();
        for i in on {
            // It moved away from the knob: a pickup catches it again.
            self.picked.remove(&i);
            if self.touched.contains(&i) {
                continue;
            }
            out.extend(self.feedback(targets, i));
        }
        out
    }

    /// What the device sent: the targets it moves written, and the
    /// feedback to send back (a bank's, a button's LED, a let-go fader's).
    pub fn input(&mut self, targets: &Targets, message: &PortMessage) -> Vec<PortMessage> {
        let mut out = Vec::new();
        for i in 0..self.controls.len() {
            let Some(c) = self.controls.get(i) else {
                continue;
            };
            let (desc, touch, input) = (c.desc.clone(), c.touch.clone(), c.input.clone());
            let kind = desc.kind;
            if let Some(v) = touch.as_ref().and_then(|t| t.hears(message, false)) {
                let touched = v >= 0.5;
                let mapped = self.mapped(targets, i);
                if touched {
                    self.touched.insert(i);
                } else {
                    self.touched.remove(&i);
                    // Let go: the motor goes to where the target is.
                    out.extend(self.feedback(targets, i));
                }
                if let Some((t, _, index)) = mapped {
                    t.touch(index, touched);
                }
                continue;
            }
            let relative = kind == ControlKind::Encoder;
            let Some(v) = input.as_ref().and_then(|a| a.hears(message, relative)) else {
                continue;
            };
            match kind {
                ControlKind::Button => {
                    if v < 0.5 {
                        continue;
                    }
                    match desc.target.as_deref() {
                        Some(BANK_NEXT) => {
                            out.extend(self.set_bank(targets, self.bank.saturating_add(1)));
                        }
                        Some(BANK_PREV) => {
                            out.extend(self.set_bank(targets, self.bank.saturating_sub(1)));
                        }
                        _ => {
                            if let Some((t, k, index)) = self.mapped(targets, i) {
                                let on = k.normalize(t.read(index).unwrap_or(k.min)) >= 0.5;
                                let _ = t.write(index, if on { k.min } else { k.max });
                                out.extend(self.feedback(targets, i));
                            }
                        }
                    }
                }
                ControlKind::Encoder => {
                    if let Some((t, k, index)) = self.mapped(targets, i) {
                        let now = t.read(index).unwrap_or(k.min);
                        let value = v.mul_add(k.detent(), now).clamp(k.min, k.max);
                        let _ = t.write(index, value);
                        out.extend(self.feedback(targets, i));
                    }
                }
                ControlKind::Fader | ControlKind::Knob => {
                    let Some((t, k, index)) = self.mapped(targets, i) else {
                        continue;
                    };
                    let last = self.last.insert(i, v);
                    if desc.takeover == Takeover::Pickup && !self.picked.contains(&i) {
                        let at = k.normalize(t.read(index).unwrap_or(k.min));
                        let crossed = last.is_some_and(|l| (l - at) * (v - at) <= 0.0);
                        if (v - at).abs() > CATCH && !crossed {
                            continue;
                        }
                        self.picked.insert(i);
                    }
                    let _ = t.write(index, k.value(v));
                }
                ControlKind::Display | ControlKind::Led => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_and_say_their_values() {
        let cc = Address::parse("CC7 ch2").unwrap();
        assert_eq!(cc.says(1.0), Some(PortMessage::Midi(vec![0xB1, 7, 127])));
        let pb = Address::parse("PB ch1").unwrap();
        assert_eq!(
            pb.says(0.5),
            Some(PortMessage::Midi(vec![0xE0, 0x00, 0x40]))
        );
        assert!(Address::parse("CC7").is_err(), "no channel");
        assert!(Address::parse("PC1 ch1").is_err());
        let sysex = Address::parse("sysex:F0 00 {text} F7").unwrap();
        assert_eq!(
            sysex.shows("Kick", 5),
            Some(PortMessage::Midi(vec![
                0xF0, 0, b'K', b'i', b'c', b'k', b' ', 0xF7
            ]))
        );
        let enc = Address::parse("CC16 ch1").unwrap();
        assert_eq!(
            enc.hears(&PortMessage::Midi(vec![0xB0, 16, 0x41]), true),
            Some(-1.0)
        );
        assert_eq!(
            enc.hears(&PortMessage::Midi(vec![0xB0, 16, 3]), true),
            Some(3.0)
        );
    }
}
