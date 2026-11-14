//! The extension showcase's pages (#710, `examples/showcase`), run as the
//! app runs them: Templater-lite's Templates tab
//! (`examples/showcase/templater-lite/pages/templates.tsx`), a TSX page on
//! `QuickJS`, draws the templates its extension's action answers with, and
//! its New button runs that extension's action for the one it is on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, ActionMeta, Effect, RawJson};
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{EventData, Pointer, ViewEvent};
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::{PageSession, Pages};

fn example(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/showcase")
        .join(path)
}

struct Pane {
    session: PageSession,
    view: RemoteDocument,
    _events: futures::channel::mpsc::UnboundedReceiver<ViewEvent>,
}

impl Pane {
    fn open(pages: &Pages, path: &Path) -> Self {
        let page = PageFile::new(path, Origin::Override).unwrap();
        let session = pages
            .open(&page, "showcase.templater", "templates", None)
            .unwrap();
        let (to, from) = futures::channel::mpsc::unbounded();
        Self {
            session,
            view: RemoteDocument::new(to),
            _events: from,
        }
    }

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

    fn text(&self, selector: &str) -> Option<String> {
        let doc = self.view.document();
        let node = doc.query_selector(selector).ok()??;
        Some(doc.get_node(node)?.text_content())
    }

    /// Applies frames until `selector`'s text has `want`.
    fn wait_for_text(&mut self, selector: &str, want: &str) {
        let started = Instant::now();
        while !self.text(selector).is_some_and(|t| t.contains(want)) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{selector} is {:?}, without {want:?}; log: {:?}",
                self.text(selector),
                self.session.status().log.lines()
            );
            self.frame(Duration::from_millis(50));
        }
    }

    fn click(&self, selector: &str) {
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
    }
}

const fn meta(id: &'static str) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "",
        method_name: id,
        display_name: id,
        description: "",
        category: "Showcase",
        group: "Showcase",
        shortcut: "",
        toggleable: false,
        undo: false,
    }
}

static LIST: ActionMeta = meta("SHOWCASE_TEMPLATER_LIST");
static NEW: ActionMeta = meta("SHOWCASE_TEMPLATER_NEW");

/// What the page hands Templater-lite's new-note action.
#[derive(facet::Facet)]
struct NewArgs {
    #[facet(default)]
    template: Option<String>,
}

// A page's action runs go to the app's runtime: a tokio one, as the app has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn templater_lites_page_draws_the_templates_and_makes_a_note_from_one() {
    tokio::task::block_in_place(run_the_page);
}

fn run_the_page() {
    let list = ActionList::new();
    // What Templater-lite's list action answers (its Luau, tested in
    // features/extensions/tests/showcase.rs).
    list.register_with(&LIST, Effect::ReadOnly, |_: &ActionCaller, (): ()| {
        Ok::<_, String>(RawJson::from(
            r#"[{"path":"Templates/Daily.md","title":"Daily","fields":["date"]},
                {"path":"Templates/Meeting.md","title":"Meeting","fields":["title","date","who"]}]"#
                .to_owned(),
        ))
    })
    .unwrap();
    let made: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = Arc::clone(&made);
    list.register_with(
        &NEW,
        Effect::Mutating,
        move |_: &ActionCaller, a: NewArgs| {
            seen.lock().unwrap().push(a.template.unwrap_or_default());
            Ok::<_, String>(RawJson::from(r#""Inbox/Standup.md""#.to_owned()))
        },
    )
    .unwrap();
    let pages = Pages::new(AppHost::new(list)).with_js();
    let mut pane = Pane::open(&pages, &example("templater-lite/pages/templates.tsx"));

    // Each template, with the fields it asks for.
    pane.wait_for_text("#templates", "Meeting");
    let all = pane.text("#templates").unwrap();
    assert!(all.contains("Daily"), "{all}");
    assert!(all.contains("title, date, who"), "{all}");
    assert!(!all.contains("No templates yet"), "{all}");

    // Its New button on the first row runs the extension's action for it.
    pane.click(".template button");
    let started = Instant::now();
    while made.lock().unwrap().is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "New ran nothing; log: {:?}",
            pane.session.status().log.lines()
        );
        pane.frame(Duration::from_millis(50));
    }
    assert_eq!(made.lock().unwrap().as_slice(), ["Templates/Daily.md"]);
}
