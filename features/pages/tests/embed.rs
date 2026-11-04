//! Inline embeds (#180): a fenced block whose language an extension
//! claims is drawn by it inside the host's document — a page in a Blitz
//! sub-document sized to its content (it grows and shrinks with the
//! fence, up to its `max_height`), a linked view in the host's own
//! document — and the plain fence comes back when the extension goes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::missing_panics_doc
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::contrib::Claim;
use architect_extensions::embeds::EmbedView;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_extensions::interface::use_extension_ui;
use architect_extensions::linked::{Builder, Extension, use_embed};
use architect_extensions::remote::mounted_views;
use architect_pages::files::{Origin, PageFile};
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
use architect_pages::ui::use_embed_pages;
use architect_settings::Registry;
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const ID: &str = "lab.embeds";

const MANIFEST: &str = r#"
id = "lab.embeds"
name = "Embeds"
version = "0.1.0"
kind = "process"

[entry]
path = "embeds-bin"

[[views]]
id = "chip"
title = "Chip"

# A page draws ```rows fences, as tall as its rows, at most 200 points.
[[embeds]]
id = "rows"
page = "pages/rows.tsx"
claims = { fence = ["rows"] }
max_height = 200

# A linked view draws ```chip fences, in the host's own document.
[[embeds]]
id = "chip"
view = "chip"
claims = { fence = ["chip"], note_type = ["song"] }
"#;

/// Each line of the fence a row 20 points tall.
const ROWS: &str = r#"import { h, useEmbed } from "architect";
export default function Rows() {
  const e = useEmbed();
  const lines = (e?.source ?? "").split("\n").filter((l) => l.trim() !== "");
  return (
    <div id="rows" data-claim={e?.claim ?? ""}>
      {lines.map((l, i) => <div key={i} class="row" style={{ height: 20 }}>{l}</div>)}
    </div>
  );
}
"#;

/// The same in Luau.
const ROWS_LUAU: &str = r#"
render(function()
  local e = embed()
  local box = div():attr("id", "rows")
  if e then
    for line in string.gmatch(e.source, "[^\n]+") do
      box:child(div():class("row"):style({ height = 20 }):child(line))
    end
  end
  return box
end)
"#;

#[component]
fn Chip() -> Element {
    let input = use_embed();
    let text = input.map(|i| i.source).unwrap_or_default();
    rsx! { span { id: "chip", "{text}" } }
}

struct Embeds;

impl Extension for Embeds {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        ext.view("chip", Chip);
    }
}

/// The fence's body, as the host's note has it.
static SOURCE: GlobalSignal<String> = Signal::global(|| "one\ntwo\nthree".to_owned());

#[derive(Props, Clone)]
struct AppProps {
    host: Host,
    pages: Pages,
}

impl PartialEq for AppProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A note: two fences, each its plain source when nothing claims it.
#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn App(props: AppProps) -> Element {
    let ui = use_extension_ui(props.host.clone(), Arc::new(ActionList::new()));
    use_context_provider(|| ui);
    let _ = use_embed_pages(props.pages);
    let source = SOURCE();
    rsx! {
        div { style: "width: 400px; display: flex; flex-direction: column;",
            div { id: "above", style: "height: 10px;" }
            EmbedView {
                claim: Claim::Fence("rows".into()),
                source: source.clone(),
                fallback: rsx! { pre { id: "plain-rows", "{source}" } },
            }
            div { id: "between", style: "height: 10px;" }
            EmbedView {
                claim: Claim::Fence("chip".into()),
                source: "a chip".to_owned(),
                fallback: rsx! { pre { id: "plain-chip", "a chip" } },
            }
        }
    }
}

fn in_app(doc: &BaseDocument, selector: &str) -> Option<String> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

/// How many `.row`s the page's sub-document has.
fn rows(doc: &BaseDocument) -> usize {
    let Some(at) = mounted_views(doc).first().copied() else {
        return 0;
    };
    let view = doc.subdoc(at).unwrap().inner();
    view.query_selector_all(".row").map_or(0, |r| r.len())
}

/// Where `selector` is in the host's document, top and height.
fn placed(doc: &BaseDocument, selector: &str) -> (f32, f32) {
    let node = doc.query_selector(selector).unwrap().unwrap();
    let n = doc.get_node(node).unwrap();
    (
        n.absolute_position(0.0, 0.0).y,
        n.final_layout().size.height,
    )
}

