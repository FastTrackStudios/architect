//! A collection: a note listing media items in order, in named sections.
//!
//! On disk (frontmatter), in either of two spellings of one shape:
//!
//! ```yaml
//! sets:                      # or `sections`
//!   - name: Opening Set
//!     songs:                 # or `items`
//!       - "[[Songs/Sleigh Ride]]"
//! songs:                     # or `items`: a flat list
//!   - "[[Songs/Sleigh Ride]]"
//! ```
//!
//! `sets`/`songs` is what Studio's setlists write (studio#406);
//! `sections`/`items` the generic names. With sections, the flat list's
//! links that no section names follow them, unnamed (Studio writes the
//! flat list as every section's songs together, so nothing repeats).

use serde_json::{Map, Value};

use crate::item::{Item, stem};
use crate::link;

/// The fields that hold sections, generic name last.
pub const SECTIONS: [&str; 2] = ["sets", "sections"];
/// The fields that hold a flat list (and a section's list).
pub const ITEMS: [&str; 2] = ["songs", "items"];

/// A collection's list, as its frontmatter writes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Collection {
    pub sections: Vec<Section>,
}

/// One section: its name (`None`: the list has none) and its links in
/// order, as written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub name: Option<String>,
    pub links: Vec<String>,
}

impl Collection {
    /// The collection `fields` (a note's frontmatter) lists; `None` when
    /// it lists nothing.
    #[must_use]
    pub fn read(fields: &Map<String, Value>) -> Option<Self> {
        let mut sections: Vec<Section> = SECTIONS
            .iter()
            .find_map(|k| fields.get(*k).and_then(Value::as_array))
            .map(|list| list.iter().filter_map(section).collect())
            .unwrap_or_default();
        let flat = ITEMS
            .iter()
            .find_map(|k| fields.get(*k))
            .map(links)
            .unwrap_or_default();
        let named: Vec<&str> = sections
            .iter()
            .flat_map(|s| s.links.iter().map(|l| key(l)))
            .collect();
        let rest: Vec<String> = flat
            .into_iter()
            .filter(|l| !named.contains(&key(l)))
            .collect();
        if !rest.is_empty() {
            sections.push(Section {
                name: None,
                links: rest,
            });
        }
        sections.retain(|s| !s.links.is_empty());
        (!sections.is_empty()).then_some(Self { sections })
    }

    /// Of [`SECTIONS`] and [`ITEMS`], those `fields` has: the property
    /// rows a view that draws the list leaves out.
    #[must_use]
    pub fn fields_in(fields: &Map<String, Value>) -> Vec<&'static str> {
        SECTIONS
            .iter()
            .chain(ITEMS.iter())
            .copied()
            .filter(|k| fields.contains_key(*k))
            .collect()
    }

    /// How many links it lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sections.iter().map(|s| s.links.len()).sum()
    }

    /// Whether it lists nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Its links resolved among `notes` (the paths the caller may read:
    /// a member's vault, a guest's listed notes), from the collection's
    /// note at `from`. A link to anything else stays unresolved.
    pub fn resolve<'a, I>(&self, from: &str, notes: I) -> Resolved
    where
        I: IntoIterator<Item = &'a str>,
        I::IntoIter: Clone,
    {
        let notes = notes.into_iter();
        let folder = link::folder_of(from);
        let sections = self
            .sections
            .iter()
            .map(|s| {
                let entries = s
                    .links
                    .iter()
                    .map(|l| Entry {
                        link: l.clone(),
                        label: link::alias(l),
                        path: link::resolve(l, folder, notes.clone()).map(str::to_owned),
                    })
                    .collect();
                (s.name.clone(), entries)
            })
            .collect();
        Resolved {
            from: from.to_owned(),
            sections,
        }
    }
}

/// A link as it compares: its target, else as written.
fn key(l: &str) -> &str {
    let l = l.trim().trim_start_matches('!');
    l.strip_prefix("[[")
        .and_then(|w| w.strip_suffix("]]"))
        .map_or(l, |w| w.split(['|', '#']).next().unwrap_or(w))
        .trim()
}

