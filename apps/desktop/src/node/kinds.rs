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
//! adds its kinds and uninstalling it takes them off, nothing restarted —
//! and the ones an embedder's extensions declare while they are loaded
//! ([`LoadedKinds`]: the window's, a development copy's among them,
//! followed by [`follow`]).
//!
//! A kind stays while any of those sources declares it: a development
//! copy unloaded takes off only what nothing else still declares.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use architect_extensions::contrib::{Contributions, DocKindDecl, Subscription};
use architect_sites::InstalledLayers;
use crdt::SharedDocKinds;

/// What [`ExtensionKinds`] last read: the installed folder's generation
/// (none without a folder), the loaded kinds' revision, and the kind ids
/// it registered.
type Held = Option<((Option<u64>, u64), BTreeSet<String>)>;

/// The kinds the extensions an embedder loads declare now (the window's
/// extension host: its compiled-in, installed and development copies),
/// as one source of a node's [`ExtensionKinds`].
///
/// Cheap to clone; clones share the kinds. Set before the node starts,
/// it starts with them; set after, the node's kinds follow at once.
#[derive(Clone, Default)]
pub struct LoadedKinds {
    inner: Arc<Mutex<Loaded>>,
}

#[derive(Default)]
struct Loaded {
    revision: u64,
    decls: Vec<DocKindDecl>,
    feed: Option<Weak<ExtensionKinds>>,
}

