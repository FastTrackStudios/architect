//! MIDI ports, OSC endpoints and control surfaces, for extensions (#311),
//! with no hardware: a virtual MIDI port (`architect_input::VirtualPorts`)
//! and OSC over localhost UDP.
//!
//! - a background extension opens a virtual MIDI port and an OSC endpoint
//!   it was granted, hears each as a `port.message` event (which starts it
//!   again once it went idle) and answers through them;
//! - what no hook takes waits for `port_receive`;
//! - a port outside `capabilities.ports` is refused, and so is another
//!   extension's port, `ports` with none, a bad entry and a surface on a
//!   port it may not open;
//! - an extension's fake eight-fader surface moves the app's targets, with
//!   feedback and bank switching.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

use std::fmt::Write as _;
use std::net::UdpSocket;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::action::ActionList;
use architect_extensions::events::{BackgroundState, FakeClock};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::hostcall::{HostError, HostRequest};
use architect_extensions::{Origin, Package};
use architect_input::ports::{decode_osc, encode_osc, midi_event};
use architect_input::{
    MidiMessage, OscArg, OscMessage, PortKind, PortMessage, Ports, Target, TargetKind, TargetSink,
    Targets, UdpOsc, VirtualPorts,
};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const ID: &str = "lab.ports";

/// 2026-11-04 08:00 UTC.
const WED_8: u64 = 1_793_779_200_000;

fn manifest(ports: &str, extra: &str) -> String {
    format!(
        r#"
id = "lab.ports"
name = "Ports"
version = "0.1.0"
kind = "luau"

[capabilities]
ports = [{ports}]

[[actions]]
id = "LAB_PORTS_CALL"
name = "Call the host"
params = '{{"type":"object"}}'
script = "call.luau"
{extra}
"#
    )
}

/// It sleeps until its devices say something, and answers them.
const ECHO: &str = r#"
[background]
idle = "30s"

[[hooks]]
on = "port.message"
script = "echo.luau"
function = "heard"
"#;

/// `{op = …}` through `host.call`: `{ok}` or `{error}`.
const CALL: &str = "return host.call(args)";

const ECHO_LUAU: &str = r#"
return {
    heard = function(event)
        local d = event.detail
        local m = d.message
        if d.kind == "midi" then
            -- An octave up, back to the device.
            host.port_send(d.port, { type = "note_on", channel = m.channel, number = m.number + 12, value = m.value })
        else
            host.port_send(d.port, { address = "/echo" .. m.address, args = m.args })
        end
        ext.state{ key = "heard", text = d.kind .. " " .. d.name }
    end,
}
"#;