/// A section from `{name, songs|items}`; a bare list is one with no name.
fn section(v: &Value) -> Option<Section> {
    match v {
        Value::Object(o) => Some(Section {
            name: o
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned),
            links: ITEMS
                .iter()
                .find_map(|k| o.get(*k))
                .map(links)
                .unwrap_or_default(),
        }),
        Value::Array(_) => Some(Section {
            name: None,
            links: links(v),
        }),
        _ => None,
    }
}

/// A list's links: its strings, trimmed, the empty ones left out.
fn links(v: &Value) -> Vec<String> {
    match v {
        Value::Array(list) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_owned()],
        _ => Vec::new(),
    }
}

/// One link of a collection, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// As written.
    pub link: String,
    /// The link's alias, when it has one.
    pub label: Option<String>,
    /// The note it names among the paths given; `None` when none.
    pub path: Option<String>,
}

/// A collection with its links resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The collection's own note.
    pub from: String,
    pub sections: Vec<(Option<String>, Vec<Entry>)>,
}

impl Resolved {
    /// Its rows, numbered from 1 through every section: each entry's item
    /// from `item_of` (its note's, with its media and cover resolved to
    /// paths the caller can play and show), or, for a link that names no
    /// note it can read, its alias or name with nothing to play.
    pub fn rows(&self, item_of: impl Fn(&str) -> Option<Row>) -> Vec<(Option<String>, Vec<Row>)> {
        let mut n = 0_usize;
        self.sections
            .iter()
            .map(|(name, entries)| {
                let rows = entries
                    .iter()
                    .map(|e| {
                        n = n.saturating_add(1);
                        let mut row = e.path.as_deref().and_then(&item_of).unwrap_or_else(|| {
                            let title = e.label.clone().unwrap_or_else(|| {
                                link::target(&e.link)
                                    .map_or_else(|| e.link.clone(), |t| stem(&t).to_owned())
                            });
                            Row {
                                item: Item {
                                    title,
                                    ..Item::default()
                                },
                                ..Row::default()
                            }
                        });
                        row.number = n;
                        row.id = format!("{}#{n}", self.from);
                        row
                    })
                    .collect();
                (name.clone(), rows)
            })
            .collect()
    }
}

/// One row of a collection's list: its item and what of it the caller
/// can play and show.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    /// Its id on the playhead: the collection's path and its number.
    pub id: String,
    /// Its number in the list, from 1.
    pub number: usize,
    /// The item (its note's path empty when the link names none).
    pub item: Item,
    /// Its media, a vault path the caller can play; `None`: nothing to
    /// play (the row is greyed).
    pub media: Option<String>,
    /// Its cover, a vault path the caller can show.
    pub art: Option<String>,
}

impl Row {
    /// The row of item `item`, its media and art resolved among `files`.
    pub fn of<'a, I>(item: Item, files: I) -> Self
    where
        I: IntoIterator<Item = &'a str>,
        I::IntoIter: Clone,
    {
        let files = files.into_iter();
        Self {
            media: item.media_in(files.clone()).map(str::to_owned),
            art: item.cover_in(files).map(str::to_owned),
            item,
            ..Self::default()
        }
    }

    /// Whether it has something to play.
    #[must_use]
    pub const fn plays(&self) -> bool {
        self.media.is_some()
    }

    /// The row as a playhead item.
    #[must_use]
    pub fn playhead_item(&self) -> Value {
        self.item
            .playhead_item(&self.id, self.media.as_deref(), self.art.as_deref())
    }
}

