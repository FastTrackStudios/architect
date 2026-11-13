//! What a share link's guest view shows (#646).
//!
//! Worked out from what the link's guest service gave
//! (`architect_spaces::ShareGuest`): which note a path opens, the list in
//! sets, the Now Playing queue, the files by note. No browser here:
//! `app::guest` draws it, tests check it natively.
//!
//! Nothing here names a note type. Sets come from the shape of the link's
//! root note (a heading over a list naming the link's other notes), so any
//! note laid out that way lists its notes in its sets.

use std::collections::{HashMap, HashSet};

use architect_spaces::{BlockKind, GuestLink, ListedFile, ListedNote, PlainBlock};
use serde_json::{Value, json};

/// What a path below a link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    /// A note, by its vault path.
    Note(String),
    /// The link's files (`_files/`).
    Files,
    /// Nothing the link lists now (a page it no longer has).
    Nothing,
}

/// What `rest` (below the link, from `/`) opens on `link`: `/_files/` its
/// files, a note's page that note, the root its root note
/// ([`root_note`]).
#[must_use]
pub fn open(link: &GuestLink, rest: &str) -> Open {
    let rest = rest.trim_start_matches('/');
    if rest.trim_end_matches('/') == "_files" {
        return Open::Files;
    }
    if rest.is_empty() {
        return root_note(link).map_or(Open::Nothing, |n| Open::Note(n.path.clone()));
    }
    let want = format!("{}{}", link.link, rest.trim_end_matches('/'));
    link.notes
        .iter()
        .find(|n| {
            n.page
                .as_deref()
                .is_some_and(|p| p.trim_end_matches('/') == want)
        })
        .map_or(Open::Nothing, |n| Open::Note(n.path.clone()))
}

/// The link's root note: the one at its own page, else the first it
/// lists with a page (a setlist shared with its songs: the setlist).
#[must_use]
pub fn root_note(link: &GuestLink) -> Option<&ListedNote> {
    link.notes
        .iter()
        .find(|n| n.page.as_deref() == Some(link.link.as_str()))
        .or_else(|| link.notes.iter().find(|n| n.page.is_some()))
}

/// A part of the list: a set's label (None: no label) and its notes, as
/// indexes into the link's notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub label: Option<String>,
    pub notes: Vec<usize>,
}

/// The link's notes in sets, from `outline`.
///
/// `outline` is the root note's blocks, `root` its path: each heading
/// followed by a list whose items name
/// notes of the link (by title) is a set of them, in the list's order.
/// The notes no set names follow, unlabelled, in the link's order; the
/// root note itself is in no set. With no set at all: one section, the
/// link's order.
#[must_use]
pub fn sections(link: &GuestLink, root: Option<&str>, outline: &[PlainBlock]) -> Vec<Section> {
    let by_title: HashMap<String, usize> = link
        .notes
        .iter()
        .enumerate()
        .filter(|(_, n)| Some(n.path.as_str()) != root)
        .map(|(i, n)| (fold(&n.title), i))
        .collect();
    let mut sets: Vec<Section> = Vec::new();
    let mut taken = HashSet::new();
    let mut label: Option<String> = None;
    for block in outline {
        match block.kind {
            BlockKind::Heading => label = Some(block.text.trim().to_owned()),
            BlockKind::List => {
                let notes: Vec<usize> = block
                    .items
                    .iter()
                    .filter_map(|item| named(&by_title, item))
                    .filter(|i| taken.insert(*i))
                    .collect();
                if notes.is_empty() {
                    continue;
                }
                match sets.last_mut() {
                    // Two lists under one heading: one set.
                    Some(last) if last.label == label => last.notes.extend(notes),
                    _ => sets.push(Section {
                        label: label.clone(),
                        notes,
                    }),
                }
            }
            _ => {}
        }
    }
    let rest: Vec<usize> = link
        .notes
        .iter()
        .enumerate()
        .filter(|(i, n)| Some(n.path.as_str()) != root && !taken.contains(i))
        .map(|(i, _)| i)
        .collect();
    if sets.iter().all(|s| s.label.is_none()) {
        // No set has a name: the link's own order, whole.
        let all = link
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| Some(n.path.as_str()) != root)
            .map(|(i, _)| i)
            .collect();
        return vec![Section {
            label: None,
            notes: all,
        }];
    }
    if !rest.is_empty() {
        sets.push(Section {
            label: None,
            notes: rest,
        });
    }
    sets
}

