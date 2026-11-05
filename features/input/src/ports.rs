//! MIDI ports and OSC endpoints: what an app opens, for itself and for its
//! extensions (#311; `docs/content/architecture/input.md`, "Ports").
//!
//! - **Messages** ([`PortMessage`]): a MIDI message as its bytes, an OSC
//!   message as its address and arguments, each with a JSON form an
//!   extension reads and writes ([`PortMessage::to_json`],
//!   [`PortMessage::from_json`]), and the codecs: [`midi_event`] and
//!   [`midi_bytes`] between bytes and a [`MidiEvent`], [`encode_osc`] and
//!   [`decode_osc`] for OSC 1.0 packets.
//! - **Backends** ([`PortBackend`]): what lists ports and opens one, its
//!   messages handed to an [`Inbox`]. [`Ports`] puts a MIDI backend and an
//!   OSC one together; [`VirtualPorts`] is an in-process one whose other
//!   end a test (or a demo) holds ([`VirtualDevice`]); [`UdpOsc`] (feature
//!   `udp`, native) sends and hears OSC over UDP. A system MIDI backend
//!   (`midir`, `CoreMIDI`) is the app's, behind the same trait.
//!
//! The types and the codecs are wasm-clean; only [`UdpOsc`] does IO.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::event::{MidiEvent, MidiMessage};

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a port speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortKind {
    /// A MIDI port, by its name.
    Midi,
    /// An OSC endpoint, by `host:port`.
    Osc,
}

impl PortKind {
    /// `midi` or `osc`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Midi => "midi",
            Self::Osc => "osc",
        }
    }

    /// The kind named `name` (`midi`, `osc`).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "midi" => Some(Self::Midi),
            "osc" => Some(Self::Osc),
            _ => None,
        }
    }
}

impl fmt::Display for PortKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Which way a port's messages go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// From the device to the app.
    In,
    /// From the app to the device.
    Out,
    /// Both.
    #[default]
    Both,
}

/// A port a backend has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortInfo {
    pub kind: PortKind,
    pub name: String,
    #[serde(default)]
    pub direction: Direction,
}

/// What to open: a MIDI port by name, or an OSC endpoint.
///
/// An OSC endpoint is `host:port`, where messages are sent, and `listen`
/// the local UDP port it hears on (none: one the system picks, which a
/// device answering its sender reaches).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub kind: PortKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<u16>,
}

/// Why a port could not be opened or a message not sent.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    #[error("no {kind} port {name:?}")]
    NotFound { kind: PortKind, name: String },
    #[error("this app has no {0} ports")]
    Unsupported(PortKind),
    #[error("not a {kind} message: {why}")]
    BadMessage { kind: PortKind, why: String },
    #[error("{0}")]
    Io(String),
    #[error("the port is closed")]
    Closed,
}

// ── Messages ─────────────────────────────────────────────────────────

/// One OSC argument. Its JSON form is the plain value: a whole number is
/// an `i`, any other number an `f`, a string an `s`, `true`/`false` a
/// `T`/`F`, `null` an `N`, an array of bytes a `b` (a blob).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OscArg {
    Nil,
    Bool(bool),
    Int(i32),
    Float(f32),
    String(String),
    Blob(Vec<u8>),
}

impl OscArg {
    /// Its value as a number (`T` 1, `F` 0), when it is one.
    #[must_use]
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(f64::from(*i)),
            Self::Float(f) => Some(f64::from(*f)),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
}

/// An OSC message: its address and its arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OscMessage {
    pub address: String,
    #[serde(default)]
    pub args: Vec<OscArg>,
}

impl OscMessage {
    #[must_use]
    pub fn new(address: impl Into<String>, args: Vec<OscArg>) -> Self {
        Self {
            address: address.into(),
            args,
        }
    }
}

/// One message through a port.
#[derive(Clone, Debug, PartialEq)]
pub enum PortMessage {
    /// One MIDI message's bytes (a sysex whole, `F0 … F7`).
    Midi(Vec<u8>),
    Osc(OscMessage),
}

impl PortMessage {
    #[must_use]
    pub const fn kind(&self) -> PortKind {
        match self {
            Self::Midi(_) => PortKind::Midi,
            Self::Osc(_) => PortKind::Osc,
        }
    }

