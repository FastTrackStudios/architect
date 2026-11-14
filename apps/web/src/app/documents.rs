//! The web vault as extensions' documents (#495 H1, H2): what
//! `architect_navigator::documents::VaultDocuments` is on a desk (#706),
//! over the web app's `MemorySource` (the demo vault in `?demo`, the
//! signed-in org's vaults otherwise), the same source the navigator and
//! the note views use, so each hears an extension's change as it hears any
//! other.
//!
//! **An edit** is the extension host's semantics
//! (`architect_extensions::documents::apply_edits`): its `DocEdit`s in
//! order on the note's text, one change. A browser has one thread, so
//! nothing saves the note between the read and the write.
//!
//! **On the org's vaults** a create or an edit is the page's copy at once
//! and the node's through the lane (`Notes::write`, sent in order from the
//! page's thread): what fails is reported as the vault's problems. A
//! rename, move or delete is the desktop app's for now: the org lane has
//! no such call, and the web navigator's layout is read-only there too.
//!
//! **Events.** The host raises `doc.*` for what an extension does here;
//! [`WebDocuments::raise_on`] raises them for everything else the source
//! says (the person's typing as it is kept, a new note, a note the node
//! changed) on the app's list, where the host hears them. The extension's
//! own changes are left out there, so each is heard once.

use std::cell::RefCell;

use architect::action::{ActionList, Principal};
use architect_extensions::documents::{
    DocEdit, DocError, DocInfo, Documents, apply_edits, is_under,
};
use architect_extensions::events::DocEvent;
use architect_navigator::markdown::note_from_markdown;
use architect_navigator::{FileOp, MemorySource, NavigatorSource as _, SourceEvent, Subscription};
use futures_channel::mpsc::UnboundedSender;

thread_local! {
    /// The paths an extension's call is changing now: its events are the
    /// host's to raise, not [`WebDocuments::raise_on`]'s.
    static QUIET: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `paths` kept quiet.
fn quietly<R>(paths: &[&str], f: impl FnOnce() -> R) -> R {
    QUIET.with(|q| q.borrow_mut().extend(paths.iter().map(|p| (*p).to_owned())));
    let out = f();
    QUIET.with(|q| {
        let mut q = q.borrow_mut();
        let keep = q.len().saturating_sub(paths.len());
        q.truncate(keep);
    });
    out
}

fn is_quiet(path: &str) -> bool {
    QUIET.with(|q| q.borrow().iter().any(|p| p == path))
}

/// Whether `path` is one of the vault's documents: a note.
fn is_note(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// The document event a source's event is, for the person's own edits:
/// none for a folder's, a file that is not a note, or an extension's own
/// change.
#[must_use]
pub fn doc_event(event: &SourceEvent) -> Option<DocEvent> {
    let ev = match event {
        SourceEvent::FileCreated(f) if is_note(&f.path) => DocEvent::Created {
            path: f.path.clone(),
        },
        SourceEvent::FileModified(f) if is_note(&f.path) => DocEvent::Saved {
            path: f.path.clone(),
        },
        SourceEvent::FileDeleted(p) if is_note(p) => DocEvent::Deleted { path: p.clone() },
        SourceEvent::FileRenamed { from, file } if is_note(from) || is_note(&file.path) => {
            DocEvent::Renamed {
                from: from.clone(),
                to: file.path.clone(),
            }
        }
        _ => return None,
    };
    let quiet = match &ev {
        DocEvent::Created { path } | DocEvent::Saved { path } | DocEvent::Deleted { path } => {
            is_quiet(path)
        }
        DocEvent::Renamed { from, to } => is_quiet(from) || is_quiet(to),
    };
    (!quiet).then_some(ev)
}

/// A change for the node: a note's whole text at its vault path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeWrite {
    pub path: String,
    pub text: String,
}

/// The web vault, as extensions' documents. Cheap to clone.
#[derive(Clone)]
pub struct WebDocuments {
    source: MemorySource,
    /// On the org's vaults: where each change goes to be sent to the node.
    node: Option<UnboundedSender<NodeWrite>>,
}

impl std::fmt::Debug for WebDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDocuments")
            .field("org", &self.node.is_some())
            .finish_non_exhaustive()
    }
}

