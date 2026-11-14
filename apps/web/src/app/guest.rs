//! A share link opened in the web app as its guest (#629, #646): what
//! `https://<node>/s/<code>/…` is. The node answers every page path of a
//! link with this app's page (`apps/desktop/src/node/share_shell.rs`); the
//! app reads the code and the path from its location (`crate::route`) and
//! draws the link from its guest service on this origin (`ShareGuest`,
//! through `ShareGuestHttpClient`):
//!
//! - the link's notes, in its order, in the sets its root note lays out
//!   (`crate::guest_view::sections`), each with its art and a play button
//!   when it has a recording;
//! - the chosen note: the view its type's island claims, as a guest
//!   (`Audience::Guest`), handed what the node gave a guest, then its
//!   page's words and images as plain blocks;
//! - the link's recordings on the Now Playing bar, streamed from the
//!   link's own file URLs ([`file_url`]);
//! - its files by note, each a download, and "Download all" (the link's
//!   `_archive.zip`);
//! - a link for signed-in people: the app's own session when this browser
//!   has one, else the node's sign-in (`/s/_signin`), whose cookie the
//!   node takes in a guest call on this origin;
//! - `_live`: a rebuild or a revoke reads the link again.
//!
//! Everything a guest's view is handed comes from the link's node, never
//! from a vault: the note's text as the node gave it (none unless the
//! view reads the note), and only the files the node listed for it
//! (`GuestNote::files`). Each file is fetched below the link on this
//! origin (no credentials, no redirect), then checked here before it is
//! handed in: its URL, parsed and normalised as the browser fetches it,
//! is on this origin below this link (`view_files::guest_file_url`); it
//! is never read past the cap (a larger `Content-Length` is refused
//! unread, the body read with a limit); its bytes match the `sha256` the
//! node listed (a file without one is refused); and it is a kind a view
//! reads at a vault path (`ext_host::ViewFile::of`). A file that fails
//! any of it is left out.

use std::fmt::Write as _;
use std::rc::Rc;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use architect_playhead::Caller;
use architect_shell::use_shell;
use architect_spaces::guest::ShareGuestHttpClient;
use architect_spaces::{BlockKind, GuestFault, GuestLink, GuestNote, ListedFile, PlainBlock};
use dioxus::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen_futures::JsFuture;
use web_sys::{RequestCredentials, RequestInit, RequestRedirect, Response};

use crate::ext_host::{Audience, ViewFile};
use crate::guest_view::{self, Open, Section};

/// This page's origin.
fn origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}

/// The link's guest service on this origin.
fn client() -> Option<ShareGuestHttpClient> {
    origin().map(ShareGuestHttpClient::at)
}

/// A guest's fault, in words.
fn said(e: &architect::ClientError<GuestFault>) -> String {
    match e {
        architect::ClientError::App(f) => f.to_string(),
        _ => "the link's node did not answer".to_owned(),
    }
}

/// GET `url` on this origin's link: no credentials, no redirect followed,
/// at most `max` bytes, never more read: a larger `Content-Length` is
/// refused before the body, and the body is read a chunk at a time.
async fn bytes_of(url: &str, max: u64) -> Result<Vec<u8>, String> {
    const TOO_LARGE: &str = "larger than a view is handed";
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::Omit);
    init.set_redirect(RequestRedirect::Error);
    let w = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(w.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|_| "the file did not load".to_owned())?
        .dyn_into()
        .map_err(|_| "the file did not load".to_owned())?;
    if !response.ok() {
        return Err(format!("the file answered {}", response.status()));
    }
    let declared = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > max) {
        return Err(TOO_LARGE.to_owned());
    }
    let body = response.body().ok_or("no body")?;
    let reader: web_sys::ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| "no body".to_owned())?;
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| "the file did not load".to_owned())?;
        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|d| d.as_bool())
            .unwrap_or(true);
        if done {
            break;
        }
        let value: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &"value".into())
            .ok()
            .and_then(|v| v.dyn_into().ok())
            .ok_or("the file did not load")?;
        let total = u64::try_from(bytes.len())
            .unwrap_or(u64::MAX)
            .saturating_add(u64::from(value.length()));
        if total > max {
            // Stop the download; nothing more of it is read.
            let _ = reader.cancel();
            return Err(TOO_LARGE.to_owned());
        }
        let start = bytes.len();
        let len = usize::try_from(value.length()).map_err(|_| TOO_LARGE.to_owned())?;
        bytes.resize(start.saturating_add(len), 0);
        if let Some(into) = bytes.get_mut(start..) {
            value.copy_to(into);
        }
    }
    Ok(bytes)
}

