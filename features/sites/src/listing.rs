//! What a share link lists (#510): the notes of a [`Scope`] a filter
//! holds for, in list order, each with the files it attaches; grouped
//! three ways; and laid out as the folders of an archive.
//!
//! The filter is a Bases filter expression ([`parse_filter`]:
//! `instrument == "violin"`, `file.hasTag("rhythm")`), evaluated by the
//! Bases engine over the vault's rows, so a filter on a share reads as
//! it does in a `.base`. There is no second filter language here.
//!
//! List order is the scope's: a note list's own order, a base's rows in
//! its views' order, otherwise by path. A file is listed under the note
//! that links or embeds it (attachments carry no properties of their
//! own), so a filter over notes is a filter over their files.

use std::collections::{HashMap, HashSet};

use vault::bases::{BaseEval, FilterNode};
use vault::obsidian::LinkIndex;
/// A vault read from its folder: what [`select`] lists from.
pub use vault::obsidian::Vault;
use vault::obsidian::base_query::vault_to_rows;
/// A vault file opened never through a symlink: what a share's archive
/// reads.
pub use vault::obsidian::open_own;

use crate::frontmatter::Meta;
use crate::scope::Scope;
use crate::site::title_of;

/// Parse a Bases filter expression; an empty one holds for every note.
///
/// # Errors
///
/// Not a filter expression (the Bases parser's message).
pub fn parse_filter(src: &str) -> Result<FilterNode, String> {
    let src = src.trim();
    if src.is_empty() {
        return Ok(FilterNode::None);
    }
    vault::bases::expr_parser::parse_filter(src).map_err(|e| e.to_string())
}

/// Read the vault at `root`, for [`select`].
///
/// # Errors
///
/// The folder could not be read.
pub fn open(root: &std::path::Path) -> Result<Vault, String> {
    Vault::open(root).map_err(|e| e.to_string())
}

/// [`open`], and what of the vault could not be read (notes that did not
/// load, entries the walk skipped), one line each.
///
/// # Errors
///
/// The folder could not be read.
pub fn open_reporting(root: &std::path::Path) -> Result<(Vault, Vec<String>), String> {
    Vault::open_reporting(root).map_err(|e| e.to_string())
}

/// One listed note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its vault path.
    pub path: String,
    pub title: String,
    /// Its frontmatter.
    pub properties: serde_json::Map<String, serde_json::Value>,
    /// The attachments it links or embeds, by vault path, in the order
    /// it names them, once each; its art ([`crate::art`]) last.
    pub files: Vec<String>,
    /// Its art, by vault path: one of `files` ([`crate::art`]).
    pub art: Option<String>,
}

impl Entry {
    /// Its art while it is still one of its files (a layer above the
    /// link's takes it out of `files`, and its art with it).
    #[must_use]
    pub fn art(&self) -> Option<&str> {
        self.art
            .as_deref()
            .filter(|a| self.files.iter().any(|f| f == a))
    }
}

/// The notes of `scope` that `filter` holds for, in list order. Hidden
/// notes (`publish: false`, drafts) are never listed.
#[must_use]
pub fn select(vault: &Vault, scope: &Scope, filter: &FilterNode) -> Vec<Entry> {
    let rows = vault_to_rows(vault);
    let eval = BaseEval::over(&rows);
    let index = LinkIndex::build(vault);
    let attachments: HashSet<&str> = vault
        .attachments
        .iter()
        .map(|a| a.rel_path.as_str())
        .collect();
    let at: HashMap<&str, usize> = vault
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.rel_path.as_str(), i))
        .collect();
    scope
        .ordered(vault)
        .into_iter()
        .filter_map(|path| {
            let i = *at.get(path.as_str())?;
            let (page, row) = (vault.pages.get(i)?, rows.get(i)?);
            let meta = Meta::of(page);
            if meta.hidden || !eval.filter_matches(filter, row) {
                return None;
            }
            let mut files: Vec<String> = Vec::new();
            for link in index.outgoing(&path) {
                let file = link
                    .resolved
                    .filter(|t| attachments.contains(t) && scope.holds_file(t));
                if let Some(file) = file {
                    if !files.iter().any(|f| f == file) {
                        files.push(file.to_owned());
                    }
                }
            }
            if let Some(media) =
                media_file(&meta, &path, &attachments).filter(|m| scope.holds_file(m))
                && !files.iter().any(|f| f == media)
            {
                files.insert(0, media.to_owned());
            }
            let art = art_file(&meta, &path, &attachments).filter(|a| scope.holds_file(a));
            if let Some(art) = art {
                if !files.iter().any(|f| f == art) {
                    files.push(art.to_owned());
                }
            }
            Some(Entry {
                art: art.map(str::to_owned),
                title: title_of(page, &meta),
                properties: row
                    .frontmatter
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                path,
                files,
            })
        })
        .collect()
}

