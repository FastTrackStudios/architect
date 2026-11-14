//! The person's vault as extensions' documents (feature `documents`, #706).
//!
//! [`VaultDocuments`] is an `architect_extensions` [`Documents`] over a
//! [`VaultSource`], with the document events for the person's own edits
//! ([`VaultDocuments::raise_on`]).
//!
//! Every change goes through the same [`VaultSource`] the app's navigator
//! and editors use, so an open note hears an extension's edit as it hears
//! any other (`SourceEvent::FileModified`) and merges it into its typing
//! (`vault_doc::merge3`, two forks of one Loro document); a rename keeps
//! the note's links (`OpenVault::rename`); a delete goes to the vault's
//! `.trash`, as the navigator's does. No write goes through a symlink
//! (`vault_live::own::through_symlink`, checked by the vault's own write
//! path), and none to a path with a hidden part (`.obsidian/…`).
//!
//! **An edit is a transaction.** Its `DocEdit`s apply in order to the
//! note's text with a Loro document's semantics
//! (`architect_extensions::documents::apply_edits`: a `replace` diffed,
//! positions in characters), and the result is one compare-and-swap
//! write ([`VaultSource::save_if`], under the vault's write lock). If the
//! note changed since it was read (the person's editor saved), the two
//! are merged by the three-way merge an open note uses and written again,
//! so neither side's characters are lost. A file keeps no authorship or
//! undo of its own: the open note takes the change as one edit.
//!
//! **Events.** The extension host raises `doc.*` for what an extension
//! does here; [`VaultDocuments::raise_on`] raises them for everything
//! else the source says (the person's typing as it saves, a rename or
//! delete in the navigator, a folder moved or deleted, an edit made
//! outside the app), on the app's Action List, where the host hears them
//! (`Host::watch`). What an extension's call changes, its rename's
//! relinked notes too, is left out there: the host raised its event. They
//! are raised in ticks ([`TICK`]): the same event twice in one is raised
//! once, and at most [`PER_TICK`] go out in one, the rest in the next, so
//! a git pull or a sync never floods every extension's hooks at once.

use std::cell::Cell;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionList, Principal};
use architect_extensions::documents::{
    DocEdit, DocError, DocInfo, Documents, Edited, apply_edits, is_under,
};
use architect_extensions::events::DocEvent;

use crate::source::{FileOp, NavigatorSource, SourceEvent, Subscription};
use crate::vault_source::VaultSource;

/// How often the person's edits go out as events.
pub const TICK: Duration = Duration::from_millis(100);
/// How many events go out in one [`TICK`] at most.
pub const PER_TICK: usize = 64;
/// How many times an edit is merged and written again when the note
/// keeps changing under it.
const TRIES: usize = 8;

thread_local! {
    /// Whether an extension's call on this thread is changing the vault:
    /// its events are the host's to raise, not the bridge's.
    static QUIET: Cell<u32> = const { Cell::new(0) };
}

/// Runs `f` with this thread's changes kept quiet.
fn quietly<R>(f: impl FnOnce() -> R) -> R {
    QUIET.with(|q| q.set(q.get().saturating_add(1)));
    let out = f();
    QUIET.with(|q| q.set(q.get().saturating_sub(1)));
    out
}

fn is_quiet() -> bool {
    QUIET.with(|q| q.get() > 0)
}