/// `file` of link `code`, fetched and checked as the module says.
async fn checked(code: &str, file: &ListedFile) -> Result<ViewFile, String> {
    use sha2::Digest as _;
    let url = origin()
        .and_then(|o| crate::view_files::guest_file_url(&o, code, &file.url))
        .ok_or_else(|| format!("{}: not a file of this link", file.path))?;
    // Only a file the node vouched for: a listed one with no hash is
    // never fetched.
    let want = file
        .sha256
        .as_deref()
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("{}: listed without its sha256", file.path))?;
    if file.size > architect_spaces::view::VIEW_FILE_MAX {
        return Err(format!("{}: too large", file.path));
    }
    let bytes = bytes_of(&url, file.size).await?;
    let got = sha2::Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    if !got.eq_ignore_ascii_case(want) {
        return Err(format!("{}: not the file the link listed", file.path));
    }
    ViewFile::of(&file.path, &bytes)
}

/// The files the node gave for `note`, each checked; what fails is left
/// out. Within the caps again, whatever the node listed.
async fn files_of(code: &str, note: &GuestNote) -> Vec<ViewFile> {
    let mut out = Vec::new();
    for f in &note.files {
        if let Ok(v) = checked(code, f).await {
            out.push(v);
        }
    }
    architect_spaces::view::within_caps(out, |f| f.path.as_str(), |f| f.size)
}

/// The Now Playing source of the link's recordings: the app's own,
/// outside the extension-id grammar (`:`), so no extension holds it.
pub const SOURCE: &str = "app:share";

thread_local! {
    /// The open link's files: vault path to its URL on this origin, as the
    /// node listed them. What the audio element and the card's art
    /// resolve a path with ([`file_url`]); nothing else is.
    static FILES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// The open link's URL for a file at vault path `path`, when the link
/// serves it: the Now Playing bar's media and art, a view's art.
#[must_use]
pub fn file_url(path: &str) -> Option<String> {
    FILES.with(|f| f.borrow().get(path).cloned())
}

thread_local! {
    /// Whether this browser's session was refused by the link's node
    /// (signed out or revoked elsewhere, not yet expired here): calls go
    /// without it from then on, so the node's own sign-in cookie counts.
    static SESSION_REFUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The token a guest call carries: this browser's session, else none
/// (the node then reads its own sign-in's cookie).
fn token() -> String {
    if SESSION_REFUSED.with(std::cell::Cell::get) {
        return String::new();
    }
    crate::browser::stored_token().unwrap_or_default()
}

/// `call` with [`token`]; when it carried this browser's session and the
/// node said to sign in, once more with none, so the node's own sign-in
/// cookie is used (a session revoked elsewhere that has not expired here
/// would otherwise keep a signed-in person at "Sign in").
async fn as_guest<T, F, Fut>(call: F) -> Result<T, architect::ClientError<GuestFault>>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, architect::ClientError<GuestFault>>>,
{
    let sent = token();
    match call(sent.clone()).await {
        Err(architect::ClientError::App(GuestFault::SignIn)) if !sent.is_empty() => {
            SESSION_REFUSED.with(|r| r.set(true));
            call(String::new()).await
        }
        answer => answer,
    }
}

/// What the page says of the release it was served by: the node's share
/// contract and version (`apps/desktop/src/node/share_shell.rs`).
fn page_meta(name: &str) -> Option<String> {
    web_sys::window()?
        .document()?
        .query_selector(&format!("meta[name=\"{name}\"]"))
        .ok()??
        .get_attribute("content")
}

/// The line the node's page shows when no app drew the link: this app
/// is drawing it.
fn drop_stall() {
    if let Some(stall) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(architect_spaces::guest::SHARE_STALL_ID))
    {
        stall.remove();
    }
}

/// What the guest service answered for the link.
#[derive(Clone, PartialEq)]
struct Opened {
    link: Rc<GuestLink>,
    files: Rc<Vec<ListedFile>>,
}

/// Why the link is not open, as the view says it.
#[derive(Clone, PartialEq)]
enum Shut {
    SignIn,
    Denied,
    Gone,
    Building,
    Held(String),
    Unreachable,
}

impl Shut {
    fn of(e: &architect::ClientError<GuestFault>) -> Self {
        match e {
            architect::ClientError::App(GuestFault::SignIn) => Self::SignIn,
            architect::ClientError::App(GuestFault::Denied) => Self::Denied,
            architect::ClientError::App(GuestFault::Gone) => Self::Gone,
            architect::ClientError::App(GuestFault::Building) => Self::Building,
            architect::ClientError::App(GuestFault::Held(l)) => Self::Held(l.clone()),
            _ => Self::Unreachable,
        }
    }
}

/// This page's path and query, for the sign-in to come back to.
fn here() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_else(|| "/".to_owned())
}

