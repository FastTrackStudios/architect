//! The lab in a browser: the Board's tab as its page file
//! (`apps/sandbox/board/pages/board.tsx`, the same file the desktop lab runs
//! on `QuickJS`) in a Web Worker (`architect_pages::web`), and the WASI
//! Counter (`apps/sandbox/wasi-counter`), the same component the desktop lab
//! runs with Wasmtime, transpiled by jco and run by the browser's
//! WebAssembly (`architect_extensions::web`).
//!
//! The page puts the component's actions on its own Action List (beside a
//! `THEME_LIST` of its own, what the counter's "Ask the app" runs through
//! the host service), and the Board's three the page runs (`board`: the
//! Board's own are its Rust extension's, which is not built for a
//! browser). It opens a page in the main pane — the Board, its Preact page
//! (`?page=preact`), or one of the checks in `pages/` (`?page=runaway`,
//! `?page=net`) — mounts the
//! counter's `counter` view below it, and shows how the page keeps up:
//! each animation frame's gap, the longest of the last second, while a
//! worker counts primes. Built and served by `just web`
//! (docs/content/architecture/pages.md, "On the web";
//! docs/content/architecture/extensions.md, "On the web").

#![cfg(target_arch = "wasm32")]
// Nothing is `Send` in a browser.
#![allow(clippy::future_not_send)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionFuture, ActionInfo, ActionList, Effect};
use architect_extensions::Manifest;
use architect_extensions::web::{Options, WebComponent, WebHost};
use architect_pages::api::{ThemeSnapshot, ThemeToken};
use architect_pages::web::{PageFolder, WebPage, WebPages};
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Where `just web` puts the transpiled component, beside the page.
const COMPONENT: &str = "wasi-counter/architect_sandbox_wasi_counter.js";
const MANIFEST: &str = include_str!("../../sandbox/wasi-counter/extension.toml");

/// The page's own themes, as the app's `THEME_LIST` answers.
const THEMES: [&str; 3] = ["catppuccin-mocha", "catppuccin-latte", "zinc"];

mod board;

/// The Board: its manifest and its page files, the desktop lab's own.
const BOARD: &str = include_str!("../../sandbox/board/extension.toml");
const BOARD_PAGES: &[(&str, &str)] = &[
    (
        "board.tsx",
        include_str!("../../sandbox/board/pages/board.tsx"),
    ),
    ("kf.tsx", include_str!("../../sandbox/board/pages/kf.tsx")),
    (
        "primes.js",
        include_str!("../../sandbox/board/pages/primes.js"),
    ),
    (
        "preact.tsx",
        include_str!("../../sandbox/board/pages/preact.tsx"),
    ),
    (
        "vendor/preact.js",
        include_str!("../../sandbox/board/pages/vendor/preact.js"),
    ),
    (
        "vendor/hooks.js",
        include_str!("../../sandbox/board/pages/vendor/hooks.js"),
    ),
];

/// The checks' extension: the network it may reach is the lab's server.
const CHECKS: &str = r#"
id = "lab.web-checks"
name = "Web checks"
version = "0.1.0"
kind = "luau"
api = "0.1"

[capabilities]
hosts = ["127.0.0.1"]
"#;
const CHECK_PAGES: &[(&str, &str)] = &[
    ("runaway.tsx", include_str!("../pages/runaway.tsx")),
    ("net.tsx", include_str!("../pages/net.tsx")),
];

/// The pages the lab opens, by `?page=`: the folder and its extension.
fn folder(page: &str) -> Option<(PageFolder, &'static str)> {
    match page {
        "board" | "preact" => Some((
            PageFolder::new(&format!("{page}.tsx"), BOARD_PAGES.iter().copied()),
            BOARD,
        )),
        "runaway" | "net" => Some((
            PageFolder::new(&format!("{page}.tsx"), CHECK_PAGES.iter().copied()),
            CHECKS,
        )),
        _ => None,
    }
}

