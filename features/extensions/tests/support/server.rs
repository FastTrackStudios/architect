//! A local HTTP server for the network's tests (#310): no real network.
//! Shared by `tests/net.rs`, `tests/wasi.rs` and `features/pages/tests/net.rs`
//! (each `#[path]`s it in).

#![allow(dead_code, clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the server heard: each request's path and headers.
#[derive(Clone, Default)]
pub struct Heard(pub Arc<Mutex<Vec<(String, String, BTreeMap<String, String>)>>>);

impl Heard {
    /// The requests to `path`: the host they named, their headers.
    pub fn to(&self, path: &str) -> Vec<(String, BTreeMap<String, String>)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _)| p == path)
            .map(|(_, h, hs)| (h.clone(), hs.clone()))
            .collect()
    }
}

pub struct Server {
    pub port: u16,
    pub heard: Heard,
}

pub const STREAMED_LINES: usize = 2000;

/// The client id and secret the mock token exchange takes.
pub const EXCHANGE_ID: &str = "client-123";
pub const EXCHANGE_SECRET: &str = "s3cret-only-the-node-has";

pub fn streamed() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for i in 0..STREAMED_LINES {
        let _ = writeln!(out, "line {i} — ✓");
    }
    out
}

/// A server on 127.0.0.1: `/hello`, `/redirect-in` (to `/hello`),
/// `/redirect-out` (to `localhost`'s `/hello`), `/big` (2 MiB), `/slow`
/// (3 s), `/stream` (many lines, in parts).
pub fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let heard = Heard::default();
    let keep = heard.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let heard = keep.clone();
            std::thread::spawn(move || answer(stream, port, &heard));
        }
    });
    Server { port, heard }
}

fn answer(stream: TcpStream, port: u16, heard: &Heard) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let length = headers
        .get("content-length")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let host = headers.get("host").cloned().unwrap_or_default();
    heard
        .0
        .lock()
        .unwrap()
        .push((path.clone(), host, headers.clone()));
    let mut out = stream;
    let send = |out: &mut TcpStream, status: &str, extra: &str, body: &[u8]| {
        let top = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        );
        let _ = out.write_all(top.as_bytes());
        let _ = out.write_all(body);
    };
    match path.as_str() {
        "/hello" => send(
            &mut out,
            "200 OK",
            "Content-Type: text/plain\r\n",
            b"hello from the server",
        ),
        "/redirect-in" => send(&mut out, "302 Found", "Location: /hello\r\n", b""),
        // A mock of Plaid's token exchange (`tests/node.rs`): it answers
        // only the client id and secret [`EXCHANGE_ID`] and
        // [`EXCHANGE_SECRET`], and a body naming a public token.
        "/item/public_token/exchange" => {
            let keys = headers.get("plaid-client-id").map(String::as_str) == Some(EXCHANGE_ID)
                && headers.get("plaid-secret").map(String::as_str) == Some(EXCHANGE_SECRET);
            let named = String::from_utf8_lossy(&body).contains("\"public_token\":\"public-");
            if keys && named {
                send(
                    &mut out,
                    "200 OK",
                    "Content-Type: application/json\r\n",
                    br#"{"access_token":"access-sandbox-kept","item_id":"item-42","request_id":"r1"}"#,
                );
            } else {
                send(
                    &mut out,
                    "400 Bad Request",
                    "Content-Type: application/json\r\n",
                    br#"{"error_code":"INVALID_API_KEYS"}"#,
                );
            }
        }
        "/redirect-out" => send(
            &mut out,
            "302 Found",
            &format!("Location: http://localhost:{port}/hello\r\n"),
            b"",
        ),
        "/big" => send(&mut out, "200 OK", "", &vec![b'x'; 2 << 20]),
        "/slow" => {
            std::thread::sleep(Duration::from_secs(3));
            send(&mut out, "200 OK", "", b"late");
        }
        "/stream" => {
            let body = streamed();
            let top = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = out.write_all(top.as_bytes());
            for part in body.as_bytes().chunks(997) {
                let _ = out.write_all(part);
                let _ = out.flush();
            }
        }
        _ => send(&mut out, "404 Not Found", "", b"no"),
    }
}