/// The location shows `path` (a page of the link), without a reload, as
/// a step of its own: Back (a phone's swipe) comes back to where the
/// guest was on the link, not out of it ([`guest_view::rest_of`]).
fn show_path(path: &str) {
    let Some(w) = web_sys::window() else { return };
    if w.location().pathname().ok().as_deref() == Some(path) {
        return;
    }
    if let Ok(h) = w.history() {
        let _ = h.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path));
    }
}

/// The location's path, below the window.
fn pathname() -> Option<String> {
    web_sys::window()?.location().pathname().ok()
}

/// `to` as a query value.
fn query_value(to: &str) -> String {
    to.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The link's recordings as the app's own source, not playing, and its
/// files as the paths [`file_url`] resolves. With `core_list` (its root
/// is a collection no island claims, #668), the queue is the list's: its
/// rows that play, in its order, as `architect_media::ui::MediaList`
/// plays them.
fn load_player(opened: &Opened, core_list: bool) {
    let o = origin().unwrap_or_default();
    let code = opened
        .link
        .link
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let map: HashMap<String, String> = opened
        .files
        .iter()
        .filter_map(|f| {
            let url = crate::view_files::guest_file_url(&o, &code, &f.url)?;
            Some((f.path.clone(), url))
        })
        .collect();
    let served: HashSet<&str> = map.keys().map(String::as_str).collect();
    let queue = match guest_view::collection_rows(&opened.link, &served).filter(|_| core_list) {
        Some(rows) => architect_media::collection::queue(&rows),
        None => guest_view::queue(&opened.link, &served),
    };
    FILES.with(|f| *f.borrow_mut() = map.clone());
    if queue.is_empty() {
        return;
    }
    let same = architect_playhead::with(|d| {
        d.source(SOURCE).is_some_and(|s| {
            s.queue.len() == queue.len()
                && s.queue
                    .iter()
                    .zip(&queue)
                    .all(|(i, q)| q["id"].as_str() == Some(i.id.as_str()))
        })
    });
    if !same {
        let _ = architect_playhead::ask(
            &Caller::App,
            "PLAYHEAD_SET",
            &json!({ "source": SOURCE, "name": opened.link.title, "queue": queue }),
        );
    }
}

/// Play item `id` of the link's source.
fn play(id: &str) {
    let _ = architect_playhead::ask(
        &Caller::App,
        "PLAYHEAD_ITEM",
        &json!({ "source": SOURCE, "id": id, "play": true }),
    );
}

/// Link `code` as its guest, opened at `rest` (below the link).
#[component]
pub fn GuestPage(code: String, rest: String) -> Element {
    let t = use_shell().theme;
    use_hook(drop_stall);
    // A page from a node of another release: say so, read nothing.
    let skew = use_hook(|| {
        crate::shell_skew(page_meta(architect_spaces::guest::SHARE_SHELL_META).as_deref())
    });
    if let Some(skew) = skew {
        let node = page_meta(architect_spaces::guest::NODE_VERSION_META).unwrap_or_default();
        return rsx! { OutOfDate { skew, node } };
    }
    // Moves on with every `_live` event, and while the link is built.
    let mut revision = use_signal(|| 0_u64);
    use_hook({
        let code = code.clone();
        move || {
            let url = format!("{}_live", architect_spaces::share::link_path(&code));
            if let Ok(events) = web_sys::EventSource::new(&url) {
                let on = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |_| {
                    revision.with_mut(|r| *r = r.wrapping_add(1));
                });
                events.set_onmessage(Some(on.as_ref().unchecked_ref()));
                on.forget();
                // A revoke ends the stream and the next one is refused:
                // read the link once more, which says it is gone.
                let gone = Closure::<dyn FnMut(web_sys::Event)>::new({
                    let events = events.clone();
                    move |_| {
                        if events.ready_state() == web_sys::EventSource::CLOSED {
                            revision.with_mut(|r| *r = r.wrapping_add(1));
                        }
                    }
                });
                events.set_onerror(Some(gone.as_ref().unchecked_ref()));
                gone.forget();
            }
        }
    });
    let opened = use_resource({
        let code = code.clone();
        move || {
            let code = code.clone();
            let _ = revision();
            async move {
                let c = client().ok_or(Shut::Unreachable)?;
                let link = as_guest(|t| c.link(code.clone(), t))
                    .await
                    .map_err(|e| Shut::of(&e))?;
                let files = as_guest(|t| c.files(code.clone(), t, None))
                    .await
                    .map_err(|e| Shut::of(&e))?;
                Ok::<_, Shut>(Opened {
                    link: Rc::new(link),
                    files: Rc::new(files),
                })
            }
        }
    });
    // While it is being built: ask again in a moment.
    let building = matches!(&*opened.read(), Some(Err(Shut::Building)));
    use_effect(use_reactive!(|building| {
        if building {
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(3_000).await;
                revision.with_mut(|r| *r = r.wrapping_add(1));
            });
        }
    }));
    let mut open = use_signal(|| None::<Open>);
    // Where on the link the location is: as opened, then each Back or
    // Forward within it (the page the guest chose is a history step).
    let mut at_path = use_signal(|| rest.clone());
    use_hook({
        let link = architect_spaces::share::link_path(&code);
        move || {
            let Some(w) = web_sys::window() else { return };
            let on = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                let Some(now) = pathname() else { return };
                // Gone with the page (never, while the app shows a link):
                // nothing to move.
                if let (Some(rest), Ok(mut path), Ok(mut chosen)) = (
                    guest_view::rest_of(&link, &now),
                    at_path.try_write(),
                    open.try_write(),
                ) {
                    rest.clone_into(&mut path);
                    *chosen = None;
                }
            });
            let _ = w.add_event_listener_with_callback("popstate", on.as_ref().unchecked_ref());
            on.forget();
        }
    });
    // Open a note or the files: the location follows, without a reload.
    let choose = use_callback(move |next: Open| {
        if let Some(Ok(o)) = &*opened.peek() {
            let page = match &next {
                Open::Note(p) => o
                    .link
                    .notes
                    .iter()
                    .find(|n| &n.path == p)
                    .and_then(|n| n.page.clone()),
                Open::Files => Some(format!("{}_files/", o.link.link)),
                Open::Nothing => None,
            };
            if let Some(page) = page {
                show_path(&page);
            }
        }
        open.set(Some(next));
    });
    let state = opened.read().clone();
    // The root note as the core list (#668): a collection whose type no
    // island claims (an island that does draws its own view instead).
    let listing = super::frames::use_listing();
    let core_list = match &state {
        Some(Ok(o)) => guest_view::root_note(&o.link).is_some_and(|r| {
            !r.sets.is_empty()
                && !listing.read().as_ref().is_some_and(|l| {
                    crate::ext_host::claimed(l, r.kind.as_deref().unwrap_or_default()).is_some()
                })
        }),
        _ => false,
    };
    if let Some(Ok(o)) = &state {
        load_player(o, core_list);
    }
    let body = match state {
        None => rsx! { p { style: "color: {t.ink_3};", "Opening the shared link…" } },
        Some(Err(shut)) => rsx! { Closed { shut } },
        Some(Ok(o)) => {
            let at = open().unwrap_or_else(|| guest_view::open(&o.link, &at_path()));
            rsx! {
                LinkHead { opened: o.clone(), at: at.clone(), on_open: choose }
                match at.clone() {
                    Open::Files => rsx! { FilesView { opened: o.clone() } },
                    // The root note: its list first (what a setlist is
                    // opened for), then its words; any other, its words.
                    Open::Note(path) if core_list && guest_view::root_note(&o.link).is_some_and(|r| r.path == path) => rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 20px;",
                            GuestCollection { opened: o.clone(), on_open: choose }
                            GuestNoteView { key: "{path}-{revision}", code: code.clone(), path: path.clone(), core_list: true }
                        }
                    },
                    Open::Note(path) if guest_view::root_note(&o.link).is_some_and(|r| r.path == path) => rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 20px;",
                            NoteList { opened: o.clone(), chosen: path.clone(), on_open: choose }
                            GuestNoteView { key: "{path}-{revision}", code: code.clone(), path: path.clone() }
                        }
                    },
                    Open::Note(path) => rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 20px;",
                            GuestNoteView { key: "{path}-{revision}", code: code.clone(), path: path.clone() }
                            if core_list {
                                GuestCollection { opened: o.clone(), on_open: choose }
                            } else {
                                NoteList { opened: o.clone(), chosen: path, on_open: choose }
                            }
                        }
                    },
                    Open::Nothing => rsx! {
                        p { "data-testid": "guest-problem", "This page is not on the link any more." }
                        if core_list {
                            GuestCollection { opened: o.clone(), on_open: choose }
                        } else {
                            NoteList { opened: o.clone(), chosen: String::new(), on_open: choose }
                        }
                    },
                }
            }
        }
    };
    rsx! {
        div { "data-testid": "guest-page",
            style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 16px 16px 120px; color: {t.ink};",
            div { style: "max-width: 760px; margin: 0 auto; display: flex; flex-direction: column; gap: 16px;",
                {body}
            }
        }
    }
}