/// The attachment note `path`'s media (`media:`, else `recording:`, a
/// media item's: `architect_media::Item`) names, of `attachments`: a file
/// it plays, listed first among its files as its embeds are (#668).
fn media_file<'a>(meta: &Meta, path: &str, attachments: &HashSet<&'a str>) -> Option<&'a str> {
    let link = meta.media.as_deref()?;
    architect_media::link::resolve(
        link,
        architect_media::link::folder_of(path),
        attachments.iter().copied(),
    )
    .filter(|f| crate::media::media_kind(f).is_some())
}

/// The attachment note `path`'s art names, of `attachments`.
fn art_file<'a>(meta: &Meta, path: &str, attachments: &HashSet<&'a str>) -> Option<&'a str> {
    let link = meta.art.as_deref()?;
    crate::art::resolve(
        link,
        crate::art::folder_of(path),
        attachments.iter().copied(),
    )
}

/// Every note of `vault`, hidden ones too, in path order, each with
/// every attachment it links or embeds: what places a shared file in a
/// layer (#528), whatever a share's scope, filter or `publish:` shows.
#[must_use]
pub fn every_note(vault: &Vault) -> Vec<Entry> {
    let rows = vault_to_rows(vault);
    let index = LinkIndex::build(vault);
    let attachments: HashSet<&str> = vault
        .attachments
        .iter()
        .map(|a| a.rel_path.as_str())
        .collect();
    let mut out: Vec<Entry> = vault
        .pages
        .iter()
        .zip(&rows)
        .map(|(page, row)| {
            let mut files: Vec<String> = Vec::new();
            for link in index.outgoing(&page.rel_path) {
                if let Some(file) = link.resolved.filter(|t| attachments.contains(t)) {
                    if !files.iter().any(|f| f == file) {
                        files.push(file.to_owned());
                    }
                }
            }
            let meta = Meta::of(page);
            if let Some(media) = media_file(&meta, &page.rel_path, &attachments)
                && !files.iter().any(|f| f == media)
            {
                files.insert(0, media.to_owned());
            }
            let art = art_file(&meta, &page.rel_path, &attachments);
            if let Some(art) = art {
                if !files.iter().any(|f| f == art) {
                    files.push(art.to_owned());
                }
            }
            Entry {
                path: page.rel_path.clone(),
                title: title_of(page, &meta),
                art: art.map(str::to_owned),
                properties: row
                    .frontmatter
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                files,
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// How a listing is grouped: the views a share page switches between.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Grouping {
    /// One list, in list order.
    Order,
    /// A group per note, in list order.
    Note,
    /// A group per value of this property, in the order the values
    /// first come; a note with several values is in each one's group.
    Property(String),
}

impl Grouping {
    /// Parse `order`, `note` or `property:<key>` (empty: `order`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        match s {
            "" | "order" => Some(Self::Order),
            "note" => Some(Self::Note),
            _ => s
                .strip_prefix("property:")
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(|key| Self::Property(key.to_owned())),
        }
    }

    /// As [`Self::parse`] reads it.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Order => "order".to_owned(),
            Self::Note => "note".to_owned(),
            Self::Property(key) => format!("property:{key}"),
        }
    }
}

/// One file of a group: whose it is, and where an archive puts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    /// Its note, as an index into the listing.
    pub entry: usize,
    /// Its vault path.
    pub file: String,
    /// Its path in an archive: folders by the grouping, numbered in
    /// list order.
    pub archive: String,
}

/// One group of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The note's title, the property's value (`""`: no value), or `""`
    /// for the one group of [`Grouping::Order`].
    pub label: String,
    /// Its notes, as indexes into the listing, in list order.
    pub entries: Vec<usize>,
    /// Its files, in list order.
    pub files: Vec<Filed>,
}

