//! The page's document without `eval`: the app's page has a
//! Content-Security-Policy with no `'unsafe-eval'` (`apps/web/app/index.html`),
//! so dioxus-web's `document::eval` (a `new Function`) cannot run there.
//!
//! [`SafeDocument`] wraps the document dioxus-web gives and answers what
//! the app's components evaluate without JavaScript text: the head's
//! elements and the title through the DOM, a known script by what it does
//! (blurring the focused field, `architect_shell`'s typing bar), and any
//! other script as a document with no scripts does (`NoOpDocument`: the
//! caller hears it is unsupported, as on Blitz). The keyboard feed the
//! shell asks a browser for by script is fed natively instead
//! ([`use_keyboard_feed`]).

use std::rc::Rc;

use dioxus::document::{
    Document, Eval, LinkProps, MetaProps, NoOpDocument, ScriptProps, StyleProps,
};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The scripts answered natively: blurring the focused element.
const BLUR: &[&str] = &["document.activeElement && document.activeElement.blur();"];

/// The page's document, its scripts answered without `eval`.
pub struct SafeDocument {
    inner: Rc<dyn Document>,
}

fn doc() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

impl Document for SafeDocument {
    fn eval(&self, js: String) -> Eval {
        if BLUR.contains(&js.trim()) {
            if let Some(el) = doc()
                .and_then(|d| d.active_element())
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = el.blur();
            }
        }
        NoOpDocument.eval(js)
    }

    fn set_title(&self, title: String) {
        if let Some(d) = doc() {
            d.set_title(&title);
        }
    }

    fn create_head_element(
        &self,
        name: &str,
        attributes: &[(&str, String)],
        contents: Option<String>,
    ) {
        let Some(d) = doc() else { return };
        let Ok(el) = d.create_element(name) else {
            return;
        };
        for (k, v) in attributes {
            let _ = el.set_attribute(k, v);
        }
        if let Some(text) = contents {
            el.set_text_content(Some(&text));
        }
        if let Some(head) = d.head() {
            let _ = head.append_child(&el);
        }
    }

    fn create_meta(&self, props: MetaProps) {
        self.inner.create_meta(props);
    }

    fn create_script(&self, props: ScriptProps) {
        self.inner.create_script(props);
    }

    fn create_style(&self, props: StyleProps) {
        self.inner.create_style(props);
    }

    fn create_link(&self, props: LinkProps) {
        self.inner.create_link(props);
    }

    fn create_head_component(&self) -> bool {
        self.inner.create_head_component()
    }
}

/// Puts [`SafeDocument`] in context over dioxus-web's, for everything
/// under the caller.
pub fn use_safe_document() {
    use_context_provider(|| {
        let inner = consume_context::<Rc<dyn Document>>();
        let safe: Rc<dyn Document> = Rc::new(SafeDocument { inner });
        safe
    });
}

/// Whether the focused element takes text (what the shell's script asks).
fn typing() -> bool {
    let Some(el) = doc().and_then(|d| d.active_element()) else {
        return false;
    };
    if el
        .dyn_ref::<web_sys::HtmlElement>()
        .is_some_and(web_sys::HtmlElement::is_content_editable)
    {
        return true;
    }
    match el.tag_name().as_str() {
        "TEXTAREA" => true,
        "INPUT" => !matches!(
            el.get_attribute("type").unwrap_or_default().as_str(),
            "button" | "checkbox" | "radio" | "range" | "submit" | "reset" | "file" | "color"
        ),
        _ => false,
    }
}

/// The on-screen keyboard's height and whether a field has the focus,
/// told to the shell (`architect_shell::set_keyboard_height`,
/// `set_typing`), what its script (`keyboard.rs` `WEB_KEYBOARD`) does in a
/// page that allows one: from `visualViewport` and the focus.
pub fn use_keyboard_feed() {
    use_hook(|| {
        let Some(w) = web_sys::window() else { return };
        let send = Closure::<dyn Fn()>::new(move || {
            let Some(w) = web_sys::window() else { return };
            let inner = w
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let kb = w
                .visual_viewport()
                .map_or(0.0, |vv| (inner - vv.height() - vv.offset_top()).max(0.0));
            architect_shell::set_keyboard_height(kb);
            architect_shell::set_typing(typing());
        });
        let f: &js_sys::Function = send.as_ref().unchecked_ref();
        if let Some(vv) = w.visual_viewport() {
            let _ = vv.add_event_listener_with_callback("resize", f);
            let _ = vv.add_event_listener_with_callback("scroll", f);
        }
        if let Some(d) = w.document() {
            let _ = d.add_event_listener_with_callback("focusin", f);
            // After the focus has moved on.
            let later = f.clone();
            let out = Closure::<dyn Fn()>::new(move || {
                if let Some(w) = web_sys::window() {
                    let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&later, 0);
                }
            });
            let _ = d.add_event_listener_with_callback("focusout", out.as_ref().unchecked_ref());
            out.forget();
        }
        let _ = f.call0(&JsValue::NULL);
        send.forget();
    });
}