/// The node that served the page is of another release than this app.
#[component]
fn OutOfDate(skew: crate::ShellSkew, node: String) -> Element {
    let t = use_shell().theme;
    let (title, line) = match skew {
        crate::ShellSkew::AppOlder => (
            "This app is out of date",
            "This server's web app is older than its node, so it cannot open this link. Reload in a while, or open the link in the Architect app.",
        ),
        crate::ShellSkew::NodeOlder => (
            "This server is out of date",
            "This server's node is older than its web app, so the link cannot open here yet. Open it in the Architect app.",
        ),
    };
    rsx! {
        div { "data-testid": "guest-page",
            style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 16px; color: {t.ink};",
            section { "data-testid": "guest-problem", "data-why": "{title}",
                style: "max-width: 760px; margin: 0 auto; display: flex; flex-direction: column; gap: 10px; padding: 24px 0;",
                h1 { style: "margin: 0; font-size: 22px;", "{title}" }
                p { style: "margin: 0; color: {t.ink_2}; line-height: 1.5;", "{line}" }
                if !node.is_empty() {
                    p { style: "margin: 0; color: {t.ink_3}; font-size: 13px;", "Node {node}" }
                }
            }
        }
    }
}

/// Why the link does not open, and what to do.
#[component]
fn Closed(shut: Shut) -> Element {
    let t = use_shell().theme;
    let signin = format!("/s/_signin?to={}", query_value(&here()));
    let (title, line) = match &shut {
        Shut::SignIn => ("Sign in to open this link", "It is for signed-in people: the members of an organization, or the people it was shared with.".to_owned()),
        Shut::Denied => ("Not shared with you", "This link was not shared with the account you are signed in as.".to_owned()),
        Shut::Gone => ("Nothing at this link", "It may have expired or been taken down.".to_owned()),
        Shut::Building => ("Being built", "This link is being built. It opens in a moment.".to_owned()),
        Shut::Held(layer) => ("Not on this link", format!("This needs the {layer} layer.")),
        Shut::Unreachable => ("Not reachable", "The link's node did not answer. Try again in a moment.".to_owned()),
    };
    rsx! {
        section { "data-testid": "guest-problem", "data-why": "{title}",
            style: "display: flex; flex-direction: column; gap: 10px; padding: 24px 0;",
            h1 { style: "margin: 0; font-size: 22px;", "{title}" }
            p { style: "margin: 0; color: {t.ink_2}; line-height: 1.5;", "{line}" }
            if shut == Shut::SignIn {
                a { "data-testid": "guest-sign-in", href: "{signin}",
                    style: "align-self: flex-start; padding: 8px 16px; border-radius: 999px; background: {t.accent}; color: {t.accent_ink}; text-decoration: none; font-weight: 600;",
                    "Sign in"
                }
            }
        }
    }
}