/// `entries` grouped `by`, each file with its place in an archive:
///
/// - [`Grouping::Order`]: `01 Violin I.pdf`, numbered file by file;
/// - [`Grouping::Note`]: `01 Joy to the World/Violin I.pdf`, a folder
///   per note, numbered by the note's place in the list;
/// - [`Grouping::Property`]: `violin/01 Joy to the World/Violin I.pdf`,
///   a folder per value, its notes numbered by their place in it.
///
/// Two files that would land on one path get ` (2)`, ` (3)`.
#[must_use]
pub fn groups(entries: &[Entry], by: &Grouping) -> Vec<Group> {
    let all = || (0..entries.len()).collect::<Vec<usize>>();
    let mut out: Vec<Group> = match by {
        Grouping::Order => vec![Group {
            label: String::new(),
            entries: all(),
            files: Vec::new(),
        }],
        Grouping::Note => all()
            .into_iter()
            .map(|i| Group {
                label: entries.get(i).map(|e| e.title.clone()).unwrap_or_default(),
                entries: vec![i],
                files: Vec::new(),
            })
            .collect(),
        Grouping::Property(key) => {
            let mut found: Vec<Group> = Vec::new();
            for (i, entry) in entries.iter().enumerate() {
                for label in labels(entry.properties.get(key)) {
                    match found.iter_mut().find(|g| g.label == label) {
                        Some(group) => group.entries.push(i),
                        None => found.push(Group {
                            label,
                            entries: vec![i],
                            files: Vec::new(),
                        }),
                    }
                }
            }
            found
        }
    };
    let mut taken: HashSet<String> = HashSet::new();
    let files_in_all = entries.iter().map(|e| e.files.len()).sum::<usize>();
    let mut nth_file = 0_usize;
    for group in &mut out {
        let folder = match by {
            Grouping::Property(key) if group.label.is_empty() => format!("No {}/", clean(key)),
            Grouping::Property(_) => format!("{}/", clean(&group.label)),
            _ => String::new(),
        };
        let in_group = group.entries.len();
        for (nth, &i) in group.entries.iter().enumerate() {
            let Some(entry) = entries.get(i) else {
                continue;
            };
            for file in &entry.files {
                let name = clean(file.rsplit('/').next().unwrap_or(file));
                let title = clean(&entry.title);
                let wanted = match by {
                    Grouping::Order => {
                        nth_file = nth_file.saturating_add(1);
                        format!("{} {name}", numbered(nth_file, files_in_all))
                    }
                    Grouping::Note => {
                        format!(
                            "{} {title}/{name}",
                            numbered(i.saturating_add(1), entries.len())
                        )
                    }
                    Grouping::Property(_) => format!(
                        "{folder}{} {title}/{name}",
                        numbered(nth.saturating_add(1), in_group)
                    ),
                };
                group.files.push(Filed {
                    entry: i,
                    file: file.clone(),
                    archive: unique(&wanted, &mut taken),
                });
            }
        }
    }
    out
}

