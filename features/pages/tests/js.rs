//! TSX pages on `QuickJS`, end to end: a page file opened through
//! `Pages`, its frames applied to the `RemoteDocument` a process view
//! uses, its events sent back as the pane sends them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageLimits, PageSession, Pages};
use architect_scripting::theme::ThemeCell;
use architect_theme::{Resolved, Scheme};

/// A pane: the page's session and the document its frames fill.
struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn open(pages: &Pages, path: &Path) -> Self {
        let page = PageFile::new(path, Origin::Override).unwrap();
        let session = pages.open(&page, "lab.test", "page", None).unwrap();
        let (to, from) = futures::channel::mpsc::unbounded();
        let mut opened = Self {
            session,
            view: RemoteDocument::new(to),
            _events: from,
        };
        opened.frame(Duration::from_secs(5));
        opened
    }

    /// Applies the next frame; false when none came in `within`.
    fn frame(&mut self, within: Duration) -> bool {
        let until = Instant::now().checked_add(within).unwrap();
        while Instant::now() < until {
            match self.session.frames.try_recv() {
                Ok(frame) => {
                    self.view.apply(frame).unwrap();
                    return true;
                }
                Err(futures::channel::mpsc::TryRecvError::Closed) => panic!("the page stopped"),
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        false
    }

    /// Applies frames until `selector`'s text is `want`.
    fn wait_for(&mut self, selector: &str, want: &str, within: Duration) -> Duration {
        let started = Instant::now();
        loop {
            if self.text(selector).as_deref() == Some(want) {
                return started.elapsed();
            }
            assert!(
                started.elapsed() < within,
                "{selector} is {:?}, not {want:?}; log: {:?}",
                self.text(selector),
                self.session.status().log.lines()
            );
            self.frame(Duration::from_millis(50));
        }
    }

    /// Applies frames until `done`.
    fn wait_until(&mut self, within: Duration, done: impl Fn(&Self) -> bool) {
        let started = Instant::now();
        while !done(self) {
            assert!(
                started.elapsed() < within,
                "log: {:?}",
                self.session.status().log.lines()
            );
            self.frame(Duration::from_millis(50));
        }
    }

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    /// Clicks `selector` as the pane would.
    fn click(&mut self, selector: &str) {
        let doc = self.view.document();
        let node = doc.query_selector(selector).unwrap().unwrap();
        let id: u64 = doc
            .get_node(node)
            .unwrap()
            .attr(blitz_dom::LocalName::from("data-dioxus-id"))
            .unwrap()
            .parse()
            .unwrap();
        self.session.event(ViewEvent {
            element: id,
            name: "click".to_owned(),
            bubbles: true,
            data: EventData::Mouse(Pointer::default()),
        });
        assert!(
            self.frame(Duration::from_secs(2)),
            "a click is answered with a frame"
        );
    }
}

fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// A save the page's watcher sees: a later modification time even on a
/// coarse clock.
fn save(path: &Path, text: &str) {
    std::thread::sleep(Duration::from_millis(15));
    std::fs::write(path, text).unwrap();
}

fn pages(list: ActionList) -> Pages {
    Pages::new(AppHost::new(list)).with_js()
}

const COUNTER: &str = r#"import { h, useState } from "architect";

type Props = {};
export default function Counter(_: Props) {
  const [n, setN] = useState<number>(0);
  return (
    <div class="counter" style={{ padding: 8, color: "$foreground" }}>
      <span id="n">{n}</span>
      <button id="inc" onClick={() => setN(n + 1)}>+1</button>
    </div>
  );
}
"#;

#[test]
fn a_tsx_page_renders_and_its_handlers_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "counter.tsx", COUNTER);
    let mut pane = Pane::open(&pages(ActionList::new()), &path);
    assert_eq!(pane.text("#n").as_deref(), Some("0"));
    pane.click("#inc");
    pane.click("#inc");
    pane.wait_for("#n", "2", Duration::from_secs(2));
    // A style through the theme: a token is a CSS variable.
    let doc = pane.view.document();
    let div = doc.query_selector(".counter").unwrap().unwrap();
    let style = doc
        .get_node(div)
        .unwrap()
        .attr(blitz_dom::LocalName::from("style"))
        .unwrap()
        .to_owned();
    assert_eq!(style, "padding: 8px; color: var(--foreground)");
}

