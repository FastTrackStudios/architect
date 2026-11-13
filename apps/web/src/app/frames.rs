//! The extension frame host (#495, route A): an extension's view, from
//! its island's web bundle, in a sandboxed frame of its own beside a note.
//!
//! The node lists its islands at `/_ext/islands.json`; the app believes a
//! listed bundle only whole (`crate::ext_host::islands`), fetches each of
//! its files without credentials and checks it against its SHA-256 before
//! any frame gets it, and runs each view in `<iframe sandbox="allow-scripts">`
//! on the node's web view document (`features/sites/assets/webview.js`
//! under `architect_sites::island::webview_headers`: an opaque origin, no
//! network, its one script by hash). The protocol and its decisions are
//! `crate::ext_host`'s; this module is the DOM: the frame, its port, its
//! timers, and what the app shows while it loads or once it died.
//!
//! Nothing of the app reaches a frame but the bundle's bytes, the theme's
//! colours and the view's props: no session token, no cookie (the frame's
//! origin is opaque), no handle on the app's document. What a frame may
//! ask is a playhead intent (`ext_host::INTENTS`), rate-limited, checked
//! here; anything else it says is dropped.
//!
//! In the demo (`/app/?demo`) the only requests are GETs of the node's
//! public `/_ext/` files, without credentials; a host with no node (a
//! static bundle alone) lists no islands and the app's own controls stay.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt as _;
use futures_util::StreamExt as _;
use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    HtmlIFrameElement, MessageChannel, MessageEvent, MessagePort, RequestCache, RequestCredentials,
    RequestInit, RequestRedirect, Response,
};

use crate::ext_host::{self, Beat, Bucket, Intent, Island, Said, Tick};

fn window() -> Option<web_sys::Window> {
    web_sys::window()
}

