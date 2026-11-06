//! The navigator in a headless Blitz document: 100,000 notes draw a
//! screenful of rows and keep drawing one as the list jumps to its end; a
//! wide sidebar has two panes, a narrow one (or a phone) one at a time; a
//! click on a folder lists its notes and a click on a note opens it; the
//! tag rows draw, a ⌘/Ctrl-click on one builds the search, and a note
//! dragged onto a tag is tagged.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use architect_navigator::ui::{Navigator, NavigatorView, Open, use_navigator};
use architect_navigator::{Key, MemorySource, Mods, NavigatorSource, NoteFile, Pane};
use blitz_dom::Document as _;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};

const NOW: i64 = 1_793_880_000_000;

thread_local! {
    static SOURCE: RefCell<Option<Arc<dyn NavigatorSource>>> = const { RefCell::new(None) };
    static NAV: RefCell<Option<Navigator>> = const { RefCell::new(None) };
    static OPENED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static PHONE: RefCell<bool> = const { RefCell::new(false) };
}

#[component]
fn App() -> Element {
    let nav = use_navigator(|| SOURCE.with(|s| s.borrow().clone().unwrap()));
    NAV.with(|n| *n.borrow_mut() = Some(nav));
    let phone = PHONE.with(|p| *p.borrow());
    rsx! {
        div { style: "position:absolute; inset:0;",
            NavigatorView {
                navigator: nav,
                phone,
                on_open: move |o: Open| OPENED.with(|v| v.borrow_mut().push(o.path)),
            }
        }
    }
}