/// The note a list item names: its title, or its title then more
/// (`Joy – in D`).
fn named(by_title: &HashMap<String, usize>, item: &str) -> Option<usize> {
    let item = fold(item);
    if let Some(i) = by_title.get(&item) {
        return Some(*i);
    }
    by_title
        .iter()
        .filter(|(title, _)| {
            !title.is_empty()
                && item.starts_with(title.as_str())
                && item
                    .get(title.len()..)
                    .is_some_and(|after| after.starts_with([' ', ',', '(', '-', '–', '—', ':']))
        })
        .max_by_key(|(title, _)| title.len())
        .map(|(_, i)| *i)
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Whether a file plays (the app's audio element: audio and video).
#[must_use]
pub fn plays(path: &str) -> bool {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp3"
            | "m4a"
            | "aac"
            | "wav"
            | "ogg"
            | "oga"
            | "opus"
            | "flac"
            | "mp4"
            | "m4v"
            | "webm"
            | "mov"
    )
}

/// The Now Playing queue of the link's recordings.
///
/// Every note's playing files, in the link's order (a note's own in the order it names them),
/// each an item of the playhead's (`architect_playhead::model::Item`):
/// the note's title (and the file's name when it has several), the
/// note's art, the file's vault path as its media (the bar names the
/// link, the source, under it). Only files the link serves (`served`).
#[must_use]
pub fn queue<S: std::hash::BuildHasher>(link: &GuestLink, served: &HashSet<&str, S>) -> Vec<Value> {
    let mut items = Vec::new();
    for note in &link.notes {
        let media: Vec<&String> = note
            .files
            .iter()
            .filter(|f| plays(f) && served.contains(f.as_str()))
            .collect();
        for file in &media {
            let name = file.rsplit('/').next().unwrap_or(file);
            let title = if media.len() > 1 {
                format!("{}: {name}", note.title)
            } else {
                note.title.clone()
            };
            // Under the title the bar names the source: the link.
            let mut item = json!({
                "id": format!("{}#{file}", note.path),
                "title": title,
                "media": file,
            });
            if let (Some(art), Some(o)) = (
                note.art.as_deref().filter(|a| served.contains(a)),
                item.as_object_mut(),
            ) {
                o.insert("art".to_owned(), Value::from(art));
            }
            items.push(item);
        }
    }
    items
}

/// The link's files by note, in its order: each listed note that
/// attaches any, with them.
#[must_use]
pub fn files_by_note<'a>(
    link: &'a GuestLink,
    files: &'a [ListedFile],
) -> Vec<(&'a ListedNote, Vec<&'a ListedFile>)> {
    link.notes
        .iter()
        .map(|n| {
            (
                n,
                files
                    .iter()
                    .filter(|f| f.note == n.path)
                    .collect::<Vec<_>>(),
            )
        })
        .filter(|(_, f)| !f.is_empty())
        .collect()
}