    /// Its JSON form, what an extension reads.
    ///
    /// MIDI: `{bytes, type, channel?, number?, value?}` — `type` one of
    /// `note_on`, `note_off`, `cc`, `program`, `pitch_bend`, `sysex`,
    /// `other`; `channel` 1–16. OSC: `{address, args}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        match self {
            Self::Osc(m) => json!({ "address": m.address, "args": m.args }),
            Self::Midi(bytes) => {
                let mut out = json!({ "bytes": bytes });
                let fields = match midi_event(bytes) {
                    Some(MidiEvent { channel, message }) => match message {
                        MidiMessage::NoteOn { note, velocity } => {
                            json!({"type": "note_on", "channel": channel, "number": note, "value": velocity})
                        }
                        MidiMessage::NoteOff { note, velocity } => {
                            json!({"type": "note_off", "channel": channel, "number": note, "value": velocity})
                        }
                        MidiMessage::ControlChange { control, value } => {
                            json!({"type": "cc", "channel": channel, "number": control, "value": value})
                        }
                        MidiMessage::ProgramChange { program } => {
                            json!({"type": "program", "channel": channel, "number": program})
                        }
                        MidiMessage::PitchBend { value } => {
                            json!({"type": "pitch_bend", "channel": channel, "value": value})
                        }
                    },
                    None if bytes.first() == Some(&0xF0) => json!({ "type": "sysex" }),
                    None => json!({ "type": "other" }),
                };
                if let (Value::Object(out), Value::Object(fields)) = (&mut out, fields) {
                    out.extend(fields);
                }
                out
            }
        }
    }

    /// A message of `kind` from its JSON form: MIDI as `{bytes}` or as
    /// `{type, channel?, number?, value?}` (channel 1 when none); OSC as
    /// `{address, args?}`.
    ///
    /// # Errors
    ///
    /// It is not one.
    pub fn from_json(kind: PortKind, value: &Value) -> Result<Self, PortError> {
        let bad = |why: String| PortError::BadMessage { kind, why };
        match kind {
            PortKind::Osc => {
                let m: OscMessage =
                    serde_json::from_value(value.clone()).map_err(|e| bad(e.to_string()))?;
                if !m.address.starts_with('/') {
                    return Err(bad(format!("{:?} does not start with /", m.address)));
                }
                Ok(Self::Osc(m))
            }
            PortKind::Midi => {
                if let Some(bytes) = value.get("bytes") {
                    let bytes: Vec<u8> =
                        serde_json::from_value(bytes.clone()).map_err(|e| bad(e.to_string()))?;
                    if bytes.first().is_none_or(|b| b & 0x80 == 0) {
                        return Err(bad("its first byte is not a status byte".to_owned()));
                    }
                    return Ok(Self::Midi(bytes));
                }
                let field = |name: &str| value.get(name).and_then(Value::as_u64);
                let channel = u8::try_from(field("channel").unwrap_or(1))
                    .ok()
                    .filter(|c| (1..=16).contains(c))
                    .ok_or_else(|| bad("a channel is 1–16".to_owned()))?;
                let seven = |name: &str| {
                    u8::try_from(field(name).unwrap_or(0))
                        .ok()
                        .filter(|n| *n <= 127)
                        .ok_or_else(|| bad(format!("{name} is 0–127")))
                };
                let message = match value.get("type").and_then(Value::as_str) {
                    Some("note_on") => MidiMessage::NoteOn {
                        note: seven("number")?,
                        velocity: seven("value")?,
                    },
                    Some("note_off") => MidiMessage::NoteOff {
                        note: seven("number")?,
                        velocity: seven("value")?,
                    },
                    Some("cc") => MidiMessage::ControlChange {
                        control: seven("number")?,
                        value: seven("value")?,
                    },
                    Some("program") => MidiMessage::ProgramChange {
                        program: seven("number")?,
                    },
                    Some("pitch_bend") => MidiMessage::PitchBend {
                        value: u16::try_from(field("value").unwrap_or(8192))
                            .ok()
                            .filter(|v| *v <= 16383)
                            .ok_or_else(|| bad("a pitch bend is 0–16383".to_owned()))?,
                    },
                    other => {
                        return Err(bad(format!(
                            "type {other:?}: note_on, note_off, cc, program or pitch_bend (or bytes)"
                        )));
                    }
                };
                Ok(Self::Midi(midi_bytes(&MidiEvent { channel, message })))
            }
        }
    }
}

