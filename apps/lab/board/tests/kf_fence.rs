//! The Board as a built-in with everything a Vault note shows of it
//! (#183, #180): its views, its `.kf` opener and its ```` ```kf ````
//! embed. In a note drawn on Blitz the fence is its chart, a sub-document
//! as tall as what it draws, growing and shrinking as the fence is edited;
//! turned off at run time its views, opener and embed all go and the note
//! shows the plain fence; turned on they all come back, and the chart with
//! them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
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
use architect_extensions::remote::mounted_views;
use architect_lab_board::{Board, ID, PAGES};
use architect_pages::files::{Origin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::runner::Pages;
use architect_pages::ui::use_embed_pages;
use architect_settings::Registry;
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

/// The fence's body, as the note has it.
static SOURCE: GlobalSignal<String> = Signal::global(|| "[Verse]\n| C | G | Am | F |".to_owned());

/// What the embed's box is in the note.
const EMBED: &str = "[data-embed=\"lab.board/kf\"]";

#[derive(Props, Clone)]
struct NoteProps {
    host: Host,
    pages: Pages,
}

impl PartialEq for NoteProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A note: a line above, the ```` ```kf ```` fence, a line below.
#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn Note(props: NoteProps) -> Element {
    let ui = use_extension_ui(props.host.clone(), Arc::new(ActionList::new()));
    use_context_provider(|| ui);
    let _ = use_embed_pages(props.pages);
    let source = SOURCE();
    rsx! {
        div { style: "width: 420px; display: flex; flex-direction: column;",
            div { id: "above", style: "height: 10px;" }
            EmbedView {
                claim: Claim::fence("kf").unwrap(),
                source: source.clone(),
                props: serde_json::json!({ "info": "kf" }),
                fallback: rsx! { pre { id: "plain-kf", "{source}" } },
            }
            div { id: "below", style: "height: 10px;" }
        }
    }
}

/// The text of `selector` in the chart's sub-document.
fn in_chart(doc: &BaseDocument, selector: &str) -> Option<String> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector(selector).ok()??;
    Some(view.get_node(node)?.text_content())
}

fn in_note(doc: &BaseDocument, selector: &str) -> Option<String> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

/// The top and height of `selector` in the note.
fn placed(doc: &BaseDocument, selector: &str) -> Option<(f32, f32)> {
    let node = doc.query_selector(selector).ok()??;
    let n = doc.get_node(node)?;
    Some((
        n.absolute_position(0.0, 0.0).y,
        n.final_layout().size.height,
    ))
}

fn embed_height(doc: &BaseDocument) -> f32 {
    placed(doc, EMBED).map_or(-1.0, |(_, h)| h)
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) {
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
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}: the chart says {:?}, the embed {} tall",
            in_chart(&doc.inner.borrow(), "#kf-count"),
            embed_height(&doc.inner.borrow()),
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn set_source(doc: &DioxusDocument, text: &str) {
    doc.vdom.in_runtime(|| text.clone_into(&mut SOURCE.write()));
}

/// The Board linked in, and its built-in pages declared as an app does
/// (`apps/lab/ui/src/extensions.rs` `lab_pages`).
fn board(app: &std::path::Path) -> (Extensions, Pages) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    let ext = Extensions::new(host, Installs::new(app.join("installs")))
        .with_linked(&Board)
        .unwrap();
    let _ = ext.sync();
    let dir = write_builtin(app, ID, PAGES).unwrap();
    let pages = Pages::new(AppHost::new(list)).with_app_dir(app).with_js();
    pages.declare(
        ID,
        "embed.kf",
        PageFile::new(dir.join("kf.tsx"), Origin::Builtin).unwrap(),
    );
    (ext, pages)
}

/// Its views (linked in, so drawn in the app's document), its `.kf`
/// opener and its ```` ```kf ```` embed, as the host has them now: all three, or
/// none.
fn adds(ext: &Extensions) -> (bool, bool, bool) {
    let host = ext.host();
    let views = ["board", "chart"]
        .iter()
        .all(|v| host.linked_view(ID, v).is_some());
    let opener = host
        .contributions()
        .open_path("intro.kf")
        .iter()
        .any(|by| by.owner == ID);
    let embed = host
        .embeds()
        .resolve(&Claim::fence("kf").unwrap())
        .is_some()
        && host.embeds().fences().contains(&"kf".to_owned());
    (views, opener, embed)
}

fn note(ext: &Extensions, pages: Pages) -> DioxusDocument {
    let vdom = VirtualDom::new_with_props(
        Note,
        NoteProps {
            host: ext.host().clone(),
            pages,
        },
    );
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 900, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
}

/// The chart is laid out in the note: what is below it is below its box.
fn laid_out_below(d: &BaseDocument) -> bool {
    let (Some((above, _)), Some((top, height)), Some((below, _))) =
        (placed(d, "#above"), placed(d, EMBED), placed(d, "#below"))
    else {
        return false;
    };
    (top - (above + 10.0)).abs() < 0.5 && (below - (top + height)).abs() < 0.5
}

/// A ```` ```kf ```` fence in a note, drawn by the Board on Blitz: as tall as its
/// chart, taller with every section the fence gains, shorter again when
/// it loses them, the note laid out around it each time.
#[test]
fn a_kf_fence_is_the_boards_chart_and_resizes_with_its_content() {
    let app = tempfile::tempdir().unwrap();
    let (ext, pages) = board(app.path());
    let mut doc = note(&ext, pages);

    pump(&mut doc, "the chart's four bars", |d| {
        in_chart(d, "#kf-count").is_some_and(|t| t.starts_with("4 bars")) && embed_height(d) > 24.0
    });
    assert!(in_note(&doc.inner.borrow(), "#plain-kf").is_none());
    pump(&mut doc, "the note laid out around it", laid_out_below);
    let one = embed_height(&doc.inner.borrow());

    // Two more sections: it grows.
    set_source(
        &doc,
        "[Verse]\n| C | G | Am | F |\n[Chorus]\n| F | G | C | C |\n[Bridge]\n| Am | F | C | G |",
    );
    pump(&mut doc, "twelve bars, taller", |d| {
        in_chart(d, "#kf-count").is_some_and(|t| t.starts_with("12 bars"))
            && embed_height(d) > one + 20.0
            && laid_out_below(d)
    });
    let three = embed_height(&doc.inner.borrow());

    // Back to one: it shrinks to what it was.
    set_source(&doc, "[Verse]\n| C | G | Am | F |");
    pump(&mut doc, "four bars, as tall as before", |d| {
        in_chart(d, "#kf-count").is_some_and(|t| t.starts_with("4 bars"))
            && (embed_height(d) - one).abs() < 0.5
            && laid_out_below(d)
    });
    assert!(three > one, "{three} > {one}");
}

/// The Board, a built-in with views, an opener and an embed, turned off
/// and on at run time (`EXT_DISABLE` / `EXT_ENABLE` do the same): all
/// three go, the note's fence is plain again; all three come back, and
/// the fence is its chart again.
#[test]
fn turned_off_and_on_its_views_opener_and_embed_go_and_come_back() {
    let app = tempfile::tempdir().unwrap();
    let (ext, pages) = board(app.path());
    assert_eq!(adds(&ext), (true, true, true));
    let mut doc = note(&ext, pages);
    pump(&mut doc, "the chart", |d| {
        in_chart(d, "#kf-count").is_some_and(|t| t.starts_with("4 bars"))
    });

    let _ = ext.disable(ID);
    assert!(ext.host().get(ID).is_none());
    assert_eq!(adds(&ext), (false, false, false));
    assert!(ext.host().contributions().of(ID).is_none());
    pump(&mut doc, "the plain fence", |d| {
        in_note(d, "#plain-kf").is_some_and(|t| t.contains("| C | G | Am | F |"))
            && mounted_views(d).is_empty()
    });

    let _ = ext.enable(ID);
    assert_eq!(adds(&ext), (true, true, true));
    let keys = ext.host().contributions().of(ID).unwrap().keys();
    assert!(
        keys.contains("embed:kf") && keys.contains("tab:board"),
        "{keys:?}"
    );
    pump(&mut doc, "the chart again", |d| {
        in_note(d, "#plain-kf").is_none()
            && in_chart(d, "#kf-count").is_some_and(|t| t.starts_with("4 bars"))
    });
}