impl WebDocuments {
    /// The demo's documents: `source` alone, in the tab's memory.
    #[must_use]
    pub const fn demo(source: MemorySource) -> Self {
        Self { source, node: None }
    }

    /// The org's: `source`, each change also sent to `node`.
    #[must_use]
    pub const fn org(source: MemorySource, node: UnboundedSender<NodeWrite>) -> Self {
        Self {
            source,
            node: Some(node),
        }
    }

    /// Raises `doc.created`, `doc.saved`, `doc.renamed` and `doc.deleted`
    /// on `list` for every change the source says that an extension did
    /// not make here (the host raised those), until the subscription is
    /// dropped.
    pub fn raise_on(&self, list: &ActionList) -> Subscription {
        let list = list.clone();
        self.source.subscribe(Box::new(move |event| {
            if let Some(ev) = doc_event(event) {
                ev.raise_on(&list);
            }
        }))
    }

    /// Note `path`'s text, while the vault lists it (a removed note's
    /// text stays in the source's memory).
    fn text(&self, path: &str) -> Option<String> {
        self.source
            .files()
            .iter()
            .any(|f| f.path == path)
            .then(|| self.source.read_text(path))
            .flatten()
    }

    /// Keeps `text` as the note at `path`'s, here and on the node.
    fn keep(&self, path: &str, text: &str) -> Result<(), DocError> {
        let now = super::vault::now();
        quietly(&[path], || {
            self.source
                .set_text(note_from_markdown(path, text, now, now), text.to_owned());
        });
        if let Some(node) = &self.node {
            node.unbounded_send(NodeWrite {
                path: path.to_owned(),
                text: text.to_owned(),
            })
            .map_err(|_| DocError::Failed("the org's vaults are gone".to_owned()))?;
        } else {
            super::vault::mark_edited();
        }
        Ok(())
    }

    /// A rename or a delete, which the org's lane has no call for.
    fn local_only(&self, what: &str) -> Result<(), DocError> {
        if self.node.is_some() {
            return Err(DocError::Failed(format!(
                "{what} a note on the org's vaults is the desktop app's for now"
            )));
        }
        Ok(())
    }
}

fn not_found(path: &str) -> DocError {
    DocError::NotFound(format!("document {path}"))
}

impl Documents for WebDocuments {
    fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError> {
        Ok(self
            .source
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| is_note(p) && is_under(p, under))
            .map(DocInfo::of)
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, DocError> {
        self.text(path).ok_or_else(|| not_found(path))
    }

    fn create(&self, path: &str, text: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        if !is_note(path) {
            return Err(DocError::Invalid(format!(
                "{path:?} is not a note's path (a vault path ending in .md)"
            )));
        }
        if self.text(path).is_some() {
            return Err(DocError::Exists(path.to_owned()));
        }
        self.keep(path, text)?;
        Ok(DocInfo::of(path))
    }

    fn edit(
        &self,
        path: &str,
        edits: &[DocEdit],
        _step: &str,
        by: &Principal,
    ) -> Result<String, DocError> {
        let base = self.text(path).ok_or_else(|| not_found(path))?;
        let text = apply_edits(&base, edits, by)?;
        self.keep(path, &text)?;
        Ok(text)
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        self.local_only("renaming")?;
        if self.text(path).is_none() {
            return Err(not_found(path));
        }
        if self.text(to).is_some() {
            return Err(DocError::Exists(to.to_owned()));
        }
        if !is_note(to) {
            return Err(DocError::Invalid(format!(
                "{to:?} is not a note's path (a vault path ending in .md)"
            )));
        }
        quietly(&[path, to], || {
            self.source.write(&[FileOp::Move {
                from: path.to_owned(),
                to: to.to_owned(),
            }])
        })
        .map_err(DocError::Failed)?;
        super::vault::mark_edited();
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        self.local_only("deleting")?;
        if self.text(path).is_none() {
            return Err(not_found(path));
        }
        quietly(&[path], || self.source.remove(path));
        super::vault::mark_edited();
        Ok(())
    }
}