/// The event `bytes` are, when they are one [`MidiMessage`] names.
#[must_use]
pub fn midi_event(bytes: &[u8]) -> Option<MidiEvent> {
    let (&status, data) = bytes.split_first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let channel = (status & 0x0F).saturating_add(1);
    let at = |i: usize| data.get(i).copied().filter(|b| *b < 0x80);
    let message = match status & 0xF0 {
        0x80 => MidiMessage::NoteOff {
            note: at(0)?,
            velocity: at(1)?,
        },
        0x90 => MidiMessage::NoteOn {
            note: at(0)?,
            velocity: at(1)?,
        },
        0xB0 => MidiMessage::ControlChange {
            control: at(0)?,
            value: at(1)?,
        },
        0xC0 => MidiMessage::ProgramChange { program: at(0)? },
        0xE0 => MidiMessage::PitchBend {
            value: u16::from(at(0)?) | (u16::from(at(1)?) << 7),
        },
        _ => return None,
    };
    Some(MidiEvent { channel, message })
}

/// `event`'s bytes.
#[must_use]
pub fn midi_bytes(event: &MidiEvent) -> Vec<u8> {
    let ch = event.channel.clamp(1, 16).saturating_sub(1);
    match event.message {
        MidiMessage::NoteOff { note, velocity } => vec![0x80 | ch, note & 0x7F, velocity & 0x7F],
        MidiMessage::NoteOn { note, velocity } => vec![0x90 | ch, note & 0x7F, velocity & 0x7F],
        MidiMessage::ControlChange { control, value } => {
            vec![0xB0 | ch, control & 0x7F, value & 0x7F]
        }
        MidiMessage::ProgramChange { program } => vec![0xC0 | ch, program & 0x7F],
        MidiMessage::PitchBend { value } => {
            let v = value.min(16383);
            let lsb = u8::try_from(v & 0x7F).unwrap_or(0);
            let msb = u8::try_from(v >> 7).unwrap_or(0x7F);
            vec![0xE0 | ch, lsb, msb]
        }
    }
}

// ── OSC 1.0 ──────────────────────────────────────────────────────────

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn osc_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    pad4(out);
}

/// `message` as an OSC 1.0 packet.
#[must_use]
pub fn encode_osc(message: &OscMessage) -> Vec<u8> {
    let mut out = Vec::new();
    osc_string(&mut out, &message.address);
    let mut tags = String::from(",");
    for a in &message.args {
        tags.push(match a {
            OscArg::Nil => 'N',
            OscArg::Bool(true) => 'T',
            OscArg::Bool(false) => 'F',
            OscArg::Int(_) => 'i',
            OscArg::Float(_) => 'f',
            OscArg::String(_) => 's',
            OscArg::Blob(_) => 'b',
        });
    }
    osc_string(&mut out, &tags);
    for a in &message.args {
        match a {
            OscArg::Int(i) => out.extend_from_slice(&i.to_be_bytes()),
            OscArg::Float(f) => out.extend_from_slice(&f.to_be_bytes()),
            OscArg::String(s) => osc_string(&mut out, s),
            OscArg::Blob(b) => {
                let len = i32::try_from(b.len()).unwrap_or(i32::MAX);
                out.extend_from_slice(&len.to_be_bytes());
                out.extend_from_slice(b);
                pad4(&mut out);
            }
            OscArg::Nil | OscArg::Bool(_) => {}
        }
    }
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("too long")?;
        let out = self.bytes.get(self.at..end).ok_or("cut short")?;
        self.at = end;
        Ok(out)
    }

    fn four(&mut self) -> Result<[u8; 4], String> {
        self.take(4)?.try_into().map_err(|_| "cut short".to_owned())
    }

    fn string(&mut self) -> Result<String, String> {
        let rest = self.bytes.get(self.at..).ok_or("cut short")?;
        let len = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or("a string with no end")?;
        let s = std::str::from_utf8(rest.get(..len).ok_or("cut short")?)
            .map_err(|e| e.to_string())?
            .to_owned();
        self.take(len.saturating_add(4) & !3)?;
        Ok(s)
    }
}

