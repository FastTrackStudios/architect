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
//! `subtitle`, `artist`, `sections`), each item with `art` the app can
//! show given its image as `art_src`, a `data:` URL ([`with_art`]). It
//! may set its source
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
//! A view compiled into the app draws first: a linked extension that
//! claims the note's type (`super::extensions::linked_claims`,
//! `ext_host::drawn`) is mounted in the app's own document
//! ([`LinkedView`]), with no bundle fetched and no frame, and handed the
//! same note in the same shape through `linked::use_embed`: its `source`
//! the note's text, its `props` `{note}`. What it is handed follows what
//! its manifest declares it reads, as an island's does. It reads the
//! playhead itself (`architect_playhead::ui::use_playhead`) and asks it
//! as its own extension (`architect_playhead::ask`).
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
use crate::ext_host::{self, Audience, Drawn, Island, Note, ViewFile};

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

/// How the app shows an item's `art` (a vault path): the image as a
/// `data:` URL, or `None` where it has none to show. Provided by the app
/// beside the Now Playing card's own (`app::App`): the demo's vault in
/// demo mode, nothing signed in, so a frame and the card always agree.
#[derive(Clone, Copy)]
pub struct ArtSource(pub Callback<String, Option<String>>);

/// Source `source`'s state on the deck, flat (#589), with the note: what
/// a view is handed. Before its extension set a source, the state of an
/// empty one.
fn props_of(source: &str, note: &Value, art: Option<ArtSource>) -> Value {
    let now = architect_playhead::now_ms();
    let mut s = architect_playhead::with(|d| d.state(source, now)).unwrap_or_else(|| {
        json!({
            "source": source, "name": "", "active": false, "playing": false,
            "time": 0, "at": now, "rate": 1, "duration": 0, "index": null,
            "tracks": 0, "clock": "silent", "item": null, "queue": [],
            "title": null, "subtitle": null, "artist": null, "sections": [],
        })
    });
    if let Some(ArtSource(art)) = art {
        with_art(&mut s, |p| art.call(p.to_owned()));
    }
    if let Some(o) = s.as_object_mut() {
        o.insert("note".to_owned(), note.clone());
    }
    s
}

