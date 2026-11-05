//! A control surface (#311): a fake eight-fader device, Mackie-style
//! (a pitch bend per channel, touch notes, bank buttons, scribble
//! strips), mapped to sixteen targets — feedback, bank switching, touch,
//! pickup and an encoder.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

use std::sync::{Arc, Mutex};

use architect_input::ports::midi_bytes;
use architect_input::{
    ControlDesc, ControlKind, MidiEvent, MidiMessage, PortMessage, Surface, SurfaceDesc, Takeover,
    Target, TargetKind, TargetSink, Targets,
};

/// Sixteen volumes, 0–100, named `T1`…`T16`.
#[derive(Clone, Default)]
struct Volumes {
    values: Arc<Mutex<Vec<f64>>>,
    touched: Arc<Mutex<Vec<(u32, bool)>>>,
    sink: Arc<Mutex<Option<TargetSink>>>,
}

impl Volumes {
    fn new() -> Self {
        let v = Self::default();
        *v.values.lock().unwrap() = vec![50.0; 16];
        v
    }

    /// Changed from elsewhere (a mouse on the mixer).
    fn set(&self, index: u32, value: f64) {
        self.values.lock().unwrap()[index as usize] = value;
        let sink = self.sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            sink(index, value);
        }
    }

    fn get(&self, index: u32) -> f64 {
        self.values.lock().unwrap()[index as usize]
    }
}

impl Target for Volumes {
    fn kind(&self) -> TargetKind {
        TargetKind {
            id: "track.volume".to_owned(),
            title: "Volume".to_owned(),
            min: 0.0,
            max: 100.0,
            step: Some(1.0),
            count: 16,
        }
    }
    fn read(&self, index: u32) -> Option<f64> {
        self.values.lock().unwrap().get(index as usize).copied()
    }
    fn write(&self, index: u32, value: f64) -> Result<(), String> {
        self.values.lock().unwrap()[index as usize] = value;
        Ok(())
    }
    fn label(&self, index: u32) -> Option<String> {
        Some(format!("T{}", index + 1))
    }
    fn touch(&self, index: u32, touched: bool) {
        self.touched.lock().unwrap().push((index, touched));
    }
    fn observe(&self, changed: TargetSink) {
        *self.sink.lock().unwrap() = Some(changed);
    }
}

fn pb(channel: u8, value: u16) -> PortMessage {
    PortMessage::Midi(midi_bytes(&MidiEvent {
        channel,
        message: MidiMessage::PitchBend { value },
    }))
}

fn note(n: u8, velocity: u8) -> PortMessage {
    PortMessage::Midi(vec![0x90, n, velocity])
}

/// Eight faders (pitch bend, channels 1–8), their touch notes (104–111),
/// scribble strips, bank buttons (notes 46, 47), and an encoder.
fn desc() -> SurfaceDesc {
    let mut controls = Vec::new();
    for s in 0..8u32 {
        controls.push(ControlDesc {
            id: format!("fader{}", s + 1),
            kind: ControlKind::Fader,
            strip: Some(s),
            input: Some(format!("PB ch{}", s + 1)),
            touch: Some(format!("Note{} ch1", 104 + s)),
            target: Some("track.volume".to_owned()),
            ..ControlDesc::default()
        });
        controls.push(ControlDesc {
            id: format!("strip{}", s + 1),
            kind: ControlKind::Display,
            strip: Some(s),
            feedback: Some(format!("sysex:F0 00 00 66 14 12 {:02X} {{text}} F7", s * 7)),
            target: Some("track.volume".to_owned()),
            ..ControlDesc::default()
        });
    }
    controls.push(ControlDesc {
        id: "bank-".to_owned(),
        kind: ControlKind::Button,
        input: Some("Note46 ch1".to_owned()),
        target: Some("bank.prev".to_owned()),
        ..ControlDesc::default()
    });
    controls.push(ControlDesc {
        id: "bank+".to_owned(),
        kind: ControlKind::Button,
        input: Some("Note47 ch1".to_owned()),
        target: Some("bank.next".to_owned()),
        ..ControlDesc::default()
    });
    SurfaceDesc {
        id: "fake8".to_owned(),
        title: "Fake 8".to_owned(),
        port: "midi:Fake 8".to_owned(),
        bank_size: Some(8),
        controls,
        ..SurfaceDesc::default()
    }
}

fn faders(out: &[PortMessage]) -> Vec<(u8, u16)> {
    out.iter()
        .filter_map(|m| match m {
            PortMessage::Midi(b) => architect_input::ports::midi_event(b),
            PortMessage::Osc(_) => None,
        })
        .filter_map(|e| match e.message {
            MidiMessage::PitchBend { value } => Some((e.channel, value)),
            _ => None,
        })
        .collect()
}

