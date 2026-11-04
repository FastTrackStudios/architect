//! Luau element pages: `div():child(...):on_click(...)`, drawn as the
//! same edits a TSX page's tree is, following the theme; their workers
//! (`worker.spawn`) and timers (`set_timeout`, `set_interval`), as a TSX
//! page's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
use architect_scripting::theme::ThemeCell;
use architect_theme::{Resolved, Scheme};

fn text(view: &RemoteDocument, selector: &str) -> Option<String> {
    let doc = view.document();
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

#[test]
fn a_luau_page_draws_with_elements_and_its_handlers_run() {
    let theme = |s, bg: &str| {
        Resolved::new(
            "t",
            s,
            std::iter::once(("background".to_owned(), bg.to_owned())).collect(),
        )
    };
    let cell = ThemeCell::new(theme(Scheme::Dark, "#000"));
    let pages =
        Pages::new(AppHost::new(ActionList::new()).with_theme(Arc::new(cell.clone()))).with_luau();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("count.luau");
    std::fs::write(
        &path,
        r#"local n = 0
render(function()
  return div():style({ padding = 12, color = "$foreground" })
    :child(span():attr("id", "n"):child("clicked " .. n))
    :child(span():attr("id", "t"):child(theme.scheme .. " " .. tostring(theme.value("background"))))
    :child(button():attr("id", "inc"):child("+1"):on_click(function() n += 1 end))
end)
"#,
    )
    .unwrap();
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let mut session = pages.open(&page, "lab.test", "count", None).unwrap();
    let (to, _from) = futures::channel::mpsc::unbounded();
    let mut view = RemoteDocument::new(to);
    let until = |session: &mut architect_pages::runner::PageSession,
                 view: &mut RemoteDocument,
                 sel: &str,
                 want: &str| {
        let start = Instant::now();
        while text(view, sel).as_deref() != Some(want) {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{sel}: {:?} {:?}",
                text(view, sel),
                session.status().log.lines()
            );
            if let Ok(f) = session.frames.try_recv() {
                view.apply(f).unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    until(&mut session, &mut view, "#n", "clicked 0");
    until(&mut session, &mut view, "#t", "dark #000");
    let doc = view.document();
    let button = doc.query_selector("#inc").unwrap().unwrap();
    let element = doc
        .get_node(button)
        .unwrap()
        .attr(blitz_dom::LocalName::from("data-dioxus-id"))
        .unwrap()
        .parse()
        .unwrap();
    session.event(ViewEvent {
        element,
        name: "click".to_owned(),
        bubbles: true,
        data: EventData::Mouse(Pointer::default()),
    });
    until(&mut session, &mut view, "#n", "clicked 1");
    cell.set(&theme(Scheme::Light, "#fff"));
    until(&mut session, &mut view, "#t", "light #fff");
}

/// A page open in a document, applying its frames.
struct Pane {
    session: architect_pages::runner::PageSession,
    view: RemoteDocument,
}

impl Pane {
    fn open(pages: &Pages, path: &std::path::Path) -> Self {
        let page = PageFile::new(path, Origin::Override).unwrap();
        let session = pages.open(&page, "lab.test", "page", None).unwrap();
        let (to, _from) = futures::channel::mpsc::unbounded();
        Self {
            session,
            view: RemoteDocument::new(to),
        }
    }

    /// Applies frames until `selector`'s text is `want`.
    fn wait_for(&mut self, selector: &str, want: &str, within: Duration) {
        let start = Instant::now();
        while text(&self.view, selector).as_deref() != Some(want) {
            assert!(
                start.elapsed() < within,
                "{selector}: {:?}, not {want:?}; log: {:?}",
                text(&self.view, selector),
                self.session.status().log.lines()
            );
            if let Ok(f) = self.session.frames.try_recv() {
                self.view.apply(f).unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn click(&mut self, selector: &str) {
        let doc = self.view.document();
        let node = doc.query_selector(selector).unwrap().unwrap();
        let element = doc
            .get_node(node)
            .unwrap()
            .attr(blitz_dom::LocalName::from("data-dioxus-id"))
            .unwrap()
            .parse()
            .unwrap();
        self.session.event(ViewEvent {
            element,
            name: "click".to_owned(),
            bubbles: true,
            data: EventData::Mouse(Pointer::default()),
        });
    }
}

#[test]
fn a_luau_pages_worker_reports_back_and_answers_its_messages() {
    let dir = tempfile::tempdir().unwrap();
    // The worker: counts with progress, then serves the page's messages.
    std::fs::write(
        dir.path().join("job.luau"),
        r#"local found = 0
for n = 1, args.upto do
  found += 1
  if n % 1000 == 0 then
    worker.progress(n, args.upto)
  end
end
worker.post({ hello = "from " .. worker.name })
worker.on_message(function(m) worker.post({ echo = m.n * 2 }) end)
return { found = found }
"#,
    )
    .unwrap();
    let path = dir.path().join("page.luau");
    std::fs::write(
        &path,
        r#"local said, progress, echo, result = "-", 0, "-", "-"
local job = nil
render(function()
  return div()
    :child(span():attr("id", "said"):child(said))
    :child(span():attr("id", "progress"):child(tostring(progress)))
    :child(span():attr("id", "echo"):child(echo))
    :child(span():attr("id", "status"):child(if job then job:status() else "none"))
    :child(button():attr("id", "go"):child("go"):on_click(function()
      job = worker.spawn("job.luau", { upto = 5000 })
      job:on_message(function(m)
        if m.hello then said = m.hello end
        if m.echo then echo = tostring(m.echo) end
      end)
      job:on_progress(function(p) progress = p.done end)
      job:post({ n = 21 })
    end))
    :child(button():attr("id", "missing"):child("missing"):on_click(function()
      local ok, why = pcall(worker.spawn, "nope.luau")
      said = if ok then "spawned?" else tostring(why)
    end))
end)
"#,
    )
    .unwrap();
    let pages = Pages::new(AppHost::new(ActionList::new())).with_luau();
    let mut pane = Pane::open(&pages, &path);
    pane.wait_for("#status", "none", Duration::from_secs(3));
    pane.click("#go");
    // Its news reaches the page with no event of the page's own: the
    // worker's wake runs its callbacks, then a frame.
    pane.wait_for("#said", "from job.luau", Duration::from_secs(5));
    pane.wait_for("#progress", "5000", Duration::from_secs(5));
    pane.wait_for("#echo", "42", Duration::from_secs(5));
    // It serves messages until its page goes: still running.
    pane.wait_for("#status", "running", Duration::from_secs(2));
    // A worker that is not a file of its folder says which there are.
    pane.click("#missing");
    let start = Instant::now();
    loop {
        if let Ok(f) = pane.session.frames.try_recv() {
            pane.view.apply(f).unwrap();
        }
        let said = text(&pane.view, "#said").unwrap_or_default();
        if said.contains("no worker") {
            assert!(said.contains("job.luau"), "{said}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3), "{said}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_luau_pages_timers_run_on_its_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clock.luau");
    std::fs::write(
        &path,
        r#"local ticks, once = 0, "waiting"
local every = set_interval(function()
  ticks += 1
end, 20)
set_timeout(function() once = "fired" end, 30)
local never = set_timeout(function() once = "cleared?" end, 10)
clear_timeout(never)
render(function()
  if ticks >= 5 then clear_interval(every) end
  return div()
    :child(span():attr("id", "ticks"):child(tostring(ticks)))
    :child(span():attr("id", "once"):child(once))
end)
"#,
    )
    .unwrap();
    let pages = Pages::new(AppHost::new(ActionList::new())).with_luau();
    let mut pane = Pane::open(&pages, &path);
    pane.wait_for("#once", "fired", Duration::from_secs(3));
    pane.wait_for("#ticks", "5", Duration::from_secs(3));
    // Cleared at five: it stays there.
    std::thread::sleep(Duration::from_millis(120));
    while let Ok(f) = pane.session.frames.try_recv() {
        pane.view.apply(f).unwrap();
    }
    assert_eq!(text(&pane.view, "#ticks").as_deref(), Some("5"));
}