fn mount(source: Arc<dyn NavigatorSource>, width: u32, phone: bool) -> DioxusDocument {
    architect_shell::motion::set_reduced_motion(Some(true));
    SOURCE.with(|s| *s.borrow_mut() = Some(source));
    PHONE.with(|p| *p.borrow_mut() = phone);
    OPENED.with(|v| v.borrow_mut().clear());
    let mut doc = DioxusDocument::new(
        VirtualDom::new(App),
        DocumentConfig {
            viewport: Some(Viewport::new(width, 700, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    settle(&mut doc);
    doc
}

/// Polls and lays out, with time for the views to be measured.
fn settle(doc: &mut DioxusDocument) {
    for _ in 0..3 {
        for _ in 0..3 {
            doc.poll(None);
            doc.inner.borrow_mut().resolve(0.0);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    doc.poll(None);
    doc.inner.borrow_mut().resolve(0.0);
}

fn nav() -> Navigator {
    NAV.with(|n| n.borrow().unwrap())
}

fn count(doc: &DioxusDocument, selector: &str) -> usize {
    doc.inner
        .borrow()
        .query_selector_all(selector)
        .unwrap()
        .len()
}

fn rect(doc: &DioxusDocument, selector: &str) -> (f64, f64, f64, f64) {
    let d = doc.inner.borrow();
    let id = d
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"));
    let r = d.get_client_bounding_rect(id).unwrap();
    (r.x, r.y, r.width, r.height)
}

fn pointer(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons,
        mods: Modifiers::empty(),
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: std::sync::Arc::default(),
    }
}

/// A click on the middle of the first element matching `selector`.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
fn click(doc: &mut DioxusDocument, selector: &str) {
    let (x, y, w, h) = rect(doc, selector);
    let (x, y) = ((x + w / 2.0) as f32, (y + h / 2.0) as f32);
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        x,
        y,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, MouseEventButtons::None)));
    settle(doc);
}

#[test]
fn a_hundred_thousand_notes_draw_a_screenful() {
    let source = MemorySource::synthetic("Big", 20, 100_000, NOW);
    let mut doc = mount(Arc::new(source), 900, false);
    // Every note, from the folders below the root.
    doc.vdom.in_runtime(|| {
        nav().settings.update(|s| s.include_descendants = true);
    });
    settle(&mut doc);
    let order_len = doc
        .vdom
        .in_runtime(|| nav().state.peek().list_order().len());
    assert_eq!(order_len, 100_000);
    let drawn = count(&doc, "[data-file]");
    assert!(drawn > 5 && drawn < 60, "{drawn} rows drawn");
    // The end: the last note drawn, in view, and still only a screenful.
    let last = doc.vdom.in_runtime(|| {
        let mut state = nav().state;
        let mut s = state.write();
        s.focus_pane(Pane::Files);
        s.key(Key::End, Mods::NONE);
        s.selection().cursor().unwrap().to_owned()
    });
    settle(&mut doc);
    let (_, y, _, h) = rect(&doc, &format!("[data-file=\"{last}\"]"));
    assert!(y >= 0.0 && y + h <= 700.5, "the last note at {y}+{h}");
    let drawn = count(&doc, "[data-file]");
    assert!(drawn < 60, "{drawn} rows drawn at the end");
}

#[test]
fn wide_is_two_panes_narrow_is_one_and_a_phone_is_one() {
    let lab = || {
        Arc::new(
            MemorySource::new("Lab")
                .with_file(NoteFile::new("Welcome.md", NOW, NOW).preview("Hello"))
                .with_file(NoteFile::new("Guides/Shell.md", NOW, NOW).preview("The shell")),
        )
    };
    let doc = mount(lab(), 700, false);
    assert_eq!(count(&doc, ".nv-dual"), 1);
    let (_, _, w, _) = rect(&doc, ".nv-nav");
    assert!((w - 200.0).abs() < 2.0, "the folders 200 wide: {w}");
    // Narrower than the folders + 150: one pane, the notes first.
    let mut doc = mount(lab(), 320, false);
    assert_eq!(count(&doc, ".nv-single"), 1);
    let (x, ..) = rect(&doc, ".nv-list");
    assert!(x.abs() < 1.0, "the notes on show: {x}");
    // Back to the folders: they slide in (at once, with reduced motion).
    click(&mut doc, "[aria-label=\"Back to the folders\"]");
    let (x, ..) = rect(&doc, ".nv-nav");
    assert!(x.abs() < 1.0, "the folders on show: {x}");
    // A phone: one pane whatever the width.
    let doc = mount(lab(), 900, true);
    assert_eq!(count(&doc, ".nv-single"), 1);
}

#[test]
fn a_click_on_a_folder_lists_it_and_a_click_on_a_note_opens_it() {
    let source = MemorySource::new("Lab")
        .with_file(NoteFile::new("Welcome.md", NOW, NOW).preview("Hello"))
        .with_file(NoteFile::new("Guides/Shell.md", NOW, NOW).preview("The shell"))
        .with_file(NoteFile::new("Guides/Workspace.md", NOW - 1, NOW - 1));
    let mut doc = mount(Arc::new(source), 700, false);
    assert_eq!(count(&doc, "[data-file=\"Welcome.md\"]"), 1);
    click(&mut doc, "[data-folder=\"Guides\"]");
    assert_eq!(count(&doc, "[data-file=\"Welcome.md\"]"), 0);
    assert_eq!(count(&doc, "[data-file]"), 2);
    click(&mut doc, "[data-file=\"Guides/Workspace.md\"]");
    assert_eq!(OPENED.with(|v| v.borrow().clone()), ["Guides/Workspace.md"]);
    assert_eq!(
        count(
            &doc,
            "[data-file=\"Guides/Workspace.md\"][aria-selected=true]"
        ),
        1
    );
}

#[test]
fn the_calendar_draws_days_and_a_click_opens_or_makes_their_notes() {
    use chrono::Datelike as _;
    let today = chrono::Local::now().date_naive();
    // Today's daily note (Notebook Navigator's default pattern), and the
    // week's other days empty.
    let path_of = |d: chrono::NaiveDate| format!("{}/{}.md", d.year(), d.format("%Y%m%d"));
    let mut note = NoteFile::new(path_of(today), NOW, NOW).preview("Today");
    note.feature_image = Some("https://example.com/sky.jpg".to_owned());
    let source = MemorySource::new("Days").with_file(note);
    let mut doc = mount(Arc::new(source.clone()), 600, false);
    assert_eq!(count(&doc, ".nv-calendar"), 0);
    click(&mut doc, "[data-testid=nv-calendar-toggle]");
    // Side by side: under the folders, one week.
    assert_eq!(count(&doc, ".nv-nav .nv-calendar"), 1);
    assert_eq!(count(&doc, ".nv-cal-day"), 7);
    assert_eq!(count(&doc, ".nv-cal-day.is-today"), 1);
    assert_eq!(count(&doc, ".nv-cal-day.has-note.has-image"), 1);
    let (_, _, w, h) = rect(&doc, ".nv-cal-day.is-today");
    assert!(w > 10.0 && h > 10.0, "{w}×{h}");
    // A day with a note opens it.
    click(&mut doc, ".nv-cal-day.is-today");
    assert_eq!(OPENED.with(|v| v.borrow().clone()), [path_of(today)]);
    // A day without one asks, then makes it and opens it.
    let other = if today.weekday() == chrono::Weekday::Sat {
        today.pred_opt().unwrap()
    } else {
        today.succ_opt().unwrap()
    };
    click(&mut doc, &format!("[data-date=\"{other}\"]"));
    assert_eq!(count(&doc, ".nv-confirm"), 1);
    click(&mut doc, "[data-testid=nv-confirm-create]");
    assert_eq!(count(&doc, ".nv-confirm"), 0);
    assert!(source.files().iter().any(|f| f.path == path_of(other)));
    assert_eq!(
        OPENED.with(|v| v.borrow().last().cloned()),
        Some(path_of(other))
    );
    // Stacked: under the notes. The whole month: four to six weeks.
    nav().settings.update(|s| {
        s.orientation = architect_navigator::Orientation::Vertical;
        s.calendar_weeks = 6;
    });
    settle(&mut doc);
    assert_eq!(count(&doc, ".nv-list .nv-calendar"), 1);
    assert_eq!(count(&doc, ".nv-nav .nv-calendar"), 0);
    let weeks = count(&doc, ".nv-cal-week");
    assert!((4..=6).contains(&weeks), "{weeks} weeks");
    // The month's name opens, or here asks to make, the monthly note.
    click(&mut doc, ".nv-cal-period[data-period=month]");
    assert_eq!(count(&doc, ".nv-confirm"), 1);
    click(&mut doc, "[data-testid=nv-confirm-cancel]");
    assert_eq!(count(&doc, ".nv-confirm"), 0);
}

/// A press at one point, moved to another in steps, let go there.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
fn drag(doc: &mut DioxusDocument, from: &str, to: &str) {
    let (fx, fy, fw, fh) = rect(doc, from);
    let (tx, ty, tw, th) = rect(doc, to);
    let (x0, y0) = (fx + fw / 2.0, fy + fh / 2.0);
    let (x1, y1) = (tx + tw / 2.0, ty + th / 2.0);
    doc.handle_ui_event(UiEvent::PointerMove(pointer(
        x0 as f32,
        y0 as f32,
        MouseEventButtons::None,
    )));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        x0 as f32,
        y0 as f32,
        MouseEventButtons::Primary,
    )));
    for i in 1..=8 {
        let t = f64::from(i) / 8.0;
        let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        doc.handle_ui_event(UiEvent::PointerMove(pointer(
            x as f32,
            y as f32,
            MouseEventButtons::Primary,
        )));
        doc.poll(None);
    }
    doc.handle_ui_event(UiEvent::PointerUp(pointer(
        x1 as f32,
        y1 as f32,
        MouseEventButtons::None,
    )));
    settle(doc);
}