fn strips(out: &[PortMessage]) -> Vec<String> {
    out.iter()
        .filter_map(|m| match m {
            PortMessage::Midi(b) if b.first() == Some(&0xF0) => Some(
                String::from_utf8_lossy(&b[7..b.len() - 1])
                    .trim_end()
                    .to_owned(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn eight_faders_move_their_bank_s_targets_with_feedback_and_bank_switching() {
    let volumes = Volumes::new();
    let targets = Targets::new();
    targets.register(volumes.clone());
    let mut surface = Surface::new(desc()).unwrap();
    assert_eq!(surface.banks(&targets), 2);

    // Started: every fader at its target, every strip named.
    let out = surface.refresh(&targets);
    assert_eq!(faders(&out).len(), 8);
    assert!(faders(&out).iter().all(|(_, v)| (8189..=8194).contains(v)));
    assert_eq!(
        strips(&out),
        ["T1", "T2", "T3", "T4", "T5", "T6", "T7", "T8"]
    );

    // Fader 3 to the top: track 3.
    assert!(surface.input(&targets, &pb(3, 16383)).is_empty());
    assert_eq!(volumes.get(2), 100.0);

    // Bank up: fader 3 is track 11; the device is told the new bank.
    let out = surface.input(&targets, &note(47, 127));
    assert_eq!(surface.bank(), 1);
    assert_eq!(strips(&out)[0], "T9");
    assert_eq!(faders(&out).len(), 8);
    surface.input(&targets, &pb(3, 0));
    assert_eq!(volumes.get(10), 0.0);
    assert_eq!(volumes.get(2), 100.0, "bank 0's track 3 is left alone");
    // Past the last bank: stays.
    surface.input(&targets, &note(47, 127));
    assert_eq!(surface.bank(), 1);
    // Back down.
    let out = surface.input(&targets, &note(46, 127));
    assert_eq!(surface.bank(), 0);
    assert!(
        faders(&out).contains(&(3, 16383)),
        "fader 3 back at track 3's top"
    );

    // A change elsewhere moves its motor fader...
    let out = surface.changed(&targets, "track.volume", 0);
    assert_eq!(faders(&out), [(1, 8192)]);
    // ...through the target's own observer too.
    let heard = Arc::new(Mutex::new(Vec::new()));
    let h = Arc::clone(&heard);
    targets
        .listen(move |kind, index, value| h.lock().unwrap().push((kind.to_owned(), index, value)));
    volumes.set(1, 25.0);
    assert_eq!(
        *heard.lock().unwrap(),
        [("track.volume".to_owned(), 1, 25.0)]
    );
    // A track on the other bank: no fader shows it.
    assert!(surface.changed(&targets, "track.volume", 12).is_empty());

    // Touched, the fader is not moved under the finger; let go, it goes
    // to where the target is.
    assert!(surface.input(&targets, &note(104, 127)).is_empty());
    volumes.set(0, 80.0);
    assert!(faders(&surface.changed(&targets, "track.volume", 0)).is_empty());
    let out = surface.input(&targets, &note(104, 0));
    assert_eq!(faders(&out), [(1, 13106)]);
    assert_eq!(*volumes.touched.lock().unwrap(), [(0, true), (0, false)]);
}

#[test]
fn a_pickup_knob_writes_only_once_it_reaches_its_target_and_an_encoder_steps() {
    let volumes = Volumes::new(); // 50 each
    let targets = Targets::new();
    targets.register(volumes.clone());
    let mut surface = Surface::new(SurfaceDesc {
        id: "knobs".to_owned(),
        port: "osc:127.0.0.1:9000".to_owned(),
        controls: vec![
            ControlDesc {
                id: "knob".to_owned(),
                kind: ControlKind::Knob,
                input: Some("CC20 ch1".to_owned()),
                target: Some("track.volume".to_owned()),
                takeover: Takeover::Pickup,
                ..ControlDesc::default()
            },
            ControlDesc {
                id: "enc".to_owned(),
                kind: ControlKind::Encoder,
                index: Some(1),
                input: Some("/enc".to_owned()),
                target: Some("track.volume".to_owned()),
                ..ControlDesc::default()
            },
        ],
        ..SurfaceDesc::default()
    })
    .unwrap();
    let cc = |v: u8| PortMessage::Midi(vec![0xB0, 20, v]);
    surface.input(&targets, &cc(10));
    assert_eq!(volumes.get(0), 50.0, "far below: not picked up");
    surface.input(&targets, &cc(30));
    assert_eq!(volumes.get(0), 50.0);
    // Crossing 50 picks it up; it follows from there.
    surface.input(&targets, &cc(70));
    assert!((volumes.get(0) - 55.1).abs() < 0.1);
    surface.input(&targets, &cc(127));
    assert_eq!(volumes.get(0), 100.0);

    // An encoder: three detents up, one down, a step each.
    let osc = |n: f32| {
        PortMessage::Osc(architect_input::OscMessage::new(
            "/enc",
            vec![architect_input::OscArg::Float(n)],
        ))
    };
    let out = surface.input(&targets, &osc(3.0));
    assert_eq!(volumes.get(1), 53.0);
    assert_eq!(out.len(), 1, "its ring shows it");
    surface.input(&targets, &osc(-1.0));
    assert_eq!(volumes.get(1), 52.0);
}

#[test]
fn a_bad_description_is_refused() {
    let mut d = desc();
    d.controls[0].input = Some("CC7".to_owned());
    assert!(Surface::new(d).unwrap_err().contains("fader1"));
    let mut d = desc();
    d.controls[1].feedback = None;
    assert!(Surface::new(d).unwrap_err().contains("feedback"));
}
