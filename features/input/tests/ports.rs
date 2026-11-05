//! OSC over localhost UDP (#311): an endpoint sends to a socket, and hears
//! what is sent to the port it listens on.

#![allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

use std::net::UdpSocket;
use std::sync::mpsc;
use std::time::Duration;

use architect_input::ports::{decode_osc, encode_osc};
use architect_input::{
    OpenRequest, OscArg, OscMessage, PortBackend, PortKind, PortMessage, UdpOsc,
};

#[test]
fn an_osc_endpoint_sends_and_hears_over_udp() {
    let device = UdpSocket::bind("127.0.0.1:0").unwrap();
    device
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let port = UdpOsc
        .open(
            &OpenRequest {
                kind: PortKind::Osc,
                name: device.local_addr().unwrap().to_string(),
                listen: None,
            },
            std::sync::Arc::new(move |m| {
                let _ = tx.send(m);
            }),
        )
        .unwrap();
    let hello = OscMessage::new("/hello", vec![OscArg::Int(1), OscArg::String("lab".into())]);
    port.send(&PortMessage::Osc(hello.clone())).unwrap();
    let mut buf = [0u8; 1024];
    let (n, from) = device.recv_from(&mut buf).unwrap();
    assert_eq!(decode_osc(&buf[..n]).unwrap(), vec![hello]);
    // The device answers its sender: the endpoint hears it.
    let reply = OscMessage::new("/fader/1", vec![OscArg::Float(0.25)]);
    device.send_to(&encode_osc(&reply), from).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        PortMessage::Osc(reply)
    );
    assert!(port.send(&PortMessage::Midi(vec![0x90, 1, 1])).is_err());
}
