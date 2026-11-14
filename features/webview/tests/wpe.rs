//! The WPE WebKit backend headless (feature `wpe`, Linux): a package's
//! page loads over the `architect-ext` protocol, renders a frame of the
//! view's size, and the runtime (`window.architect`) speaks to the host over
//! the `architect` message handler, all with no window and no display
//! server. Run in `devenv --profile wpe shell` (`nix/wpewebkit.nix`).
#![cfg(all(feature = "wpe", target_os = "linux"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use architect_extensions::webview;
use architect_webview::wpe::{Answer, FramePixels, Renderer, WpeWeb, iterate};

#[test]
fn a_package_page_renders_and_speaks_to_the_host() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><body style='margin:0;background:#3366cc'>\
         <script>window.addEventListener('load',()=>architect.log('loaded'))</script></body>",
    )
    .unwrap();
    let root = dir.path().to_owned();
    let web = WpeWeb::new(
        &Renderer::Cpu,
        (320, 200),
        webview::SCHEME,
        &webview::boot_script(),
        Box::new(move |uri: &str| {
            let path = uri.split_once("://").map_or("", |(_, r)| r.split_once('/').map_or("", |(_, p)| p));
            webview::serve("test.page", &root, &format!("/{path}")).ok().map(|a| Answer {
                status: 200,
                mime: a.mime.to_owned(),
                body: a.body,
                headers: a.headers.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect(),
            })
        }),
        || {},
    )
    .expect("WPE starts headless");
    web.load(&webview::url("test.page", "index.html"));

    let (mut frame, mut messages) = (None, Vec::new());
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(20) && (frame.is_none() || !messages.iter().any(|m: &String| m.contains("loaded"))) {
        iterate();
        let inbox = web.take();
        if let Some(f) = inbox.frame {
            web.frame_done(f.id);
            frame = Some(f);
        }
        messages.extend(inbox.messages);
        std::thread::sleep(Duration::from_millis(5));
    }
    let frame = frame.expect("a frame");
    assert_eq!((frame.width, frame.height), (320, 200));
    let FramePixels::Shm { pixels, stride } = &frame.pixels else { panic!("a CPU frame") };
    // The page's own colour, in the middle of the frame (BGRA).
    let at = (100 * *stride + 160 * 4) as usize;
    assert_eq!(&pixels[at..at + 3], &[0xcc, 0x66, 0x33], "the page's background");
    // The runtime's ready, then the page's own call, through the bridge.
    assert!(messages.iter().any(|m| m.contains(r#""t":"ready""#)), "{messages:?}");
    let call = messages.iter().find(|m| m.contains("loaded")).expect("the page's call");
    assert!(call.contains(r#""op":"log""#), "{call}");
}