/// A size in words (`1.2 MB`), in tenths.
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut whole = bytes;
    let mut tenth = 0;
    let mut unit = 0_usize;
    while whole >= 1000 && unit < 4 {
        tenth = (whole % 1000) / 100;
        whole /= 1000;
        unit = unit.saturating_add(1);
    }
    let name = UNITS.get(unit).copied().unwrap_or("TB");
    if unit == 0 {
        format!("{whole} {name}")
    } else {
        format!("{whole}.{tenth} {name}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn note(path: &str, title: &str, page: Option<&str>, files: &[&str]) -> ListedNote {
        ListedNote {
            path: path.into(),
            title: title.into(),
            kind: None,
            page: page.map(str::to_owned),
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            art: None,
        }
    }

    fn gig() -> GuestLink {
        let mut joy = note(
            "Gig/joy.md",
            "Joy",
            Some("/s/c/joy/"),
            &["Gig/joy.mp3", "Gig/art/joy.png"],
        );
        joy.art = Some("Gig/art/joy.png".into());
        GuestLink {
            link: "/s/c/".into(),
            title: "The gig".into(),
            listen: "full".into(),
            audience: "link".into(),
            notes: vec![
                note("Gig/set.md", "The gig", Some("/s/c/set/"), &[]),
                joy,
                note(
                    "Gig/noel.md",
                    "Noel",
                    Some("/s/c/noel/"),
                    &["Gig/noel.wav", "Gig/noel.pdf"],
                ),
                note("Gig/eve.md", "Eve", Some("/s/c/eve/"), &[]),
            ],
        }
    }

    fn heading(text: &str) -> PlainBlock {
        PlainBlock::new(BlockKind::Heading, text)
    }

    fn list(items: &[&str]) -> PlainBlock {
        let mut b = PlainBlock::new(BlockKind::List, "");
        b.items = items.iter().map(|i| (*i).to_owned()).collect();
        b
    }

    #[test]
    fn a_path_opens_its_note_the_root_its_root_and_files_its_files() {
        let link = gig();
        assert_eq!(open(&link, "/"), Open::Note("Gig/set.md".into()));
        assert_eq!(open(&link, "/joy/"), Open::Note("Gig/joy.md".into()));
        assert_eq!(open(&link, "/joy"), Open::Note("Gig/joy.md".into()));
        assert_eq!(open(&link, "/_files/"), Open::Files);
        assert_eq!(open(&link, "/gone/"), Open::Nothing);
        let mut at_root = gig();
        at_root.notes[2].page = Some("/s/c/".into());
        assert_eq!(open(&at_root, "/"), Open::Note("Gig/noel.md".into()));
    }

    #[test]
    fn a_root_note_of_headings_over_lists_sets_the_list() {
        let link = gig();
        let outline = [
            heading("Set 1"),
            list(&["Noel", "joy – in D", "Nobody"]),
            heading("Set 2"),
            list(&["Joy"]),
        ];
        let s = sections(&link, Some("Gig/set.md"), &outline);
        assert_eq!(
            s,
            [
                Section {
                    label: Some("Set 1".into()),
                    notes: vec![2, 1]
                },
                Section {
                    label: None,
                    notes: vec![3]
                },
            ]
        );
        // No heading: the link's order.
        let flat = sections(&link, Some("Gig/set.md"), &[list(&["Noel"])]);
        assert_eq!(
            flat,
            [Section {
                label: None,
                notes: vec![1, 2, 3]
            }]
        );
    }

    #[test]
    fn the_queue_is_the_links_recordings_in_its_order() {
        let link = gig();
        let served: HashSet<&str> = ["Gig/joy.mp3", "Gig/art/joy.png", "Gig/noel.wav"].into();
        let q = queue(&link, &served);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0]["media"], "Gig/joy.mp3");
        assert_eq!(q[0]["art"], "Gig/art/joy.png");
        assert!(q[0].get("subtitle").is_none());
        assert_eq!(q[1]["title"], "Noel");
        // A file the link does not serve (a layer above it): not queued.
        let less: HashSet<&str> = ["Gig/noel.wav"].into();
        assert_eq!(queue(&link, &less).len(), 1);
        assert!(queue(&link, &less)[0].get("art").is_none());
    }

    #[test]
    fn sizes_read() {
        assert_eq!(size(12), "12 B");
        assert_eq!(size(1_200), "1.2 KB");
        assert_eq!(size(3_400_000), "3.4 MB");
    }
}