#[test]
fn a_save_renders_the_new_page_and_a_broken_one_keeps_the_last_good() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "page.tsx", &COUNTER.replace("+1", "add"));
    let mut pane = Pane::open(&pages(ActionList::new()), &path);
    pane.click("#inc");
    pane.wait_for("#inc", "add", Duration::from_secs(1));
    // Edited live: the pane follows within a second.
    save(&path, &COUNTER.replace("+1", "plus one"));
    let took = pane.wait_for("#inc", "plus one", Duration::from_secs(2));
    assert!(took < Duration::from_secs(1), "the reload took {took:?}");
    let reload = pane.session.status().last_reload();
    eprintln!("reload: {took:?} from the save, {reload:?} from noticing it to its frame");
    // Broken: the last good page stays, a banner says why, `.error` too.
    save(&path, "export default function () {\n  return <div>;\n}\n");
    pane.wait_until(Duration::from_secs(2), |p| p.text(".page-error").is_some());
    let banner = pane.text(".page-error").unwrap();
    assert!(
        banner.starts_with("⚠ page.tsx: page.tsx:3:1: Unexpected token"),
        "{banner}"
    );
    assert!(
        banner.ends_with("the last good page is showing; why is in page.tsx.error"),
        "{banner}"
    );
    assert_eq!(pane.text("#inc").as_deref(), Some("plus one"));
    let error = std::fs::read_to_string(dir.path().join("page.tsx.error")).unwrap();
    assert!(
        error.starts_with("page.tsx:3:1: Unexpected token"),
        "{error}"
    );
    assert!(pane.session.status().error().is_some());
    // Fixed: both go.
    save(&path, COUNTER);
    pane.wait_for("#inc", "+1", Duration::from_secs(2));
    assert!(pane.text(".page-error").is_none());
    assert!(!dir.path().join("page.tsx.error").exists());
}

#[test]
fn a_runaway_render_is_stopped_and_says_to_use_a_worker() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "spin.tsx",
        r#"import { h, useState } from "architect";
export default function Spin() {
  const [spin, setSpin] = useState(false);
  if (spin) { while (true) {} }
  return <button id="go" onClick={() => setSpin(true)}>go</button>;
}
"#,
    );
    let limits = PageLimits {
        frame: Duration::from_millis(100),
        ..PageLimits::default()
    };
    let pages = pages(ActionList::new()).with_limits(limits);
    let mut pane = Pane::open(&pages, &path);
    let started = Instant::now();
    pane.click("#go");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let why = pane.session.status().error().unwrap_or_default();
    assert!(why.contains("ran past its time of 100 ms"), "{why}");
    assert!(why.contains("worker.spawn"), "{why}");
    // The last good page still shows.
    assert_eq!(pane.text("#go").as_deref(), Some("go"));
}

