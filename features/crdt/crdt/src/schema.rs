//! Schema versioning for a Loro document — the version stamp a replica
//! carries, plus an ordered migration runner that brings an older doc up
//! to the version the running app expects.
//!
//! The stamp lives in its own root map ([`META_ROOT`]) under
//! [`SCHEMA_VERSION_KEY`], so it never collides with an entity's
//! `EntityCrdt::ROOT`. A doc that has never been stamped reads as
//! version `0`.
//!
//! ## Shape
//!
//! An app declares its steps once, as a `const` slice, and runs them
//! wherever it opens a doc:
//!
//! ```ignore
//! use crdt::schema::{self, Migration};
//!
//! pub const SCHEMA: u32 = 3;
//! pub const MIGRATIONS: &[Migration] = &[
//!     Migration { to: 1, name: "seed roots", apply: v1_seed_roots },
//!     Migration { to: 2, name: "notes → LoroText", apply: v2_notes_to_text },
//!     Migration { to: 3, name: "drop legacy tags", apply: v3_drop_tags },
//! ];
//!
//! let doc = CrdtDoc::open(id, persistence).await?;
//! let report = schema::migrate(doc.loro(), SCHEMA, MIGRATIONS)?;
//! ```
//!
//! ## Where the stamp lives
//!
//! [`SchemaStamp`] names the root map + key. The free functions use
//! [`SchemaStamp::ARCHITECT`]; a doc that already stamps itself
//! elsewhere keeps its layout by naming it — Session's `SessionDoc`
//! writes `meta.schema`, so it adopts this as
//! `SchemaStamp::new("meta", "schema").migrate(doc, SCHEMA, MIGRATIONS)`.
//! Run it after the initial load/sync: migrating an empty joiner doc
//! stamps it, which a "has anything arrived yet?" check would misread.
//!
//! ## Rules
//!
//! - Steps run in order, one version at a time: the step with
//!   `to == current + 1` runs, the stamp is bumped to `to`, the doc is
//!   committed, and the next step runs. A gap in the list is an error,
//!   not a silent skip.
//! - A doc stamped *newer* than the app's target is refused
//!   ([`RepoError::Conflict`]). An older binary must not write into a
//!   layout it doesn't understand, and there is no downgrade path.
//! - **Steps must be idempotent.** Two peers that open the same old doc
//!   while offline both migrate it; when their updates merge, both
//!   steps' ops are in the doc. Write steps so re-applying them is a
//!   no-op (check-then-write, deletes of keys that may already be
//!   gone). The stamp itself merges cleanly: both peers wrote the same
//!   number.
//! - **Don't seed content into mergeable containers.** Two peers that
//!   both `ensure_mergeable_text(k)` and insert the old value get it
//!   twice after the merge. Seed a plain container instead
//!   (`insert_container`, last-writer-wins on creation);
//!   `crate::codec::upgrade_to_text` does exactly that for a legacy
//!   string → `LoroText` step.

use architect::RepoError;
use loro::{LoroDoc, LoroValue, ValueOrContainer};

use crate::codec::loro_err;

/// Root map holding document-level metadata. Reserved: entity roots
/// must not use this name.
pub const META_ROOT: &str = "__architect_meta";

/// Key under [`META_ROOT`] holding the schema version (`i64`).
pub const SCHEMA_VERSION_KEY: &str = "schema_version";

/// One migration step: brings a doc stamped `to - 1` up to `to`.
#[derive(Clone, Copy)]
pub struct Migration {
    /// The version this step produces.
    pub to: u32,
    /// Human label, carried into the [`MigrationReport`] and errors.
    pub name: &'static str,
    /// The hook. Runs against the whole doc; must be idempotent (see
    /// the module docs). An `Err` aborts the run with the doc stamped at
    /// the last step that succeeded.
    pub apply: fn(&LoroDoc) -> Result<(), RepoError>,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("to", &self.to)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// What a [`migrate`] run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    /// The stamp before the run.
    pub from: u32,
    /// The stamp after the run (the target, on success).
    pub to: u32,
    /// `name` of every step that ran, in order.
    pub applied: Vec<&'static str>,
}

impl MigrationReport {
    /// True if the doc was already at the target.
    #[must_use]
    pub const fn is_noop(&self) -> bool {
        self.applied.is_empty()
    }
}

/// Where a doc keeps its schema version: `root` map, `key` entry,
/// stored as an `i64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaStamp {
    /// Root map name.
    pub root: &'static str,
    /// Key under `root`.
    pub key: &'static str,
}

impl SchemaStamp {
    /// Architect's default: `__architect_meta.schema_version`.
    pub const ARCHITECT: Self = Self::new(META_ROOT, SCHEMA_VERSION_KEY);

    /// A stamp at `root.key`.
    #[must_use]
    pub const fn new(root: &'static str, key: &'static str) -> Self {
        Self { root, key }
    }