fn now() -> f64 {
    window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn hidden() -> bool {
    window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

fn js(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(e, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| "the request failed".to_owned())
}

/// GET `url` on this origin: no credentials, no redirect followed.
async fn get(url: &str, cache: RequestCache) -> Result<Vec<u8>, String> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::Omit);
    init.set_redirect(RequestRedirect::Error);
    init.set_cache(cache);
    let w = window().ok_or("no window")?;
    let response: Response = JsFuture::from(w.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| js(&e))?
        .dyn_into()
        .map_err(|e| js(&e))?;
    if !response.ok() {
        return Err(format!("{url} answered {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(|e| js(&e))?)
        .await
        .map_err(|e| js(&e))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// An island's bundle, every file checked against its digest.
#[derive(Debug)]
pub struct Bundle {
    /// The island it is.
    pub island: Island,
    files: BTreeMap<String, Vec<u8>>,
    sheet: Option<String>,
    /// How long fetching and checking it took, ms.
    pub fetched_ms: f64,
}

impl PartialEq for Bundle {
    fn eq(&self, other: &Self) -> bool {
        self.island == other.island
    }
}

thread_local! {
    /// The listing, read once a page.
    static LISTING: RefCell<Option<Rc<Vec<Island>>>> = const { RefCell::new(None) };
    /// Each island's bundle once checked, by its folder.
    static BUNDLES: RefCell<HashMap<String, Rc<Bundle>>> = RefCell::new(HashMap::new());
    /// The views whose frame died on this page: never booted again here.
    static DEAD: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    /// Every frame's asks, together.
    static BUCKET: RefCell<Option<Bucket>> = const { RefCell::new(None) };
}

/// The node's islands this app believes (none without a node).
async fn listing() -> Rc<Vec<Island>> {
    if let Some(l) = LISTING.with(|l| l.borrow().clone()) {
        return l;
    }
    let list = get(ext_host::LISTING, RequestCache::NoCache)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|v| ext_host::islands(&v))
        .unwrap_or_default();
    let list = Rc::new(list);
    LISTING.with(|l| *l.borrow_mut() = Some(list.clone()));
    list
}

/// `island`'s bundle, fetched and checked (once a page).
async fn bundle(island: &Island) -> Result<Rc<Bundle>, String> {
    if let Some(b) = BUNDLES.with(|b| b.borrow().get(&island.folder).cloned()) {
        return Ok(b);
    }
    let started = now();
    let mut files = BTreeMap::new();
    for (name, digest) in &island.files {
        let bytes = get(&format!("{}{name}", island.folder), RequestCache::Default).await?;
        ext_host::check(name, &bytes, digest)?;
        files.insert(name.clone(), bytes);
    }
    // Its sheet, pinned the same way; without it the views still draw.
    let mut sheet = None;
    if let Some((name, digest)) = &island.sheet {
        if let Ok(bytes) = get(&format!("{}{name}", island.folder), RequestCache::Default).await {
            if ext_host::check(name, &bytes, digest).is_ok() {
                sheet = String::from_utf8(bytes).ok();
            }
        }
    }
    let b = Rc::new(Bundle {
        island: island.clone(),
        files,
        sheet,
        fetched_ms: now() - started,
    });
    BUNDLES.with(|m| m.borrow_mut().insert(island.folder.clone(), b.clone()));
    Ok(b)
}

/// The first listed island with a web bundle, fetched and checked: `None`
/// while it loads, `Some(Err)` when there is none or it failed.
pub fn use_island() -> Resource<Result<Rc<Bundle>, String>> {
    use_resource(|| async {
        let list = listing().await;
        let island = list
            .first()
            .ok_or_else(|| "no island on this node".to_owned())?;
        bundle(island).await
    })
}

/// The colours a frame is handed (`--name: value`) and its scheme.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameTheme {
    /// Its variables (checked by `ext_host::theme_vars` before use).
    pub vars: Vec<(String, String)>,
    /// `dark` or `light`.
    pub dark: bool,
}

/// How a frame is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameState {
    /// Created, not ready yet.
    Loading,
    /// Its view is drawn.
    Ready,
    /// Taken out, for good on this page: why.
    Gone(String),
}

enum Event {
    Ready,
    Died(String),
    Ask(Intent),
}

/// The DOM half of one frame: its element, its port and its timers, and
/// the protocol's state.
struct Frame {
    key: String,
    bundle: Rc<Bundle>,
    view: String,
    fill: bool,
    theme: FrameTheme,
    holder: Option<web_sys::Element>,
    iframe: Option<HtmlIFrameElement>,
    port: Option<MessagePort>,
    loads: u32,
    hello: bool,
    ready: bool,
    dead: bool,
    beat: Beat,
    created: f64,
    rtts: Vec<f64>,
    props: Option<String>,
    duration: f64,
    ready_by: Option<i32>,
    beating: Option<i32>,
    events: Option<Coroutine<Event>>,
    // Kept alive while the frame is; dropped only on unmount, never from
    // inside one of them.
    on_load: Option<Closure<dyn FnMut()>>,
    on_message: Option<Closure<dyn FnMut(MessageEvent)>>,
    on_late: Option<Closure<dyn FnMut()>>,
    on_beat: Option<Closure<dyn FnMut()>>,
    on_visible: Option<Closure<dyn FnMut()>>,
}

type Shared = Rc<RefCell<Frame>>;

fn obj(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = js_sys::Object::new();
    for (k, v) in pairs {
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str(k), v);
    }
    o.into()
}

impl Frame {
    fn post(&self, message: &JsValue) {
        if let (Some(port), false) = (&self.port, self.dead) {
            let _ = port.post_message(message);
        }
    }

    fn mark(&self, name: &str, value: &str) {
        if let Some(h) = &self.holder {
            let _ = h.set_attribute(name, value);
        }
    }

    fn clear_timers(&mut self) {
        if let Some(w) = window() {
            if let Some(t) = self.ready_by.take() {
                w.clear_timeout_with_handle(t);
            }
            if let Some(t) = self.beating.take() {
                w.clear_interval_with_handle(t);
            }
        }
    }

    /// Taken out for good: its element gone, its port closed, the app's
    /// own control back.
    fn die(&mut self, why: &str) {
        if self.dead {
            return;
        }
        self.dead = true;
        self.clear_timers();
        if let Some(port) = self.port.take() {
            port.set_onmessage(None);
            port.close();
        }
        if let Some(f) = self.iframe.take() {
            f.remove();
        }
        DEAD.with(|d| d.borrow_mut().insert(self.key.clone()));
        self.mark("data-ext-state", "gone");
        self.mark("data-ext-why", why);
        if let Some(events) = self.events {
            events.send(Event::Died(why.to_owned()));
        }
    }