static SLOW: ActionMeta = ActionMeta {
    id: "LAB_SLOW",
    trait_name: "Lab",
    method_name: "slow",
    display_name: "Slow",
    description: "answers after a Tokio timer",
    category: "Lab",
    group: "Lab",
    shortcut: "",
    toggleable: false,
    undo: false,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pages_actions_run_on_the_apps_runtime_and_feed_live_data() {
    let list = ActionList::new();
    list.register_async(&SLOW, Effect::ReadOnly, |(): ()| async {
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok::<_, String>(vec!["a".to_owned(), "b".to_owned()])
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "data.tsx",
        r#"import { h, useData, notify } from "architect";
export default function Data() {
  const pins = useData<string[]>("LAB_SLOW");
  if (pins.value) notify(`${pins.value.length} pins`);
  return <ul id="pins">{(pins.value ?? []).map((p) => <li key={p}>{p}</li>)}</ul>;
}
"#,
    );
    let pages = pages(list);
    let mut pane = tokio::task::block_in_place(|| Pane::open(&pages, &path));
    tokio::task::block_in_place(|| pane.wait_for("#pins", "ab", Duration::from_secs(3)));
    assert!(pages.host().notices().iter().any(|n| n.text == "2 pins"));
}

#[test]
fn a_worker_does_the_heavy_work_and_messages_come_back() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "primes.js",
        r#"import { post } from "architect/worker";
export default function (args) {
  let found = 0;
  for (let n = 2; n < args.below; n++) {
    let prime = true;
    for (let d = 2; d * d <= n; d++) if (n % d === 0) { prime = false; break; }
    if (prime) found++;
    if (n % 10000 === 0) post({ at: n });
  }
  return found;
}
"#,
    );
    let path = write(
        dir.path(),
        "primes.tsx",
        r#"import { h, useState, worker } from "architect";
export default function Primes() {
  const [result, setResult] = useState("none yet");
  const [seen, setSeen] = useState(0);
  const go = () => {
    const w = worker.spawn("primes.js", { below: 50000 });
    w.onMessage((m: any) => setSeen((s) => s + 1));
    w.onDone((found, error) => setResult(error ? `failed: ${error}` : `${found} primes`));
  };
  return <div><button id="go" onClick={go}>count</button><span id="out">{result}</span><span id="seen">{seen}</span></div>;
}
"#,
    );
    let mut pane = Pane::open(&pages(ActionList::new()), &path);
    pane.click("#go");
    pane.wait_for("#out", "5133 primes", Duration::from_secs(10));
    assert_eq!(pane.text("#seen").as_deref(), Some("4"));
}

#[test]
fn imports_stay_in_the_pages_folder_and_there_is_no_node() {
    let dir = tempfile::tempdir().unwrap();
    let inner = dir.path().join("page");
    std::fs::create_dir(&inner).unwrap();
    write(dir.path(), "outside.js", "export const secret = 1;\n");
    write(
        &inner,
        "util.ts",
        "export const shout = (s: string): string => s.toUpperCase();\n",
    );
    let path = write(
        &inner,
        "main.tsx",
        r#"import { h } from "architect";
import { shout } from "./util";
export default function Main() {
  const missing = [typeof require, typeof process, typeof fetch, typeof Deno].join(",");
  console.log("drawn", 1);
  return <div><span id="shout">{shout("hi")}</span><span id="missing">{missing}</span></div>;
}
"#,
    );
    let pages = pages(ActionList::new());
    let pane = Pane::open(&pages, &path);
    assert_eq!(pane.text("#shout").as_deref(), Some("HI"));
    assert_eq!(
        pane.text("#missing").as_deref(),
        Some("undefined,undefined,undefined,undefined")
    );
    assert!(
        pane.session
            .status()
            .log
            .lines()
            .iter()
            .any(|l| l == "[info] drawn 1")
    );
    // Out of its folder, or a package: refused, with why.
    for import in ["../outside.js", "fs", "react"] {
        let bad = write(
            &inner,
            "bad.tsx",
            &format!(
                "import {{ h }} from \"architect\";\nimport \"{import}\";\nexport default () => <div/>;\n"
            ),
        );
        let refused = PageFile::new(&bad, Origin::Override).unwrap();
        let why = pages.check(&refused).unwrap_err();
        assert!(
            why.contains("its own folder") || why.contains("no such file in the page's folder"),
            "{import}: {why}"
        );
    }
}