fn decode_message(bytes: &[u8]) -> Result<OscMessage, String> {
    let mut r = Reader { bytes, at: 0 };
    let address = r.string()?;
    if !address.starts_with('/') {
        return Err(format!("{address:?} is not an address"));
    }
    if r.at >= bytes.len() {
        return Ok(OscMessage::new(address, Vec::new()));
    }
    let tags = r.string()?;
    let tags = tags.strip_prefix(',').ok_or("no type tags")?;
    let mut args = Vec::new();
    for t in tags.chars() {
        args.push(match t {
            'i' => OscArg::Int(i32::from_be_bytes(r.four()?)),
            'f' => OscArg::Float(f32::from_be_bytes(r.four()?)),
            's' | 'S' => OscArg::String(r.string()?),
            'b' => {
                let len =
                    usize::try_from(i32::from_be_bytes(r.four()?)).map_err(|e| e.to_string())?;
                let blob = r.take(len)?.to_vec();
                r.take(len.next_multiple_of(4).saturating_sub(len))?;
                OscArg::Blob(blob)
            }
            'T' => OscArg::Bool(true),
            'F' => OscArg::Bool(false),
            'N' | 'I' => OscArg::Nil,
            // Wider numbers, as the nearest of ours.
            'h' | 't' => {
                let n = i64::from_be_bytes(r.take(8)?.try_into().map_err(|_| "cut short")?);
                OscArg::Int(i32::try_from(n).unwrap_or(if n < 0 { i32::MIN } else { i32::MAX }))
            }
            'd' => {
                let d = f64::from_be_bytes(r.take(8)?.try_into().map_err(|_| "cut short")?);
                // An `f` is what this side has.
                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                OscArg::Float(d as f32)
            }
            other => return Err(format!("type tag {other:?}")),
        });
    }
    Ok(OscMessage::new(address, args))
}

/// The messages in OSC packet `bytes`: one, or every one of a bundle
/// (bundles inside bundles too), its time tags ignored.
///
/// # Errors
///
/// It is not an OSC packet.
pub fn decode_osc(bytes: &[u8]) -> Result<Vec<OscMessage>, PortError> {
    let bad = |why: String| PortError::BadMessage {
        kind: PortKind::Osc,
        why,
    };
    if let Some(rest) = bytes.strip_prefix(b"#bundle\0") {
        let mut r = Reader {
            bytes: rest,
            at: 8, // the time tag
        };
        let mut out = Vec::new();
        while r.at < rest.len() {
            let len = usize::try_from(u32::from_be_bytes(r.four().map_err(bad)?))
                .map_err(|e| bad(e.to_string()))?;
            out.extend(decode_osc(r.take(len).map_err(bad)?)?);
        }
        return Ok(out);
    }
    decode_message(bytes).map(|m| vec![m]).map_err(bad)
}

// ── Backends ─────────────────────────────────────────────────────────

/// Where an open port's messages go, from whatever thread hears them.
pub type Inbox = Arc<dyn Fn(PortMessage) + Send + Sync>;

/// A port, open: what it sends. Dropped, it closes.
pub trait Port: Send + Sync {
    /// Sends `message` out of it.
    ///
    /// # Errors
    ///
    /// It is the wrong kind, or the device is gone.
    fn send(&self, message: &PortMessage) -> Result<(), PortError>;
}

/// What lists ports and opens them: the system's MIDI, OSC over UDP, an
/// in-process [`VirtualPorts`].
pub trait PortBackend: Send + Sync {
    /// The ports there are now (an OSC backend, which has endpoints and
    /// no list, none).
    fn list(&self) -> Vec<PortInfo>;

    /// Opens `request`'s port: what it hears goes to `inbox`.
    ///
    /// # Errors
    ///
    /// There is no such port, or it would not open.
    fn open(&self, request: &OpenRequest, inbox: Inbox) -> Result<Box<dyn Port>, PortError>;
}

/// A MIDI backend and an OSC one, together: each request goes to its
/// kind's.
#[derive(Clone, Default)]
pub struct Ports {
    midi: Option<Arc<dyn PortBackend>>,
    osc: Option<Arc<dyn PortBackend>>,
}

