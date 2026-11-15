//! Screenshots of the review view: the linked view in a headless Blitz
//! document over a real vault (a song whose light show has two versions,
//! comments at a time, over a range with a part of the frame, a reply),
//! the playhead at the range so its region shows, rendered on the CPU at
//! 390 px and 1280 px, in the app's built-in light and dark themes.
//!
//! `SHOTS_DIR=<folder> cargo test -p architect-review --test shots --
//! --ignored --nocapture`.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    clippy::print_stdout,
    clippy::needless_pass_by_value,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyrender::PaintScene as _;
use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::Installs;
use architect_theme::{Registry, RootCss, Scheme};
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use futures::executor::block_on;
use peniko::kurbo::{Affine, Rect};
use peniko::{Color, Fill};
use serde_json::{Value, json};

const SONG: &str = "Songs/Joy/Joy.md";

fn run(list: &ActionList, id: &str, args: Value) -> Value {
    let out = block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    serde_json::from_slice(&out).unwrap_or(Value::Null)
}

/// The app with Threads, the playhead and Review over a seeded vault.
fn seeded(dir: &Path) -> (ActionList, Extensions) {
    let vault = dir.join("vault");
    std::fs::create_dir_all(vault.join("Songs/Joy")).unwrap();
    std::fs::write(
        vault.join(SONG),
        "---\ntitle: Joy to the World\nartist: Senior Recital · Lights\nduration: \"3:20\"\n\
         review:\n  - name: Lights\n    versions:\n      - \"[[Joy - Lights v1.mp4]]\"\n      - \"[[Joy - Lights v2.mp4]]\"\n\
         \x20 - name: Mix\n    versions:\n      - \"[[Joy.ogg]]\"\n---\n",
    )
    .unwrap();
    let app = Arc::new(ActionList::new());
    let open = vault::actions::OpenVault::open(&vault).unwrap();
    vault::actions::register(&app, &open).unwrap();
    let types: &'static architect_bases::model::ViewTypes =
        Box::leak(Box::new(architect_bases::model::ViewTypes::new()));
    architect_bases::actions::register(
        &app,
        &open,
        types,
        architect_bases::actions::local_queries(&open),
    )
    .unwrap();
    run(&app, "VAULT_RELOAD", Value::Null);
    let list = ActionList::new();
    let host = Host::new(
        list.clone(),
        SharedSections::new(architect_settings::Registry::new()),
    );
    host.set_app_list((*app).clone());
    host.forward_actions_to(&app);
    let ext = Extensions::new(host, Installs::new(dir.join("installs")))
        .with_linked(&architect_threads::Threads)
        .unwrap()
        .with_linked(&architect_playhead::Playhead)
        .unwrap()
        .with_linked(&architect_review::Review)
        .unwrap();
    ext.install_contribution_actions().unwrap();
    let _ = ext.sync();
    let v1 = "Songs/Joy/Joy - Lights v1.mp4";
    for (start, end, rect, body, who) in [
        (
            json!(12),
            Value::Null,
            Value::Null,
            "Intro wash should fade in over the first bar, not snap",
            "sam",
        ),
        (
            json!(33),
            Value::Null,
            Value::Null,
            "Strobe hits a beat early @alex",
            "cody",
        ),
        (
            json!(60),
            json!(75.5),
            json!({"x": 0.5, "y": 0.0, "w": 0.5, "h": 0.5}),
            "Bridge drags; the top right wash is too blue",
            "alex",
        ),
        (
            json!(64),
            json!(70),
            Value::Null,
            "Guitars too loud under the vocal here",
            "jt",
        ),
        (
            json!(150),
            Value::Null,
            Value::Null,
            "Final hit: blackout on the downbeat",
            "cody",
        ),
    ] {
        let out = run(
            &list,
            "REVIEW_COMMENT",
            json!({ "path": SONG, "file": v1, "start": start, "end": end, "rect": rect, "body": body, "author": who }),
        );
        if who == "alex" {
            run(
                &list,
                "THREADS_REPLY",
                json!({ "thread": out["thread"]["id"], "comment": out["comment"]["id"], "body": "Agreed — will warm it in v2", "author": "cody" }),
            );
        }
        if who == "sam" {
            run(
                &list,
                "THREADS_RESOLVE",
                json!({ "thread": out["thread"]["id"], "by": "cody" }),
            );
        }
    }
    run(
        &list,
        "REVIEW_OPEN",
        json!({ "path": SONG, "file": v1, "open": false }),
    );
    run(&list, "REVIEW_PLAY", json!({ "t": 66, "play": false }));
    (list, ext)
}

fn root_css(scheme: Scheme) -> String {
    let theme = architect_theme::builtin::fallback(scheme);
    let resolved = architect_theme::resolve(&theme, &Registry::builtin());
    let mut vars = String::new();
    for (k, v) in resolved.css_tokens() {
        let _ = write!(vars, "--{k}: {v}; ");
    }
    format!(":root {{ {vars} }}")
}

#[derive(Props, Clone, PartialEq)]
struct AppProps {
    css: String,
}

#[allow(non_snake_case)]
fn App(props: AppProps) -> Element {
    let css = props.css.clone();
    use_context_provider(|| RootCss(Signal::new(css)));
    let sheet = format!(
        "{} html, body {{ margin: 0; background: var(--background); color: var(--foreground); font-family: sans-serif; }}",
        props.css
    );
    rsx! {
        style { "{sheet}" }
        div { style: "position: absolute; left: 0; top: 0; right: 0; bottom: 0;", architect_review::ReviewView {} }
    }
}

fn pump(doc: &mut DioxusDocument, rounds: u32) {
    let waker = futures::task::noop_waker();
    for _ in 0..rounds {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn shoot(doc: &DioxusDocument, (w, h, scale): (u32, u32, f64), bg: Color, path: &Path) {
    let (pw, ph) = ((f64::from(w) * scale) as u32, (f64::from(h) * scale) as u32);
    let pixels = anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                bg,
                None,
                &Rect::new(0.0, 0.0, f64::from(pw), f64::from(ph)),
            );
            blitz_paint::paint_scene(scene, &mut doc.inner.borrow_mut(), scale, pw, ph, 0, 0);
        },
        pw,
        ph,
    );
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, pw, ph);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
    println!("{}", path.display());
}

#[test]
#[ignore = "writes screenshots: SHOTS_DIR=… --ignored --nocapture"]
fn shots() {
    let Some(out) = std::env::var_os("SHOTS_DIR").map(PathBuf::from) else {
        println!("set SHOTS_DIR");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let (_list, ext) = seeded(tmp.path());
    for (w, h, scale) in [(390_u32, 844_u32, 1.0_f64), (1280, 800, 1.0)] {
        for (scheme, cs, name) in [
            (Scheme::Light, ColorScheme::Light, "light"),
            (Scheme::Dark, ColorScheme::Dark, "dark"),
        ] {
            let vdom = VirtualDom::new_with_props(
                App,
                AppProps {
                    css: root_css(scheme),
                },
            );
            let mut doc = DioxusDocument::new(
                vdom,
                DocumentConfig {
                    viewport: Some(Viewport::new(w, h, 1.0, cs)),
                    ..Default::default()
                },
            );
            doc.initial_build();
            pump(&mut doc, 40);
            let bg = if name == "dark" {
                Color::from_rgb8(0x14, 0x14, 0x16)
            } else {
                Color::WHITE
            };
            shoot(
                &doc,
                (w, h, scale),
                bg,
                &out.join(format!("review-{w}-{name}.png")),
            );
        }
    }
    drop(ext);
}