/// The link's title, who it is for, and the way to its files.
#[component]
fn LinkHead(opened: Opened, at: Open, on_open: EventHandler<Open>) -> Element {
    let t = use_shell().theme;
    let link = opened.link.clone();
    let count = opened.files.len();
    let whom = match link.audience.as_str() {
        "public" => "Public",
        "org" => "For an organization's members",
        "invited" => "For the people it was shared with",
        _ => "Anyone with the link",
    };
    let files_on = at == Open::Files;
    let root = guest_view::root_note(&link).map(|n| n.path.clone());
    let pill = |on: bool| {
        if on {
            format!(
                "padding: 5px 12px; border-radius: 999px; border: 1px solid {}; background: {}; color: {}; font: inherit; font-size: 13px; cursor: pointer;",
                t.accent, t.accent, t.accent_ink
            )
        } else {
            format!(
                "padding: 5px 12px; border-radius: 999px; border: 1px solid {}; background: transparent; color: {}; font: inherit; font-size: 13px; cursor: pointer;",
                t.rule, t.ink
            )
        }
    };
    rsx! {
        header { style: "display: flex; flex-direction: column; gap: 8px;",
            div { style: "font-size: 12px; color: {t.ink_3}; letter-spacing: 0.02em;", "Shared with you · {whom}" }
            h1 { "data-testid": "guest-title", style: "margin: 0; font-size: 24px; line-height: 1.2;", "{link.title}" }
            nav { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                if let Some(root) = root {
                    button { "data-testid": "guest-notes-tab", style: pill(!files_on),
                        onclick: move |_| on_open.call(Open::Note(root.clone())),
                        "Notes ({link.notes.len()})"
                    }
                }
                if count > 0 {
                    button { "data-testid": "guest-files-tab", style: pill(files_on),
                        onclick: move |_| on_open.call(Open::Files),
                        "Files ({count})"
                    }
                }
            }
        }
    }
}