    /// The doc's schema version; `0` if it has never been stamped.
    ///
    /// # Errors
    ///
    /// [`RepoError::Internal`] if the stamp holds something other than
    /// a non-negative integer that fits `u32`.
    pub fn version(self, doc: &LoroDoc) -> Result<u32, RepoError> {
        let meta = doc.get_map(self.root);
        match meta.get(self.key) {
            None | Some(ValueOrContainer::Value(LoroValue::Null)) => Ok(0),
            Some(ValueOrContainer::Value(LoroValue::I64(n))) => u32::try_from(n).map_err(|_| {
                RepoError::Internal(format!("{}.{} out of range: {n}", self.root, self.key))
            }),
            Some(other) => Err(RepoError::Internal(format!(
                "expected integer at {}.{}, got {other:?}",
                self.root, self.key
            ))),
        }
    }

    /// Stamp the doc with `version` (does not commit).
    ///
    /// # Errors
    ///
    /// [`RepoError::Internal`] if Loro rejects the write.
    pub fn set(self, doc: &LoroDoc, version: u32) -> Result<(), RepoError> {
        doc.get_map(self.root)
            .insert(self.key, i64::from(version))
            .map_err(loro_err)
    }

    /// Bring `doc` up to `target` by running `migrations` in order.
    ///
    /// A fresh (unstamped) doc runs every step from `to = 1`. Each step
    /// is committed with its stamp, so a crash or an `Err` mid-run
    /// leaves the doc at a consistent, resumable version.
    ///
    /// # Errors
    ///
    /// - [`RepoError::Conflict`] if the doc is stamped newer than
    ///   `target`.
    /// - [`RepoError::Internal`] if no step leads from the current
    ///   version to the next one, or the stamp can't be read/written.
    /// - Whatever a step's `apply` returns, unchanged.
    pub fn migrate(
        self,
        doc: &LoroDoc,
        target: u32,
        migrations: &[Migration],
    ) -> Result<MigrationReport, RepoError> {
        let from = self.version(doc)?;
        if from > target {
            return Err(RepoError::Conflict(format!(
                "doc schema version {from} is newer than this app's {target}; refusing to open it"
            )));
        }
        let mut current = from;
        let mut applied = Vec::new();
        while current < target {
            let next = current
                .checked_add(1)
                .ok_or_else(|| RepoError::Internal("schema version overflow".into()))?;
            let step = migrations.iter().find(|m| m.to == next).ok_or_else(|| {
                RepoError::Internal(format!(
                    "no migration to schema version {next} (doc is at {current}, target {target})"
                ))
            })?;
            (step.apply)(doc)?;
            self.set(doc, next)?;
            doc.commit();
            applied.push(step.name);
            current = next;
        }
        Ok(MigrationReport {
            from,
            to: current,
            applied,
        })
    }
}

/// [`SchemaStamp::version`] at [`SchemaStamp::ARCHITECT`].
///
/// # Errors
///
/// As [`SchemaStamp::version`].
pub fn schema_version(doc: &LoroDoc) -> Result<u32, RepoError> {
    SchemaStamp::ARCHITECT.version(doc)
}

/// [`SchemaStamp::set`] at [`SchemaStamp::ARCHITECT`].
///
/// # Errors
///
/// As [`SchemaStamp::set`].
pub fn set_schema_version(doc: &LoroDoc, version: u32) -> Result<(), RepoError> {
    SchemaStamp::ARCHITECT.set(doc, version)
}

/// [`SchemaStamp::migrate`] at [`SchemaStamp::ARCHITECT`].
///
/// # Errors
///
/// As [`SchemaStamp::migrate`].
pub fn migrate(
    doc: &LoroDoc,
    target: u32,
    migrations: &[Migration],
) -> Result<MigrationReport, RepoError> {
    SchemaStamp::ARCHITECT.migrate(doc, target, migrations)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // Steps keep the `Migration::apply` signature even when infallible.
    clippy::unnecessary_wraps
)]
mod tests {
    use super::*;
    use loro::ExportMode;

    fn v1(doc: &LoroDoc) -> Result<(), RepoError> {
        doc.get_map("notes")
            .insert("seeded", true)
            .map_err(loro_err)
    }
    fn v2(doc: &LoroDoc) -> Result<(), RepoError> {
        // Idempotent + concurrent-safe: legacy string → LWW LoroText;
        // a key that's already text is left alone.
        let m = doc.get_map("notes");
        crate::codec::upgrade_to_text(&m, "body", None).map(|_| ())
    }
    fn v3(doc: &LoroDoc) -> Result<(), RepoError> {
        let _ = doc.get_map("notes").delete("legacy_tags");
        Ok(())
    }
    fn boom(_: &LoroDoc) -> Result<(), RepoError> {
        Err(RepoError::Internal("boom".into()))
    }

