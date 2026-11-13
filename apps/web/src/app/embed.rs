//! A note's extension view, chosen by the note's type (#495): an island
//! that claims the note's `type` (its listing's `embeds`, from its
//! manifest's `[[embeds]] claims = { note_type = […] }`) draws that view
//! in a frame at the head of the note.
//!
//! The island is chosen from the listing alone (`ext_host::claimed`: the
//! first island listed that claims the type, its first claim of it), and
//! only that island's bundle is fetched; a note whose type nobody claims
//! fetches nothing.
//!
//! The view is given what `ext_host::note_props` lets through of the note
//! for who is viewing (`note`; `null` unless its claim and its extension
//! both name reading it) beside the playhead's state of its extension's
//! source, flat, as `architect.playhead` v0.1 gives a frame (#589:
//! `source`, `name`, `active`, `playing`, `time`, `at`, `rate`,
//! `duration`, `index`, `tracks`, `clock`, `item`, `queue`, `title`,
//! `subtitle`, `artist`, `sections`). It may set its source
//! (`PLAYHEAD_SET`), make one item current (`PLAYHEAD_ITEM`) and use the
//! playhead's intents on it.
//!
//! The source is the island's extension's own on the app's playhead
//! (`architect_playhead`): its asks go to the deck as that extension
//! (`frames`), and the Now Playing bar shows and moves the same source,
//! so a view and the bar drive each other. Nothing here knows what kind
//! of note it is: the claim chooses the view.
//!
//! A view whose extension declares `reads = ["files"]` is handed the
//! note's files too (#669, `ext_host::note_props`): a member's from their
//! own vault ([`member_files`]: what the note and the notes it links
//! reference), a guest's only those the link's node gave for the note
//! (`super::guest`), fetched and checked there.
//!
//! A favourite's `{view}` (#607) opens any view its extension's island
//! lists the same way, in a pane of its own ([`Embedded`] with `pane`),
//! with the same props: the note, and its source's playhead state.

// States are read with `value["key"]`: serde_json's index answers `Null`
// for what is missing, never a panic.
#![allow(clippy::indexing_slicing)]

use std::rc::Rc;

use dioxus::prelude::*;
use serde_json::{Value, json};

use architect_shell::use_shell;

use super::frames::{Bundle, ExtensionFrame, FrameTheme, use_bundle, use_listing};
use crate::ext_host::{self, Audience, Island, Note, ViewFile};

/// The theme a frame is handed: the shell's colours and its font.
fn frame_theme() -> FrameTheme {
    let t = use_shell().theme;
    let dark = architect_theme::use_resolved().scheme().is_dark();
    FrameTheme {
        vars: [
            ("--foreground", &t.ink),
            ("--muted-foreground", &t.ink_3),
            ("--background", &t.sheet),
            ("--accent", &t.accent),
            ("--accent-foreground", &t.accent_ink),
            ("--border", &t.rule),
            ("--font", &t.font),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_string()))
        .collect(),
        dark,
    }
}

/// Source `source`'s state on the deck, flat (#589), with the note: what
/// a view is handed. Before its extension set a source, the state of an
/// empty one.
fn props_of(source: &str, note: &Value) -> Value {
    let now = architect_playhead::now_ms();
    let mut s = architect_playhead::with(|d| d.state(source, now)).unwrap_or_else(|| {
        json!({
            "source": source, "name": "", "active": false, "playing": false,
            "time": 0, "at": now, "rate": 1, "duration": 0, "index": null,
            "tracks": 0, "clock": "silent", "item": null, "queue": [],
            "title": null, "subtitle": null, "artist": null, "sections": [],
        })
    });
    if let Some(o) = s.as_object_mut() {
        o.insert("note".to_owned(), note.clone());
    }
    s
}

/// The files a member's view of `island` on the note at `path` (of type
/// `note_type`) may read in `vault`: nothing unless its extension declares
/// reading files (`Island::reads_files_of`), else what the note and the
/// notes it links reference (`crate::view_files`).
#[must_use]
pub fn member_files(
    vault: &super::vault::WebVault,
    island: &Island,
    note_type: &str,
    path: &str,
) -> Vec<ViewFile> {
    use architect_navigator::NavigatorSource as _;
    if !island.reads_files_of(note_type) {
        return Vec::new();
    }
    let source = &vault.source;
    let Some(text) = source.read_text(path) else {
        return Vec::new();
    };
    let paths: Vec<String> = source.files().into_iter().map(|f| f.path).collect();
    let text_of = |p: &str| source.read_text(p);
    let bytes = |p: &str| {
        source
            .resource_url(p)
            .and_then(|u| crate::view_files::data_bytes(&u))
            .or_else(|| source.read_text(p).map(String::into_bytes))
    };
    crate::view_files::member_files(
        &crate::view_files::Vault {
            paths: &paths,
            text: &text_of,
            bytes: &bytes,
        },
        path,
        &text,
    )
}