/// The playhead queue of `rows`: every row that plays, in list order.
#[must_use]
pub fn queue(rows: &[(Option<String>, Vec<Row>)]) -> Vec<Value> {
    rows.iter()
        .flat_map(|(_, r)| r.iter())
        .filter(|r| r.plays())
        .map(Row::playhead_item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front;

    const DEMO: &str = "---\ntitle: Merry & Bright\ntype: setlist\nsets:\n  - name: Opening Set\n    songs:\n    - \"[[Songs/A/A|A]]\"\n    - \"[[Songs/B/B|B]]\"\n  - name: Concert Set\n    songs:\n    - \"[[Songs/C/C|C]]\"\nsongs:\n  - \"[[Songs/A/A|A]]\"\n  - \"[[Songs/B/B|B]]\"\n  - \"[[Songs/C/C|C]]\"\n  - \"[[Songs/D/D|D]]\"\n---\n";

    #[test]
    fn studios_named_sets_read_with_the_flat_lists_rest_after() {
        let c = Collection::read(&front::fields(DEMO)).unwrap();
        assert_eq!(c.sections.len(), 3);
        assert_eq!(c.sections[0].name.as_deref(), Some("Opening Set"));
        assert_eq!(c.sections[0].links.len(), 2);
        assert_eq!(c.sections[1].name.as_deref(), Some("Concert Set"));
        // D is in the flat list only: after the sets, unnamed.
        assert_eq!(c.sections[2].name, None);
        assert_eq!(c.sections[2].links, vec!["[[Songs/D/D|D]]".to_owned()]);
        assert_eq!(c.len(), 4);
        assert_eq!(
            Collection::fields_in(&front::fields(DEMO)),
            vec!["sets", "songs"]
        );
    }

    #[test]
    fn the_generic_names_and_a_flat_list_read_the_same() {
        let c = Collection::read(&front::fields(
            "---\ntype: playlist\nsections:\n  - name: Side A\n    items: [\"[[One]]\", \"[[Two]]\"]\n---\n",
        ))
        .unwrap();
        assert_eq!(c.sections[0].name.as_deref(), Some("Side A"));
        assert_eq!(c.len(), 2);
        let flat = Collection::read(&front::fields(
            "---\nitems:\n  - \"[[One]]\"\n  - Two.md\n---\n",
        ))
        .unwrap();
        assert_eq!(flat.sections.len(), 1);
        assert_eq!(flat.sections[0].name, None);
        assert_eq!(flat.len(), 2);
    }

    #[test]
    fn a_note_with_no_list_is_no_collection() {
        for text in [
            "---\ntitle: x\ntype: song\n---\n",
            "---\nsongs: []\n---\n",
            "---\nsets:\n  - name: Empty\n---\n",
            "no frontmatter",
        ] {
            assert_eq!(Collection::read(&front::fields(text)), None, "{text}");
        }
    }

    #[test]
    fn rows_resolve_number_and_queue_only_what_plays() {
        let c = Collection::read(&front::fields(DEMO)).unwrap();
        let notes = ["Songs/A/A.md", "Songs/B/B.md", "Songs/C/C.md"];
        let r = c.resolve("Setlists/M.md", notes);
        assert_eq!(r.sections[2].1[0].path, None, "D is not in the vault");
        let files = ["Rec/A.mp3", "Songs/A/cover.png"];
        let rows = r.rows(|path| {
            let text = match path {
                "Songs/A/A.md" => "---\ntitle: A\nartist: X\nrecording: Rec/A.mp3\ncover: cover.png\nduration: 60\n---\n",
                "Songs/B/B.md" => "---\ntitle: B\n---\n",
                _ => "---\ntitle: C\nrecording: Rec/missing.mp3\n---\n",
            };
            Some(Row::of(Item::read(path, &front::fields(text)), files))
        });
        let flat: Vec<&Row> = rows.iter().flat_map(|(_, r)| r).collect();
        assert_eq!(flat.len(), 4);
        assert_eq!(flat[0].number, 1);
        assert_eq!(flat[3].number, 4);
        assert_eq!(flat[3].item.title, "D", "the alias of a link to nothing");
        assert_eq!(flat[0].id, "Setlists/M.md#1");
        assert!(flat[0].plays());
        assert_eq!(flat[0].art.as_deref(), Some("Songs/A/cover.png"));
        // A recording the vault does not have plays nothing.
        assert!(!flat[2].plays());
        let q = queue(&rows);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["media"], "Rec/A.mp3");
        assert_eq!(q[0]["subtitle"], "X");
    }

    #[test]
    fn a_guest_resolves_only_the_listed_notes() {
        let c = Collection::read(&front::fields(DEMO)).unwrap();
        // The share lists the setlist and A: B, C and D are outside it.
        let r = c.resolve("Setlists/M.md", ["Setlists/M.md", "Songs/A/A.md"]);
        let found: Vec<Option<&str>> = r
            .sections
            .iter()
            .flat_map(|(_, e)| e.iter().map(|e| e.path.as_deref()))
            .collect();
        assert_eq!(found, vec![Some("Songs/A/A.md"), None, None, None]);
    }
}
