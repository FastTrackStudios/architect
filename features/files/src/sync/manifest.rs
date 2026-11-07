//! A root's manifest: one Loro document per root, on the peer lanes.
//!
//! `meta` (a map) names the root. `versions` is an append-only list:
//! every save or delete of a path is one entry, naming the versions it
//! replaced (`base`). `checkpoints` is another: a named set of heads.
//! Appends from two devices never conflict, so both sides' offline edits
//! survive the merge; every question (a path's heads, its history, a
//! fork) is answered from the list alone.

use std::collections::{BTreeMap, BTreeSet};

use loro::{LoroDoc, LoroValue};
use uuid::Uuid;

#[cfg(test)]
use crate::id::RootId;
use crate::id::VersionId;
pub use crate::roots::{Checkpoint, FileVersion, RootMeta, Section};

const META: &str = "meta";
const VERSIONS: &str = "versions";
const CHECKPOINTS: &str = "checkpoints";

impl RootMeta {
    /// The manifest's document id: derived from the place, so two
    /// devices that find the same tree on their own seed one document
    /// (the second seed adopts the first's lineage, #412).
    #[must_use]
    pub fn doc_id(&self) -> Uuid {
        doc_id_for(&self.place())
    }
}

/// The document id of the manifest at `place` (`<org>/<section>/<name>`).
#[must_use]
pub fn doc_id_for(place: &str) -> Uuid {
    Uuid::new_v5(&NAMESPACE, format!("architect-files:{place}").as_bytes())
}

/// The manifests' name on the peer lane: `files/<place>`.
#[must_use]
pub fn lane_name(place: &str) -> String {
    format!("files/{place}")
}

/// The place a lane name holds, when it is a manifest's.
#[must_use]
pub fn place_of(lane_name: &str) -> Option<&str> {
    let place = lane_name.strip_prefix("files/")?;
    let mut parts = place.splitn(3, '/');
    let (org, section, name) = (parts.next()?, parts.next()?, parts.next()?);
    (!org.is_empty() && Section::from_dir(section).is_some() && !name.is_empty()).then_some(place)
}

pub(super) const NAMESPACE: Uuid = Uuid::from_u128(0x5e6f_3c2a_9b1d_4e8f_a7c3_1d2e_3f4a_5b6c);

/// How a path's heads settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// One head, or several that agree on content.
    One(FileVersion),
    /// Several that differ: the newest wins the path, the rest are
    /// written beside it as conflict copies.
    Fork {
        winner: FileVersion,
        losers: Vec<FileVersion>,
    },
}

/// A manifest over a root's Loro document.
#[derive(Clone, Copy)]
pub struct Manifest<'a> {
    doc: &'a LoroDoc,
}

impl<'a> Manifest<'a> {
    #[must_use]
    pub const fn new(doc: &'a LoroDoc) -> Self {
        Self { doc }
    }

    /// What the root is, once some device has said.
    #[must_use]
    pub fn meta(&self) -> Option<RootMeta> {
        let map = self.doc.get_map(META);
        let json = map.get("json")?.into_value().ok()?;
        let LoroValue::String(json) = json else {
            return None;
        };
        facet_json::from_str(&json).ok()
    }

    /// Say what the root is (the same on every device, so concurrent sets agree).
    pub fn set_meta(&self, meta: &RootMeta) {
        if self.meta().as_ref() == Some(meta) {
            return;
        }
        let _ = self
            .doc
            .get_map(META)
            .insert("json", facet_json::to_string(meta).unwrap_or_default());
        self.doc.commit();
    }

    /// Append `versions` and commit them as one change.
    pub fn push(&self, versions: &[FileVersion]) {
        if versions.is_empty() {
            return;
        }
        let list = self.doc.get_list(VERSIONS);
        for v in versions {
            let _ = list.push(facet_json::to_string(v).unwrap_or_default());
        }
        self.doc.commit();
    }

    /// Append a checkpoint.
    pub fn push_checkpoint(&self, checkpoint: &Checkpoint) {
        let _ = self
            .doc
            .get_list(CHECKPOINTS)
            .push(facet_json::to_string(checkpoint).unwrap_or_default());
        self.doc.commit();
    }

    /// Every version, in list order.
    #[must_use]
    pub fn versions(&self) -> Vec<FileVersion> {
        strings(self.doc.get_list(VERSIONS).get_value())
            .filter_map(|s| facet_json::from_str(&s).ok())
            .collect()
    }