/// The lab's theme, as a page reads it: the colours of `index.html`.
fn theme() -> ThemeSnapshot {
    let tokens = [
        ("background", "#18181b"),
        ("foreground", "#e4e4e7"),
        ("border", "#3f3f46"),
        ("input", "#3f3f46"),
        ("card", "#27272a"),
        ("primary", "#4f46e5"),
        ("primary-foreground", "#ffffff"),
        ("secondary", "#27272a"),
        ("secondary-foreground", "#e4e4e7"),
        ("muted-foreground", "#a1a1aa"),
        ("destructive", "#b91c1c"),
        ("destructive-foreground", "#ffffff"),
    ];
    ThemeSnapshot {
        id: "lab-web-zinc".to_owned(),
        scheme: "dark".to_owned(),
        revision: 1,
        tokens: tokens
            .iter()
            .map(|(name, value)| ThemeToken {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
    }
}

thread_local! {
    /// The page open in the main pane, for `lab_status` and `lab_reload`.
    static OPEN: RefCell<Option<(String, WebPage)>> = const { RefCell::new(None) };
}

/// What the open page is doing, as JSON: `page`, `running`, `error`,
/// `renders`, `log` (what the lab's browser tests read).
#[wasm_bindgen]
#[must_use]
pub fn lab_status() -> String {
    OPEN.with(|open| {
        open.borrow().as_ref().map_or_else(
            || "null".to_owned(),
            |(name, page)| {
                json!({
                    "page": name,
                    "running": page.running(),
                    "error": page.error(),
                    "renders": page.renders(),
                    "log": page.log(),
                })
                .to_string()
            },
        )
    })
}

/// A save of the open page's file: `text` in place of its entry's.
#[wasm_bindgen]
pub fn lab_reload(text: String) {
    OPEN.with(|open| {
        if let Some((name, page)) = open.borrow().as_ref()
            && let Some((mut folder, _)) = self::folder(name)
        {
            folder.files.insert(folder.entry.clone(), text);
            page.reload(&folder);
        }
    });
}

fn log_line(text: &str) {
    if let Some(log) = by_id("log") {
        let had = log.text_content().unwrap_or_default();
        log.set_text_content(Some(&format!("{text}\n{had}")));
    }
}

/// Opens the page `?page=` names (the Board by default) in `#pane`.
fn open_page(pages: &WebPages, list: &ActionList) -> Result<(), String> {
    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let name = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("page="))
        .unwrap_or("board")
        .to_owned();
    let (folder, manifest) = folder(&name).ok_or_else(|| format!("no page {name}"))?;
    let manifest = Manifest::parse(manifest).map_err(|e| e.to_string())?;
    let host = WebHost::new(manifest, list.clone());
    let pane = by_id("pane").ok_or("no #pane")?;
    let opened = pages.open(&folder, host, &pane);
    opened.on_log(|level, text| log_line(&format!("{level} [page] {text}")));
    OPEN.with(|open| *open.borrow_mut() = Some((name, opened)));
    Ok(())
}

fn doc() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn by_id(id: &str) -> Option<web_sys::Element> {
    doc()?.get_element_by_id(id)
}

fn say(id: &str, text: &str) {
    if let Some(el) = by_id(id) {
        el.set_text_content(Some(text));
    }
}

/// The page's start.
#[wasm_bindgen(start)]
pub fn main() {
    spawn_local(async {
        if let Err(why) = run().await {
            say("status", &format!("it did not start: {why}"));
        }
    });
    heartbeat();
}

async fn run() -> Result<(), String> {
    let manifest = Manifest::parse(MANIFEST).map_err(|e| e.to_string())?;
    let list = ActionList::new();
    let themes = ActionInfo {
        id: "THEME_LIST".to_owned(),
        display_name: "Themes".to_owned(),
        description: "The page's themes.".to_owned(),
        category: "Theme".to_owned(),
        group: String::new(),
        shortcut: String::new(),
        effect: Effect::ReadOnly,
        params_schema: None,
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    };
    list.register_forwarded(
        &themes,
        Arc::new(|_: &[u8]| {
            let out: ActionFuture = Box::pin(async { Ok(json!(THEMES).to_string().into_bytes()) });
            Ok(out)
        }),
    )
    .map_err(|e| e.to_string())?;
    let board = Manifest::parse(BOARD).map_err(|e| e.to_string())?;
    board::register(&list, &board).map_err(|e| e.to_string())?;
    let pages = WebPages::new();
    pages.set_theme(theme());
    let port = web_sys::window()
        .and_then(|w| w.location().port().ok())
        .unwrap_or_default();
    pages.set_shared("lab.port", &json!(port));
    pages.on_notify(|n| {
        say("status", &format!("{}: {}", n.kind, n.text));
    });
    if let Err(why) = open_page(&pages, &list) {
        say("status", &format!("the page did not open: {why}"));
    }
    // The pages' app lives as long as the lab.
    std::mem::forget(pages);
    let host = WebHost::new(manifest, list.clone());
    host.on_log(|line| {
        log_line(&format!("{} [{}] {}", line.level, line.source, line.text));
    });
    let component = WebComponent::start(COMPONENT, host, Options::default()).await?;
    component
        .register_actions(&list)
        .map_err(|e| e.to_string())?;
    if let Some(pane) = by_id("counter") {
        let _pane = component.mount("counter", &pane);
    }
    say("status", "running");
    buttons(&list, &component);
    report(component);
    Ok(())
}

/// A button for each of its actions on the page's list.
fn buttons(list: &ActionList, component: &WebComponent) {
    let Some(at) = by_id("actions") else { return };
    let Some(doc) = doc() else { return };
    for id in component.actions() {
        let Some(info) = list.info(&id) else { continue };
        let Ok(button) = doc.create_element("button") else {
            continue;
        };
        let _ = button.set_attribute("data-action", &id);
        button.set_text_content(Some(&info.display_name));
        let list = list.clone();
        let click = Closure::<dyn FnMut()>::new(move || {
            let list = list.clone();
            let id = id.clone();
            spawn_local(async move {
                let said = list.run_json_async(&id, b"").await;
                let text = match said {
                    Ok(out) => format!("{id}: {}", String::from_utf8_lossy(&out)),
                    Err(e) => format!("{id}: {e}"),
                };
                say("status", &text);
            });
        });
        let _ = button.add_event_listener_with_callback("click", click.as_ref().unchecked_ref());
        click.forget();
        let _ = at.append_child(&button);
    }
}

/// What the component runs, every half second.
fn report(component: WebComponent) {
    let tick = Closure::<dyn FnMut()>::new(move || {
        let r = component.report();
        say(
            "report",
            &serde_json::to_string_pretty(&r).unwrap_or_default(),
        );
    });
    if let Some(w) = web_sys::window() {
        let _ = w.set_interval_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            500,
        );
    }
    tick.forget();
}

/// Each animation frame's gap: the longest of the last second, shown, so
/// a page kept waiting would say so.
fn heartbeat() {
    let last = Rc::new(Cell::new(0.0_f64));
    let worst = Rc::new(Cell::new(0.0_f64));
    let since = Rc::new(Cell::new(0.0_f64));
    let frames = Rc::new(Cell::new(0_u32));
    let f: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&f);
    *f.borrow_mut() = Some(Closure::new(move |now: f64| {
        let gap = now - last.get();
        if last.get() > 0.0 && gap > worst.get() {
            worst.set(gap);
        }
        last.set(now);
        frames.set(frames.get().saturating_add(1));
        if now - since.get() >= 1000.0 {
            say(
                "beat",
                &format!(
                    "{} frames/s, longest gap {:.0} ms",
                    frames.get(),
                    worst.get()
                ),
            );
            since.set(now);
            frames.set(0);
            worst.set(0.0);
        }
        if let (Some(w), Some(cb)) = (web_sys::window(), again.borrow().as_ref()) {
            let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    if let (Some(w), Some(cb)) = (web_sys::window(), f.borrow().as_ref()) {
        let _ = w.request_animation_frame(cb.as_ref().unchecked_ref());
    }
}