impl fmt::Debug for Ports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ports")
            .field("midi", &self.midi.is_some())
            .field("osc", &self.osc.is_some())
            .finish()
    }
}

impl Ports {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Its MIDI ports from `backend`.
    #[must_use]
    pub fn with_midi(mut self, backend: impl PortBackend + 'static) -> Self {
        self.midi = Some(Arc::new(backend));
        self
    }

    /// Its OSC endpoints through `backend`.
    #[must_use]
    pub fn with_osc(mut self, backend: impl PortBackend + 'static) -> Self {
        self.osc = Some(Arc::new(backend));
        self
    }

    fn of(&self, kind: PortKind) -> Option<&Arc<dyn PortBackend>> {
        match kind {
            PortKind::Midi => self.midi.as_ref(),
            PortKind::Osc => self.osc.as_ref(),
        }
    }
}

impl PortBackend for Ports {
    fn list(&self) -> Vec<PortInfo> {
        [&self.midi, &self.osc]
            .into_iter()
            .flatten()
            .flat_map(|b| b.list())
            .collect()
    }

    fn open(&self, request: &OpenRequest, inbox: Inbox) -> Result<Box<dyn Port>, PortError> {
        self.of(request.kind)
            .ok_or(PortError::Unsupported(request.kind))?
            .open(request, inbox)
    }
}

/// One virtual port: who has it open, and what was sent to the device.
#[derive(Default)]
struct VirtualPort {
    open: BTreeMap<u64, Inbox>,
    sent: Vec<PortMessage>,
}

#[derive(Default)]
struct VirtualState {
    ports: BTreeMap<String, VirtualPort>,
    next: u64,
}

/// In-process ports of one kind, each a [`VirtualDevice`] at its other
/// end.
///
/// A loopback a test drives, or a demo device in an app. A message the
/// device sends reaches everyone who has its port open; what they send is
/// kept for the device to read.
#[derive(Clone)]
pub struct VirtualPorts {
    kind: PortKind,
    state: Arc<Mutex<VirtualState>>,
}

impl fmt::Debug for VirtualPorts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtualPorts")
            .field("kind", &self.kind)
            .field("ports", &lock(&self.state).ports.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl VirtualPorts {
    #[must_use]
    pub fn new(kind: PortKind) -> Self {
        Self {
            kind,
            state: Arc::default(),
        }
    }

    /// Adds port `name` (again: the same one): its device end.
    #[must_use]
    pub fn add(&self, name: impl Into<String>) -> VirtualDevice {
        let name = name.into();
        lock(&self.state).ports.entry(name.clone()).or_default();
        VirtualDevice {
            name,
            kind: self.kind,
            state: Arc::clone(&self.state),
        }
    }
}

impl PortBackend for VirtualPorts {
    fn list(&self) -> Vec<PortInfo> {
        lock(&self.state)
            .ports
            .keys()
            .map(|name| PortInfo {
                kind: self.kind,
                name: name.clone(),
                direction: Direction::Both,
            })
            .collect()
    }

    fn open(&self, request: &OpenRequest, inbox: Inbox) -> Result<Box<dyn Port>, PortError> {
        let id = {
            let mut s = lock(&self.state);
            let id = s.next;
            s.next = s.next.wrapping_add(1);
            s.ports
                .get_mut(&request.name)
                .ok_or_else(|| PortError::NotFound {
                    kind: self.kind,
                    name: request.name.clone(),
                })?
                .open
                .insert(id, inbox);
            id
        };
        Ok(Box::new(VirtualOpen {
            id,
            kind: self.kind,
            name: request.name.clone(),
            state: Arc::clone(&self.state),
        }))
    }
}

struct VirtualOpen {
    id: u64,
    kind: PortKind,
    name: String,
    state: Arc<Mutex<VirtualState>>,
}

impl Port for VirtualOpen {
    fn send(&self, message: &PortMessage) -> Result<(), PortError> {
        if message.kind() != self.kind {
            return Err(PortError::BadMessage {
                kind: self.kind,
                why: format!("a {} message", message.kind()),
            });
        }
        lock(&self.state)
            .ports
            .get_mut(&self.name)
            .ok_or(PortError::Closed)?
            .sent
            .push(message.clone());
        Ok(())
    }
}

impl Drop for VirtualOpen {
    fn drop(&mut self) {
        if let Some(p) = lock(&self.state).ports.get_mut(&self.name) {
            p.open.remove(&self.id);
        }
    }
}

/// A virtual port's device end: what it sends the app, and what the app
/// sent it.
#[derive(Clone)]
pub struct VirtualDevice {
    name: String,
    kind: PortKind,
    state: Arc<Mutex<VirtualState>>,
}

impl fmt::Debug for VirtualDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtualDevice")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl VirtualDevice {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sends `message` to everyone with its port open: how many.
    // How many is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn send(&self, message: &PortMessage) -> usize {
        let inboxes: Vec<Inbox> = lock(&self.state)
            .ports
            .get(&self.name)
            .map(|p| p.open.values().cloned().collect())
            .unwrap_or_default();
        for inbox in &inboxes {
            inbox(message.clone());
        }
        inboxes.len()
    }

    /// What the app has sent it since the last call.
    #[must_use]
    pub fn take(&self) -> Vec<PortMessage> {
        lock(&self.state)
            .ports
            .get_mut(&self.name)
            .map(|p| std::mem::take(&mut p.sent))
            .unwrap_or_default()
    }

    /// How many have its port open.
    #[must_use]
    pub fn listeners(&self) -> usize {
        lock(&self.state)
            .ports
            .get(&self.name)
            .map_or(0, |p| p.open.len())
    }
}