/// The page embed's box in the host's document: its height.
fn embed_height(doc: &BaseDocument) -> f32 {
    placed(doc, "[data-embed=\"lab.embeds/rows\"]").1
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) -> Duration {
    let started = Instant::now();
    let waker = futures::task::noop_waker();
    loop {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        if until(&doc.inner.borrow()) {
            return started.elapsed();
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}: {} rows, the embed {} tall",
            rows(&doc.inner.borrow()),
            doc.inner
                .borrow()
                .query_selector("[data-embed=\"lab.embeds/rows\"]")
                .ok()
                .flatten()
                .map_or(-1.0, |_| embed_height(&doc.inner.borrow())),
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn set_source(doc: &DioxusDocument, text: &str) {
    doc.vdom.in_runtime(|| *SOURCE.write() = text.to_owned());
}

#[test]
fn a_claimed_fence_mounts_its_embed_sized_to_its_content_and_unloading_restores_it() {
    claimed_fence("rows.tsx", ROWS);
}

#[test]
fn a_luau_page_embed_is_sized_and_given_its_input_the_same() {
    claimed_fence("rows.luau", ROWS_LUAU);
}

fn claimed_fence(name: &str, text: &str) {
    let app_dir = tempfile::tempdir().unwrap();
    let host = Host::new(ActionList::new(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(app_dir.path().join("installs")))
        .with_linked(&Embeds)
        .unwrap();
    let _ = ext.sync();
    // A built-in's page file, as an app writes its own out.
    let page = app_dir.path().join(name);
    std::fs::write(&page, text).unwrap();
    let pages = Pages::new(AppHost::new(ActionList::new()))
        .with_app_dir(app_dir.path())
        .with_js()
        .with_luau();
    pages.declare(
        ID,
        "embed.rows",
        PageFile::new(page, Origin::Builtin).unwrap(),
    );

    // The claims resolve: a page, and a linked view.
    let embeds = ext.host().embeds();
    let rows_spec = embeds.resolve(&Claim::Fence("ROWS".into())).unwrap();
    assert_eq!(
        (rows_spec.extension.as_str(), rows_spec.view.as_str()),
        (ID, "embed.rows")
    );
    assert_eq!(rows_spec.max_height, 200);
    assert_eq!(
        embeds
            .resolve(&Claim::NoteType("song".into()))
            .unwrap()
            .embed
            .id,
        "chip"
    );
    assert!(embeds.resolve(&Claim::Fence("rust".into())).is_none());
    assert_eq!(embeds.fences(), ["chip", "rows"]);

    let vdom = VirtualDom::new_with_props(
        App,
        AppProps {
            host: ext.host().clone(),
            pages,
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();

    // Mounted, given the fence, as tall as its three rows.
    let took = pump(&mut doc, "three rows, 60 points tall", |d| {
        rows(d) == 3 && (embed_height(d) - 60.0).abs() < 0.5
    });
    eprintln!("the page embed mounted and sized in {took:?}");
    assert_eq!(mounted_views(&doc.inner.borrow()).len(), 1);
    // The linked view is in the host's own document, below it.
    pump(&mut doc, "the chip", |d| {
        in_app(d, "#chip").as_deref() == Some("a chip")
    });
    {
        let d = doc.inner.borrow();
        let (between, _) = placed(&d, "#between");
        let (above, _) = placed(&d, "#above");
        assert_eq!(between - above, 10.0 + 60.0, "the host lays out its height");
        assert!(in_app(&d, "#plain-rows").is_none());
    }

    // The fence is edited: the embed draws again and grows.
    set_source(&doc, "one\ntwo\nthree\nfour\nfive\nsix");
    let took = pump(&mut doc, "six rows, 120 points tall", |d| {
        rows(d) == 6 && (embed_height(d) - 120.0).abs() < 0.5
    });
    eprintln!("an edit redrawn and resized in {took:?}");
    // And shrinks.
    set_source(&doc, "one");
    pump(&mut doc, "one row, 24 points tall at least", |d| {
        rows(d) == 1 && (embed_height(d) - 24.0).abs() < 0.5
    });
    // Past its max_height it stops growing; its sub-document scrolls.
    set_source(&doc, &"row\n".repeat(30));
    pump(&mut doc, "thirty rows, 200 points tall", |d| {
        rows(d) == 30 && (embed_height(d) - 200.0).abs() < 0.5
    });

    // Unloaded: the plain fences come back, no sub-document left.
    let _ = ext.disable(ID);
    pump(&mut doc, "the plain fences", |d| {
        in_app(d, "#plain-rows").is_some()
            && in_app(d, "#plain-chip").as_deref() == Some("a chip")
            && in_app(d, "#chip").is_none()
            && mounted_views(d).is_empty()
    });
    assert!(
        ext.host()
            .embeds()
            .resolve(&Claim::Fence("rows".into()))
            .is_none()
    );
    // On again: drawn again.
    let _ = ext.enable(ID);
    pump(&mut doc, "the embed again", |d| rows(d) == 30);
}
