//! The view probe: a linked extension whose views stand where an island's
//! did, for the browser tests (`apps/web/test/app-linked-views.test.mjs`,
//! `apps/web/test/app-guest-linked.test.mjs`).
//!
//! Loaded only when the page asks (`probe=views`), in the demo and for a
//! share link's guest, never signed in (`super::extensions`). It is
//! compiled into the app as a distribution's extensions are, so what the
//! tests see is the path a distribution's own views take:
//!
//! - `note` draws notes of type `probe-note` as the note's page (its
//!   embed says `fill`), `card` notes of type `probe-card` at their head;
//!   each shows what it was handed (`use_embed`: the claim, the source,
//!   the props) and sets and plays its own source on the playhead;
//! - `home` is the demo's home with `probe=views,home`, showing the
//!   `{home: …}` it was handed;
//! - `more` is the view a favourite's tab opens;
//! - `window.architectViewProbe(request)` makes one host-service call as
//!   the probe, through its own handle, as its views would.
//!
//! With `probe=views,silent` its manifest declares no read
//! (`view_probe_silent.toml`): its views are handed no note.
//!
//! It gives a guest nothing the link did not: its views show what the app
//! handed them, and its host is the guest's, which refuses every vault
//! request.

use architect_extensions::linked::{Builder, Extension, Handle, use_embed};
use architect_playhead::Caller;
use dioxus::prelude::*;
use serde_json::{Value, json};

/// Its id.
pub const ID: &str = "web.views";
/// Its home view.
pub const HOME: &str = "home";

/// The probe: with what it reads, or declaring no read.
pub struct ViewProbe {
    silent: bool,
}

/// It reads notes and their files.
pub static READING: ViewProbe = ViewProbe { silent: false };
/// It declares no read.
pub static SILENT: ViewProbe = ViewProbe { silent: true };

/// The handle of the copy the host built.
static HANDLE: std::sync::Mutex<Option<Handle>> = std::sync::Mutex::new(None);

fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

impl Extension for ViewProbe {
    fn manifest(&self) -> &'static str {
        if self.silent {
            include_str!("view_probe_silent.toml")
        } else {
            include_str!("view_probe.toml")
        }
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ext.handle());
        ext.view("note", NoteProbe)
            .view("card", NoteProbe)
            .view(HOME, HomeProbe)
            .view("more", MoreProbe);
    }
}

/// One host-service call as the probe, `request` a `HostRequest` as JSON:
/// `{"ok": answer}` or `{"error": why}`, as JSON.
async fn call(request: String) -> String {
    let answer = match (
        handle(),
        serde_json::from_str::<architect_extensions::hostcall::HostRequest>(&request),
    ) {
        (Some(handle), Ok(request)) => match handle.host(request).await {
            Ok(answer) => json!({ "ok": answer }),
            Err(e) => json!({ "error": e.to_string() }),
        },
        (None, _) => json!({ "error": "the probe is not running" }),
        (_, Err(e)) => json!({ "error": format!("not a host request: {e}") }),
    };
    answer.to_string()
}

/// `window.architectViewProbe(request)`: [`call`], as a promise of its
/// JSON text.
pub fn install() {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    let ask = Closure::<dyn Fn(String) -> js_sys::Promise>::new(move |request: String| {
        wasm_bindgen_futures::future_to_promise(async move { Ok(call(request).await.into()) })
    });
    let _ = js_sys::Reflect::set(
        &window,
        &"architectViewProbe".into(),
        ask.as_ref().unchecked_ref(),
    );
    ask.forget();
}

/// Sets the probe's own source on the playhead and plays it, as the
/// extension it is.
fn play() {
    let me = Caller::Extension(ID.to_owned());
    let set = architect_playhead::ask(
        &me,
        "PLAYHEAD_SET",
        &json!({ "name": "Probe", "queue": [{ "id": "probe#1", "title": "Probe tone", "duration": 60 }] }),
    );
    if set.is_ok() {
        let _ = architect_playhead::ask(&me, "PLAYHEAD_PLAY", &json!({}));
    }
}

/// What the app handed this view, as text: the claim, the source and the
/// props (JSON); empty where it was handed nothing.
fn handed() -> (String, String, String) {
    use_embed().map_or_else(
        || (String::new(), String::new(), Value::Null.to_string()),
        |input| (input.claim, input.source, input.props.to_string()),
    )
}

/// A note's view: what it was handed, and the playhead's state of its own
/// source.
#[component]
fn NoteProbe() -> Element {
    let (claim, source, props) = handed();
    let now = architect_playhead::ui::use_playhead();
    let mine = now().active.filter(|a| a.source == ID);
    let playing = mine.as_ref().is_some_and(|a| a.playing);
    let tracks = mine.as_ref().map_or(0, |a| a.tracks);
    rsx! {
        div { "data-testid": "probe-note", "data-claim": "{claim}", "data-props": "{props}",
            "data-playing": "{playing}", "data-tracks": "{tracks}",
            style: "padding: 12px 16px; font-size: 14px; line-height: 1.5;",
            div { style: "font-weight: 600;", "Probe view ({claim})" }
            pre { "data-testid": "probe-source", style: "white-space: pre-wrap; margin: 8px 0;", "{source}" }
            button { r#type: "button", "data-testid": "probe-play", onclick: move |_| play(),
                if playing { "Playing" } else { "Play" }
            }
        }
    }
}

/// The home: what it was handed, and a way to its first row's note.
#[component]
fn HomeProbe() -> Element {
    let input = use_embed();
    let (claim, _, props) = handed();
    let first = input
        .as_ref()
        .and_then(|i| i.props.pointer("/home/orgs/0/rows/0/open"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let rows = input
        .as_ref()
        .and_then(|i| i.props.pointer("/home/orgs/0/rows"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    rsx! {
        div { "data-testid": "probe-home", "data-claim": "{claim}", "data-props": "{props}", "data-rows": "{rows}",
            style: "padding: 12px 16px; font-size: 14px;",
            div { style: "font-weight: 600;", "Probe home ({rows} notes)" }
            if let Some(path) = first {
                button { r#type: "button", "data-testid": "probe-home-open",
                    onclick: move |_| {
                        let path = path.clone();
                        spawn(async move {
                            let _ = call(json!({ "op": "open", "path": path }).to_string()).await;
                        });
                    },
                    "Open the first note"
                }
            }
        }
    }
}

/// The view a favourite's tab opens.
#[component]
fn MoreProbe() -> Element {
    let (claim, _, props) = handed();
    rsx! {
        div { "data-testid": "probe-more", "data-claim": "{claim}", "data-props": "{props}",
            style: "padding: 12px 16px; font-size: 14px;",
            "Probe more"
        }
    }
}
