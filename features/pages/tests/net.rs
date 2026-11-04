//! A page's network (#310): a TSX page and a Luau page, each its
//! extension's, fetch from a host in `capabilities.hosts` through `net`
//! (the host service's `fetch`, as the extension), and are refused
//! another host and a redirect to one. Against a local server, no real
//! network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections, gate};
use architect_extensions::install::Installs;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::ViewEvent;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};
use architect_settings::Registry;

#[path = "../../extensions/tests/support/server.rs"]
mod server;

const ID: &str = "lab.page-net";

const MANIFEST: &str = r#"
id = "lab.page-net"
name = "Page network"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
hosts = ["127.0.0.1"]
"#;

const TSX: &str = r#"import { h, useState, useEffect, net } from "architect";

export default function Net() {
  const [out, setOut] = useState<string>("");
  useEffect(() => {
    net.fetch("BASE/hello").then((r) =>
      net.fetch("OTHER/hello").then(
        () => setOut(r.body + "|reached"),
        (e) => net.fetch("BASE/redirect-out").then(
          () => setOut(r.body + "|" + String(e.message) + "|followed"),
          (e2) => setOut(r.body + "|" + String(e.message) + "|" + String(e2.message)))));
  }, []);
  return <span id="out">{out}</span>;
}
"#;

const LUAU: &str = r#"
local r = net.fetch("BASE/hello")
local _, other = pcall(net.fetch, "OTHER/hello")
local _, redirect = pcall(net.fetch, "BASE/redirect-out")
render(function()
  return span():attr("id", "out"):child(r.body .. "|" .. tostring(other) .. "|" .. tostring(redirect))
end)
"#;

fn extensions() -> (Extensions, Pages, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(tmp.path().join("installs")));
    // The page's calls to its extension's host (`EXT_PAGE_CALL`).
    ext.install_contribution_actions().unwrap();
    let dir = tmp.path().join("dev/page-net");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.toml"), MANIFEST).unwrap();
    let _ = ext.dev(&dir);
    assert!(ext.host().get(ID).is_some(), "{:?}", ext.problems());
    let pages = Pages::new(AppHost::new(list)).with_js().with_luau();
    (ext, pages, tmp)
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

fn check(out: &str) {
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "hello from the server", "{out}");
    assert!(parts[1].contains("localhost") && parts[1].contains("hosts"), "{out}");
    assert!(parts[2].contains("redirected") && parts[2].contains("localhost"), "{out}");
}

fn page(text: &str, port: u16) -> String {
    text.replace("BASE", &format!("http://127.0.0.1:{port}"))
        .replace("OTHER", &format!("http://localhost:{port}"))
}

#[test]
fn a_tsx_page_fetches_from_its_host_and_is_refused_another() {
    let server = server::serve();
    let (ext, pages, _tmp) = extensions();
    check(&out_of(&ext, &pages, "net.tsx", &page(TSX, server.port)));
}

#[test]
fn a_luau_page_fetches_from_its_host_and_is_refused_another() {
    let server = server::serve();
    let (ext, pages, _tmp) = extensions();
    check(&out_of(&ext, &pages, "net.luau", &page(LUAU, server.port)));
}