    fn send_props(&self) {
        if !self.ready {
            return;
        }
        if let Some(p) = self
            .props
            .as_deref()
            .and_then(|p| js_sys::JSON::parse(p).ok())
        {
            self.post(&obj(&[("t", "props".into()), ("props", p)]));
        }
    }

    fn boot(&self) {
        let vars = js_sys::Object::new();
        for (k, v) in ext_host::theme_vars(
            self.theme
                .vars
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        ) {
            let _ = js_sys::Reflect::set(&vars, &JsValue::from_str(&k), &JsValue::from_str(&v));
        }
        let scheme = if self.theme.dark { "dark" } else { "light" };
        self.post(&obj(&[
            ("t", "theme".into()),
            ("vars", vars.into()),
            ("scheme", scheme.into()),
        ]));
        // Copies: each frame gets bytes of its own, the checked ones kept.
        let files = js_sys::Object::new();
        for (name, bytes) in &self.bundle.files {
            let copy = js_sys::Uint8Array::from(bytes.as_slice()).buffer();
            let _ = js_sys::Reflect::set(&files, &JsValue::from_str(name), &copy);
        }
        self.post(&obj(&[
            ("t", "boot".into()),
            ("entry", self.bundle.island.entry.as_str().into()),
            ("view", self.view.as_str().into()),
            ("files", files.into()),
            (
                "sheet",
                self.bundle
                    .sheet
                    .as_deref()
                    .map_or(JsValue::NULL, JsValue::from_str),
            ),
        ]));
    }
}

/// What a frame said, acted on.
fn heard(shared: &Shared, data: &JsValue) {
    let Some(text) = js_sys::JSON::stringify(data)
        .ok()
        .and_then(|s| s.as_string())
    else {
        return;
    };
    let mut f = shared.borrow_mut();
    if f.dead {
        return;
    }
    match ext_host::said(&text) {
        Said::Hello => {
            // Answered once: its files are copied and booted once.
            if f.hello {
                return;
            }
            f.hello = true;
            let at = now() - f.created;
            f.mark("data-ext-hello-ms", &format!("{at:.1}"));
            f.boot();
        }
        Said::Ready => {
            if f.ready || !f.hello {
                return;
            }
            f.ready = true;
            if let (Some(w), Some(t)) = (window(), f.ready_by.take()) {
                w.clear_timeout_with_handle(t);
            }
            let at = now() - f.created;
            f.mark("data-ext-mount-ms", &format!("{at:.1}"));
            f.mark("data-ext-state", "ready");
            f.send_props();
            if let (Some(w), Some(beat)) = (window(), f.on_beat.as_ref()) {
                f.beating = w
                    .set_interval_with_callback_and_timeout_and_arguments_0(
                        beat.as_ref().unchecked_ref(),
                        i32::try_from(ext_host::BEAT_MS).unwrap_or(2000),
                    )
                    .ok();
            }
            if let Some(events) = f.events {
                events.send(Event::Ready);
            }
        }
        Said::Pong(n) => {
            if let Some(rtt) = f.beat.pong(n, now()) {
                f.rtts.push(rtt);
                if f.rtts.len() > 60 {
                    f.rtts.remove(0);
                }
                let list = f
                    .rtts
                    .iter()
                    .map(|r| format!("{r:.2}"))
                    .collect::<Vec<_>>()
                    .join(",");
                f.mark("data-ext-rtt", &list);
            }
        }
        Said::Size(h) => {
            // A frame the app sizes: what it says it draws is not asked.
            if f.fill {
                return;
            }
            if let Some(el) = &f.iframe {
                let _ = el.style().set_property("height", &format!("{h}px"));
            }
        }
        Said::Ask(action, args) => {
            let allowed = BUCKET.with(|b| {
                let mut b = b.borrow_mut();
                b.get_or_insert_with(|| Bucket::new(now())).take(now())
            });
            if !allowed {
                f.mark("data-ext-refused", "too many requests");
                return;
            }
            if let (Some(i), Some(events)) =
                (ext_host::intent(&action, &args, f.duration), f.events)
            {
                events.send(Event::Ask(i));
            }
        }
        Said::Error(why) => f.die(&why),
        Said::Nothing => {}
    }
}