    const STEPS: &[Migration] = &[
        Migration {
            to: 1,
            name: "seed",
            apply: v1,
        },
        Migration {
            to: 2,
            name: "body to text",
            apply: v2,
        },
        Migration {
            to: 3,
            name: "drop tags",
            apply: v3,
        },
    ];

    #[test]
    fn fresh_doc_reads_zero_and_migrates_all_the_way() {
        let doc = LoroDoc::new();
        assert_eq!(schema_version(&doc).unwrap(), 0);
        let r = migrate(&doc, 3, STEPS).unwrap();
        assert_eq!(r.from, 0);
        assert_eq!(r.to, 3);
        assert_eq!(r.applied, vec!["seed", "body to text", "drop tags"]);
        assert_eq!(schema_version(&doc).unwrap(), 3);
        // Second open is a no-op.
        assert!(migrate(&doc, 3, STEPS).unwrap().is_noop());
    }

    #[test]
    fn resumes_from_the_stamped_version() {
        let doc = LoroDoc::new();
        let m = doc.get_map("notes");
        m.insert("body", "legacy").unwrap();
        m.insert("legacy_tags", "a\tb").unwrap();
        set_schema_version(&doc, 1).unwrap();
        let r = migrate(&doc, 3, STEPS).unwrap();
        assert_eq!(r.applied, vec!["body to text", "drop tags"]);
        assert_eq!(crate::codec::read_text(&m, "body").unwrap(), "legacy");
        assert!(m.get("legacy_tags").is_none());
    }

    #[test]
    fn custom_stamp_location_reads_an_existing_layout() {
        // Session's layout: `meta.schema` as i64, already at 1.
        let session = SchemaStamp::new("meta", "schema");
        let doc = LoroDoc::new();
        doc.get_map("meta").insert("schema", 1_i64).unwrap();
        assert_eq!(session.version(&doc).unwrap(), 1);
        assert_eq!(
            schema_version(&doc).unwrap(),
            0,
            "default stamp is separate"
        );
        let r = session.migrate(&doc, 3, STEPS).unwrap();
        assert_eq!(r.applied, vec!["body to text", "drop tags"]);
        assert_eq!(session.version(&doc).unwrap(), 3);
    }

    #[test]
    fn newer_doc_is_refused() {
        let doc = LoroDoc::new();
        set_schema_version(&doc, 4).unwrap();
        assert!(matches!(
            migrate(&doc, 3, STEPS),
            Err(RepoError::Conflict(_))
        ));
        assert_eq!(schema_version(&doc).unwrap(), 4);
    }

    #[test]
    fn gap_is_an_error_not_a_skip() {
        let doc = LoroDoc::new();
        let gappy = &[STEPS[0], STEPS[2]];
        let err = migrate(&doc, 3, gappy).unwrap_err();
        assert!(
            matches!(&err, RepoError::Internal(m) if m.contains("no migration to schema version 2")),
            "{err:?}"
        );
        // Stopped at the last good step.
        assert_eq!(schema_version(&doc).unwrap(), 1);
    }

    #[test]
    fn failing_step_leaves_the_last_good_stamp() {
        let doc = LoroDoc::new();
        let steps = &[
            STEPS[0],
            Migration {
                to: 2,
                name: "boom",
                apply: boom,
            },
        ];
        assert!(migrate(&doc, 2, steps).is_err());
        assert_eq!(schema_version(&doc).unwrap(), 1);
    }

    #[test]
    fn concurrent_migrations_on_two_peers_converge() {
        // Shared v1 doc with a legacy string body.
        let a = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        a.get_map("notes").insert("body", "hello").unwrap();
        set_schema_version(&a, 1).unwrap();
        a.commit();
        let b = LoroDoc::new();
        b.set_peer_id(2).unwrap();
        b.import(&a.export(ExportMode::all_updates()).unwrap())
            .unwrap();

        // Both peers migrate offline, then sync.
        migrate(&a, 3, STEPS).unwrap();
        migrate(&b, 3, STEPS).unwrap();
        let a_to_b = a.export(ExportMode::updates(&b.oplog_vv())).unwrap();
        let b_to_a = b.export(ExportMode::updates(&a.oplog_vv())).unwrap();
        b.import(&a_to_b).unwrap();
        a.import(&b_to_a).unwrap();

        assert_eq!(schema_version(&a).unwrap(), 3);
        assert_eq!(schema_version(&b).unwrap(), 3);
        let ta = crate::codec::read_text(&a.get_map("notes"), "body").unwrap();
        let tb = crate::codec::read_text(&b.get_map("notes"), "body").unwrap();
        assert_eq!(ta, tb);
        assert_eq!(ta, "hello", "idempotent step must not duplicate the body");
    }
}
