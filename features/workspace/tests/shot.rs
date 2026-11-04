//! `APP_SHOT` against a headless document standing in for the window:
//! the PNG is the window's size in pixels, painted as the page is, and a
//! pane's shot is cropped to it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_workspace::shot::{APP_SHOT, LiveDocument, use_app_shot};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};

#[component]
fn Page() -> Element {
    let list = use_context::<Arc<ActionList>>();
    use_app_shot(&list);
    rsx! {
        div { style: "position: absolute; left: 0; top: 0; width: 100px; height: 100px; background: rgb(255, 0, 0);" }
    }
}

/// Runs `id` with `args` on a thread, pumping the document here until it
/// answers (the shot is taken on the document's own thread).
fn run(
    doc: &mut DioxusDocument,
    list: &Arc<ActionList>,
    id: &str,
    args: &str,
) -> Result<serde_json::Value, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (list, id, args) = (Arc::clone(list), id.to_owned(), args.to_owned());
    std::thread::spawn(move || {
        let out = futures::executor::block_on(list.run_json_async(&id, args.as_bytes()));
        let _ = tx.send(out);
    });
    let started = Instant::now();
    loop {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
        if let Ok(out) = rx.recv_timeout(Duration::from_millis(5)) {
            return out
                .map(|bytes| serde_json::from_slice(&bytes).unwrap())
                .map_err(|e| e.to_string());
        }
        if started.elapsed() >= Duration::from_secs(60) {
            return Err("no answer".to_owned());
        }
    }
}

#[test]
fn a_shot_is_the_window_at_its_pixel_size() {
    let list = Arc::new(ActionList::new());
    let vdom = VirtualDom::new(Page).with_root_context(Arc::clone(&list));
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(320, 200, 2.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    let live = LiveDocument::new(doc.inner.clone());
    doc.vdom
        .in_scope(ScopeId::ROOT, move || provide_context(live));
    doc.initial_build();
    doc.inner.borrow_mut().resolve(0.0);
    assert!(list.info(APP_SHOT).is_some(), "APP_SHOT is on the list");

    let path = std::env::temp_dir().join(format!("app-shot-{}.png", std::process::id()));
    let args = serde_json::json!({ "path": path.display().to_string() }).to_string();
    let shot = run(&mut doc, &list, APP_SHOT, &args).unwrap();
    assert_eq!(shot["path"], path.display().to_string());
    assert_eq!(
        (shot["width"].as_u64(), shot["height"].as_u64()),
        (Some(320), Some(200))
    );
    assert_eq!(shot["scale"].as_f64(), Some(2.0));

    let image = image_of(&path);
    assert_eq!((image.0, image.1), (320, 200));
    // 100 points of red at scale 2: the page's pixel at (50, 50) points,
    // white past it.
    assert_eq!(pixel(&image, 100, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 300, 150), [255, 255, 255, 255]);

    // A misspelt argument, a path that is not a PNG, a pane there is not.
    assert!(
        run(&mut doc, &list, APP_SHOT, r#"{"paht":"/tmp/x.png"}"#)
            .unwrap_err()
            .contains("valid arguments: path, pane")
    );
    assert!(
        run(&mut doc, &list, APP_SHOT, r#"{"path":"/tmp/x.jpg"}"#)
            .unwrap_err()
            .contains(".png")
    );
    assert!(run(&mut doc, &list, APP_SHOT, r#"{"pane":"nope"}"#).is_err());
    let _ = std::fs::remove_file(&path);
}

fn image_of(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn pixel(image: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let at = y
        .checked_mul(image.0)
        .and_then(|row| row.checked_add(x))
        .and_then(|p| p.checked_mul(4))
        .and_then(|b| usize::try_from(b).ok())
        .expect("a pixel of the image");
    *image
        .2
        .get(at..)
        .and_then(|px| px.first_chunk::<4>())
        .expect("a pixel of the image")
}
