//! An agent's UI in a browser: the lab's web host for `architect-a2ui`.
//!
//! On start an agent renders a surface of more than ten catalog components
//! with `A2UI_RENDER` (src/surface.json), drawn by the renderer with
//! architect-ui on the DOM. Its fields write to the surface's Loro
//! document; its Save button runs `LAB_SAVE`, a write, so it waits for the
//! person's yes in the confirmation sheet. The page's own Tailwind sheet
//! (app.css) styles the page around it, next to the renderer's
//! `THEME_CSS` and `UTILITIES_CSS`.
//!
//! The page reaches the Action List as an agent would over MCP:
//! `a2ui(id, argsJson)`, exported to JavaScript, runs an action as the
//! agent and answers its JSON (or rejects with the error). A press through
//! it still waits for the person. The browser tests (e2e/) patch the
//! surface live that way.
//!
//! ```text
//! just lab-web        # build it into apps/lab/web/dist and serve it
//! just lab-web-e2e    # build it and run the browser tests
//! ```

#[cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        dead_code,
        reason = "drawn only in a browser; its setup is tested here"
    )
)]
mod host;

#[cfg(target_arch = "wasm32")]
fn main() {
    dioxus::launch(host::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "architect-lab-web runs in a browser: `just lab-web` builds it for wasm32 and serves it"
    );
    std::process::ExitCode::FAILURE
}

/// The page's door to the Action List: runs action `id` with `args` (JSON)
/// as the agent, and answers its JSON.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn a2ui(id: String, args: String) -> js_sys::Promise {
    use wasm_bindgen::JsValue;
    let list = host::list();
    wasm_bindgen_futures::future_to_promise(async move {
        let list = list.ok_or_else(|| JsValue::from_str("the app is not up yet"))?;
        list.run_json_async_with(&host::agent(), &id, args.as_bytes())
            .await
            .map(|out| JsValue::from_str(&String::from_utf8_lossy(&out)))
            .map_err(|e| JsValue::from_str(&e.to_string()))
    })
}
