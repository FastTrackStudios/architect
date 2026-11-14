//! A collection note's native list in the web app (#668): a note whose
//! frontmatter lists media items (`sets`/`songs`, `sections`/`items`,
//! `architect_media::Collection`) draws as `architect_media::ui::MediaList`
//! at its head, unless an extension's island claims its type (then the
//! island's view draws instead, `super::embed::NoteEmbed`).
//!
//! A member's rows come from their own vault (the demo's, or an org's):
//! each link resolved among the vault's notes, each item's media and cover
//! among its files. Media the app can play without the file being in the
//! vault's listing (the demo's recordings, `super::audio::media_url`)
//! plays too. The list is the app's own source on the playhead,
//! [`SOURCE`].

use architect_media::{Collection, Item, Row, front};
use architect_navigator::NavigatorSource as _;
use dioxus::prelude::*;
use serde_json::{Map, Value};

use super::vault::WebVault;

/// The app's own source for a collection's list: outside the
/// extension-id grammar (`:`), so no extension can hold it
/// (`architect_playhead::deck::reserved`).
pub const SOURCE: &str = "app:list";

/// The list collection note `path` (frontmatter `fields`) makes in
/// `vault`: its sections and rows. `None` when it is no collection.
#[must_use]
pub fn member_rows(
    vault: &architect_navigator::MemorySource,
    path: &str,
    fields: &Map<String, Value>,
) -> Option<Vec<(Option<String>, Vec<Row>)>> {
    let list = Collection::read(fields)?;
    let files: Vec<String> = vault.files().into_iter().map(|f| f.path).collect();
    let notes = files
        .iter()
        .map(String::as_str)
        .filter(|p| p.ends_with(".md"));
    let resolved = list.resolve(path, notes);
    Some(resolved.rows(|p| {
        let text = vault.read_text(p)?;
        let mut row = Row::of(
            Item::read(p, &front::fields(&text)),
            files.iter().map(String::as_str),
        );
        if row.media.is_none() {
            // A recording the app serves itself (the demo's).
            row.media = row
                .item
                .media
                .clone()
                .filter(|m| super::audio::media_url(m).is_some());
        }
        Some(row)
    }))
}

/// Whether the note with frontmatter `fields` draws as the core list:
/// it is a collection.
#[must_use]
pub fn is_collection(fields: &Map<String, Value>) -> bool {
    Collection::read(fields).is_some()
}

/// The core list of collection note `path` (its text `text`, its title
/// `title`), at its head. On an org's vault, the notes it lists that the
/// app has not read yet are read from the node, and the list fills in.
#[component]
pub fn CollectionHead(path: String, title: String, text: String) -> Element {
    let vault = use_context::<WebVault>();
    let art = try_use_context::<super::embed::ArtSource>().map(|a| a.0);
    let workspace = try_use_context::<super::vault::VaultWorkspace>();
    let mut read = use_signal(|| 0_u32);
    let fields = front::fields(&text);
    // The listed notes not read yet: read from the node once.
    {
        let (vault, path, fields) = (vault.clone(), path.clone(), fields.clone());
        use_hook(move || {
            let Some(org) = vault.org else { return };
            let Some(list) = Collection::read(&fields) else {
                return;
            };
            let source = vault.source.clone();
            let notes: Vec<String> = source
                .files()
                .into_iter()
                .map(|f| f.path)
                .filter(|p| p.ends_with(".md"))
                .collect();
            let missing: Vec<String> = list
                .resolve(&path, notes.iter().map(String::as_str))
                .sections
                .into_iter()
                .flat_map(|(_, e)| e.into_iter().filter_map(|e| e.path))
                .filter(|p| source.read_text(p).is_none())
                .collect();
            if missing.is_empty() {
                return;
            }
            spawn(async move {
                for p in missing {
                    if let Ok(t) = org.read(&p).await {
                        let now = super::vault::now();
                        source.set_text(
                            architect_navigator::markdown::note_from_markdown(&p, &t, now, now),
                            t,
                        );
                        read.with_mut(|n| *n = n.wrapping_add(1));
                    }
                }
            });
        });
    }
    let _ = read();
    let Some(sections) = member_rows(&vault.source, &path, &fields) else {
        return rsx! {};
    };
    let on_open =
        workspace.map(|w| EventHandler::new(move |p: String| super::vault::show(w.signal(), &p)));
    rsx! {
        architect_media::ui::MediaList {
            source: SOURCE.to_owned(),
            name: title,
            sections,
            art,
            on_open,
        }
    }
}

/// A collection note's words, read: its body's headings and paragraphs,
/// without the fences that repeat the list (`architect_media::is_listing_fence`,
/// such as Studio's ```` ```studio-setlist ````), which the list says
/// already. Any other fence as its text.
#[component]
pub fn ReadingBody(body: String) -> Element {
    let t = architect_shell::use_shell().theme;
    let blocks = crate::reading::blocks(&body);
    rsx! {
        div { "data-testid": "note-reading", style: "display: flex; flex-direction: column; gap: 10px; margin: 0 0 16px;",
            for (i, b) in blocks.into_iter().enumerate() {
                match b {
                    crate::reading::Reading::Heading(level, text) => {
                        let size = 26_u8.saturating_sub(level.saturating_mul(3)).max(15);
                        rsx! { div { key: "{i}", style: "font-size: {size}px; font-weight: 700; color: {t.ink}; margin-top: 4px;", "{text}" } }
                    }
                    crate::reading::Reading::Paragraph(text) => rsx! {
                        p { key: "{i}", style: "margin: 0; font-size: 15px; line-height: 1.6; color: {t.ink_2};", "{text}" }
                    },
                    crate::reading::Reading::Fence(text) => rsx! {
                        pre { key: "{i}", style: "margin: 0; padding: 10px 12px; border-radius: 8px; background: {t.pill}; font-size: 13px; white-space: pre-wrap; overflow-x: auto;", "{text}" }
                    },
                }
            }
        }
    }
}