/// OSC over UDP (feature `udp`, native): an endpoint is `host:port`,
/// messages sent there from a socket of its own, which hears on `listen`
/// (or a port the system picks).
#[cfg(all(feature = "udp", not(target_arch = "wasm32")))]
#[derive(Clone, Copy, Debug, Default)]
pub struct UdpOsc;

#[cfg(all(feature = "udp", not(target_arch = "wasm32")))]
mod udp {
    use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::{
        Inbox, OpenRequest, Port, PortBackend, PortError, PortInfo, PortKind, PortMessage, UdpOsc,
        decode_osc, encode_osc,
    };

    struct Open {
        socket: UdpSocket,
        to: SocketAddr,
        closed: Arc<AtomicBool>,
    }

    impl Port for Open {
        fn send(&self, message: &PortMessage) -> Result<(), PortError> {
            let PortMessage::Osc(m) = message else {
                return Err(PortError::BadMessage {
                    kind: PortKind::Osc,
                    why: "a MIDI message".to_owned(),
                });
            };
            self.socket
                .send_to(&encode_osc(m), self.to)
                .map(|_| ())
                .map_err(|e| PortError::Io(e.to_string()))
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            self.closed.store(true, Ordering::Relaxed);
        }
    }

    impl PortBackend for UdpOsc {
        fn list(&self) -> Vec<PortInfo> {
            Vec::new()
        }

        fn open(&self, request: &OpenRequest, inbox: Inbox) -> Result<Box<dyn Port>, PortError> {
            let io = |e: std::io::Error| PortError::Io(format!("{}: {e}", request.name));
            let to = request
                .name
                .to_socket_addrs()
                .map_err(io)?
                .next()
                .ok_or_else(|| PortError::NotFound {
                    kind: PortKind::Osc,
                    name: request.name.clone(),
                })?;
            // A loopback endpoint is heard on loopback only.
            let ip = if to.ip().is_loopback() {
                "127.0.0.1"
            } else {
                "0.0.0.0"
            };
            let socket = UdpSocket::bind((ip, request.listen.unwrap_or(0))).map_err(io)?;
            socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(io)?;
            let closed = Arc::new(AtomicBool::new(false));
            let (hear, done) = (socket.try_clone().map_err(io)?, Arc::clone(&closed));
            std::thread::Builder::new()
                .name(format!("osc {}", request.name))
                .spawn(move || {
                    let mut buf = vec![0u8; 65_536];
                    while !done.load(Ordering::Relaxed) {
                        let Ok((n, _from)) = hear.recv_from(&mut buf) else {
                            continue;
                        };
                        if let Ok(messages) = decode_osc(buf.get(..n).unwrap_or_default()) {
                            for m in messages {
                                inbox(PortMessage::Osc(m));
                            }
                        }
                    }
                })
                .map_err(io)?;
            Ok(Box::new(Open { socket, to, closed }))
        }
    }

