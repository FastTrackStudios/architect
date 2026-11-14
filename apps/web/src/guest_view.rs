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
/// the note's title (and the file's name when it has several), its
/// subtitle (an artist) and, with one recording, its length, the note's
/// art, the file's vault path as its media (the bar names the link, the
/// source, after the subtitle). Only files the link serves (`served`).
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
            if let Some(o) = item.as_object_mut() {
                if let Some(art) = note.art.as_deref().filter(|a| served.contains(a)) {
                    o.insert("art".to_owned(), Value::from(art));
                }
                // Its artist (the card shows it before the link's name)
                // and, its one recording, its length.
                if let Some(sub) = note.subtitle.as_deref().filter(|s| !s.is_empty()) {
                    o.insert("subtitle".to_owned(), Value::from(sub));
                }
                if let (1, Some(ms)) = (media.len(), note.duration_ms.filter(|ms| *ms > 0)) {
                    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
                    o.insert("duration".to_owned(), Value::from(ms as f64 / 1000.0));
                }
            }
            items.push(item);
        }
    }
    items
}

/// The link's root note as a collection (#668).
///
/// Its sets, as the node listed them (`ListedNote::sets`: only notes this link lists), each a
/// row of `architect_media`'s list numbered through every set, with its
/// subtitle and length, its media (its `media`, else its first file that
/// plays) and its art, each only when the link serves it (`served`).
/// `None` when the root note is no collection.
#[must_use]
pub fn collection_rows<S: std::hash::BuildHasher>(
    link: &GuestLink,
    served: &HashSet<&str, S>,
) -> Option<Vec<(Option<String>, Vec<architect_media::Row>)>> {
    let root = root_note(link).filter(|r| !r.sets.is_empty())?;
    let by_path: HashMap<&str, &ListedNote> =
        link.notes.iter().map(|n| (n.path.as_str(), n)).collect();
    let mut n = 0_usize;
    let sections = root
        .sets
        .iter()
        .map(|set| {
            let rows = set
                .notes
                .iter()
                .filter_map(|p| by_path.get(p.as_str()))
                .map(|note| {
                    n = n.saturating_add(1);
                    let served = |f: &&String| served.contains(f.as_str());
                    let media = note
                        .media
                        .as_ref()
                        .filter(served)
                        .or_else(|| note.files.iter().filter(served).find(|f| plays(f)))
                        .cloned();
                    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
                    let duration = note.duration_ms.map(|ms| ms as f64 / 1000.0);
                    architect_media::Row {
                        id: format!("{}#{n}", root.path),
                        number: n,
                        item: architect_media::Item {
                            path: note.path.clone(),
                            title: note.title.clone(),
                            subtitle: note.subtitle.clone(),
                            duration,
                            media: None,
                            cover: None,
                        },
                        media,
                        art: note.art.as_ref().filter(served).cloned(),
                    }
                })
                .collect::<Vec<_>>();
            (set.name.clone(), rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect::<Vec<_>>();
    (!sections.is_empty()).then_some(sections)
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
            subtitle: None,
            duration_ms: None,
            media: None,
            sets: Vec::new(),
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
    fn a_queued_song_names_its_artist_and_its_one_recordings_length() {
        let mut link = gig();
        link.notes[1].subtitle = Some("Isaac Watts".into());
        link.notes[1].duration_ms = Some(151_500);
        link.notes[2].subtitle = Some("Traditional".into());
        link.notes[2].duration_ms = Some(90_000);
        let served: HashSet<&str> = ["Gig/joy.mp3", "Gig/noel.wav", "Gig/noel.pdf"].into();
        let q = queue(&link, &served);
        assert_eq!(q[0]["subtitle"], "Isaac Watts");
        assert_eq!(q[0]["duration"], 151.5);
        assert_eq!(q[1]["subtitle"], "Traditional");
        assert_eq!(q[1]["duration"], 90.0);
        // Two recordings of one note: whose length it is, nobody knows.
        link.notes[1].files.push("Gig/joy-live.mp3".into());
        let served: HashSet<&str> = ["Gig/joy.mp3", "Gig/joy-live.mp3"].into();
        let q = queue(&link, &served);
        assert_eq!(q.len(), 2);
        assert!(
            q.iter()
                .all(|i| i.get("duration").is_none() && i["subtitle"] == "Isaac Watts")
        );
    }

    #[test]
    fn a_collection_roots_sets_are_the_list_and_only_what_the_link_serves_plays() {
        let mut link = gig();
        link.notes[0].sets = vec![
            architect_spaces::ListedSet {
                name: Some("Opening".into()),
                notes: vec!["Gig/noel.md".into(), "Gig/joy.md".into()],
            },
            architect_spaces::ListedSet {
                name: Some("Encore".into()),
                notes: vec!["Gig/eve.md".into(), "Gig/elsewhere.md".into()],
            },
        ];
        link.notes[1].subtitle = Some("Choir".into());
        link.notes[1].duration_ms = Some(186_000);
        link.notes[1].media = Some("Gig/joy.mp3".into());
        let served: HashSet<&str> = ["Gig/joy.mp3", "Gig/art/joy.png"].into();
        let rows = collection_rows(&link, &served).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0.as_deref(), Some("Opening"));
        let titles: Vec<(usize, &str, bool)> = rows
            .iter()
            .flat_map(|(_, r)| {
                r.iter()
                    .map(|r| (r.number, r.item.title.as_str(), r.plays()))
            })
            .collect();
        // Noel's recording is not served; a note the link does not list
        // is no row.
        assert_eq!(
            titles,
            vec![(1, "Noel", false), (2, "Joy", true), (3, "Eve", false)]
        );
        let joy = &rows[0].1[1];
        assert_eq!(joy.id, "Gig/set.md#2");
        assert_eq!(joy.item.duration, Some(186.0));
        assert_eq!(joy.art.as_deref(), Some("Gig/art/joy.png"));
        let q = architect_media::collection::queue(&rows);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["subtitle"], "Choir");
        // A root with no sets is no collection.
        assert!(collection_rows(&gig(), &served).is_none());
    }

    #[test]
    fn sizes_read() {
        assert_eq!(size(12), "12 B");
        assert_eq!(size(1_200), "1.2 KB");
        assert_eq!(size(3_400_000), "3.4 MB");
    }
}
