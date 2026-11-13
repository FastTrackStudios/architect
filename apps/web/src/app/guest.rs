//! A share link opened in the app as its guest (#629, #669): the first
//! step of #646, where `/s/<code>/` serves this app in guest mode.
//!
//! Internal for now: reached only at `/app/#share=<code>` (a fragment, so
//! the code never reaches a server's logs), linked from nowhere, and
//! `/s/<code>/` still serves the node's page. It is what #646 builds on:
//! the link's guest service on this origin (`ShareGuest`, through
//! `ShareGuestHttpClient`), the link's notes in its order, and a note drawn
//! by the view its type's island claims, as a guest (`Audience::Guest`).
//!
//! Everything a guest's view is handed comes from the link's node, never
//! from a vault: the note's text as the node gave it (none unless the
//! view reads the note), and only the files the node listed for it
//! (`GuestNote::files`). Each file is fetched below the link on this
//! origin (no credentials, no redirect), then checked here before it is
//! handed in: its URL is below this link, its size within the caps, its
//! bytes match its `sha256` when the node has one, and it is a kind a
//! view reads at a vault path (`ext_host::ViewFile::of`). A file that
//! fails any of it is left out.

use std::fmt::Write as _;
use std::rc::Rc;

use architect_shell::use_shell;
use architect_spaces::guest::ShareGuestHttpClient;
use architect_spaces::{GuestFault, GuestLink, GuestNote, ListedFile};
use dioxus::prelude::*;
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use web_sys::{RequestCredentials, RequestInit, RequestRedirect, Response};

use crate::ext_host::{Audience, ViewFile};

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
/// at most `max` bytes.
async fn bytes_of(url: &str, max: u64) -> Result<Vec<u8>, String> {
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
    let buffer = JsFuture::from(response.array_buffer().map_err(|_| "no body")?)
        .await
        .map_err(|_| "no body".to_owned())?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err("larger than a view is handed".to_owned());
    }
    Ok(bytes)
}

/// `file` of link `code`, fetched and checked as the module says.
async fn checked(code: &str, file: &ListedFile) -> Result<ViewFile, String> {
    use sha2::Digest as _;
    let below = format!(
        "{}/",
        architect_spaces::share::link_path(code).trim_end_matches('/')
    );
    let url = &file.url;
    if !url.starts_with(&below) || url.contains("/../") || url.contains('?') || url.contains('#') {
        return Err(format!("{}: not a file of this link", file.path));
    }
    if file.size > architect_spaces::view::VIEW_FILE_MAX {
        return Err(format!("{}: too large", file.path));
    }
    let bytes = bytes_of(url, architect_spaces::view::VIEW_FILE_MAX).await?;
    if let Some(want) = &file.sha256 {
        let got = sha2::Sha256::digest(&bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
        if !got.eq_ignore_ascii_case(want) {
            return Err(format!("{}: not the file the link listed", file.path));
        }
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

/// Link `code`, as its guest: its notes, and the one chosen drawn.
#[component]
pub fn GuestPage(code: String) -> Element {
    let t = use_shell().theme;
    let link = use_resource({
        let code = code.clone();
        move || {
            let code = code.clone();
            async move {
                let c = client().ok_or("no origin")?;
                c.link(code, String::new()).await.map_err(|e| said(&e))
            }
        }
    });
    let mut chosen = use_signal(|| None::<String>);
    let state = link.read().clone();
    let body = match state {
        None => rsx! { p { style: "color: {t.ink_3};", "Opening the link…" } },
        Some(Err(why)) => rsx! { p { "data-testid": "guest-problem", "{why}" } },
        Some(Ok(l)) => {
            let first = l
                .notes
                .iter()
                .find(|n| n.kind.is_some())
                .or_else(|| l.notes.first());
            let path = chosen().or_else(|| first.map(|n| n.path.clone()));
            rsx! {
                GuestNotes { link: Rc::new(l.clone()), chosen: path.clone(), on_choose: move |p| chosen.set(Some(p)) }
                if let Some(path) = path {
                    GuestNoteView { key: "{path}", code, path }
                }
            }
        }
    };
    rsx! {
        div { "data-testid": "guest-page",
            style: "flex: 1 1 0; min-height: 0; overflow: auto; padding: 20px 16px 96px; color: {t.ink};",
            div { style: "max-width: 760px; margin: 0 auto; display: flex; flex-direction: column; gap: 16px;",
                {body}
            }
        }
    }
}

/// The link's title and its notes, the chosen one marked.
#[component]
fn GuestNotes(
    link: Rc<GuestLink>,
    chosen: Option<String>,
    on_choose: EventHandler<String>,
) -> Element {
    let t = use_shell().theme;
    rsx! {
        h1 { style: "margin: 0; font-size: 22px;", "data-testid": "guest-title", "{link.title}" }
        nav { style: "display: flex; flex-wrap: wrap; gap: 6px;",
            for n in link.notes.iter().cloned() {
                button {
                    key: "{n.path}",
                    "data-guest-note": "{n.title}",
                    style: if chosen.as_deref() == Some(n.path.as_str()) {
                        format!("padding: 4px 10px; border-radius: 999px; border: 1px solid {}; background: {}; color: {};", t.accent, t.accent, t.accent_ink)
                    } else {
                        format!("padding: 4px 10px; border-radius: 999px; border: 1px solid {}; background: transparent; color: {};", t.rule, t.ink)
                    },
                    onclick: move |_| on_choose.call(n.path.clone()),
                    "{n.title}"
                }
            }
        }
    }
}

/// Note `path` of link `code`: the view its type's island claims, handed
/// what the node gave a guest, then its page's words.
#[component]
fn GuestNoteView(code: String, path: String) -> Element {
    let t = use_shell().theme;
    let note = use_resource({
        move || {
            let (code, path) = (code.clone(), path.clone());
            async move {
                let c = client().ok_or("no origin")?;
                let note = c
                    .read(code.clone(), String::new(), path)
                    .await
                    .map_err(|e| said(&e))?;
                let files = files_of(&code, &note).await;
                Ok::<_, String>((Rc::new(note), files))
            }
        }
    });
    let state = note.read().clone();
    match state {
        None => rsx! {},
        Some(Err(why)) => rsx! { p { "data-testid": "guest-problem", "{why}" } },
        Some(Ok((note, files))) => {
            let kind = note.kind.clone().unwrap_or_default();
            let count = files.len();
            rsx! {
                section { "data-testid": "guest-note", "data-files": "{count}",
                    "data-text": "{note.source.is_some()}",
                    h2 { style: "margin: 4px 0 8px; font-size: 18px;", "{note.title}" }
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
                    for (i, b) in note.blocks.iter().enumerate() {
                        if !b.text.is_empty() {
                            p { key: "{i}", style: "margin: 8px 0; color: {t.ink_2}; line-height: 1.6;", "{b.text}" }
                        }
                    }
                }
            }
        }
    }
}