/// The property keys worth grouping `entries` by: the ones some note
/// gives a plain value (text, a number, yes or no, or a list of them),
/// by name; never `title`, `publish`, `draft` or `aliases`.
#[must_use]
pub fn property_keys(entries: &[Entry]) -> Vec<String> {
    const NEVER: [&str; 6] = [
        "title",
        "publish",
        "draft",
        "aliases",
        "alias",
        "cssclasses",
    ];
    let plain = |v: &serde_json::Value| match v {
        serde_json::Value::Array(items) => {
            !items.is_empty() && items.iter().all(|i| !i.is_object() && !i.is_array())
        }
        serde_json::Value::Object(_) | serde_json::Value::Null => false,
        _ => true,
    };
    let mut keys: Vec<String> = entries
        .iter()
        .flat_map(|e| e.properties.iter())
        .filter(|(k, v)| !NEVER.contains(&k.to_lowercase().as_str()) && plain(v))
        .map(|(k, _)| k.clone())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// A property's value as group labels: one per item of a list, `""`
/// for none. A `[[wikilink]]` reads as its name.
fn labels(value: Option<&serde_json::Value>) -> Vec<String> {
    let one = |v: &serde_json::Value| {
        let text = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        let text = text.trim();
        text.strip_prefix("[[")
            .and_then(|t| t.strip_suffix("]]"))
            .map(|t| t.rsplit('|').next().unwrap_or(t))
            .map_or(text, |t| t.rsplit('/').next().unwrap_or(t))
            .trim()
            .to_owned()
    };
    let mut out: Vec<String> = match value {
        Some(serde_json::Value::Array(items)) => items.iter().map(one).collect(),
        Some(v) => vec![one(v)],
        None => Vec::new(),
    };
    out.dedup();
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// `n` with as many digits as `of` has, at least two: `01`, `012`.
fn numbered(n: usize, of: usize) -> String {
    let width = of.to_string().len().max(2);
    format!("{n:0width$}")
}

/// A name safe as one segment of an archive path on any system.
fn clean(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "untitled".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// `path`, or `path (2)` before its extension when it is taken.
fn unique(path: &str, taken: &mut HashSet<String>) -> String {
    let mut candidate = path.to_owned();
    let mut n = 1_usize;
    while !taken.insert(candidate.to_lowercase()) {
        n = n.saturating_add(1);
        let (folder, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
        let (stem, ext) = file
            .rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map_or((file, String::new()), |(s, e)| (s, format!(".{e}")));
        let sep = if folder.is_empty() { "" } else { "/" };
        candidate = format!("{folder}{sep}{stem} ({n}){ext}");
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, instrument: serde_json::Value, files: &[&str]) -> Entry {
        let mut properties = serde_json::Map::new();
        if !instrument.is_null() {
            properties.insert("instrument".to_owned(), instrument);
        }
        Entry {
            path: format!("Set/{title}.md"),
            title: title.to_owned(),
            properties,
            files: files.iter().map(|f| (*f).to_owned()).collect(),
            art: None,
        }
    }

    fn archive(groups: &[Group]) -> Vec<&str> {
        groups
            .iter()
            .flat_map(|g| g.files.iter().map(|f| f.archive.as_str()))
            .collect()
    }

    fn set() -> Vec<Entry> {
        vec![
            entry(
                "Joy to the World",
                serde_json::json!(["violin", "cello"]),
                &["Set/joy/Violin I.pdf", "Set/joy/Cello.pdf"],
            ),
            entry(
                "Silent Night",
                serde_json::json!("violin"),
                &["Set/silent/Violin I.pdf"],
            ),
            entry("Interval", serde_json::Value::Null, &[]),
            entry(
                "Noel: Finale",
                serde_json::json!("cello"),
                &["Set/noel/Cello.pdf"],
            ),
        ]
    }

    #[test]
    fn the_three_views_group_and_number_in_list_order() {
        let set = set();
        let order = groups(&set, &Grouping::Order);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].entries, vec![0, 1, 2, 3]);
        assert_eq!(
            archive(&order),
            [
                "01 Violin I.pdf",
                "02 Cello.pdf",
                "03 Violin I.pdf",
                "04 Cello.pdf"
            ]
        );

        let note = groups(&set, &Grouping::Note);
        assert_eq!(
            note.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(),
            [
                "Joy to the World",
                "Silent Night",
                "Interval",
                "Noel: Finale"
            ]
        );
        // The note's place in the list, a note with no files counted;
        // a colon is not a path character everywhere.
        assert_eq!(
            archive(&note),
            [
                "01 Joy to the World/Violin I.pdf",
                "01 Joy to the World/Cello.pdf",
                "02 Silent Night/Violin I.pdf",
                "04 Noel- Finale/Cello.pdf"
            ]
        );

        let by = groups(&set, &Grouping::Property("instrument".to_owned()));
        assert_eq!(
            by.iter()
                .map(|g| (g.label.as_str(), g.entries.clone()))
                .collect::<Vec<_>>(),
            [("violin", vec![0, 1]), ("cello", vec![0, 3]), ("", vec![2])]
        );
        // A note in two groups brings its files to both: a file belongs
        // to its note, not to one of the note's values.
        assert_eq!(
            archive(&by),
            [
                "violin/01 Joy to the World/Violin I.pdf",
                "violin/01 Joy to the World/Cello.pdf",
                "violin/02 Silent Night/Violin I.pdf",
                "cello/01 Joy to the World/Violin I.pdf",
                "cello/01 Joy to the World/Cello.pdf",
                "cello/02 Noel- Finale/Cello.pdf"
            ]
        );
        assert_eq!(property_keys(&set), ["instrument"]);
    }

    #[test]
    fn groupings_parse_and_names_never_clash() {
        assert_eq!(Grouping::parse(""), Some(Grouping::Order));
        assert_eq!(Grouping::parse("note"), Some(Grouping::Note));
        let by = Grouping::parse("property: instrument").unwrap();
        assert_eq!(by, Grouping::Property("instrument".to_owned()));
        assert_eq!(Grouping::parse(&by.label()), Some(by));
        assert_eq!(Grouping::parse("property:"), None);
        assert_eq!(Grouping::parse("shuffle"), None);

        let twice = vec![entry(
            "Joy",
            serde_json::Value::Null,
            &["a/Part.pdf", "b/Part.pdf", "c/part.PDF"],
        )];
        assert_eq!(
            archive(&groups(&twice, &Grouping::Note)),
            [
                "01 Joy/Part.pdf",
                "01 Joy/Part (2).pdf",
                "01 Joy/part (3).PDF"
            ]
        );
        assert_eq!(clean("../.."), "-");
        assert_eq!(numbered(7, 120), "007");
    }

    #[test]
    fn filters_are_bases_filters() {
        assert_eq!(parse_filter("  "), Ok(FilterNode::None));
        assert!(parse_filter("instrument == \"violin\"").is_ok());
        assert!(parse_filter("instrument ==").is_err());
    }
}