/// The files a member's view on the note at `path` may read in `vault`:
/// what the note and the notes it links reference (`crate::view_files`).
///
/// Gathered only for a view that reads files (`reads_files`: its island's
/// `Island::reads_files_of`, a linked extension's `ext_host::linked_reads`),
/// else none.
#[must_use]
pub fn member_files(
    vault: &super::vault::WebVault,
    reads_files: bool,
    path: &str,
) -> Vec<ViewFile> {
    use architect_navigator::NavigatorSource as _;
    if !reads_files {
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

/// Each item's art the app can show (`item` and every `queue` item with
/// `art`, a vault path) as `art_src`, an image's `data:` URL from
/// `resolve` (the app's [`ArtSource`]): a frame has no network and its
/// policy allows `img-src data:` only, so the app hands it the image
/// itself.
fn with_art(state: &mut Value, resolve: impl Fn(&str) -> Option<String>) {
    let add = |i: &mut Value| {
        let src = i["art"].as_str().and_then(&resolve);
        if let (Some(src), Some(o)) = (src, i.as_object_mut()) {
            o.insert("art_src".to_owned(), Value::from(src));
        }
    };
    if let Some(i) = state.get_mut("item") {
        add(i);
    }
    if let Some(q) = state.get_mut("queue").and_then(Value::as_array_mut) {
        q.iter_mut().for_each(add);
    }
}

/// The view that claims a note of type `note_type`, at the note's head;
/// nothing when no view claims it, or until an island's bundle is
/// checked. A linked extension's claim draws before an island's
/// (`ext_host::drawn`).
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
    /// The view fills the place it is given (the note is its view: a
    /// link's page), never a strip at the note's head.
    #[props(default)]
    pane: bool,
) -> Element {
    let listing = use_listing();
    let linked = super::extensions::use_linked_claims();
    let vault = try_consume_context::<super::vault::WebVault>();
    let files = use_memo(use_reactive(
        (&audience, &path, &note_type, &body, &files),
        move |(audience, path, note_type, _body, given)| {
            if audience == Audience::Guest {
                return Rc::new(given);
            }
            let list = listing.read().clone();
            let reads = ext_host::reads_of(
                &linked.read(),
                list.as_ref().map_or(&[][..], |l| l.as_slice()),
                &note_type,
            );
            Rc::new(
                vault
                    .as_ref()
                    .map(|v| member_files(v, reads.files, &path))
                    .unwrap_or_default(),
            )
        },
    ));
    let files = files();
    let note = Note {
        path: &path,
        title: &title,
        note_type: &note_type,
        properties: &properties,
        body: &body,
        files: &files,
    };
    let linked = linked();
    // A linked claim needs no listing: it draws while the node's loads,
    // and where there is no node at all.
    let list = listing.read().clone();
    let islands = list.as_ref().map_or(&[][..], |l| l.as_slice());
    match ext_host::drawn(&linked, islands, &note_type) {
        Some(Drawn::Linked(claim)) => {
            let handed = ext_host::linked_note_props(audience, claim, &note);
            // The text it reads, as its props have it (no comments);
            // none where its manifest reads no note.
            let source = handed["body"].as_str().unwrap_or_default().to_owned();
            rsx! {
                LinkedView {
                    key: "{claim.extension}/{claim.view}/{pane}",
                    extension: claim.extension.clone(),
                    view: claim.view.clone(),
                    claim: format!("note_type:{}", note_type.trim()),
                    source,
                    props: json!({ "note": handed }),
                    guest: audience == Audience::Guest,
                    pane,
                    max_height: claim.max_height,
                }
            }
        }
        Some(Drawn::Island(island, claim)) => {
            let note = ext_host::note_props(audience, island, &note);
            rsx! {
                Claimed {
                    // A frame is made as tall as its box or as it says once, at
                    // its mount: a view that becomes the page is mounted again.
                    key: "{island.folder}/{claim.view}/{pane}",
                    island: island.clone(),
                    view: claim.view.clone(),
                    note,
                    pane,
                }
            }
        }
        None => rsx! {},
    }
}

/// View `view` of linked extension `extension`, in the app's own document,
/// handed `claim`, `source` and `props` through `linked::use_embed`;
/// nothing when the extension is not running here as the app's own
/// (unloaded, or overridden by another copy of its id).
///
/// At a note's head it grows to `max_height` px and scrolls past it; with
/// `pane` it fills the place it is given.
///
/// `guest` is a share link's guest's content: drawn only where the app's
/// policy lets its own linked views draw it and its host is a guest's
/// (`architect_extensions::embeds::GuestEmbedPolicy`, set at the app's
/// root). What it is handed is the caller's to keep within the link's
/// permission; this decides only whether the view may draw at all.
#[component]
pub fn LinkedView(
    extension: String,
    view: String,
    claim: String,
    #[props(default)] source: String,
    #[props(default)] props: Value,
    #[props(default)] guest: bool,
    #[props(default)] pane: bool,
    #[props(default = architect_extensions::embeds::DEFAULT_MAX_HEIGHT)] max_height: u32,
) -> Element {
    use architect_extensions::contrib::EmbedInput;
    let now = EmbedInput {
        claim,
        source,
        props,
    };
    let mut input = use_signal(|| now.clone());
    use_effect(use_reactive!(|now| {
        if *input.peek() != now {
            input.set(now);
        }
    }));
    let policy = try_consume_context::<architect_extensions::embeds::GuestEmbedPolicy>();
    let Some(ui) = try_use_context::<architect_extensions::interface::ExtensionUi>() else {
        return rsx! {};
    };
    let _ = ui.revision();
    let host = ui.host();
    if guest && !policy.unwrap_or_default().draws_linked(&host) {
        return rsx! {};
    }
    if host.linked_view(&extension, &view).is_none() {
        return rsx! {};
    }
    let (testid, style) = if pane {
        (
            "linked-view-pane",
            "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; overflow: auto;"
                .to_owned(),
        )
    } else {
        // At a note's head: the same gap below it as a frame's view
        // leaves ([`Embedded`]).
        (
            "linked-note-embed",
            format!("margin: 0 0 20px; max-height: {max_height}px; overflow-y: auto;"),
        )
    };
    let id = format!("{extension}/{view}");
    rsx! {
        section { class: "web-note-embed", "data-testid": testid, "data-linked-view": "{id}",
            "data-view": "{view}", "data-guest": if guest { "true" }, style,
            // Keyed in a list: another view in this place is mounted
            // afresh, its hooks its own.
            for key in std::iter::once(id.clone()) {
                LinkedMount { key: "{key}", extension: extension.clone(), view: view.clone(), input }
            }
        }
    }
}

/// A linked view's component, mounted with its input in context.
#[component]
fn LinkedMount(
    extension: String,
    view: String,
    input: Signal<architect_extensions::contrib::EmbedInput>,
) -> Element {
    use_context_provider(|| architect_extensions::linked::EmbedContext(input));
    let root = try_use_context::<architect_extensions::interface::ExtensionUi>()
        .and_then(|ui| ui.host().linked_view(&extension, &view));
    root.map_or_else(|| rsx! {}, |root| root())
}

/// The claim a linked view is handed as a page of its own (a favourite's
/// view that draws no note): `linked::use_embed`'s `claim`.
pub const PAGE_CLAIM: &str = "embed_type:view";

/// The claim the home is handed.
pub const HOME_CLAIM: &str = "embed_type:home";

/// `island`'s view `view`, once its bundle is fetched and checked.
#[component]
pub(super) fn Claimed(
    island: Island,
    view: String,
    note: Value,
    #[props(default)] pane: bool,
) -> Element {
    let bundle = use_bundle(island);
    let checked: Option<Rc<Bundle>> = bundle
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok().cloned());
    let Some(bundle) = checked else {
        return rsx! {};
    };
    rsx! { Embedded { bundle, view, note, pane } }
}

