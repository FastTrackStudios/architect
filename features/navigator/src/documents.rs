//! The person's vault as extensions' documents (feature `documents`,
//! #706): [`VaultDocuments`], an `architect_extensions` [`Documents`]
//! over a [`VaultSource`], and the document events for the person's own
//! edits ([`VaultDocuments::raise_on`]).
//!
//! Every change goes through the same [`VaultSource`] the app's navigator
//! and editors use, so an open note hears an extension's edit as it hears
//! any other (`SourceEvent::FileModified`) and merges it into its typing
//! (`vault_doc::merge3`, two forks of one Loro document); a rename keeps
//! the note's links (`OpenVault::rename`); a delete goes to the vault's
//! `.trash`, as the navigator's does.
//!
//! **An edit is a transaction.** Its `DocEdit`s apply in order to the
//! note's text as one Loro commit made as the extension
//! (`architect_extensions::documents::apply_edits`), and the result is one
//! write. If the note changed between the read and the write (the
//! person's editor saved), the two are merged by the same three-way merge
//! an open note uses, so neither side's characters are lost.
//!
//! **Events.** The extension host raises `doc.*` for what an extension
//! does here; [`VaultDocuments::raise_on`] raises them for everything
//! else the source says (the person's typing as it saves, a rename or
//! delete in the navigator, an edit made outside the app), on the app's
//! Action List, where the host hears them (`Host::watch`). The
//! extension's own changes are left out there, so each is heard once.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionList, Principal};
use architect_extensions::documents::{
    DocEdit, DocError, DocInfo, Documents, apply_edits, is_under,
};
use architect_extensions::events::DocEvent;

use crate::source::{FileOp, NavigatorSource, SourceEvent, Subscription};
use crate::vault_source::VaultSource;

thread_local! {
    /// The paths an extension's call on this thread is changing: its
    /// events are the host's to raise, not the bridge's.
    static QUIET: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with `paths` kept quiet on this thread.
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

/// The document event a source's event is, for the person's own edits
/// (none for a folder's, a file that is not a note, or the extension's
/// own change on this thread).
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

/// The person's vault, as extensions' documents: its notes (`.md`), by
/// their vault paths (`Projects/Plan.md`; the first folder is the
/// document's space). Cheap to clone; clones share the source.
#[derive(Clone)]
pub struct VaultDocuments {
    source: VaultSource,
    /// One extension write at a time: a read and its write together.
    writing: Arc<Mutex<()>>,
}

impl std::fmt::Debug for VaultDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultDocuments")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl VaultDocuments {
    /// The documents of `source`: the same source the app's navigator and
    /// editors use, so they hear every change.
    #[must_use]
    pub fn new(source: VaultSource) -> Self {
        Self {
            source,
            writing: Arc::new(Mutex::new(())),
        }
    }

    /// The source it changes.
    #[must_use]
    pub const fn source(&self) -> &VaultSource {
        &self.source
    }

    /// Raises `doc.created`, `doc.saved`, `doc.renamed` and `doc.deleted`
    /// on `list` for every change the source says that an extension did
    /// not make through these documents (the host raised those): the
    /// person's typing as it is saved, a new note, a rename, move or
    /// delete in the navigator, an edit made outside the app. Until the
    /// subscription is dropped.
    pub fn raise_on(&self, list: &ActionList) -> Subscription {
        let list = list.clone();
        self.source.subscribe(Box::new(move |event| {
            if let Some(ev) = doc_event(event) {
                ev.raise_on(&list);
            }
        }))
    }

    fn vault_text(&self, path: &str) -> Option<String> {
        self.source.vault().text(path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn not_found(path: &str) -> DocError {
    DocError::NotFound(format!("document {path}"))
}

impl Documents for VaultDocuments {
    fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError> {
        Ok(self.source.vault().with(|v| {
            v.pages
                .iter()
                .map(|p| p.rel_path.as_str())
                .filter(|p| is_under(p, under))
                .map(DocInfo::of)
                .collect()
        }))
    }

    fn read(&self, path: &str) -> Result<String, DocError> {
        self.vault_text(path).ok_or_else(|| not_found(path))
    }

    fn create(&self, path: &str, text: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        if !is_note(path) {
            return Err(DocError::Invalid(format!(
                "{path:?} is not a note's path (a vault path ending in .md)"
            )));
        }
        let _w = self.lock();
        if self.vault_text(path).is_some() {
            return Err(DocError::Exists(path.to_owned()));
        }
        quietly(&[path], || {
            self.source.write(&[FileOp::CreateFile {
                path: path.to_owned(),
                text: text.to_owned(),
            }])
        })
        .map_err(DocError::Failed)?;
        Ok(DocInfo::of(path))
    }

    fn edit(
        &self,
        path: &str,
        edits: &[DocEdit],
        // A file on disk keeps no undo of its own: the open note takes
        // the change as one edit of its typing.
        _step: &str,
        by: &Principal,
    ) -> Result<String, DocError> {
        let _w = self.lock();
        let base = self.vault_text(path).ok_or_else(|| not_found(path))?;
        let ours = apply_edits(&base, edits, by)?;
        // The person's editor may have saved since: both are kept.
        let now = self.vault_text(path).ok_or_else(|| not_found(path))?;
        let text = if now == base {
            ours
        } else {
            vault_doc::merge3(&base, &ours, &now)
        };
        quietly(&[path], || self.source.save(path, &text)).map_err(DocError::Failed)?;
        Ok(text)
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        let _w = self.lock();
        if self.vault_text(path).is_none() {
            return Err(not_found(path));
        }
        if self.vault_text(to).is_some() {
            return Err(DocError::Exists(to.to_owned()));
        }
        if !is_note(to) {
            return Err(DocError::Invalid(format!(
                "{to:?} is not a note's path (a vault path ending in .md)"
            )));
        }
        quietly(&[path, to], || self.source.rename(path, to)).map_err(DocError::Failed)?;
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        let _w = self.lock();
        if self.vault_text(path).is_none() {
            return Err(not_found(path));
        }
        quietly(&[path], || {
            self.source.write(&[FileOp::Trash(path.to_owned())])
        })
        .map_err(DocError::Failed)
    }
}