fn loaded(shared: &Shared) {
    let mut f = shared.borrow_mut();
    f.loads = f.loads.saturating_add(1);
    // Its one document, loaded once: any other load is the frame gone
    // somewhere else.
    if f.loads > 1 {
        f.die("it left its document");
        return;
    }
    if f.dead || f.port.is_some() {
        return;
    }
    let Some(target) = f
        .iframe
        .as_ref()
        .and_then(HtmlIFrameElement::content_window)
    else {
        return;
    };
    let Ok(chan) = MessageChannel::new() else {
        return;
    };
    let port = chan.port1();
    if let Some(on) = f.on_message.as_ref() {
        port.set_onmessage(Some(on.as_ref().unchecked_ref()));
    }
    f.port = Some(port);
    let transfer = js_sys::Array::of1(&chan.port2());
    // The frame's origin is opaque: `*` is the only target it matches.
    let _ = target.post_message_with_transfer(&JsValue::from_str("architect-web"), "*", &transfer);
}

/// Builds the frame in `holder` and wires its closures.
fn start(shared: &Shared, holder: &web_sys::Element) {
    let Some(doc) = window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(el) = doc.create_element("iframe") else {
        return;
    };
    let Ok(iframe) = el.dyn_into::<HtmlIFrameElement>() else {
        return;
    };
    {
        let mut f = shared.borrow_mut();
        let _ = iframe.set_attribute("sandbox", "allow-scripts");
        let _ = iframe.set_attribute("title", &f.view);
        let _ = iframe.set_attribute("referrerpolicy", "no-referrer");
        let _ = iframe.set_attribute("data-ext-frame", &f.view);
        let height = if f.fill { "100%" } else { "0px" };
        let _ = iframe.set_attribute(
            "style",
            &format!("display: block; width: 100%; height: {height}; border: 0; background: transparent; color-scheme: normal;"),
        );
        let weak = Rc::downgrade(shared);
        f.on_message = Some(Closure::new(move |m: MessageEvent| {
            if let Some(s) = weak.upgrade() {
                heard(&s, &m.data());
            }
        }));
        let weak = Rc::downgrade(shared);
        f.on_load = Some(Closure::new(move || {
            if let Some(s) = weak.upgrade() {
                loaded(&s);
            }
        }));
        let weak = Rc::downgrade(shared);
        f.on_beat = Some(Closure::new(move || {
            let Some(s) = weak.upgrade() else { return };
            let mut f = s.borrow_mut();
            if f.dead || hidden() {
                return;
            }
            match f.beat.tick(now()) {
                Tick::Ping(n) => f.post(&obj(&[
                    ("t", "ping".into()),
                    (
                        "n",
                        JsValue::from_f64(f64::from(u32::try_from(n).unwrap_or(u32::MAX))),
                    ),
                ])),
                Tick::Wait => {}
                Tick::Dead => f.die("it stopped answering"),
            }
        }));
        // Not ready in time: gone, unless the page is hidden (its timers
        // and the frame's may stop then), when it waits for the page.
        let weak = Rc::downgrade(shared);
        f.on_late = Some(Closure::new(move || {
            let Some(s) = weak.upgrade() else { return };
            let mut f = s.borrow_mut();
            if f.ready || f.dead {
                return;
            }
            if hidden() {
                rearm(&mut f);
                return;
            }
            f.die("not ready in time");
        }));
        let weak = Rc::downgrade(shared);
        f.on_visible = Some(Closure::new(move || {
            let Some(s) = weak.upgrade() else { return };
            let mut f = s.borrow_mut();
            if hidden() || f.dead {
                return;
            }
            f.beat.shown(now());
            if !f.ready {
                rearm(&mut f);
            }
        }));
        if let Some(on) = f.on_load.as_ref() {
            let _ = iframe.add_event_listener_with_callback("load", on.as_ref().unchecked_ref());
        }
        if let Some(on) = f.on_visible.as_ref() {
            let _ = doc
                .add_event_listener_with_callback("visibilitychange", on.as_ref().unchecked_ref());
        }
        f.created = now();
        rearm(&mut f);
        let _ = iframe.set_attribute("src", &f.bundle.island.webview);
        f.mark("data-ext-state", "loading");
        f.holder = Some(holder.clone());
        f.iframe = Some(iframe.clone());
    }
    let _ = holder.append_child(&iframe);
}

fn rearm(f: &mut Frame) {
    let Some(w) = window() else { return };
    if let Some(t) = f.ready_by.take() {
        w.clear_timeout_with_handle(t);
    }
    if let Some(late) = f.on_late.as_ref() {
        f.ready_by = w
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                late.as_ref().unchecked_ref(),
                i32::try_from(ext_host::READY_MS).unwrap_or(15_000),
            )
            .ok();
    }
}

