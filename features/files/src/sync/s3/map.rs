//! Buckets and keys: an org is a bucket, and a key is
//! `<Projects|Assets>/<root>/<path in the root>`.
//!
//! A bucket per org, not per root: a project's name (`Right Here`) is not
//! a valid bucket name, the org is what an access key is issued in and
//! what a role is held in, and one bucket holds the org's tree as it is
//! on the node's disk and under the `WebDAV` mount, so `rclone sync` of a
//! bucket is a copy of the org's tree and a copy between two projects
//! moves no bytes.

use std::collections::BTreeMap;

use crate::roots::{RootMeta, Section};

/// A key's root and its path in it: `(section, root name, path)`.
#[must_use]
pub fn split_key(key: &str) -> Option<(Section, &str, &str)> {
    let mut parts = key.splitn(3, '/');
    let section = Section::from_dir(parts.next()?)?;
    let name = parts.next().filter(|n| !n.is_empty())?;
    Some((section, name, parts.next().unwrap_or_default()))
}

/// The key of `path` in the root `meta`.
#[must_use]
pub fn key_of(meta: &RootMeta, path: &str) -> String {
    format!("{}/{}/{path}", meta.section.dir(), meta.name)
}

/// Whether a root's keys (all under `<section>/<name>/`) can match `prefix`.
#[must_use]
pub fn root_may_match(meta: &RootMeta, prefix: &str) -> bool {
    let root = format!("{}/{}/", meta.section.dir(), meta.name);
    root.starts_with(prefix) || prefix.starts_with(&root)
}

/// One page of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub objects: Vec<(String, T)>,
    /// Rolled up at the delimiter (`Projects/`, `Projects/Right Here/Audio Files/`).
    pub prefixes: Vec<String>,
    /// The last key or prefix of this page, when there is more.
    pub next: Option<String>,
}

/// A page of `keys` (sorted) under `prefix`, after `after`, rolled up
/// at `delimiter`, at most `max` entries (objects and prefixes).
#[must_use]
pub fn page<T: Clone>(
    keys: &BTreeMap<String, T>,
    prefix: &str,
    delimiter: Option<&str>,
    after: Option<&str>,
    max: usize,
) -> Page<T> {
    let mut out = Page {
        objects: Vec::new(),
        prefixes: Vec::new(),
        next: None,
    };
    let from = after.map_or(std::ops::Bound::Included(prefix), std::ops::Bound::Excluded);
    let mut count = 0usize;
    let mut last_prefix: Option<String> = None;
    for (key, value) in keys.range::<str, _>((from, std::ops::Bound::Unbounded)) {
        if !key.starts_with(prefix) {
            if key.as_str() > prefix {
                break;
            }
            continue;
        }
        let rolled = delimiter.filter(|d| !d.is_empty()).and_then(|d| {
            let rest = key.get(prefix.len()..)?;
            let i = rest.find(d)?;
            Some(format!(
                "{prefix}{}",
                rest.get(..i.saturating_add(d.len()))?
            ))
        });
        // A key inside the prefix the page ended on (or already rolled up).
        if let Some(p) = &rolled
            && (last_prefix.as_ref() == Some(p) || after.is_some_and(|a| a.starts_with(p.as_str())))
        {
            continue;
        }
        if count == max {
            out.next = out
                .objects
                .last()
                .map(|(k, _)| k.clone())
                .into_iter()
                .chain(last_prefix)
                .max();
            break;
        }
        count = count.saturating_add(1);
        match rolled {
            Some(p) => {
                last_prefix = Some(p.clone());
                out.prefixes.push(p);
            }
            None => out.objects.push((key.clone(), value.clone())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::RootId;
    use uuid::Uuid;

    fn keys(list: &[&str]) -> BTreeMap<String, u32> {
        list.iter().map(|k| ((*k).to_owned(), 0)).collect()
    }

    #[test]
    fn a_key_names_its_root_and_path() {
        assert_eq!(
            split_key("Projects/Right Here/Audio Files/kick.wav"),
            Some((Section::Projects, "Right Here", "Audio Files/kick.wav"))
        );
        assert_eq!(
            split_key("Assets/Tracks"),
            Some((Section::Assets, "Tracks", ""))
        );
        assert_eq!(split_key("Elsewhere/x/y"), None);
        assert_eq!(split_key("Projects//y"), None);
        let meta = RootMeta {
            id: RootId::new(Uuid::nil()),
            org: "fts".into(),
            section: Section::Projects,
            name: "Right Here".into(),
        };
        assert_eq!(key_of(&meta, "a/b.wav"), "Projects/Right Here/a/b.wav");
        assert!(root_may_match(&meta, "Projects/Ri"));
        assert!(root_may_match(&meta, "Projects/Right Here/a/"));
        assert!(!root_may_match(&meta, "Assets/"));
    }

    #[test]
    fn a_delimiter_rolls_folders_up_and_pages_resume() {
        let all = keys(&[
            "Assets/Tracks/a.md",
            "Projects/A/s.rpp",
            "Projects/A/x/1.wav",
            "Projects/A/x/2.wav",
            "Projects/B/s.rpp",
        ]);
        let top = page(&all, "", Some("/"), None, 1000);
        assert_eq!(top.prefixes, vec!["Assets/", "Projects/"]);
        assert!(top.objects.is_empty());
        let a = page(&all, "Projects/A/", Some("/"), None, 1000);
        assert_eq!(a.prefixes, vec!["Projects/A/x/"]);
        assert_eq!(a.objects.len(), 1);
        let flat = page(&all, "Projects/", None, None, 2);
        assert_eq!(flat.objects.len(), 2);
        let rest = page(&all, "Projects/", None, flat.next.as_deref(), 2);
        assert_eq!(
            rest.objects
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>(),
            vec!["Projects/A/x/2.wav", "Projects/B/s.rpp"]
        );
        assert_eq!(rest.next, None);
        let rolled = page(&all, "Projects/", Some("/"), None, 1);
        assert_eq!(rolled.prefixes, vec!["Projects/A/"]);
        let more = page(&all, "Projects/", Some("/"), rolled.next.as_deref(), 1);
        assert_eq!(more.prefixes, vec!["Projects/B/"]);
    }
}
