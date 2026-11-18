//! The kinds of document the node's extensions declare, kept in the
//! shared set its vaults' registry and catalogue read
//! (`vault_collab::VaultCollab::kinds`).
//!
//! An extension's `[[doc_kinds]]` (`architect_extensions::contrib::DocKindDecl`)
//! make a vault file of its extension a document the node keeps in the
//! file itself and syncs like a note, for as long as the extension is
//! there: its compiled-in extensions' (`Distribution::doc_kinds`) always,
//! the ones installed in [`super::Config::extensions_dir`] while they are
//! installed — read again whenever that folder changes, so installing one
//! adds its kinds and uninstalling it takes them off, nothing restarted.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use architect_extensions::contrib::DocKindDecl;
use architect_sites::InstalledLayers;
use crdt::SharedDocKinds;

/// What [`ExtensionKinds`] last read: the installed folder's generation
/// (none without a folder) and the kind ids it registered.
type Held = Option<(Option<u64>, BTreeSet<String>)>;

/// Feeds a [`SharedDocKinds`] from the node's extensions.
#[derive(Debug)]
pub struct ExtensionKinds {
    kinds: SharedDocKinds,
    given: Vec<DocKindDecl>,
    installed: Option<InstalledLayers>,
    held: Mutex<Held>,
}

impl ExtensionKinds {
    /// `given` (a distribution's compiled-in extensions' kinds) and those
    /// of the extensions installed in `dir`, into `kinds`.
    #[must_use]
    pub fn new(kinds: SharedDocKinds, given: Vec<DocKindDecl>, dir: Option<PathBuf>) -> Self {
        let this = Self {
            kinds,
            given,
            installed: dir.map(InstalledLayers::new),
            held: Mutex::new(None),
        };
        this.refresh();
        this
    }

    /// Brings the kinds up to what is installed now: a new extension's
    /// registered, a removed one's taken off. Cheap when the folder has
    /// not changed.
    pub fn refresh(&self) {
        let read = self.installed.as_ref().map(InstalledLayers::read);
        let generation = read.as_ref().map(|r| r.generation);
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.as_ref().is_some_and(|(at, _)| *at == generation) {
            return;
        }
        let before = held.take().map(|(_, ids)| ids).unwrap_or_default();
        let declared: Vec<&DocKindDecl> = self
            .given
            .iter()
            .chain(
                read.iter()
                    .flat_map(|r| r.packages.iter())
                    .flat_map(|p| p.doc_kinds.iter()),
            )
            .filter(|d| !d.files.is_empty())
            .collect();
        let now: BTreeSet<String> = declared.iter().map(|d| d.id.clone()).collect();
        for gone in before.difference(&now) {
            self.kinds.unregister(gone);
            tracing::info!(kind = %gone, "an extension's document kind is gone: its files no longer sync");
        }
        for decl in declared {
            if !before.contains(&decl.id) {
                tracing::info!(kind = %decl.id, "an extension's document kind: its files sync like notes");
            }
            self.kinds.register(decl.to_doc_kind());
        }
        *held = Some((generation, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Installing an extension adds its kinds, at the schema it says, and
    /// uninstalling it takes them off; a compiled-in one's stay.
    #[test]
    fn an_installed_extensions_kinds_come_and_go_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let kinds = SharedDocKinds::new();
        let given = DocKindDecl {
            id: "test.chart".into(),
            name: "Chart".into(),
            files: vec!["kf".into()],
            ..DocKindDecl::default()
        };
        let feed = ExtensionKinds::new(kinds.clone(), vec![given], Some(dir.path().to_owned()));
        assert!(kinds.for_path("a.kf").is_some());
        assert!(kinds.for_path("a.session").is_none());

        let slot = dir.path().join("test.sessions");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::write(
            slot.join("extension.toml"),
            "id = \"test.sessions\"\nname = \"Sessions\"\nversion = \"0.1.0\"\nkind = \"data\"\n\n\
             [[doc_kinds]]\nid = \"test.session\"\nname = \"Session\"\nfiles = [\"session\"]\nschema = 2\n",
        )
        .unwrap();
        feed.refresh();
        let kind = kinds.for_path("Songs/a.session").expect("installed");
        assert_eq!((kind.id.as_str(), kind.stamp), ("test.session", 2));

        std::fs::remove_dir_all(&slot).unwrap();
        feed.refresh();
        assert!(kinds.for_path("a.session").is_none(), "uninstalled");
        assert!(kinds.for_path("a.kf").is_some(), "compiled in");
    }
}