fn load(host: &Host, dir: &Path, manifest: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    std::fs::write(dir.join("call.luau"), CALL).unwrap();
    std::fs::write(dir.join("echo.luau"), ECHO_LUAU).unwrap();
    let package = Package::read(dir).map_err(|e| e.to_string())?;
    host.load(&package, Origin::Dev)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A host on a fake clock, its hooks run as they start, its MIDI ports
/// virtual and its OSC over UDP.
fn host(clock: &FakeClock, midi: &VirtualPorts) -> Host {
    Host::new(ActionList::new(), SharedSections::new(Registry::new()))
        .with_spawn(|work| {
            let _ = block_on(work);
        })
        .with_clock(clock.clone())
        .with_ports(Ports::new().with_midi(midi.clone()).with_osc(UdpOsc))
}

fn call(host: &Host, id: &str, request: Value) -> Result<Value, HostError> {
    let request: HostRequest = serde_json::from_value(request).unwrap();
    block_on(host.api(id).call(request))
}

/// Through its Luau script's `host.call`, as the extension runs it.
fn script(host: &Host, request: &Value) -> Value {
    let out = block_on(
        host.list()
            .run_json_async("LAB_PORTS_CALL", request.to_string().as_bytes()),
    )
    .unwrap();
    serde_json::from_slice(&out).unwrap()
}

fn state(host: &Host, key: &str) -> Option<String> {
    host.contributions()
        .state(architect_extensions::host::now_ms())
        .into_iter()
        .find(|s| s.source == format!("ext:{ID}") && s.key == key)
        .map(|s| s.text)
}

fn refused(r: Result<Value, HostError>) -> (String, String) {
    match r {
        Err(HostError::Refused { capability, why }) => (capability, why),
        other => panic!("not refused: {other:?}"),
    }
}

fn device_socket() -> UdpSocket {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

fn recv_osc(socket: &UdpSocket) -> (OscMessage, std::net::SocketAddr) {
    let mut buf = [0u8; 2048];
    let (n, from) = socket.recv_from(&mut buf).unwrap();
    (decode_osc(&buf[..n]).unwrap().remove(0), from)
}

// ── Ports ───────────────────────────────────────────────────────────

#[test]
fn a_background_extension_hears_and_answers_a_virtual_midi_port_and_an_osc_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::at(WED_8);
    let midi = VirtualPorts::new(PortKind::Midi);
    let device = midi.add("Lab Surface");
    let _other = midi.add("Other");
    let host = host(&clock, &midi);
    load(
        &host,
        &tmp.path().join("ports"),
        &manifest(r#""midi:Lab*", "osc:127.0.0.1:*""#, ECHO),
    )
    .unwrap();
    assert_eq!(host.background()[0].state, BackgroundState::Stopped);

    // Its script lists the MIDI ports and opens one, and an OSC endpoint.
    let listed = script(&host, &json!({"op": "ports", "kind": "midi"}));
    assert_eq!(
        listed["ok"],
        json!([
            {"kind": "midi", "name": "Lab Surface", "direction": "both"},
            {"kind": "midi", "name": "Other", "direction": "both"}
        ])
    );
    let opened = script(
        &host,
        &json!({"op": "open_port", "kind": "midi", "name": "Lab Surface"}),
    );
    let midi_port = opened["ok"]["port"].as_str().unwrap().to_owned();
    assert_eq!(device.listeners(), 1);
    let osc_device = device_socket();
    let endpoint = osc_device.local_addr().unwrap().to_string();
    let opened = script(
        &host,
        &json!({"op": "open_port", "kind": "osc", "name": endpoint}),
    );
    let osc_port = opened["ok"]["port"].as_str().unwrap().to_owned();

    // It sends to both.
    let sent = script(
        &host,
        &json!({"op": "port_send", "port": midi_port, "message": {"type": "cc", "channel": 2, "number": 7, "value": 100}}),
    );
    assert_eq!(sent, json!({"ok": null}));
    assert_eq!(device.take(), [PortMessage::Midi(vec![0xB1, 7, 100])]);
    script(
        &host,
        &json!({"op": "port_send", "port": osc_port, "message": {"address": "/hello", "args": ["lab", 1]}}),
    );
    let (hello, endpoint_addr) = recv_osc(&osc_device);
    assert_eq!(
        hello,
        OscMessage::new("/hello", vec![OscArg::String("lab".into()), OscArg::Int(1)])
    );

    // Idle past its timeout, it stops; its ports stay open.
    clock.advance(Duration::from_secs(31));
    assert_eq!(host.tick().stopped, [ID]);
    assert_eq!(device.listeners(), 1);

    // A note from the device starts it again, and it answers an octave up.
    assert_eq!(device.send(&PortMessage::Midi(vec![0x90, 60, 99])), 1);
    let bg = host.background()[0].clone();
    assert_eq!((bg.state, bg.starts), (BackgroundState::Running, 2));
    assert_eq!(state(&host, "heard").as_deref(), Some("midi Lab Surface"));
    assert_eq!(device.take(), [PortMessage::Midi(vec![0x90, 72, 99])]);

    // An OSC message to the endpoint: heard, and echoed.
    osc_device
        .send_to(
            &encode_osc(&OscMessage::new("/fader/1", vec![OscArg::Float(0.25)])),
            endpoint_addr,
        )
        .unwrap();
    let (echo, _) = recv_osc(&osc_device);
    assert_eq!(
        echo,
        OscMessage::new("/echo/fader/1", vec![OscArg::Float(0.25)])
    );
    // Its state is set after it answers, on the endpoint's thread.
    let start = std::time::Instant::now();
    while state(&host, "heard") != Some(format!("osc {endpoint}")) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            state(&host, "heard")
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // Closed, the port is gone; unloaded, every one is.
    assert_eq!(
        script(&host, &json!({"op": "close_port", "port": midi_port}))["ok"],
        true
    );
    assert_eq!(device.listeners(), 0);
    assert!(host.unload(ID));
}

#[test]
fn what_no_hook_takes_waits_for_port_receive() {
    let tmp = tempfile::tempdir().unwrap();
    let midi = VirtualPorts::new(PortKind::Midi);
    let device = midi.add("Keys");
    let host = host(&FakeClock::at(WED_8), &midi);
    load(
        &host,
        &tmp.path().join("ports"),
        &manifest(r#""midi:Keys""#, ""),
    )
    .unwrap();
    let port = call(
        &host,
        ID,
        json!({"op": "open_port", "kind": "midi", "name": "Keys"}),
    )
    .unwrap()["port"]
        .as_str()
        .unwrap()
        .to_owned();
    for n in 0..3u8 {
        device.send(&PortMessage::Midi(vec![0x90, 60 + n, 100]));
    }
    let got = call(
        &host,
        ID,
        json!({"op": "port_receive", "port": port, "max": 2}),
    )
    .unwrap();
    assert_eq!(got.as_array().unwrap().len(), 2);
    assert_eq!(got[0]["type"], "note_on");
    assert_eq!(got[0]["number"], 60);
    let rest = call(&host, ID, json!({"op": "port_receive", "port": port})).unwrap();
    assert_eq!(rest[0]["number"], 62);
    assert_eq!(
        call(&host, ID, json!({"op": "port_receive", "port": port})).unwrap(),
        json!([])
    );
    // A message it cannot make is not sent.
    assert!(matches!(
        call(
            &host,
            ID,
            json!({"op": "port_send", "port": port, "message": {"type": "cc", "channel": 0}})
        ),
        Err(HostError::Failed { .. })
    ));
}

#[test]
fn ports_outside_its_capability_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let midi = VirtualPorts::new(PortKind::Midi);
    let _ = midi.add("Lab Surface");
    let _ = midi.add("Other");
    let host = host(&FakeClock::at(WED_8), &midi);
    load(
        &host,
        &tmp.path().join("ports"),
        &manifest(r#""midi:Lab Surface", "osc:127.0.0.1:9000""#, ""),
    )
    .unwrap();
    let open = |kind: &str, name: &str, listen: Option<u16>| {
        call(
            &host,
            ID,
            json!({"op": "open_port", "kind": kind, "name": name, "listen": listen}),
        )
    };
    let (capability, why) = refused(open("midi", "Other", None));
    assert_eq!(capability, "ports.midi:Other");
    assert!(why.contains("capabilities.ports"), "{why}");
    assert_eq!(
        refused(open("osc", "127.0.0.1:9001", None)).0,
        "ports.osc:127.0.0.1:9001"
    );
    assert_eq!(
        refused(open("osc", "10.0.0.1:9000", None)).0,
        "ports.osc:10.0.0.1:9000"
    );
    // Hearing on a port of this computer is a capability of its own.
    assert!(
        refused(open("osc", "127.0.0.1:9000", Some(9100)))
            .1
            .contains("hearing on 9100")
    );
    assert_eq!(refused(open("serial", "x", None)).0, "ports");

    // Its own port, not another extension's.
    let port = open("midi", "Lab Surface", None).unwrap()["port"]
        .as_str()
        .unwrap()
        .to_owned();
    load(
        &host,
        &tmp.path().join("none"),
        &manifest("", "")
            .replace("lab.ports", "lab.none")
            .replace("LAB_PORTS_CALL", "LAB_NONE_CALL"),
    )
    .unwrap();
    assert!(matches!(
        call(
            &host,
            "lab.none",
            json!({"op": "port_send", "port": port, "message": {"bytes": [144, 60, 1]}})
        ),
        Err(HostError::NotFound { .. })
    ));
    // With no ports at all, not even the list.
    assert_eq!(
        refused(call(&host, "lab.none", json!({"op": "ports"}))).0,
        "ports"
    );
    // Through its Luau script too: the refusal, not the port.
    let out = script(
        &host,
        &json!({"op": "open_port", "kind": "midi", "name": "Other"}),
    );
    assert_eq!(out["error"]["kind"], "refused", "{out}");

    // A bad entry does not load.
    let err = load(
        &host,
        &tmp.path().join("bad"),
        &manifest(r#""serial:/dev/tty""#, "")
            .replace("lab.ports", "lab.bad")
            .replace("LAB_PORTS_CALL", "LAB_BAD_CALL"),
    )
    .unwrap_err();
    assert!(err.contains("ports"), "{err}");

    // What a person is asked.
    let caps = architect_extensions::manifest::Capabilities {
        ports: vec![
            "midi:X-Touch*".into(),
            "osc:127.0.0.1:9000".into(),
            "osc:listen:9001".into(),
        ],
        ..Default::default()
    };
    let says: Vec<String> = caps.asks().into_iter().map(|a| a.says).collect();
    assert_eq!(
        says,
        [
            "Use your MIDI ports named X-Touch…",
            "Send and receive OSC at 127.0.0.1:9000",
            "Listen for OSC on port 9001 of this computer"
        ]
    );
    // Granted less, it may open less.
    let granted = architect_extensions::manifest::Capabilities {
        ports: vec!["osc:127.0.0.1:9000".into()],
        ..Default::default()
    };
    let may = caps.within(&granted);
    assert!(may.may_open_port("osc", "127.0.0.1:9000", None));
    assert!(!may.may_open_port("osc", "127.0.0.1:9000", Some(9001)));
    assert!(!may.may_open_port("midi", "X-Touch One", None));
    assert!(caps.may_open_port("midi", "X-Touch One", None));
}

// ── Control surfaces ────────────────────────────────────────────────

/// A kind of lab target: `count` values in a range, changed by the
/// surface or from elsewhere.
#[derive(Clone)]
struct Values {
    kind: TargetKind,
    at: Arc<Mutex<Vec<f64>>>,
    sink: Arc<Mutex<Option<TargetSink>>>,
}

impl Values {
    fn new(id: &str, min: f64, max: f64, count: u32, at: f64) -> Self {
        Self {
            kind: TargetKind {
                id: id.to_owned(),
                title: id.to_owned(),
                min,
                max,
                step: None,
                count,
            },
            at: Arc::new(Mutex::new(vec![at; count as usize])),
            sink: Arc::default(),
        }
    }

    fn get(&self, i: u32) -> f64 {
        self.at.lock().unwrap()[i as usize]
    }

    /// Changed from elsewhere (the app's own controls).
    fn set(&self, i: u32, v: f64) {
        self.at.lock().unwrap()[i as usize] = v;
        let sink = self.sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            sink(i, v);
        }
    }
}

impl Target for Values {
    fn kind(&self) -> TargetKind {
        self.kind.clone()
    }
    fn read(&self, i: u32) -> Option<f64> {
        self.at.lock().unwrap().get(i as usize).copied()
    }
    fn write(&self, i: u32, v: f64) -> Result<(), String> {
        self.at.lock().unwrap()[i as usize] = v;
        Ok(())
    }
    fn observe(&self, changed: TargetSink) {
        *self.sink.lock().unwrap() = Some(changed);
    }
}

/// Eight faders (a pitch bend per channel) on twelve lab levels, a tempo
/// knob, and bank buttons.
fn surface_manifest(port: &str) -> String {
    let mut controls = String::new();
    for s in 0..8 {
        let _ = write!(
            controls,
            r#"{{ id = "fader{n}", kind = "fader", strip = {s}, input = "PB ch{n}", touch = "Note{t} ch1", target = "lab.level" }}, "#,
            n = s + 1,
            t = 104 + s
        );
    }
    controls.push_str(
        r#"{ id = "tempo", kind = "knob", input = "CC20 ch1", target = "lab.metronome.tempo", takeover = "pickup" },
{ id = "bank-", kind = "button", input = "Note46 ch1", target = "bank.prev" },
{ id = "bank+", kind = "button", input = "Note47 ch1", target = "bank.next" }"#,
    );
    manifest(
        r#""midi:Fake 8""#,
        &format!(
            r#"
[[contribute."control.surfaces"]]
id = "fake8"
title = "Fake 8"
port = "{port}"
bank_size = 8
controls = [{controls}]
"#
        ),
    )
}

fn faders(out: &[PortMessage]) -> Vec<(u8, u16)> {
    out.iter()
        .filter_map(|m| match m {
            PortMessage::Midi(b) => midi_event(b),
            PortMessage::Osc(_) => None,
        })
        .filter_map(|e| match e.message {
            MidiMessage::PitchBend { value } => Some((e.channel, value)),
            _ => None,
        })
        .collect()
}

fn pb(channel: u8, value: u16) -> PortMessage {
    PortMessage::Midi(vec![
        0xE0 | (channel - 1),
        (value & 0x7F) as u8,
        (value >> 7) as u8,
    ])
}

#[test]
fn an_extension_s_eight_fader_surface_moves_lab_targets_with_feedback_and_banks() {
    let tmp = tempfile::tempdir().unwrap();
    let midi = VirtualPorts::new(PortKind::Midi);
    let device = midi.add("Fake 8");
    let levels = Values::new("lab.level", 0.0, 1.0, 12, 0.0);
    let tempo = Values::new("lab.metronome.tempo", 40.0, 240.0, 1, 120.0);
    let targets = Targets::new();
    targets.register(levels.clone());
    targets.register(tempo.clone());
    let host = host(&FakeClock::at(WED_8), &midi).with_targets(targets);
    load(
        &host,
        &tmp.path().join("ports"),
        &surface_manifest("midi:Fake 8"),
    )
    .unwrap();

    let status = host.surfaces();
    assert_eq!(status.len(), 1);
    assert_eq!(
        (
            status[0].id.as_str(),
            status[0].from.as_str(),
            status[0].banks,
            status[0].error.clone()
        ),
        ("fake8", ID, 2, None)
    );
    // Started: every motor fader sent to its level.
    assert_eq!(
        faders(&device.take()),
        (1..=8).map(|c| (c, 0)).collect::<Vec<_>>()
    );

    // Fader 2 up: level 2.
    device.send(&pb(2, 16383));
    assert_eq!(levels.get(1), 1.0);
    // A level changed in the app: its motor fader follows.
    levels.set(0, 0.5);
    assert_eq!(faders(&device.take()), [(1, 8192)]);

    // Bank up: faders 1–4 are levels 9–12, 5–8 none.
    device.send(&PortMessage::Midi(vec![0x90, 47, 127]));
    assert_eq!(host.surfaces()[0].bank, 1);
    let shown = faders(&device.take());
    assert_eq!(shown.len(), 8);
    assert_eq!(shown[0], (1, 0));
    device.send(&pb(1, 16383));
    assert_eq!(levels.get(8), 1.0);
    assert_eq!(levels.get(0), 0.5, "bank 0's left alone");
    device.send(&pb(6, 16383));
    assert!(
        (0..12).filter(|i| levels.get(*i) == 1.0).count() == 2,
        "fader 6 has no level on bank 1"
    );
    // Its extension switches back, through the host service.
    assert_eq!(
        call(
            &host,
            ID,
            json!({"op": "surface_bank", "surface": "fake8", "bank": 0})
        )
        .unwrap(),
        json!(0)
    );
    assert!(faders(&device.take()).contains(&(2, 16383)));
    assert_eq!(
        call(&host, ID, json!({"op": "surfaces"})).unwrap()[0]["bank"],
        0
    );
    let kinds = call(&host, ID, json!({"op": "targets"})).unwrap();
    assert_eq!(kinds.as_array().unwrap().len(), 2);
    assert_eq!(kinds[0]["id"], "lab.level");
    assert_eq!(kinds[0]["values"][1], 1.0);

    // The tempo knob picks up: far from 120, nothing; past it, it follows.
    device.send(&PortMessage::Midi(vec![0xB0, 20, 0]));
    assert_eq!(tempo.get(0), 120.0);
    device.send(&PortMessage::Midi(vec![0xB0, 20, 127]));
    assert_eq!(tempo.get(0), 240.0);

    // Unloaded, its surface goes and its port closes.
    assert!(host.unload(ID));
    assert!(host.surfaces().is_empty());
    assert_eq!(device.listeners(), 0);
}

#[test]
fn a_surface_on_a_port_it_may_not_open_does_not_start() {
    let tmp = tempfile::tempdir().unwrap();
    let midi = VirtualPorts::new(PortKind::Midi);
    let device = midi.add("Elsewhere");
    let targets = Targets::new();
    targets.register(Values::new("lab.level", 0.0, 1.0, 8, 0.0));
    let host = host(&FakeClock::at(WED_8), &midi).with_targets(targets);
    load(
        &host,
        &tmp.path().join("ports"),
        &surface_manifest("midi:Elsewhere"),
    )
    .unwrap();
    let status = host.surfaces();
    assert!(
        status[0]
            .error
            .as_deref()
            .unwrap()
            .contains("capabilities.ports"),
        "{status:?}"
    );
    assert_eq!(device.listeners(), 0);
}