/// View `view` of extension `extension`'s island, filling the page: a
/// favourite's tab whose view claims no note type (the lyrics of what
/// plays, its arrangement). It is handed where its source is and no note.
#[component]
pub fn IslandView(extension: String, view: String) -> Element {
    // The extension's own code draws it, when it is compiled in.
    let linked =
        try_use_context::<architect_extensions::interface::ExtensionUi>().is_some_and(|ui| {
            let _ = ui.revision();
            ui.host().linked_view(&extension, &view).is_some()
        });
    let islands = use_resource(super::frames::islands);
    if linked {
        return rsx! {
            div { "data-testid": "page-view", "data-view": "{view}",
                style: "display: flex; flex-direction: column; min-height: 70vh; margin: 0 -16px;",
                LinkedView {
                    extension, view: view.clone(), claim: PAGE_CLAIM.to_owned(),
                    props: json!({ "note": Value::Null }),
                    guest: true, pane: true,
                }
            }
        };
    }
    let found = islands
        .read()
        .as_ref()
        .and_then(|l| l.iter().find(|i| i.id == extension).cloned());
    match found {
        Some(island) => rsx! {
            div { "data-testid": "page-view", "data-view": "{view}",
                style: "display: flex; flex-direction: column; min-height: 70vh; margin: 0 -16px;",
                Claimed { island, view: view.clone(), note: Value::Null, pane: true }
            }
        },
        None => rsx! {},
    }
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
    let art = try_use_context::<ArtSource>();
    let source = bundle.island.id.clone();
    let mut state = use_signal({
        let source = source.clone();
        move || props_of(&source, &note.peek(), art)
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
                        state.set(props_of(&source, &note.peek(), art));
                    }
                }
            }
        });
    }
    // A new note: handed at once.
    {
        let source = source.clone();
        use_effect(move || state.set(props_of(&source, &note(), art)));
    }
    let props = use_memo(move || state());
    let theme = frame_theme();
    let s = state();
    let (testid, style) = if pane {
        (
            "ext-view-pane",
            "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column;",
        )
    } else {
        // At a note's head: the same gap below it as the core list
        // leaves (`architect_media::ui::MediaList`), so the property
        // table does not butt against the view.
        ("note-embed", "margin: 0 0 20px;")
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
