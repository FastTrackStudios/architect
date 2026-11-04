//! The lab in a browser: the WASI Counter (`apps/lab/wasi-counter`), the
//! same component the desktop lab runs with Wasmtime, transpiled by jco
//! and run by the browser's WebAssembly (`architect_extensions::web`).
//!
//! The page puts the component's actions on its own Action List (beside a
//! `THEME_LIST` of its own, what the counter's "Ask the app" runs through
//! the host service), mounts its `counter` view, and shows how the page
//! keeps up: each animation frame's gap, the longest of the last second,
//! while the primes job runs in a Web Worker. Built and served by
//! `just lab-web` (docs/content/architecture/extensions.md, "On the web").

#![cfg(target_arch = "wasm32")]
// Nothing is `Send` in a browser.
#![allow(clippy::future_not_send)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionFuture, ActionInfo, ActionList, Effect};
use architect_extensions::Manifest;
use architect_extensions::web::{Options, WebComponent, WebHost};
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// Where `just lab-web` puts the transpiled component, beside the page.
const COMPONENT: &str = "wasi-counter/architect_lab_wasi_counter.js";
const MANIFEST: &str = include_str!("../../wasi-counter/extension.toml");

/// The page's own themes, as the app's `THEME_LIST` answers.
const THEMES: [&str; 3] = ["catppuccin-mocha", "catppuccin-latte", "zinc"];

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
    let host = WebHost::new(manifest, list.clone());
    host.on_log(|line| {
        if let Some(log) = by_id("log") {
            let had = log.text_content().unwrap_or_default();
            log.set_text_content(Some(&format!(
                "{} [{}] {}\n{had}",
                line.level, line.source, line.text
            )));
        }
    });
    let component = WebComponent::start(COMPONENT, host, Options::default()).await?;
    component
        .register_actions(&list)
        .map_err(|e| e.to_string())?;
    if let Some(pane) = by_id("pane") {
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
