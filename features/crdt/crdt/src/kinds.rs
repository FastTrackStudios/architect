//! Kinds of document (`docs/content/architecture/documents.md`, "A
//! document kind").
//!
//! What an app declares — an id, a name, its file's extension, its
//! schema's stamp and the migrations up to it — and architect reads,
//! writes and upgrades.

use loro::LoroDoc;

/// The map a document keeps its own facts in.
const META: &str = "meta";
const STAMP: &str = "schema";
const KIND: &str = "kind";

/// One step from a schema stamp to the next.
#[derive(Clone)]
pub struct Migration {
    /// The stamp it upgrades from (to `from + 1`).
    pub from: u32,
    pub name: &'static str,
    pub run: fn(&LoroDoc) -> Result<(), String>,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("from", &self.from)
            .field("name", &self.name)
            .finish()
    }
}

/// A kind of document an app keeps.
#[derive(Debug, Clone)]
pub struct DocKind {
    /// `studio.song`, `task.note`.
    pub id: String,
    /// "Song".
    pub name: String,
    /// The file's extension, without the dot (`session`).
    pub extension: String,
    /// The schema this build writes.
    pub stamp: u32,
    pub migrations: Vec<Migration>,
}

impl DocKind {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            extension: String::new(),
            stamp: 1,
            migrations: Vec::new(),
        }
    }

    #[must_use]
    pub fn extension(mut self, ext: impl Into<String>) -> Self {
        self.extension = ext.into();
        self
    }

    /// The schema this build writes, and how to come up to it.
    #[must_use]
    pub fn schema(mut self, stamp: u32, migrations: Vec<Migration>) -> Self {
        self.stamp = stamp;
        self.migrations = migrations;
        self
    }

    /// Marks a new document as this kind, at this build's schema.
    pub fn stamp(&self, doc: &LoroDoc) {
        let meta = doc.get_map(META);
        let _ = meta.insert(KIND, self.id.as_str());
        let _ = meta.insert(STAMP, i64::from(self.stamp));
        doc.commit();
    }

    /// Brings `doc` up to this build's schema, a migration at a time.
    /// The stamp it is at now.
    ///
    /// # Errors
    ///
    /// A migration that failed (the document stays at the stamp before
    /// it), or a document from a newer build than this one.
    pub fn upgrade(&self, doc: &LoroDoc) -> Result<u32, String> {
        let mut at = stamp_of(doc).unwrap_or(1);
        if at > self.stamp {
            return Err(format!(
                "this {} was written by a newer version (schema {at}, this build reads {})",
                self.name, self.stamp
            ));
        }
        while at < self.stamp {
            let step = self
                .migrations
                .iter()
                .find(|m| m.from == at)
                .ok_or_else(|| format!("no migration from schema {at} of {}", self.id))?;
            (step.run)(doc).map_err(|e| format!("{}: {e}", step.name))?;
            at = at.saturating_add(1);
            let _ = doc.get_map(META).insert(STAMP, i64::from(at));
            doc.commit();
        }
        Ok(at)
    }
}

/// The stamp `doc` says it is at.
#[must_use]
pub fn stamp_of(doc: &LoroDoc) -> Option<u32> {
    let v = doc.get_map(META).get(STAMP)?;
    let n = v.into_value().ok()?.into_i64().ok()?;
    u32::try_from(n).ok()
}

/// The kind `doc` says it is.
#[must_use]
pub fn kind_of(doc: &LoroDoc) -> Option<String> {
    let v = doc.get_map(META).get(KIND)?;
    v.into_value()
        .ok()?
        .into_string()
        .ok()
        .map(|s| s.to_string())
}

/// Every kind an app (and its extensions) keeps.
#[derive(Debug, Clone, Default)]
pub struct DocKinds {
    kinds: Vec<DocKind>,
}

impl DocKinds {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `kind`; one of the same id is replaced.
    pub fn register(&mut self, kind: DocKind) {
        self.kinds.retain(|k| k.id != kind.id);
        self.kinds.push(kind);
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&DocKind> {
        self.kinds.iter().find(|k| k.id == id)
    }

    /// The kind a file of this extension is.
    #[must_use]
    pub fn for_extension(&self, ext: &str) -> Option<&DocKind> {
        let ext = ext.trim_start_matches('.');
        self.kinds
            .iter()
            .find(|k| k.extension.eq_ignore_ascii_case(ext))
    }

    pub fn iter(&self) -> impl Iterator<Item = &DocKind> {
        self.kinds.iter()
    }
}