    /// Every checkpoint, oldest first.
    #[must_use]
    pub fn checkpoints(&self) -> Vec<Checkpoint> {
        let mut out: Vec<Checkpoint> = strings(self.doc.get_list(CHECKPOINTS).get_value())
            .filter_map(|s| facet_json::from_str(&s).ok())
            .collect();
        out.sort_by_key(|c| c.at_us);
        out
    }

    /// The manifest as a [`Tree`]: the queries over the versions.
    #[must_use]
    pub fn tree(&self) -> Tree {
        Tree::new(self.versions())
    }
}

fn strings(value: LoroValue) -> impl Iterator<Item = String> {
    let items = match value {
        LoroValue::List(items) => items.to_vec(),
        _ => Vec::new(),
    };
    items.into_iter().filter_map(|v| match v {
        LoroValue::String(s) => Some(s.to_string()),
        _ => None,
    })
}

/// The versions, grouped and answered: pure, so it is tested alone.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    by_path: BTreeMap<String, Vec<FileVersion>>,
}

impl Tree {
    #[must_use]
    pub fn new(versions: Vec<FileVersion>) -> Self {
        let mut by_path: BTreeMap<String, Vec<FileVersion>> = BTreeMap::new();
        for v in versions {
            by_path.entry(v.path.clone()).or_default().push(v);
        }
        for list in by_path.values_mut() {
            list.sort_by(|a, b| (a.at_us, &a.by, a.id).cmp(&(b.at_us, &b.by, b.id)));
        }
        Self { by_path }
    }

    /// Every path that has ever had a version.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.by_path.keys().map(String::as_str)
    }

    /// A path's versions, oldest first.
    #[must_use]
    pub fn history(&self, path: &str) -> &[FileVersion] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// A version by id.
    #[must_use]
    pub fn version(&self, id: VersionId) -> Option<&FileVersion> {
        self.by_path.values().flatten().find(|v| v.id == id)
    }

    /// The versions of `path` no other version replaced.
    #[must_use]
    pub fn heads(&self, path: &str) -> Vec<FileVersion> {
        let list = self.history(path);
        let replaced: BTreeSet<VersionId> = list.iter().flat_map(|v| v.base.clone()).collect();
        list.iter()
            .filter(|v| !replaced.contains(&v.id))
            .cloned()
            .collect()
    }

    /// How `path` settles, or `None` when it has no versions.
    #[must_use]
    pub fn settled(&self, path: &str) -> Option<Settled> {
        let mut heads = self.heads(path);
        // Newest content wins; ties go to the device name, then the id,
        // so every device picks the same.
        heads.sort_by(|a, b| (b.at_us, &b.by, b.id).cmp(&(a.at_us, &a.by, a.id)));
        let winner = heads.first()?.clone();
        let losers: Vec<FileVersion> = heads
            .into_iter()
            .skip(1)
            .filter(|v| v.content != winner.content && !v.is_delete())
            .collect();
        Some(if losers.is_empty() {
            Settled::One(winner)
        } else {
            Settled::Fork { winner, losers }
        })
    }

    /// The version a path is at: the settled winner, `None` if deleted.
    #[must_use]
    pub fn current(&self, path: &str) -> Option<FileVersion> {
        let v = match self.settled(path)? {
            Settled::One(v) | Settled::Fork { winner: v, .. } => v,
        };
        (!v.is_delete()).then_some(v)
    }

    /// Every live path and its current version.
    #[must_use]
    pub fn live(&self) -> BTreeMap<String, FileVersion> {
        self.paths()
            .filter_map(|p| self.current(p).map(|v| (p.to_owned(), v)))
            .collect()
    }

    /// Whether `ancestor` is `of` or one of the versions it replaced, transitively.
    #[must_use]
    pub fn descends(&self, of: VersionId, ancestor: VersionId) -> bool {
        let mut stack = vec![of];
        let mut seen = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if id == ancestor {
                return true;
            }
            if seen.insert(id)
                && let Some(v) = self.version(id)
            {
                stack.extend(v.base.iter().copied());
            }
        }
        false
    }

    /// Whether `content` is or was ever held at `path` (a pull only
    /// deletes or overwrites what the line once held: Task's
    /// "an empty replica never deletes").
    #[must_use]
    pub fn ever_held(&self, path: &str, content: &str) -> bool {
        self.history(path)
            .iter()
            .any(|v| v.content.as_deref() == Some(content))
    }

    /// The bytes of the live heads: the root's share of the org's quota.
    #[must_use]
    pub fn used(&self) -> u64 {
        self.live().values().map(|v| v.size).sum()
    }
}