impl std::fmt::Debug for LoadedKinds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loaded = self.lock();
        f.debug_struct("LoadedKinds")
            .field("revision", &loaded.revision)
            .field(
                "kinds",
                &loaded.decls.iter().map(|d| &d.id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl LoadedKinds {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Loaded> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The loaded extensions' kinds are `decls` now: the node registers
    /// the new ones and takes off the gone ones nothing else declares.
    /// Nothing happens when they are what they were.
    pub fn set(&self, decls: Vec<DocKindDecl>) {
        let feed = {
            let mut loaded = self.lock();
            if loaded.decls == decls {
                return;
            }
            loaded.decls = decls;
            loaded.revision = loaded.revision.wrapping_add(1);
            loaded.feed.as_ref().and_then(Weak::upgrade)
        };
        if let Some(feed) = feed {
            feed.refresh();
        }
    }

    fn now(&self) -> (u64, Vec<DocKindDecl>) {
        let loaded = self.lock();
        (loaded.revision, loaded.decls.clone())
    }

    fn attach(&self, feed: &Arc<ExtensionKinds>) {
        self.lock().feed = Some(Arc::downgrade(feed));
    }
}

/// Keeps `loaded` at the document kinds `contributions` holds, while the
/// subscription is held.
///
/// `contributions` is an extension host's (every extension loaded in
/// it): `loaded` follows it now and after every change — a development
/// copy loading, reloading without its `[[doc_kinds]]`, unloading.
#[must_use = "the kinds follow while the subscription is held"]
pub fn follow(contributions: &Contributions, loaded: LoadedKinds) -> Subscription {
    loaded.set(declared(contributions));
    let heard = contributions.clone();
    contributions.subscribe(move |_| loaded.set(declared(&heard)))
}

/// The document kinds `contributions` holds, whoever declared them.
fn declared(contributions: &Contributions) -> Vec<DocKindDecl> {
    contributions
        .doc_kinds()
        .into_iter()
        .map(|by| by.item)
        .collect()
}

/// Feeds a [`SharedDocKinds`] from the node's extensions.
#[derive(Debug)]
pub struct ExtensionKinds {
    kinds: SharedDocKinds,
    given: Vec<DocKindDecl>,
    installed: Option<InstalledLayers>,
    loaded: LoadedKinds,
    held: Mutex<Held>,
}

impl ExtensionKinds {
    /// `given` (a distribution's compiled-in extensions' kinds), those
    /// of the extensions installed in `dir` and those `loaded` holds,
    /// into `kinds`.
    #[must_use]
    pub fn new(
        kinds: SharedDocKinds,
        given: Vec<DocKindDecl>,
        dir: Option<PathBuf>,
        loaded: LoadedKinds,
    ) -> Arc<Self> {
        let this = Arc::new(Self {
            kinds,
            given,
            installed: dir.map(InstalledLayers::new),
            loaded,
            held: Mutex::new(None),
        });
        this.loaded.attach(&this);
        this.refresh();
        this
    }

    /// Brings the kinds up to what is installed and loaded now: a new
    /// extension's registered, a removed one's taken off. Cheap when
    /// neither has changed.
    pub fn refresh(&self) {
        let read = self.installed.as_ref().map(InstalledLayers::read);
        let (revision, loaded) = self.loaded.now();
        let at = (read.as_ref().map(|r| r.generation), revision);
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.as_ref().is_some_and(|(was, _)| *was == at) {
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
            .chain(loaded.iter())
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
        *held = Some((at, now));
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
        let feed = ExtensionKinds::new(
            kinds.clone(),
            vec![given],
            Some(dir.path().to_owned()),
            LoadedKinds::new(),
        );
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

    /// A development copy at `dir`, declaring `.session` files unless
    /// `kinds` is false.
    fn sessions_copy(dir: &std::path::Path, kinds: bool) {
        std::fs::create_dir_all(dir).unwrap();
        let mut manifest = String::from(
            "id = \"test.sessions\"\nname = \"Sessions\"\nversion = \"0.1.0\"\nkind = \"data\"\n",
        );
        if kinds {
            manifest.push_str(
                "\n[[doc_kinds]]\nid = \"test.session\"\nname = \"Session\"\nfiles = [\"session\"]\n",
            );
        }
        std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    }

    /// The window's extension host, its kinds followed into `loaded`.
    fn window(
        tmp: &std::path::Path,
        loaded: LoadedKinds,
    ) -> (architect_extensions::extensions::Extensions, Subscription) {
        let host = architect_extensions::host::Host::new(
            architect::action::ActionList::new(),
            architect_extensions::host::SharedSections::default(),
        );
        let extensions = architect_extensions::extensions::Extensions::new(
            host,
            architect_extensions::install::Installs::new(tmp.join("extensions")),
        );
        let following = follow(extensions.host().contributions(), loaded);
        (extensions, following)
    }

    /// A development copy the window loads adds its kinds to the node's;
    /// reloaded without them, or ended, it takes them off.
    #[test]
    fn a_development_copys_kinds_come_and_go_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let kinds = SharedDocKinds::new();
        let loaded = LoadedKinds::new();
        let _feed = ExtensionKinds::new(kinds.clone(), Vec::new(), None, loaded.clone());
        let (extensions, _following) = window(tmp.path(), loaded);
        let dir = tmp.path().join("dev/test.sessions");
        sessions_copy(&dir, true);
        assert!(kinds.for_path("Songs/a.session").is_none());

        let _ = extensions.dev(&dir);
        assert!(extensions.host().get("test.sessions").is_some(), "loaded");
        let kind = kinds
            .for_path("Songs/a.session")
            .expect("loaded in the window");
        assert_eq!((kind.id.as_str(), kind.stamp), ("test.session", 1));

        sessions_copy(&dir, false);
        let package = architect_extensions::Package::read(&dir).unwrap();
        extensions
            .host()
            .load(&package, architect_extensions::catalog::Origin::Dev)
            .unwrap();
        assert!(
            kinds.for_path("Songs/a.session").is_none(),
            "reloaded without them"
        );

        sessions_copy(&dir, true);
        let package = architect_extensions::Package::read(&dir).unwrap();
        extensions
            .host()
            .load(&package, architect_extensions::catalog::Origin::Dev)
            .unwrap();
        assert!(
            kinds.for_path("Songs/a.session").is_some(),
            "reloaded with them"
        );

        let _ = extensions.end_dev(&dir);
        assert!(extensions.host().get("test.sessions").is_none(), "unloaded");
        assert!(kinds.for_path("Songs/a.session").is_none(), "unloaded");
    }

    /// A kind another source declares stays when the window's copy of it
    /// goes; one the window loaded before the node started is there from
    /// the start.
    #[test]
    fn a_kind_another_source_declares_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = LoadedKinds::new();
        let (extensions, _following) = window(tmp.path(), loaded.clone());
        let dir = tmp.path().join("dev/test.sessions");
        sessions_copy(&dir, true);
        let _ = extensions.dev(&dir);

        let kinds = SharedDocKinds::new();
        let compiled_in = DocKindDecl {
            id: "test.session".into(),
            name: "Session".into(),
            files: vec!["session".into()],
            ..DocKindDecl::default()
        };
        let _feed = ExtensionKinds::new(kinds.clone(), vec![compiled_in], None, loaded);
        assert!(kinds.for_path("a.session").is_some(), "both declare it");

        let _ = extensions.end_dev(&dir);
        assert!(extensions.host().get("test.sessions").is_none(), "unloaded");
        assert!(kinds.for_path("a.session").is_some(), "compiled in");
    }
}
