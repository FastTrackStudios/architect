//! The sandbox's Studio-shaped demo of decks (#441): a linked extension
//! with a setlist in the navigator (a provider: `SETLISTS_NAV_*`), each
//! song an item that loads onto a deck (`Open::Context`: a click onto the
//! deck on show, a ⌘-click or a middle-click onto a new one), and the
//! song's two views, which read their deck's context
//! (`use_deck_context`). Its `[[deck_templates]]` ships the "Song"
//! screenset every song deck starts in (`setlists.toml`, `song.json`).

use architect_extensions::linked::{Builder, Extension};
use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
use dioxus::prelude::*;

/// The context type a song loads as.
pub const SONG: &str = "song";

/// The demo vault's songs: `(path, text)`.
const SONGS: [(&str, &str); 3] = [
    (
        "Songs/Amazing Grace.md",
        include_str!("../demo-vault/Songs/Amazing Grace.md"),
    ),
    (
        "Songs/Slow River.md",
        include_str!("../demo-vault/Songs/Slow River.md"),
    ),
    (
        "Songs/Blue Moon Walk.md",
        include_str!("../demo-vault/Songs/Blue Moon Walk.md"),
    ),
];

/// Two setlists: `(id, title, the songs' paths in order)`.
const SETLISTS: [(&str, &str, &[&str]); 2] = [
    (
        "sunday",
        "Sunday Service",
        &[
            "Songs/Amazing Grace.md",
            "Songs/Slow River.md",
            "Songs/Blue Moon Walk.md",
        ],
    ),
    (
        "rehearsal",
        "Thursday Rehearsal",
        &["Songs/Blue Moon Walk.md", "Songs/Slow River.md"],
    ),
];

/// A song's properties and its body.
fn song(path: &str) -> Option<(Vec<(String, String)>, String)> {
    let (_, text) = SONGS.iter().find(|(p, _)| *p == path)?;
    let (props, body) = crate::note::frontmatter(text);
    Some((props, body.to_owned()))
}

fn prop(props: &[(String, String)], key: &str) -> Option<String> {
    props.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// A song's title: its `title`, else its file's name.
fn title_of(path: &str) -> String {
    song(path)
        .and_then(|(p, _)| prop(&p, "title"))
        .unwrap_or_else(|| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".md")
                .to_owned()
        })
}

/// The setlists, as the navigator shows them.
struct Setlists;

impl NavigatorProvider for Setlists {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        let mut snap = Snapshot::new("Setlists").ordered();
        for (id, title, songs) in SETLISTS {
            snap = snap.node(Node::new(id, title).icon("list-music"));
            for path in songs {
                let props = song(path).map(|(p, _)| p).unwrap_or_default();
                let mut item = Item::new(format!("{id}|{path}"), id, title_of(path))
                    .opens(Open::Context {
                        context_type: SONG.to_owned(),
                        id: (*path).to_owned(),
                        title: title_of(path),
                        folder: None,
                    })
                    .tag("music/songs");
                item.icon = Some("music".to_owned());
                // A song with no dates of its own: the setlist's day.
                let when = |key: &str| {
                    prop(&props, key)
                        .or_else(|| Some("2026-11-01".to_owned()))
                        .and_then(|d| architect_navigator_api::date_millis(&d))
                        .unwrap_or(0)
                };
                item = item.dates(when("created"), when("modified"));
                if let Some(key) = prop(&props, "key") {
                    item = item.property("key", key);
                }
                if let Some(tempo) = prop(&props, "tempo") {
                    item = item.property("tempo", tempo);
                }
                snap = snap.item(item);
            }
        }
        Box::pin(async move { Ok(snap) })
    }
}

/// The song on show in this deck: its path, when the deck is a song's.
fn deck_song() -> Option<(String, String)> {
    architect_workspace::use_deck_context()
        .filter(|c| c.kind == SONG)
        .map(|c| (c.id, c.title))
}

/// The song's chart: its body, as written.
#[allow(non_snake_case)]
fn SongView() -> Element {
    let Some((path, title)) = deck_song() else {
        return rsx! { div { style: "padding: 24px; opacity: 0.7;", "Load a song from a setlist." } };
    };
    let body = song(&path).map(|(_, b)| b).unwrap_or_default();
    rsx! {
        div { "data-song-chart": "{path}",
            style: "height: 100%; overflow: auto; padding: 20px 24px; box-sizing: border-box;",
            h2 { style: "margin: 0 0 12px; font-size: 20px;", "{title}" }
            pre { style: "margin: 0; white-space: pre-wrap; font-size: 13px; line-height: 1.5;", "{body}" }
        }
    }
}

/// The song's details: its properties.
#[allow(non_snake_case)]
fn InfoView() -> Element {
    let Some((path, title)) = deck_song() else {
        return rsx! { div { style: "padding: 24px; opacity: 0.7;", "No song" } };
    };
    let props = song(&path).map(|(p, _)| p).unwrap_or_default();
    rsx! {
        div { "data-song-info": "{path}",
            style: "height: 100%; overflow: auto; padding: 20px 24px; box-sizing: border-box;",
            h3 { style: "margin: 0 0 12px; font-size: 15px;", "{title}" }
            for (k, v) in props.into_iter().filter(|(k, _)| k != "title") {
                div { key: "{k}", style: "display: flex; gap: 12px; padding: 4px 0; font-size: 13px;",
                    span { style: "width: 90px; opacity: 0.6;", "{k}" }
                    span { "{v}" }
                }
            }
        }
    }
}

/// The extension, linked in by the sandbox.
#[derive(Debug, Clone, Copy, Default)]
pub struct SetlistsExtension;

impl Extension for SetlistsExtension {
    fn manifest(&self) -> &'static str {
        include_str!("setlists.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[("song.json", SONG_LAYOUT)]
    }

    fn build(&self, ext: &mut Builder) {
        ext.navigator("SETLISTS_NAV", Setlists);
        ext.view("song", SongView).view("info", InfoView);
    }
}

/// The "Song" screenset: the chart beside the song's details, the
/// navigator in the left dock.
const SONG_LAYOUT: &str = r#"{"windows":[{"id":"main",
  "center":{"split":{"axis":"row","children":[
    {"share":0.64,"node":{"group":{"tabs":[{"pane":"setlists-song","view":"setlists/song","title":"Chart"}]}}},
    {"share":0.36,"node":{"group":{"tabs":[{"pane":"setlists-info","view":"setlists/info","title":"Song"}]}}}]}},
  "docks":[{"side":"left","root":{"group":{"tabs":[{"pane":"files","view":"files","title":"Files"}]}}}]}],
  "active":"setlists-song"}"#;