/// `<stem> (conflict <device>).<ext>` beside `path`.
#[must_use]
pub fn conflict_path(path: &str, device: &str) -> String {
    let (dir, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
    let (stem, ext) = match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (file, None),
    };
    let name = ext.map_or_else(
        || format!("{stem} (conflict {device})"),
        |ext| format!("{stem} (conflict {device}).{ext}"),
    );
    if dir.is_empty() {
        name
    } else {
        format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(
        id: u128,
        path: &str,
        content: Option<&str>,
        at: i64,
        by: &str,
        base: &[u128],
    ) -> FileVersion {
        FileVersion {
            id: VersionId(Uuid::from_u128(id)),
            path: path.to_owned(),
            content: content.map(str::to_owned),
            size: 1,
            by: by.to_owned(),
            at_us: at,
            base: base
                .iter()
                .map(|b| VersionId(Uuid::from_u128(*b)))
                .collect(),
        }
    }

    #[test]
    fn a_line_of_saves_has_one_head() {
        let t = Tree::new(vec![
            v(1, "a", Some("x"), 1, "n", &[]),
            v(2, "a", Some("y"), 2, "n", &[1]),
        ]);
        assert_eq!(t.current("a").map(|v| v.content), Some(Some("y".into())));
        assert!(t.descends(VersionId(Uuid::from_u128(2)), VersionId(Uuid::from_u128(1))));
    }

    #[test]
    fn two_offline_saves_fork_and_the_newest_wins() {
        let t = Tree::new(vec![
            v(1, "s.rpp", Some("x"), 1, "node", &[]),
            v(2, "s.rpp", Some("laptop"), 5, "laptop", &[1]),
            v(3, "s.rpp", Some("desk"), 4, "desk", &[1]),
        ]);
        let Some(Settled::Fork { winner, losers }) = t.settled("s.rpp") else {
            panic!("a fork");
        };
        assert_eq!(winner.by, "laptop");
        assert_eq!(losers.len(), 1);
        assert_eq!(losers[0].by, "desk");
    }

    #[test]
    fn heads_that_agree_on_content_are_settled() {
        let t = Tree::new(vec![
            v(1, "c", Some("same"), 1, "a", &[]),
            v(2, "c", Some("same"), 2, "b", &[]),
        ]);
        assert!(matches!(t.settled("c"), Some(Settled::One(_))));
    }

    #[test]
    fn a_delete_leaves_the_path_out_of_the_live_set() {
        let t = Tree::new(vec![
            v(1, "a", Some("x"), 1, "n", &[]),
            v(2, "a", None, 2, "n", &[1]),
        ]);
        assert!(t.live().is_empty());
        assert!(t.ever_held("a", "x"));
    }

    #[test]
    fn a_conflict_copy_sits_beside_its_file() {
        assert_eq!(
            conflict_path("Right Here/Right Here.RPP", "desk"),
            "Right Here/Right Here (conflict desk).RPP"
        );
        assert_eq!(conflict_path("README", "desk"), "README (conflict desk)");
    }

    #[test]
    fn versions_from_two_docs_merge() {
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        Manifest::new(&a).push(&[v(1, "x", Some("1"), 1, "a", &[])]);
        b.import(&a.export(loro::ExportMode::Snapshot).unwrap())
            .unwrap();
        Manifest::new(&a).push(&[v(2, "x", Some("2"), 2, "a", &[1])]);
        Manifest::new(&b).push(&[v(3, "x", Some("3"), 3, "b", &[1])]);
        b.import(&a.export(loro::ExportMode::Snapshot).unwrap())
            .unwrap();
        let t = Manifest::new(&b).tree();
        assert_eq!(t.heads("x").len(), 2);
        assert!(matches!(t.settled("x"), Some(Settled::Fork { .. })));
    }

    #[test]
    fn a_lane_name_round_trips() {
        let meta = RootMeta {
            id: RootId(Uuid::nil()),
            org: "codywright".into(),
            section: Section::Projects,
            name: "Right Here - Cody Wright".into(),
        };
        assert_eq!(
            place_of(&lane_name(&meta.place())),
            Some(meta.place().as_str())
        );
        assert_eq!(place_of("notes/a.md"), None);
        assert_eq!(doc_id_for(&meta.place()), meta.doc_id());
    }
}