#[test]
fn tag_rows_draw_a_command_click_searches_and_a_note_dropped_on_a_tag_is_tagged() {
    let source = MemorySource::new("Tags")
        .with_file(NoteFile::new("Plan.md", NOW, NOW).tag("work"))
        .with_file(NoteFile::new("Loose.md", NOW - 1000, NOW - 1000));
    let mut doc = mount(Arc::new(source.clone()), 900, false);
    assert_eq!(
        count(&doc, "[role=\"treeitem\"][data-tag=\"__tagged__\"]"),
        1
    );
    assert_eq!(count(&doc, "[role=\"treeitem\"][data-tag=\"work\"]"), 1);
    assert_eq!(
        count(&doc, "[role=\"treeitem\"][data-tag=\"__untagged__\"]"),
        1
    );
    // ⌘ (Ctrl elsewhere) and a click: into the search, over the list.
    let (x, y, w, h) = rect(&doc, "[role=\"treeitem\"][data-tag=\"work\"]");
    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
    let (x, y) = ((x + w / 2.0) as f32, (y + h / 2.0) as f32);
    let command = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    };
    let with = |mut e: BlitzPointerEvent| {
        e.mods = command;
        e
    };
    doc.handle_ui_event(UiEvent::PointerDown(with(pointer(
        x,
        y,
        MouseEventButtons::Primary,
    ))));
    doc.handle_ui_event(UiEvent::PointerUp(with(pointer(
        x,
        y,
        MouseEventButtons::None,
    ))));
    settle(&mut doc);
    assert_eq!(nav().state.peek().query().text(), "#work");
    assert_eq!(count(&doc, "[data-query=\"#work\"]"), 1);
    click(&mut doc, "[aria-label=\"Clear the search\"]");
    assert!(nav().state.peek().query().is_empty());
    // A note dragged onto the tag: tagged.
    drag(
        &mut doc,
        "[data-file=\"Loose.md\"]",
        "[role=\"treeitem\"][data-tag=\"work\"]",
    );
    let tags = source
        .files()
        .into_iter()
        .find(|f| f.path == "Loose.md")
        .unwrap()
        .tags;
    assert_eq!(tags, ["work"]);
}
