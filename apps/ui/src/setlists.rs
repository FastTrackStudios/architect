//! The sandbox's Studio-shaped demo of scope tabs (#441, #476): a linked
//! extension with a setlist in the navigator (a provider:
//! `SETLISTS_NAV_*`), each song an item that opens in a scope tab
//! (`Open::Context`: a click onto the one on show, or a new one from the
//! general tabs; a ⌘-click or a middle-click onto a new one), and the
//! song's two views, which read their scope tab's context
//! (`use_deck_context`). Its `[[deck_templates]]` ships the "Song"
//! screenset every song starts in (`setlists.toml`, `song.json`); its
//! `[[tab_kinds]]` make a song's tab wide (its title over its key and
//! tempo, in its colour) and a setlist's own view a custom tab, drawn by
//! [`setlist_tab`].

use architect_extensions::linked::{Builder, Extension};
use architect_navigator_api::{Item, NavigatorProvider, Node, Open, ProviderFuture, Snapshot};
use dioxus::prelude::*;

/// The context type a song loads as.
pub const SONG: &str = "song";

/// The custom tab kind a setlist's view is (`[[tab_kinds]] id`).
pub const SETLIST_TAB: &str = "setlist";

/// Each song's colour on its tab, as Studio gives a song one.
const SONG_COLOURS: [(&str, &str); 3] = [
    ("Songs/Amazing Grace.md", "#e8913a"),
    ("Songs/Slow River.md", "#3f8fd1"),
    ("Songs/Blue Moon Walk.md", "#7c5cff"),
];

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

/// When song `path`'s file in the vault on disk last changed.
fn file_mtime(path: &str) -> Option<architect_navigator_api::Millis> {
    let vault = crate::notes::open_vault()?;
    let at = std::fs::metadata(vault.root().join(path))
        .ok()?
        .modified()
        .ok()?;
    let ms = at.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    i64::try_from(ms).ok()
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
                // What its scope tab says: its key, its tempo, its colour.
                let mut tab: Vec<(String, String)> = [
                    ("key", prop(&props, "key")),
                    (
                        "tempo",
                        prop(&props, "tempo").or_else(|| prop(&props, "bpm")),
                    ),
                ]
                .into_iter()
                .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
                .collect();
                if let Some((_, c)) = SONG_COLOURS.iter().find(|(p, _)| p == path) {
                    tab.push(("color".to_owned(), (*c).to_owned()));
                }
                let mut item = Item::new(format!("{id}|{path}"), id, title_of(path))
                    .opens(Open::Context {
                        context_type: SONG.to_owned(),
                        id: (*path).to_owned(),
                        title: title_of(path),
                        folder: None,
                        properties: tab,
                    })
                    .tag("music/songs");
                item.icon = Some("music".to_owned());
                // Its own dates, else its file's on disk; none (no date
                // shown) in the sandbox without a vault on disk.
                let mtime = file_mtime(path);
                let when = |key: &str| {
                    prop(&props, key)
                        .and_then(|d| architect_navigator_api::date_millis(&d))
                        .or(mtime)
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

/// A setlist's songs, in order: the custom tab's view. Which setlist is
/// its pane's id (`setlist-<id>`, `WORKSPACE_OPEN {view:
/// "setlists/setlist", id: "setlist-sunday"}`).
#[allow(non_snake_case)]
fn SetlistView() -> Element {
    let pane = architect_workspace::use_pane();
    let id = pane
        .id
        .0
        .strip_prefix("setlist-")
        .unwrap_or("sunday")
        .to_owned();
    let Some((_, title, songs)) = SETLISTS.iter().find(|(s, _, _)| *s == id) else {
        return rsx! { div { style: "padding: 24px; opacity: 0.7;", "No such setlist" } };
    };
    rsx! {
        div { "data-setlist": "{id}",
            style: "height: 100%; overflow: auto; padding: 20px 24px; box-sizing: border-box;",
            h2 { style: "margin: 0 0 12px; font-size: 20px;", "{title}" }
            for (i, path) in songs.iter().enumerate() {
                div { key: "{path}", style: "display: flex; gap: 12px; padding: 6px 0; font-size: 13px;",
                    span { style: "width: 20px; opacity: 0.5;", "{i + 1}" }
                    span { style: "flex: 1;", "{title_of(path)}" }
                    span { style: "opacity: 0.6;",
                        {song(path).and_then(|(p, _)| prop(&p, "key")).unwrap_or_default()}
                    }
                }
            }
        }
    }
}

/// How a setlist's custom tab draws (its kind's renderer, #476): the
/// setlist glyph in its colour, its title, and how many songs it has.
#[must_use]
pub fn setlist_tab(ctx: architect_workspace::TabRenderCtx) -> Element {
    let t = ctx.theme;
    let songs = SETLISTS
        .iter()
        .find(|(_, title, _)| *title == ctx.face.title)
        .map_or(0, |(_, _, s)| s.len());
    let colour = ctx
        .face
        .colour
        .clone()
        .unwrap_or_else(|| t.accent.to_string());
    let weight = if ctx.on { 600 } else { 500 };
    rsx! {
        span { "data-setlist-tab": "{ctx.face.title}", style: "flex: 0 0 auto; display: flex;",
            architect_shell::lucide::LucideIcon { name: "list-music".to_owned(), size: 14, color: Some(colour.clone()) }
        }
        span { style: "flex: 1 1 auto; min-width: 0; overflow: hidden; white-space: nowrap; font-size: 12px; font-weight: {weight};",
            "{ctx.face.title}"
        }
        span { style: "flex: 0 0 auto; padding: 1px 7px; border-radius: 999px; font-size: 10px; font-weight: 700; \
                       background: {colour}; color: #fff;",
            "{songs} songs"
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
        ext.view("song", SongView)
            .view("info", InfoView)
            .view("setlist", SetlistView);
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