/// Unmounted: everything let go, from outside every closure.
fn stop(shared: &Shared) {
    let mut f = shared.borrow_mut();
    f.dead = true;
    f.clear_timers();
    if let Some(port) = f.port.take() {
        port.set_onmessage(None);
        port.close();
    }
    if let Some(el) = f.iframe.take() {
        if let Some(on) = f.on_load.as_ref() {
            let _ = el.remove_event_listener_with_callback("load", on.as_ref().unchecked_ref());
        }
        el.remove();
    }
    if let (Some(doc), Some(on)) = (window().and_then(|w| w.document()), f.on_visible.as_ref()) {
        let _ = doc
            .remove_event_listener_with_callback("visibilitychange", on.as_ref().unchecked_ref());
    }
    f.events = None;
    f.on_load = None;
    f.on_message = None;
    f.on_late = None;
    f.on_beat = None;
    f.on_visible = None;
}

/// View `view` of `bundle`'s island in a frame of its own; `fallback`
/// (the app's own control) shows until it is ready and for good if it
/// dies. `fill`: as tall as its box (else as tall as it says). Its asks,
/// checked and rate-limited, come out as `on_ask`.
#[component]
pub fn ExtensionFrame(
    bundle: Rc<Bundle>,
    view: String,
    #[props(default)] fill: bool,
    props: ReadSignal<Value>,
    duration: f64,
    theme: FrameTheme,
    on_ask: EventHandler<Intent>,
    fallback: Element,
) -> Element {
    let key = format!("{}/{view}", bundle.island.id);
    let mut state = use_signal(|| {
        if DEAD.with(|d| d.borrow().contains(&key)) {
            FrameState::Gone("it died on this page".to_owned())
        } else {
            FrameState::Loading
        }
    });
    let events = use_coroutine(move |mut rx: UnboundedReceiver<Event>| async move {
        while let Some(e) = rx.next().await {
            match e {
                Event::Ready => state.set(FrameState::Ready),
                Event::Died(why) => state.set(FrameState::Gone(why)),
                Event::Ask(i) => on_ask.call(i),
            }
        }
    });
    let shared: Shared = use_hook(|| {
        Rc::new(RefCell::new(Frame {
            key: key.clone(),
            bundle: bundle.clone(),
            view: view.clone(),
            fill,
            theme: theme.clone(),
            holder: None,
            iframe: None,
            port: None,
            loads: 0,
            hello: false,
            ready: false,
            dead: false,
            beat: Beat::default(),
            created: 0.0,
            rtts: Vec::new(),
            props: None,
            duration,
            ready_by: None,
            beating: None,
            events: Some(events),
            on_load: None,
            on_message: None,
            on_late: None,
            on_beat: None,
            on_visible: None,
        }))
    });
    // The props as they change, data only.
    {
        let shared = shared.clone();
        use_effect(move || {
            let text = props.read().to_string();
            let mut f = shared.borrow_mut();
            f.duration = duration;
            if f.props.as_deref() != Some(text.as_str()) {
                f.props = Some(text);
                f.send_props();
            }
        });
    }
    {
        let shared = shared.clone();
        use_drop(move || stop(&shared));
    }
    let gone = matches!(state(), FrameState::Gone(_));
    let ready = state() == FrameState::Ready;
    let label = match state() {
        FrameState::Loading => "loading",
        FrameState::Ready => "ready",
        FrameState::Gone(_) => "gone",
    };
    let shown = if ready { "block" } else { "none" };
    let height = if fill { "height: 100%;" } else { "" };
    rsx! {
        div {
            "data-testid": "ext-frame",
            "data-ext-view": "{view}",
            "data-ext-island": "{bundle.island.id}",
            "data-ext-fetch-ms": "{bundle.fetched_ms:.1}",
            "data-ext-shown": "{label}",
            style: "display: {shown}; {height}",
            onmounted: {
                move |e: MountedEvent| {
                    if gone || shared.borrow().iframe.is_some() || shared.borrow().dead {
                        return;
                    }
                    if let Some(el) = e.data().try_as_web_event() {
                        start(&shared, &el);
                    }
                }
            },
        }
        if !ready {
            {fallback}
        }
    }
}