#[test]
fn a_page_follows_the_theme() {
    let theme = |scheme, bg: &str| {
        Resolved::new(
            "test",
            scheme,
            std::iter::once(("background".to_owned(), bg.to_owned())).collect(),
        )
    };
    let cell = ThemeCell::new(theme(Scheme::Dark, "#000000"));
    let pages =
        Pages::new(AppHost::new(ActionList::new()).with_theme(Arc::new(cell.clone()))).with_js();
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "theme.tsx",
        r#"import { h, useTheme } from "architect";
export default () => { const t = useTheme(); return <span id="t">{t.scheme} {t.value("background")}</span>; };
"#,
    );
    let mut pane = Pane::open(&pages, &path);
    assert_eq!(pane.text("#t").as_deref(), Some("dark #000000"));
    cell.set(&theme(Scheme::Light, "#ffffff"));
    pane.wait_for("#t", "light #ffffff", Duration::from_secs(2));
}

#[test]
fn a_check_renders_once_and_counts_what_it_drew() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "c.tsx", COUNTER);
    let pages = pages(ActionList::new());
    let checked = pages
        .check(&PageFile::new(&path, Origin::Override).unwrap())
        .unwrap();
    // div, span, "0", button, "+1".
    assert_eq!(checked.nodes, 5);
    let broken = write(dir.path(), "b.tsx", "export default () => <div>;");
    let why = pages
        .check(&PageFile::new(&broken, Origin::Override).unwrap())
        .unwrap_err();
    assert!(why.starts_with("b.tsx:1:"), "{why}");
}

/// What a TSX page throws as it runs says the `.tsx` file's own line and
/// column (its source map), in `<file>.error`, `APP_STATUS`'s problems
/// and `EXT_CHECK`'s check: not the transpiled module's.
#[test]
fn a_tsx_pages_runtime_error_says_its_own_line_and_column() {
    // Types and JSX above the throw move the printed module's lines; the
    // throw is on line 13 of the file (QuickJS places it at its `Error`
    // call, column 29).
    const THROWS: &str = r#"import { h, useState } from "architect";

type Pin = { title: string };
interface Props {
  pins?: Pin[];
}

function Row({ title }: Pin) {
  return <li class="row">{title}</li>;
}

function boom(n: number): never {
  throw new Error(`no pin ${n}`);
}

export default function Board({ pins = [] }: Props) {
  const [bad, setBad] = useState<boolean>(false);
  if (bad) boom(7);
  return (
    <ul>
      {pins.map((p) => <Row key={p.title} title={p.title} />)}
      <button id="go" onClick={() => setBad(true)}>go</button>
    </ul>
  );
}
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "board.tsx", THROWS);
    let pages = pages(ActionList::new());
    let mut pane = Pane::open(&pages, &path);
    pane.click("#go");
    pane.wait_until(Duration::from_secs(2), |p| p.text(".page-error").is_some());
    let error = std::fs::read_to_string(dir.path().join("board.tsx.error")).unwrap();
    let first = error.lines().next().unwrap_or_default();
    assert!(
        first.contains("board.tsx:13:29") && first.contains("no pin 7"),
        "{error}"
    );
    // Its stack is the file's too: `boom` threw at 13, `Board` called it
    // at 18.
    assert!(error.contains("board.tsx:18:"), "{error}");
    assert!(!error.contains(&dir.path().display().to_string()), "{error}");
    let problems = pages.status_problems();
    assert!(
        problems.iter().any(|p| p.message.contains("board.tsx:13:29")),
        "{problems:?}"
    );
    let banner = pane.text(".page-error").unwrap();
    assert!(banner.contains("board.tsx:13:29"), "{banner}");
    // EXT_CHECK: a page that throws as it first renders, at its place.
    let thrown_at_once = THROWS.replace("useState<boolean>(false)", "useState<boolean>(true)");
    let checked = write(dir.path(), "check.tsx", &thrown_at_once);
    let why = pages.check_file(&checked).unwrap_err();
    assert!(why.starts_with("check.tsx:13:29: "), "{why}");
}
