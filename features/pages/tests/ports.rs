//! A page's MIDI ports (#311): a TSX page and a Luau page, each its
//! extension's, open a virtual MIDI port its `capabilities.ports` names
//! and send through it (the host service's `open_port` and `port_send`,
//! as the extension), and are refused another port.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation
)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_input::{PortKind, PortMessage, Ports, VirtualDevice, VirtualPorts};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;

const ID: &str = "lab.page-ports";

const MANIFEST: &str = r#"
id = "lab.page-ports"
name = "Page ports"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
ports = ["midi:Keys"]
"#;

const TSX: &str = r#"import { h, useState, useEffect, ports } from "architect";

export default function Ports() {
  const [out, setOut] = useState<string>("");
  useEffect(() => {
    ports.open("midi", "Keys").then((p) =>
      ports.send(p.port, { type: "cc", channel: 1, number: 7, value: 3 }).then(() =>
        ports.open("midi", "Other").then(
          () => setOut("sent|opened"),
          (e) => setOut("sent|" + String(e.message)))));
  }, []);
  return <span id="out">{out}</span>;
}
"#;

const LUAU: &str = r#"
local p = ports.open("midi", "Keys")
ports.send(p.port, { type = "cc", channel = 1, number = 7, value = 3 })
local _, other = pcall(ports.open, "midi", "Other")
render(function()
  return span():attr("id", "out"):child("sent|" .. tostring(other))
end)
"#;

fn extensions() -> (Extensions, Pages, VirtualDevice, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let midi = VirtualPorts::new(PortKind::Midi);
    let keys = midi.add("Keys");
    let _ = midi.add("Other");
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
        .with_ports(Ports::new().with_midi(midi));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    // The page's calls to its extension's host (`EXT_PAGE_CALL`).
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-ports");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), MANIFEST).unwrap();
    let _ = ext.dev(&dir);
    assert!(ext.host().get(ID).is_some(), "{:?}", ext.problems());
    let pages = Pages::new(AppHost::new(list)).with_js().with_luau();
    (ext, pages, keys, tmp)
}

fn out_of(ext: &Extensions, pages: &Pages, name: &str, text: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, text).unwrap();
    let manifest = ext.host().get(ID).unwrap().manifest;
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let mut session = pages
        .open(&page, ID, "page", Some(gate(&manifest)))
        .unwrap();
    let (to, _from) = futures::channel::mpsc::unbounded::<ViewEvent>();
    let mut view = RemoteDocument::new(to);
    let started = Instant::now();
    loop {
        if let Some(out) = text_of(&view, "#out").filter(|t| !t.is_empty()) {
            return out;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no answer; log: {:?}",
            session.status().log.lines()
        );
        apply(&mut session, &mut view);
    }
}

fn text_of(view: &RemoteDocument, selector: &str) -> Option<String> {
    let doc = view.document();
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

fn apply(session: &mut PageSession, view: &mut RemoteDocument) {
    match session.frames.try_recv() {
        Ok(frame) => view.apply(frame).unwrap(),
        Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
        Err(_) => std::thread::sleep(Duration::from_millis(5)),
    }
}

fn check(out: &str, keys: &VirtualDevice) {
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "sent", "{out}");
    assert!(parts[1].contains("capabilities.ports"), "{out}");
    assert_eq!(keys.take(), [PortMessage::Midi(vec![0xB0, 7, 3])]);
}

#[test]
fn a_tsx_page_opens_its_port_and_is_refused_another() {
    let (ext, pages, keys, _tmp) = extensions();
    check(&out_of(&ext, &pages, "ports.tsx", TSX), &keys);
}

#[test]
fn a_luau_page_opens_its_port_and_is_refused_another() {
    let (ext, pages, keys, _tmp) = extensions();
    check(&out_of(&ext, &pages, "ports.luau", LUAU), &keys);
}