/// Whether `path` is one of the vault's documents: a note.
fn is_note(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `path` under folder `from`, moved to folder `to`.
fn moved(path: &str, from: &str, to: &str) -> Option<String> {
    path.strip_prefix(from)
        .filter(|r| r.starts_with('/'))
        .map(|r| format!("{to}{r}"))
}

/// The document events a source's event is, one per note.
///
/// Given the notes there were before it (`known`, kept up to date), a
/// folder's moved or deleted notes are each their own. None for a file
/// that is not a note.
#[must_use]
pub fn doc_events(event: &SourceEvent, known: &mut BTreeSet<String>) -> Vec<DocEvent> {
    match event {
        SourceEvent::FileCreated(f) if is_note(&f.path) => {
            known.insert(f.path.clone());
            vec![DocEvent::Created {
                path: f.path.clone(),
            }]
        }
        SourceEvent::FileModified(f) if is_note(&f.path) => {
            known.insert(f.path.clone());
            vec![DocEvent::Saved {
                path: f.path.clone(),
            }]
        }
        SourceEvent::FileDeleted(p) if is_note(p) => {
            known.remove(p);
            vec![DocEvent::Deleted { path: p.clone() }]
        }
        SourceEvent::FileRenamed { from, file } if is_note(from) || is_note(&file.path) => {
            known.remove(from);
            known.insert(file.path.clone());
            vec![DocEvent::Renamed {
                from: from.clone(),
                to: file.path.clone(),
            }]
        }
        SourceEvent::FolderRenamed { from, to } => {
            let (inside, rest): (BTreeSet<String>, BTreeSet<String>) = std::mem::take(known)
                .into_iter()
                .partition(|p| moved(p, from, to).is_some());
            *known = rest;
            inside
                .into_iter()
                .filter_map(|p| {
                    let n = moved(&p, from, to)?;
                    known.insert(n.clone());
                    Some(DocEvent::Renamed { from: p, to: n })
                })
                .collect()
        }
        SourceEvent::FolderDeleted(folder) => {
            let (inside, rest): (BTreeSet<String>, BTreeSet<String>) = std::mem::take(known)
                .into_iter()
                .partition(|p| is_under(p, Some(folder)));
            *known = rest;
            inside
                .into_iter()
                .map(|p| DocEvent::Deleted { path: p })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The events waiting for the next tick, the same one once.
#[derive(Default)]
struct Pending {
    queue: VecDeque<DocEvent>,
}

impl Pending {
    fn push(&mut self, ev: DocEvent) {
        if !self.queue.contains(&ev) {
            self.queue.push_back(ev);
        }
    }

    fn take(&mut self, n: usize) -> Vec<DocEvent> {
        let n = n.min(self.queue.len());
        self.queue.drain(..n).collect()
    }
}

/// The person's vault, as extensions' documents: its notes (`.md`), by
/// their vault paths (`Projects/Plan.md`; the first folder is the
/// document's space). Cheap to clone; clones share the source.
#[derive(Clone)]
pub struct VaultDocuments {
    source: VaultSource,
    /// One extension create, rename or delete at a time.
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
    /// delete in the navigator (a folder's: each note in it), an edit made
    /// outside the app. In ticks of [`TICK`], at most [`PER_TICK`] each,
    /// the same event once. Until the subscription is dropped.
    pub fn raise_on(&self, list: &ActionList) -> Subscription {
        let known: BTreeSet<String> = self
            .source
            .vault()
            .with(|v| v.pages.iter().map(|p| p.rel_path.clone()).collect());
        let known = Arc::new(Mutex::new(known));
        let pending = Arc::new(Mutex::new(Pending::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sub = {
            let (known, pending) = (Arc::clone(&known), Arc::clone(&pending));
            self.source.subscribe(Box::new(move |event| {
                let mut known = known.lock().unwrap_or_else(PoisonError::into_inner);
                let events = doc_events(event, &mut known);
                drop(known);
                // The extension's own change: the host raised it.
                if is_quiet() {
                    return;
                }
                let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
                for ev in events {
                    pending.push(ev);
                }
            }))
        };
        let (list, flag) = (list.clone(), Arc::clone(&stop));
        let _ = std::thread::Builder::new()
            .name("vault-doc-events".to_owned())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(TICK);
                    let out = pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take(PER_TICK);
                    for ev in out {
                        ev.raise_on(&list);
                    }
                }
            });
        Subscription::new(move || {
            stop.store(true, Ordering::Relaxed);
            drop(sub);
        })
    }

    fn vault_text(&self, path: &str) -> Option<String> {
        self.source.vault().text(path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writing.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// `path` as a case- and normalisation-blind file system names it (APFS,
/// NTFS): `Notes/Café.md` and `notes/cafe\u{301}.md` are one file there.
fn folded(path: &str) -> String {
    use unicode_normalization::UnicodeNormalization as _;
    path.nfc().collect::<String>().to_lowercase()
}

/// The note other than `except` that `path` would be on such a file
/// system, if there is one: a create or rename there is refused on every
/// platform, so a vault behaves the same on the desk and on iOS.
fn collides(docs: &VaultDocuments, path: &str, except: Option<&str>) -> Option<String> {
    let key = folded(path);
    docs.source.vault().with(|v| {
        v.pages
            .iter()
            .map(|p| p.rel_path.as_str())
            .find(|p| Some(*p) != except && folded(p) == key)
            .map(str::to_owned)
    })
}

fn not_found(path: &str) -> DocError {
    DocError::NotFound(format!("document {path}"))
}

/// `path` a note's, with no hidden part (`.obsidian/…`, `.trash/…`).
fn note_path(path: &str) -> Result<(), DocError> {
    if path.split('/').any(|p| p.starts_with('.')) {
        return Err(DocError::Invalid(format!(
            "{path:?} has a hidden part: not one of the vault's notes"
        )));
    }
    if !is_note(path) {
        return Err(DocError::Invalid(format!(
            "{path:?} is not a note's path (a vault path ending in .md)"
        )));
    }
    Ok(())
}

impl Documents for VaultDocuments {
    fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError> {
        Ok(self.source.vault().with(|v| {
            v.pages
                .iter()
                .map(|p| p.rel_path.as_str())
                .filter(|p| is_under(p, under) && note_path(p).is_ok())
                .map(DocInfo::of)
                .collect()
        }))
    }

    fn read(&self, path: &str) -> Result<String, DocError> {
        note_path(path)?;
        self.vault_text(path).ok_or_else(|| not_found(path))
    }

    fn create(&self, path: &str, text: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        note_path(path)?;
        let _w = self.lock();
        if let Some(there) = collides(self, path, None) {
            return Err(DocError::Exists(there));
        }
        quietly(|| {
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
    ) -> Result<Edited, DocError> {
        note_path(path)?;
        let base = self.vault_text(path).ok_or_else(|| not_found(path))?;
        let ours = apply_edits(&base, edits, by)?;
        let mut now = base.clone();
        for _ in 0..TRIES {
            // The person's editor may have saved since: both are kept.
            let text = if now == base {
                ours.clone()
            } else {
                vault_doc::merge3(&base, &ours, &now)
            };
            if text == now {
                return Ok(Edited {
                    text,
                    changed: false,
                });
            }
            if quietly(|| self.source.save_if(path, &now, &text)).map_err(DocError::Failed)? {
                return Ok(Edited {
                    text,
                    changed: true,
                });
            }
            now = self.vault_text(path).ok_or_else(|| not_found(path))?;
        }
        Err(DocError::Failed(format!(
            "{path} kept changing while it was edited: nothing written"
        )))
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        note_path(path)?;
        note_path(to)?;
        let _w = self.lock();
        if self.vault_text(path).is_none() {
            return Err(not_found(path));
        }
        if let Some(there) = collides(self, to, Some(path)) {
            return Err(DocError::Exists(there));
        }
        quietly(|| self.source.rename(path, to)).map_err(DocError::Failed)?;
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        note_path(path)?;
        let _w = self.lock();
        if self.vault_text(path).is_none() {
            return Err(not_found(path));
        }
        quietly(|| self.source.write(&[FileOp::Trash(path.to_owned())])).map_err(DocError::Failed)
    }
}
