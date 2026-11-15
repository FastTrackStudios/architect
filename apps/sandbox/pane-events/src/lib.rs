//! The browser applier's events in a real browser (#720 H4): `web::Pane`
//! over a button that listens for `click`, `contextmenu`, `pointerdown`
//! and `wheel`, each event it sends recorded on `window.paneEvents` as
//! `"<name>:<kind>"`. A browser sends `click` and `contextmenu` as
//! `PointerEvent`s; they must arrive as mouse data, or a guest's `onclick`
//! panics.
//!
//! Built and driven by `features/extensions/web/test/rust-pane.test.mjs`
//! (headless Firefox): `cargo build -p architect-sandbox-pane-events
//! --target wasm32-unknown-unknown`, then `wasm-bindgen --target web`. On
//! any other target it is empty.

#[cfg(target_arch = "wasm32")]
mod page {
    use architect_extensions::web::Pane;
    use architect_extensions::wire::{Edit, EventData, Value, ViewEvent};
    use wasm_bindgen::prelude::*;

    fn kind(data: &EventData) -> &'static str {
        match data {
            EventData::Mouse(_) => "mouse",
            EventData::Pointer(_) => "pointer",
            EventData::Wheel(_) => "wheel",
            EventData::Touch(_) => "touch",
            EventData::Key(_) => "key",
            EventData::Form(_) => "form",
            EventData::Focus => "focus",
            EventData::Scroll(_) => "scroll",
            EventData::Composition(_) => "composition",
        }
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let window = web_sys::window().ok_or("no window")?;
        let document = window.document().ok_or("no document")?;
        let root = document.create_element("div")?;
        document.body().ok_or("no body")?.append_child(&root)?;
        let seen = js_sys::Array::new();
        js_sys::Reflect::set(&window, &"paneEvents".into(), &seen)?;
        let mut pane = Pane::new(root, move |e: ViewEvent| {
            seen.push(&format!("{}:{}", e.name, kind(&e.data)).into());
        });
        let mut edits = vec![
            Edit::PushId { id: 0 },
            Edit::CreateElement {
                tag: "button".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 1 },
            Edit::SetAttribute {
                name: "id".to_owned(),
                namespace: None,
                value: Value::Text("b".to_owned()),
            },
        ];
        for name in ["click", "contextmenu", "pointerdown", "wheel"] {
            edits.push(Edit::Listen {
                name: name.to_owned(),
            });
        }
        edits.extend([Edit::AppendChildren { m: 1 }, Edit::Pop]);
        pane.apply(edits).map_err(|e| JsValue::from_str(&e))?;
        // The pane lives as long as the page.
        std::mem::forget(pane);
        js_sys::Reflect::set(&window, &"paneReady".into(), &true.into())?;
        Ok(())
    }
}
