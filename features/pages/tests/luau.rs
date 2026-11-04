//! Luau element pages: `div():child(...):on_click(...)`, drawn as the
//! same edits a TSX page's tree is, following the theme.

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