/// The view that claims a note of type `note_type`, at the note's head;
/// nothing when no island claims it, or until its bundle is checked.
/// `audience` is who is viewing: what of the note the view is handed
/// depends on it (`ext_host::note_props`). `files` are a guest's, as the
/// link's node gave them and the app checked them; a member's are read
/// here from their vault.
#[component]
pub fn NoteEmbed(
    audience: Audience,
    path: String,
    title: String,
    note_type: String,
    properties: Vec<(String, String)>,
    body: String,
    #[props(default)] files: Vec<ViewFile>,
) -> Element {
    let listing = use_listing();
    let vault = try_consume_context::<super::vault::WebVault>();
    let files = use_memo(use_reactive(
        (&audience, &path, &note_type, &body, &files),
        move |(audience, path, note_type, _body, given)| {
            if audience == Audience::Guest {
                return Rc::new(given);
            }
            let list = listing.read().clone();
            let found = list.as_ref().and_then(|l| ext_host::claimed(l, &note_type));
            Rc::new(match (found, vault.as_ref()) {
                (Some((island, _)), Some(v)) => member_files(v, island, &note_type, &path),
                _ => Vec::new(),
            })
        },
    ));
    let Some(list) = listing.read().clone() else {
        return rsx! {};
    };
    let Some((island, claim)) = ext_host::claimed(&list, &note_type) else {
        return rsx! {};
    };
    let files = files();
    let note = ext_host::note_props(
        audience,
        island,
        &Note {
            path: &path,
            title: &title,
            note_type: &note_type,
            properties: &properties,
            body: &body,
            files: &files,
        },
    );
    rsx! {
        Claimed {
            key: "{island.folder}/{claim.view}",
            island: island.clone(),
            view: claim.view.clone(),
            note,
        }
    }
}

/// `island`'s view `view`, once its bundle is fetched and checked.
#[component]
fn Claimed(island: Island, view: String, note: Value) -> Element {
    let bundle = use_bundle(island);
    let checked: Option<Rc<Bundle>> = bundle
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok().cloned());
    let Some(bundle) = checked else {
        return rsx! {};
    };
    rsx! { Embedded { bundle, view, note } }
}

/// View `view` of `bundle`'s island on `note`, its playhead source its
/// island's extension's on the deck: at a note's head, or filling a pane
/// of its own (`pane`, a favourite's `{view}`).
#[component]
pub fn Embedded(
    bundle: Rc<Bundle>,
    view: String,
    note: Value,
    #[props(default)] pane: bool,
) -> Element {
    let note = use_memo(use_reactive((&note,), |(n,)| n));
    let source = bundle.island.id.clone();
    let mut state = use_signal({
        let source = source.clone();
        move || props_of(&source, &note.peek())
    });
    // On every change of the deck, and ten times a second while it plays.
    {
        let source = source.clone();
        use_future(move || {
            let source = source.clone();
            async move {
                let mut seen = u64::MAX;
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(100)).await;
                    let rev = architect_playhead::revision();
                    let playing = state.peek()["playing"] == json!(true);
                    if playing || rev != seen {
                        seen = rev;
                        state.set(props_of(&source, &note.peek()));
                    }
                }
            }
        });
    }
    // A new note: handed at once.
    {
        let source = source.clone();
        use_effect(move || state.set(props_of(&source, &note())));
    }
    let props = use_memo(move || state());
    let theme = frame_theme();
    let s = state();
    let (testid, style) = if pane {
        (
            "ext-view-pane",
            "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; padding: 12px 16px;",
        )
    } else {
        ("note-embed", "")
    };
    let current = s["item"]["id"].as_str().unwrap_or_default().to_owned();
    let tracks = s["tracks"].as_u64().unwrap_or(0);
    let playing = s["playing"] == json!(true);
    let time = s["time"].as_f64().unwrap_or(0.0);
    rsx! {
        section { class: "web-note-embed", "data-testid": testid, "data-view": "{view}",
            "data-ext-view-of": "{view}", style,
            "data-source": "{source}",
            "data-queue": "{tracks}",
            "data-current": "{current}",
            "data-playing": "{playing}",
            "data-time": "{time:.1}",
            div { style: if pane { "flex: 1 1 0; min-height: 0; border-radius: 10px; overflow: hidden;" } else { "" },
                ExtensionFrame {
                    bundle, view: view.clone(), props, theme, fill: pane,
                    fallback: rsx! {},
                }
            }
        }
    }
}