    impl UdpOsc {
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_round_trips_through_its_packet() {
        let m = OscMessage::new(
            "/strip/1/fader",
            vec![
                OscArg::Float(0.5),
                OscArg::Int(-3),
                OscArg::String("Kick".to_owned()),
                OscArg::Bool(true),
                OscArg::Nil,
                OscArg::Blob(vec![1, 2, 3]),
            ],
        );
        let bytes = encode_osc(&m);
        assert_eq!(bytes.len() % 4, 0);
        assert_eq!(decode_osc(&bytes).unwrap(), vec![m.clone()]);
        // In a bundle.
        let mut bundle = b"#bundle\0".to_vec();
        bundle.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        for _ in 0..2 {
            bundle.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
            bundle.extend_from_slice(&bytes);
        }
        assert_eq!(decode_osc(&bundle).unwrap(), vec![m.clone(), m]);
        assert!(decode_osc(b"nope").is_err());
    }

    #[test]
    fn midi_bytes_and_events_and_json_agree() {
        for bytes in [
            vec![0x90, 60, 100],
            vec![0x81, 60, 0],
            vec![0xB0, 7, 127],
            vec![0xC9, 5],
            vec![0xE0, 0x00, 0x40],
        ] {
            let event = midi_event(&bytes).unwrap();
            assert_eq!(midi_bytes(&event), bytes);
            let json = PortMessage::Midi(bytes.clone()).to_json();
            assert_eq!(
                PortMessage::from_json(PortKind::Midi, &json).unwrap(),
                PortMessage::Midi(bytes.clone())
            );
            // Without its bytes: from its fields.
            let mut fields = json.clone();
            fields.as_object_mut().unwrap().remove("bytes");
            assert_eq!(
                PortMessage::from_json(PortKind::Midi, &fields).unwrap(),
                PortMessage::Midi(bytes)
            );
        }
        assert_eq!(
            midi_event(&[0xE0, 0x00, 0x40]).unwrap().message,
            MidiMessage::PitchBend { value: 8192 }
        );
        assert_eq!(
            PortMessage::Midi(vec![0xF0, 1, 0xF7]).to_json()["type"],
            "sysex"
        );
        assert!(
            PortMessage::from_json(PortKind::Midi, &json!({"type": "cc", "channel": 17})).is_err()
        );
        assert!(PortMessage::from_json(PortKind::Osc, &json!({"address": "x"})).is_err());
        let osc = json!({"address": "/a", "args": [1, 0.5, "s", true, null]});
        let PortMessage::Osc(m) = PortMessage::from_json(PortKind::Osc, &osc).unwrap() else {
            panic!("osc");
        };
        assert_eq!(
            m.args,
            vec![
                OscArg::Int(1),
                OscArg::Float(0.5),
                OscArg::String("s".to_owned()),
                OscArg::Bool(true),
                OscArg::Nil
            ]
        );
    }

    #[test]
    fn a_virtual_port_carries_both_ways_and_closes_on_drop() {
        let midi = VirtualPorts::new(PortKind::Midi);
        let device = midi.add("Lab Surface");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let h = Arc::clone(&heard);
        let port = midi
            .open(
                &OpenRequest {
                    kind: PortKind::Midi,
                    name: "Lab Surface".to_owned(),
                    listen: None,
                },
                Arc::new(move |m| lock(&h).push(m)),
            )
            .unwrap();
        assert_eq!(device.send(&PortMessage::Midi(vec![0xB0, 1, 2])), 1);
        assert_eq!(*lock(&heard), vec![PortMessage::Midi(vec![0xB0, 1, 2])]);
        port.send(&PortMessage::Midi(vec![0x90, 60, 1])).unwrap();
        assert_eq!(device.take(), vec![PortMessage::Midi(vec![0x90, 60, 1])]);
        drop(port);
        assert_eq!(device.listeners(), 0);
        assert!(matches!(
            midi.open(
                &OpenRequest {
                    kind: PortKind::Midi,
                    name: "Nope".to_owned(),
                    listen: None
                },
                Arc::new(|_| {})
            ),
            Err(PortError::NotFound { .. })
        ));
    }
}