/// The link's root note as a collection's core list (#668): its sets as
/// the node listed them (only notes the link lists), each row's art and
/// media only what the link serves ([`file_url`]); a row's chevron, and a
/// row with nothing to play, opens its note.
#[component]
fn GuestCollection(opened: Opened, on_open: EventHandler<Open>) -> Element {
    let served: HashSet<String> = FILES.with(|f| f.borrow().keys().cloned().collect());
    let served: HashSet<&str> = served.iter().map(String::as_str).collect();
    let Some(sections) = guest_view::collection_rows(&opened.link, &served) else {
        return rsx! {};
    };
    let art = use_callback(|p: String| file_url(&p));
    rsx! {
        architect_media::ui::MediaList {
            source: SOURCE.to_owned(),
            name: opened.link.title.clone(),
            sections,
            art: Some(art),
            on_open: move |p: String| on_open.call(Open::Note(p)),
        }
    }
}

/// The link's notes, in its sets, the chosen one marked: each row its
/// number, art, title and a play button for its recording.
#[component]
fn NoteList(opened: Opened, chosen: String, on_open: EventHandler<Open>) -> Element {
    let t = use_shell().theme;
    let link = opened.link.clone();
    let root = guest_view::root_note(&link).map(|n| n.path.clone());
    // The root note's outline, for its sets; read again with the link.
    let code = link
        .link
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let asked = root.clone();
    let outline = use_resource(use_reactive!(|(asked, code, opened)| {
        let _ = &opened;
        let root = asked;
        async move {
            let root = root?;
            let c = client()?;
            as_guest(|t| c.read(code.clone(), t, root.clone()))
                .await
                .ok()
                .map(|n| n.blocks)
        }
    }));
    let blocks = outline.read().clone().flatten().unwrap_or_default();
    let sections: Vec<Section> = guest_view::sections(&link, root.as_deref(), &blocks);
    let playing = architect_playhead::ui::use_playhead();
    let now = playing().active.as_ref().and_then(|a| {
        (a.source == SOURCE && a.playing)
            .then(|| a.item.as_ref().map(|i| i.id.clone()))
            .flatten()
    });
    let mut number = 0_usize;
    rsx! {
        section { "data-testid": "guest-list", style: "display: flex; flex-direction: column; gap: 4px;",
            for (s, section) in sections.into_iter().enumerate() {
                if let Some(label) = section.label.clone() {
                    div { key: "set-{s}", "data-guest-set": "{label}",
                        style: "margin: 14px 0 4px; font-size: 12px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: {t.ink_3};",
                        "{label}"
                    }
                }
                for i in section.notes {
                    if let Some(n) = link.notes.get(i).cloned() {
                        {
                            number += 1;
                            let on = n.path == chosen;
                            let art = n.art.as_deref().and_then(file_url);
                            let track = n.files.iter().find(|f| guest_view::plays(f) && file_url(f).is_some()).map(|f| format!("{}#{f}", n.path));
                            let is_now = track.is_some() && track == now;
                            let path = n.path.clone();
                            rsx! {
                                div { key: "{n.path}", "data-guest-note": "{n.title}",
                                    style: format!("display: flex; align-items: center; gap: 12px; padding: 6px 8px; border-radius: 10px; background: {};", if on { t.active.to_string() } else { "transparent".to_owned() }),
                                    span { style: "flex: none; width: 24px; text-align: right; font-size: 13px; color: {t.ink_3}; font-variant-numeric: tabular-nums;", "{number}" }
                                    match art {
                                        Some(src) => rsx! { img { src: "{src}", alt: "", style: "flex: none; width: 40px; height: 40px; border-radius: 6px; object-fit: cover;" } },
                                        None => rsx! { div { style: "flex: none; width: 40px; height: 40px; border-radius: 6px; background: {t.pill};" } },
                                    }
                                    button { style: "flex: 1; min-width: 0; text-align: left; border: 0; background: transparent; color: {t.ink}; font: inherit; font-size: 15px; cursor: pointer; padding: 6px 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                        onclick: move |_| on_open.call(Open::Note(path.clone())),
                                        "{n.title}"
                                    }
                                    if let Some(id) = track {
                                        button { "data-guest-play": "{n.title}",
                                            "aria-label": if is_now { "Pause" } else { "Play" },
                                            style: "flex: none; width: 34px; height: 34px; border-radius: 999px; border: 1px solid {t.rule}; background: transparent; color: {t.ink}; cursor: pointer; font-size: 13px;",
                                            onclick: move |_| {
                                                if is_now {
                                                    architect_playhead::ui::press("PLAYHEAD_TOGGLE", &json!({}));
                                                } else {
                                                    play(&id);
                                                }
                                            },
                                            if is_now { "❚❚" } else { "▶" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The link's files by note, each a download, and "Download all".
#[component]
fn FilesView(opened: Opened) -> Element {
    let t = use_shell().theme;
    let o = origin().unwrap_or_default();
    let link = opened.link.clone();
    let code = link
        .link
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let groups = guest_view::files_by_note(&link, &opened.files);
    let total = opened.files.len();
    let all = format!("{}_archive.zip?by=note", link.link);
    rsx! {
        section { "data-testid": "guest-files", style: "display: flex; flex-direction: column; gap: 14px;",
            if total == 0 {
                p { style: "color: {t.ink_3};", "No files are attached here." }
            } else {
                a { "data-testid": "guest-download-all", href: "{all}", download: "",
                    style: "align-self: flex-start; padding: 8px 16px; border-radius: 999px; background: {t.accent}; color: {t.accent_ink}; text-decoration: none; font-weight: 600;",
                    "Download all ({total})"
                }
            }
            for (note, files) in groups {
                div { key: "{note.path}", style: "border-top: 1px solid {t.rule}; padding-top: 10px;",
                    div { style: "font-weight: 600; margin-bottom: 6px;", "{note.title}" }
                    for f in files {
                        if let Some(url) = crate::view_files::guest_file_url(&o, &code, &f.url) {
                            a { key: "{f.path}", href: "{url}", download: "", "data-guest-file": "{f.path}",
                                style: "display: flex; justify-content: space-between; gap: 12px; padding: 6px 0; color: {t.ink}; text-decoration: none; font-size: 14px;",
                                span { style: "min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                    {f.path.rsplit('/').next().unwrap_or(&f.path).to_owned()}
                                }
                                span { style: "flex: none; color: {t.ink_3}; font-variant-numeric: tabular-nums;", {guest_view::size(f.size)} }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Note `path` of link `code`: the view its type's island claims, handed
/// what the node gave a guest, then its page's words and images.
#[component]
fn GuestNoteView(code: String, path: String, #[props(default)] core_list: bool) -> Element {
    // Read again when the note asked for changes (props are not signals).
    let note = use_resource(use_reactive!(|(code, path)| {
        {
            async move {
                let c = client().ok_or("no origin")?;
                let note = as_guest(|t| c.read(code.clone(), t, path.clone()))
                    .await
                    .map_err(|e| said(&e))?;
                let files = files_of(&code, &note).await;
                Ok::<_, String>((Rc::new(note), files, code))
            }
        }
    }));
    let state = note.read().clone();
    match state {
        None => rsx! {},
        Some(Err(why)) => rsx! { p { "data-testid": "guest-problem", "{why}" } },
        Some(Ok((note, files, code))) => {
            let kind = note.kind.clone().unwrap_or_default();
            let count = files.len();
            rsx! {
                article { "data-testid": "guest-note", "data-files": "{count}",
                    "data-text": "{note.source.is_some()}",
                    style: "display: flex; flex-direction: column; gap: 10px;",
                    h2 { style: "margin: 0; font-size: 20px;", "{note.title}" }
                    if !kind.is_empty() {
                        super::embed::NoteEmbed {
                            audience: Audience::Guest,
                            // A guest's view is never handed a vault path.
                            path: String::new(),
                            title: note.title.clone(),
                            note_type: kind,
                            properties: Vec::new(),
                            body: note.source.clone().unwrap_or_default(),
                            files,
                        }
                    }
                    Blocks {
                        code,
                        // The core list says what a listing fence repeats.
                        blocks: note
                            .blocks
                            .iter()
                            .filter(|b| {
                                !(core_list
                                    && b.kind == BlockKind::Code
                                    && b.info.as_deref().is_some_and(architect_media::is_listing_fence))
                            })
                            .cloned()
                            .collect::<Vec<_>>(),
                    }
                }
            }
        }
    }
}

/// A page's plain blocks: text as text, images from below the link.
#[component]
fn Blocks(code: String, blocks: Vec<PlainBlock>) -> Element {
    let t = use_shell().theme;
    let o = origin().unwrap_or_default();
    // A medium's fallback line (its file's path, under it): the medium
    // is said once.
    let echoes: Vec<bool> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            b.kind == BlockKind::Paragraph
                && !b.text.contains(' ')
                && i.checked_sub(1)
                    .and_then(|p| blocks.get(p))
                    .is_some_and(|m| {
                        matches!(m.kind, BlockKind::Audio | BlockKind::Video)
                            && !m.text.is_empty()
                            && b.text.ends_with(m.text.as_str())
                    })
        })
        .collect();
    rsx! {
        for (i, b) in blocks.into_iter().enumerate().filter(|(i, _)| !echoes.get(*i).copied().unwrap_or(false)) {
            {
                let key = i.to_string();
                match b.kind {
                    BlockKind::Heading => {
                        let size = 22_i32.saturating_sub(2_i32.saturating_mul(i32::from(b.level))).max(14);
                        rsx! { div { key: "{key}", style: "font-size: {size}px; font-weight: 700; margin-top: 6px;", "{b.text}" } }
                    }
                    BlockKind::Paragraph => rsx! {
                        p { key: "{key}", style: "margin: 0; color: {t.ink_2}; line-height: 1.6; white-space: pre-wrap;", "{b.text}" }
                    },
                    BlockKind::Quote => rsx! {
                        blockquote { key: "{key}", style: "margin: 0; color: {t.ink_2}; line-height: 1.6; border-left: 3px solid {t.rule}; padding-left: 12px;", "{b.text}" }
                    },
                    BlockKind::List => {
                        let ordered = b.level == 1;
                        rsx! {
                            div { key: "{key}", style: "display: flex; flex-direction: column; gap: 4px; color: {t.ink_2}; line-height: 1.5;",
                                for (n, item) in b.items.iter().enumerate() {
                                    div { key: "{n}",
                                        if ordered { "{n.saturating_add(1)}. {item}" } else { "• {item}" }
                                    }
                                }
                            }
                        }
                    }
                    BlockKind::Code => rsx! {
                        pre { key: "{key}",
                            style: "margin: 0; padding: 10px 12px; border-radius: 8px; background: {t.pill}; font-family: ui-monospace, Menlo, monospace; font-size: 13px; white-space: pre-wrap; overflow-x: auto;",
                            "{b.text}"
                        }
                    },
                    BlockKind::Rule => rsx! { div { key: "{key}", style: "height: 1px; background: {t.rule}; margin: 8px 0;" } },
                    BlockKind::Image => {
                        match b.src.as_deref().and_then(|s| crate::view_files::guest_file_url(&o, &code, s)) {
                            Some(src) => rsx! { img { key: "{key}", "data-guest-image": "{b.text}", src: "{src}", alt: "{b.text}", style: "max-width: 100%; height: auto; border-radius: 8px; align-self: flex-start;" } },
                            None => rsx! { div { key: "{key}", style: "font-size: 13px; color: {t.ink_3};", "Image: {b.text}" } },
                        }
                    }
                    BlockKind::Audio | BlockKind::Video => rsx! {
                        div { key: "{key}", style: "font-size: 13px; color: {t.ink_3};", "{b.text}: in the Now Playing bar" }
                    },
                    BlockKind::Drawn => rsx! {
                        div { key: "{key}", style: "font-size: 13px; color: {t.ink_3}; white-space: pre-wrap;", "{b.text}" }
                    },
                }
            }
        }
    }
}
