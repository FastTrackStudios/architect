//! A share link opened in the web app as its guest (#629, #646): what
//! `https://<node>/s/<code>/…` is. A link is this same app as a guest
//! (`docs/content/guide/shares.md`); this guest-only page is a second
//! implementation to replace with the app's own vault views (#804). The node answers every page path of a
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

/// The page's scrollers back at their top (the guest page's content
/// scrolls in an element of its own, `data-guest-scroll`).
fn to_top() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(list) = doc.query_selector_all("[data-guest-scroll]") {
        for i in 0..list.length() {
            if let Some(el) = list
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            {
                el.set_scroll_top(0);
            }
        }
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
                Open::View(..) | Open::Nothing => None,
            };
            if let Some(page) = page {
                show_path(&page);
            }
        }
        open.set(Some(next));
        // From its top: the page before it may have been scrolled far down
        // (the link's list of notes), and the note would open off screen.
        to_top();
    });
    // A view's ask to open a note of this link (`ext_host::NOTE_OPEN`: a
    // set's row, to its song): only a note the link lists, as a press in
    // the list of its notes.
    use_hook(move || {
        super::frames::set_note_opener(Some(std::rc::Rc::new(move |path: &str| {
            let listed = matches!(&*opened.peek(), Some(Ok(o)) if o.link.notes.iter().any(|n| n.path == path));
            if listed {
                choose.call(Open::Note(path.to_owned()));
            }
            listed
        })));
    });
    use_drop(|| super::frames::set_note_opener(None));
    // The same chooser for the shell's bottom tabs and the sidebar, which
    // are drawn outside this page ([`press_tab`], [`GuestSide`]).
    use_hook(move || {
        CHOOSER.with(|c| *c.borrow_mut() = Some(Rc::new(move |next: Open| choose.call(next))));
    });
    use_drop(|| {
        CHOOSER.with(|c| *c.borrow_mut() = None);
        *MENU.write() = None;
        *BAR.write() = Bar::default();
        *SIDE.write() = None;
    });
    // The bottom tabs: the favourites an extension declares for the type
    // of the link's root note (a setlist's Setlist, Charts, Lyrics,
    // Arrangement), each a view of that extension's; and which is current.
    let declared = use_resource(super::frames::favourites);
    let islands = use_resource(super::frames::islands);
    use_effect(move || {
        let (link, files) = match &*opened.read() {
            Some(Ok(o)) => ((*o.link).clone(), o.files.len()),
            _ => return,
        };
        let at = open().unwrap_or_else(|| guest_view::open(&link, &at_path()));
        let bar = bar_of(
            &link,
            &at,
            declared.read().as_deref().map_or(&[][..], |d| d.as_slice()),
            islands.read().as_deref().map_or(&[][..], |i| i.as_slice()),
        );
        if *BAR.peek() != bar {
            *BAR.write() = bar;
        }
        let side = Side { link, files, at };
        if SIDE.peek().as_ref() != Some(&side) {
            *SIDE.write() = Some(side);
        }
    });
    let state = opened.read().clone();
    // The root note as the core list (#668): a collection whose type no
    // island claims (an island that does draws its own view instead).
    let listing = super::frames::use_listing();
    // A root collection an island claims: its view lists it, so it comes
    // first and the link's notes follow under their own heading.
    let (core_list, claimed_list) = match &state {
        Some(Ok(o)) => guest_view::root_note(&o.link)
            .filter(|r| !r.sets.is_empty())
            .map_or((false, false), |r| {
                let claimed = listing.read().as_ref().is_some_and(|l| {
                    crate::ext_host::claimed(l, r.kind.as_deref().unwrap_or_default()).is_some()
                });
                (!claimed, claimed)
            }),
        _ => (false, false),
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
                    Open::View(extension, view) => rsx! { super::embed::IslandView { key: "{extension}/{view}", extension, view } },
                    // The root note: its list first (what a setlist is
                    // opened for), then its words; any other, its words.
                    Open::Note(path) if core_list && guest_view::root_note(&o.link).is_some_and(|r| r.path == path) => rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 20px;",
                            GuestCollection { opened: o.clone(), on_open: choose }
                            GuestNoteView { key: "{path}-{revision}", code: code.clone(), path: path.clone(), core_list: true }
                        }
                    },
                    // The root collection an island claims: its view (the
                    // list as its extension draws it) first, then the
                    // link's notes to open, under their own heading.
                    Open::Note(path) if claimed_list && guest_view::root_note(&o.link).is_some_and(|r| r.path == path) => rsx! {
                        div { style: "display: flex; flex-direction: column; gap: 20px;",
                            // The link is named for this note: its title is the page's heading already.
                            // The link's other notes and its files are in the sidebar ([`GuestSide`]).
                            GuestNoteView { key: "{path}-{revision}", code: code.clone(), path: path.clone(), untitled: true }
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
        TabMenu {}
        div { "data-testid": "guest-page",
            "data-guest-scroll": "", style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 6px 16px 120px; color: {t.ink};",
            div { style: "max-width: 760px; margin: 0 auto; display: flex; flex-direction: column; gap: 10px;",
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
            "data-guest-scroll": "", style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 16px; color: {t.ink};",
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

/// The top of a link: one thin line, the link's name centred and quiet,
/// and the way to the sidebar, where who it is for, its notes and its
/// files are ([`GuestSide`]). The page under it is the note.
#[component]
fn LinkHead(opened: Opened, at: Open, on_open: EventHandler<Open>) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let _ = (at, on_open);
    rsx! {
        header { style: "position: relative; display: flex; align-items: center; justify-content: center; min-height: 32px;",
            button { r#type: "button", "data-testid": "guest-side-open", "aria-label": "This link: who it is for, its notes and its files",
                style: "position: absolute; left: -10px; top: 50%; transform: translateY(-50%); width: 44px; height: 44px; display: grid; place-items: center; border: 0; background: transparent; color: {t.ink_3}; cursor: pointer; border-radius: 10px;",
                onclick: move |_| shell.toggle(architect_shell::Side::Left),
                svg { view_box: "0 0 24 24", width: "18", height: "18", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", "aria-hidden": "true",
                    path { d: "M4 7h16M4 12h16M4 17h10" }
                }
            }
            span { "data-testid": "guest-title",
                style: "max-width: calc(100% - 96px); font-size: 13px; color: {t.ink_3}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                "{opened.link.title}"
            }
        }
    }
}

/// What the sidebar and the bottom tabs are drawn from: the open link,
/// how many files it has and what is open. Set by the page, read by the
/// shell's own slots, which are drawn outside it.
#[derive(Clone, PartialEq)]
pub(super) struct Side {
    link: GuestLink,
    files: usize,
    at: Open,
}

/// One bottom tab of a link: a declared favourite.
#[derive(Clone, PartialEq)]
pub(super) struct BarTab {
    pub id: &'static str,
    pub label: &'static str,
    pub glyph: &'static str,
    /// The extension whose view it opens, and the view.
    extension: String,
    view: String,
    /// The note types that view claims; none: it fills the page.
    claims: Vec<String>,
    /// Its menu's entries, for a tab that opens one above itself (a
    /// phone's "more"); empty for a tab that opens a view.
    pub menu: Vec<BarTab>,
}

/// A link's bottom tabs, which is current, and its round button.
#[derive(Clone, Default, PartialEq)]
pub(super) struct Bar {
    pub tabs: Vec<BarTab>,
    pub current: Option<&'static str>,
    pub primary: Option<architect_favourites::Primary>,
}

pub(super) static BAR: GlobalSignal<Bar> = Signal::global(Bar::default);
pub(super) static SIDE: GlobalSignal<Option<Side>> = Signal::global(|| None);
/// The tab whose menu is open above the bar, by its id.
pub(super) static MENU: GlobalSignal<Option<&'static str>> = Signal::global(|| None);

thread_local! {
    /// Opens a note, the files or a view on the page ([`GuestPage`]'s `choose`).
    static CHOOSER: RefCell<Option<Rc<dyn Fn(Open)>>> = const { RefCell::new(None) };
    /// The declared ids and labels, kept for the page's life: the shell's
    /// tabs name theirs with `&'static str`.
    static NAMES: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
}

fn kept(name: &str) -> &'static str {
    NAMES.with(|n| {
        *n.borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(|| Box::leak(name.to_owned().into_boxed_str()))
    })
}

fn choose_on_page(next: Open) {
    let chooser = CHOOSER.with(|c| c.borrow().clone());
    if let Some(choose) = chooser {
        choose(next);
    }
}

/// The bar for `link` with `at` open: the favourites declared for its
/// root note's type whose targets are views of an island the node lists.
fn bar_of(
    link: &GuestLink,
    at: &Open,
    declared: &[architect_favourites::Listed],
    islands: &[crate::ext_host::Island],
) -> Bar {
    use architect_favourites::Target;
    let Some(kind) = guest_view::root_note(link).and_then(|r| r.kind.clone()) else {
        return Bar::default();
    };
    let Some(set) = declared
        .iter()
        .find(|d| d.favourites.is_for(&kind) && !d.favourites.tabs.is_empty())
    else {
        return Bar::default();
    };
    let Some(island) = islands.iter().find(|i| i.id == set.by.extension) else {
        return Bar::default();
    };
    let tab_of = |f: &architect_favourites::Favourite| -> Option<BarTab> {
        let (view, menu) = match &f.target {
            Target::View(view) => (
                view.rsplit('/').next().unwrap_or(view).to_owned(),
                Vec::new(),
            ),
            // Filled below: a menu's entries are tabs of their own.
            Target::Menu(_) => (String::new(), Vec::new()),
            Target::Action(_) | Target::Place(_) => return None,
        };
        let claims = island
            .embeds
            .iter()
            .find(|e| !view.is_empty() && e.view == view)
            .map(|e| e.note_types.clone())
            .unwrap_or_default();
        Some(BarTab {
            id: kept(&f.id),
            label: kept(&f.label),
            glyph: kept(&f.icon),
            extension: island.id.clone(),
            view,
            claims,
            menu,
        })
    };
    let tabs: Vec<BarTab> = set
        .favourites
        .tabs
        .iter()
        .filter_map(|f| {
            let mut tab = tab_of(f)?;
            if let Target::Menu(entries) = &f.target {
                tab.menu = entries.iter().filter_map(&tab_of).collect();
                if tab.menu.is_empty() {
                    return None;
                }
            }
            Some(tab)
        })
        .collect();
    let claims = |t: &BarTab, kind: &str| t.claims.iter().any(|c| c.eq_ignore_ascii_case(kind));
    let current = match at {
        Open::Note(path) => link
            .notes
            .iter()
            .find(|n| &n.path == path)
            .and_then(|n| n.kind.as_deref())
            .and_then(|k| tabs.iter().find(|t| claims(t, k)))
            .map(|t| t.id),
        Open::View(_, view) => tabs
            .iter()
            .find(|t| &t.view == view || t.menu.iter().any(|m| &m.view == view))
            .map(|t| t.id),
        Open::Files | Open::Nothing => None,
    };
    Bar {
        tabs,
        current,
        primary: set.favourites.primary.clone(),
    }
}

/// A bottom tab pressed. A tab whose view claims a note type opens a note
/// of that type: the link's root when it is one, else the one whose
/// recording is playing, else the first. A tab whose view claims none
/// fills the page with that view.
pub(super) fn press_tab(id: &str) {
    let found = {
        let bar = BAR.peek();
        bar.tabs
            .iter()
            .chain(bar.tabs.iter().flat_map(|t| t.menu.iter()))
            .find(|t| t.id == id)
            .cloned()
    };
    let Some(tab) = found else {
        return;
    };
    // A tab with a menu opens it above itself (pressed again: closes it).
    if !tab.menu.is_empty() {
        let open = *MENU.peek() == Some(tab.id);
        *MENU.write() = (!open).then_some(tab.id);
        return;
    }
    *MENU.write() = None;
    let Some(side) = SIDE.peek().clone() else {
        return;
    };
    if tab.claims.is_empty() {
        choose_on_page(Open::View(tab.extension, tab.view));
        return;
    }
    let of_type = |n: &&architect_spaces::ListedNote| {
        n.kind
            .as_deref()
            .is_some_and(|k| tab.claims.iter().any(|c| c.eq_ignore_ascii_case(k)))
    };
    let playing = architect_playhead::with(|d| {
        d.active()
            .and_then(|a| d.state(a, architect_playhead::now_ms()))
    })
    .and_then(|s| s["item"]["media"].as_str().map(str::to_owned));
    let note = guest_view::root_note(&side.link)
        .filter(of_type)
        .or_else(|| {
            let media = playing.as_deref()?;
            side.link
                .notes
                .iter()
                .filter(of_type)
                .find(|n| n.media.as_deref() == Some(media))
        })
        .or_else(|| side.link.notes.iter().find(of_type));
    if let Some(note) = note {
        choose_on_page(Open::Note(note.path.clone()));
    }
}

/// The menu open above a bottom tab (a phone's "more"): its entries, one
/// a row, over the page; a press outside closes it.
#[component]
pub(super) fn TabMenu() -> Element {
    let t = use_shell().theme;
    let Some(open) = MENU() else {
        return rsx! {};
    };
    let bar = BAR();
    let Some(tab) = bar.tabs.iter().find(|t| t.id == open).cloned() else {
        return rsx! {};
    };
    let now = SIDE().map(|s| s.at);
    rsx! {
        div { "data-testid": "tab-menu-scrim", style: "position: fixed; inset: 0; z-index: 60;",
            onclick: move |_| *MENU.write() = None,
        }
        div { "data-testid": "tab-menu", role: "menu", "aria-label": "{tab.label}",
            style: "position: fixed; z-index: 61; right: 76px; bottom: calc(env(safe-area-inset-bottom, 0px) + 76px); min-width: 208px; padding: 6px; border-radius: 14px; background: {t.sheet}; border: 1px solid {t.rule}; box-shadow: 0 2px 6px rgba(0,0,0,.12), 0 18px 40px -12px rgba(0,0,0,.45); display: flex; flex-direction: column; gap: 2px;",
            for entry in tab.menu.iter().cloned() {
                button { key: "{entry.id}", r#type: "button", role: "menuitem", "data-testid": "tab-menu-item",
                    "aria-current": if matches!(&now, Some(Open::View(_, v)) if *v == entry.view) { "page" },
                    style: "display: flex; align-items: center; gap: 12px; width: 100%; min-height: 46px; padding: 0 12px; border: 0; border-radius: 10px; background: transparent; color: {t.ink}; font: inherit; font-size: 16px; text-align: left; cursor: pointer;",
                    onclick: move |_| press_tab(entry.id),
                    span { style: "display: grid; place-items: center; width: 22px; height: 22px; color: {t.ink_3};",
                        {architect_shell::Tab::named(entry.id, entry.label, entry.glyph).draw(architect_shell::Icon { size: 20, color: t.ink_3.clone() })}
                    }
                    "{entry.label}"
                }
            }
        }
    }
}

/// The link in the shell's sidebar (a phone's drawer): what it is, who it
/// is for, its notes and its files. What the top of the page used to hold.
#[component]
pub(super) fn GuestSide() -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let Some(side) = SIDE() else {
        return rsx! {};
    };
    let whom = match side.link.audience.as_str() {
        "public" => "Public",
        "org" => "For an organization's members",
        "invited" => "For the people it was shared with",
        _ => "Anyone with the link",
    };
    let go = use_callback(move |next: Open| {
        choose_on_page(next);
        if architect_shell::drawer_open() {
            shell.toggle(architect_shell::Side::Left);
        }
    });
    let row = |on: bool| {
        format!(
            "display: block; width: 100%; text-align: left; border: 0; border-radius: 8px; padding: 9px 10px; min-height: 40px; font: inherit; font-size: 15px; cursor: pointer; color: {}; background: {}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
            t.ink,
            if on {
                t.pill.to_string()
            } else {
                "transparent".to_owned()
            }
        )
    };
    let files_on = side.at == Open::Files;
    rsx! {
        nav { "data-testid": "guest-side", "aria-label": "{side.link.title}",
            style: "display: flex; flex-direction: column; gap: 2px; padding: 14px 10px; overflow-y: auto; height: 100%; box-sizing: border-box; color: {t.ink};",
            div { style: "padding: 2px 10px 10px;",
                div { style: "font-size: 17px; font-weight: 650; line-height: 1.25;", "{side.link.title}" }
                div { style: "margin-top: 4px; font-size: 13px; color: {t.ink_3};", "Shared with you. {whom}." }
            }
            for note in side.link.notes.iter().cloned() {
                button { key: "{note.path}", r#type: "button", "data-testid": "guest-side-note",
                    style: row(side.at == Open::Note(note.path.clone())),
                    onclick: move |_| go.call(Open::Note(note.path.clone())),
                    "{note.title}"
                }
            }
            if side.files > 0 {
                button { r#type: "button", "data-testid": "guest-side-files",
                    style: "margin-top: 8px; {row(files_on)}",
                    onclick: move |_| go.call(Open::Files),
                    "Files ({side.files})"
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
fn GuestNoteView(
    code: String,
    path: String,
    #[props(default)] core_list: bool,
    /// The note's title is on the page already (the link's root, under the
    /// link's own heading): not drawn a second time.
    #[props(default)]
    untitled: bool,
) -> Element {
    let listing = super::frames::use_listing();
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
            // An extension claims this type and the node gave its view the
            // note's text: the view draws the note.
            let claimed = note.source.is_some()
                && listing
                    .read()
                    .as_ref()
                    .is_some_and(|l| crate::ext_host::claimed(l, &kind).is_some());
            let count = files.len();
            rsx! {
                article { "data-testid": "guest-note", "data-files": "{count}",
                    "data-text": "{note.source.is_some()}",
                    style: "display: flex; flex-direction: column; gap: 10px;",
                    if !untitled && !claimed {
                        h2 { style: "margin: 0; font-size: 20px;", "{note.title}" }
                    }
                    if claimed {
                        // An extension draws this type: its view is the
                        // note here, the height of the page. The note's
                        // words are what the view reads, not a second
                        // copy under it.
                        div { "data-testid": "guest-note-view", style: "display: flex; flex-direction: column; height: calc(100dvh - 232px); min-height: 320px;",
                            super::embed::NoteEmbed {
                                audience: Audience::Guest,
                                path: String::new(),
                                title: note.title.clone(),
                                note_type: kind.clone(),
                                properties: Vec::new(),
                                body: note.source.clone().unwrap_or_default(),
                                files: files.clone(),
                                pane: true,
                            }
                        }
                    } else if !kind.is_empty() {
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
                    if !claimed {
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
