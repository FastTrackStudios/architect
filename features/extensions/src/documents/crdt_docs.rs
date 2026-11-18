//! Documents kept as `crdt` documents, in spaces (feature `crdt`): the
//! lab's, and any app's that keeps its texts so.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use architect::action::Principal;
use crdt::loro::{ID, LoroDoc, LoroText, UpdateOptions};
use crdt::{CrdtDoc, History};

use super::{DocEdit, DocError};
#[cfg(not(target_family = "wasm"))]
use super::{DocInfo, Documents, Edited, is_under};

/// How many steps of each document's undo are kept.
const KEEP: usize = 100;

/// Documents as `crdt` documents ([`CrdtDoc`]), each with its own undo
/// ([`History`]), in named spaces: a document's path is its space, then
/// its place in it (`notes/Inbox.md`). Cheap to clone.
///
/// Its text is the document's root text [`Self::TEXT`]: what the app's
/// editor binds (`crdt::TextBinding`) and its sync carries, so an
/// extension's transaction and the person's typing merge.
#[derive(Clone, Default)]
pub struct CrdtDocuments {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
// The undo step is Luau's and pages' (not in a browser).
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct Inner {
    /// None: any path is one.
    spaces: BTreeSet<String>,
    docs: BTreeMap<String, Entry>,
    /// A script's run, while it is one undo step across documents
    /// ([`architect_scripting::Transactions`]): what it changed, to keep
    /// or take back as one.
    step: Option<Step>,
}

/// One run's step across documents.
#[derive(Default)]
// The undo step is Luau's and pages' (not in a browser).
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct Step {
    name: String,
    /// Documents whose own history has the step open.
    edited: BTreeSet<String>,
    /// Documents it made: gone again if it is taken back.
    created: Vec<String>,
    /// Documents it deleted, as they were.
    deleted: Vec<(String, Entry)>,
    /// What it renamed, `(from, to)`, in order.
    renamed: Vec<(String, String)>,
}

struct Entry {
    doc: CrdtDoc,
    history: History,
}

impl std::fmt::Debug for CrdtDocuments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("CrdtDocuments")
            .field("spaces", &inner.spaces)
            .field("docs", &inner.docs.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl CrdtDocuments {
    /// The root text a document's text is.
    pub const TEXT: &'static str = "text";

    /// None yet, in no spaces (any path is one).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// With space `name`: once it has one, a document's path starts with
    /// one of its spaces.
    #[must_use]
    pub fn with_space(self, name: &str) -> Self {
        self.add_space(name);
        self
    }

    /// [`Self::with_space`] on one already shared.
    pub fn add_space(&self, name: &str) {
        self.lock().spaces.insert(name.to_owned());
    }

    /// Its spaces.
    #[must_use]
    pub fn spaces(&self) -> Vec<String> {
        self.lock().spaces.iter().cloned().collect()
    }

    /// The text of `doc` ([`Self::TEXT`]).
    #[must_use]
    pub fn text_of(doc: &LoroDoc) -> LoroText {
        doc.get_text(Self::TEXT)
    }

    /// Document `path`: what the app's editor and its sync use (the
    /// person's side of a merge).
    #[must_use]
    pub fn doc(&self, path: &str) -> Option<CrdtDoc> {
        self.lock().docs.get(path).map(|e| e.doc.clone())
    }

    /// Document `path`'s text now.
    #[must_use]
    pub fn text(&self, path: &str) -> Option<String> {
        self.doc(path).map(|d| Self::text_of(d.loro()).to_string())
    }

    /// Keeps `doc` (one the app opened, its persistence its own) as
    /// document `path`.
    ///
    /// # Errors
    ///
    /// A document is there already, or `path` is in no space.
    pub fn adopt(&self, path: &str, doc: CrdtDoc) -> Result<(), DocError> {
        let mut inner = self.lock();
        inner.place(path)?;
        let history = History::new(doc.loro(), KEEP);
        inner.docs.insert(path.to_owned(), Entry { doc, history });
        drop(inner);
        Ok(())
    }

    /// The names of document `path`'s undo steps, oldest first.
    #[must_use]
    pub fn steps(&self, path: &str) -> Vec<String> {
        self.lock()
            .docs
            .get(path)
            .map(|e| e.history.steps().to_vec())
            .unwrap_or_default()
    }

    /// Undoes document `path`'s last step: its name.
    // What it undid is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn undo(&self, path: &str) -> Option<String> {
        self.lock().docs.get_mut(path)?.history.undo()
    }

    /// Redoes the step of document `path` undone last: its name.
    // What it redid is for whoever wants it.
    #[allow(clippy::must_use_candidate)]
    pub fn redo(&self, path: &str) -> Option<String> {
        self.lock().docs.get_mut(path)?.history.redo()
    }

    /// Who changed document `path`: each change's author (its commit's
    /// message, `extension:acme.tidy for user:bob`), each once, in the
    /// order first seen.
    #[must_use]
    pub fn authors(&self, path: &str) -> Vec<String> {
        self.doc(path)
            .map(|d| authors_of(d.loro()))
            .unwrap_or_default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Every change's commit message in `doc`, each once.
#[must_use]
pub fn authors_of(doc: &LoroDoc) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (peer, end) in doc.oplog_vv().iter() {
        let mut at = 0;
        while at < *end {
            let Some(change) = doc.get_change(ID::new(*peer, at)) else {
                break;
            };
            if let Some(m) = change.message.as_deref()
                && !out.iter().any(|o| o == m)
            {
                out.push(m.to_owned());
            }
            let len = i32::try_from(change.len.max(1)).unwrap_or(i32::MAX);
            at = change.id.counter.saturating_add(len);
        }
    }
    out
}

impl Inner {
    /// Whether a document may be at `path`.
    fn place(&self, path: &str) -> Result<(), DocError> {
        if self.docs.contains_key(path) {
            return Err(DocError::Exists(path.to_owned()));
        }
        if self.spaces.is_empty() {
            return Ok(());
        }
        match path.split_once('/') {
            Some((space, _)) if self.spaces.contains(space) => Ok(()),
            _ => Err(DocError::NotFound(format!(
                "space for {path} (the spaces: {})",
                self.spaces.iter().cloned().collect::<Vec<_>>().join(", ")
            ))),
        }
    }
}

/// The text's length in characters after each of `edits`, from `len`:
/// why not when one is outside it.
fn check_edits(mut len: usize, edits: &[DocEdit]) -> Result<(), DocError> {
    for (i, edit) in edits.iter().enumerate() {
        len = match edit {
            DocEdit::Insert { text, .. } | DocEdit::Append { text } => {
                len.saturating_add(text.chars().count())
            }
            DocEdit::Delete { at, len: n } => {
                if at.saturating_add(*n) > len {
                    return Err(DocError::Invalid(format!(
                        "edit {i}: deletes {n} from {at}, past the text's end ({len})"
                    )));
                }
                len.saturating_sub(*n)
            }
            DocEdit::Replace { text } => text.chars().count(),
        };
    }
    Ok(())
}

/// Applies `edits` to `text`.
fn apply(text: &LoroText, edits: &[DocEdit]) -> Result<(), String> {
    for edit in edits {
        match edit {
            DocEdit::Insert { at, text: t } => {
                let at = (*at).min(text.len_unicode());
                text.insert(at, t).map_err(|e| e.to_string())?;
            }
            DocEdit::Delete { at, len } => text.delete(*at, *len).map_err(|e| e.to_string())?,
            DocEdit::Append { text: t } => text
                .insert(text.len_unicode(), t)
                .map_err(|e| e.to_string())?,
            DocEdit::Replace { text: t } => text
                .update(t, UpdateOptions::default())
                .map_err(|e| format!("{e:?}"))?,
        }
    }
    Ok(())
}

/// `edits` applied to `text` as one Loro transaction made as `by`.
///
/// The way [`CrdtDocuments`] applies them, for a document set kept as
/// plain text (a vault on disk): a `replace` is diffed and positions
/// count characters the same everywhere. Its text after.
///
/// # Errors
///
/// An edit outside the text ([`DocError::Invalid`]; nothing applied).
pub fn apply_edits(text: &str, edits: &[DocEdit], by: &Principal) -> Result<String, DocError> {
    let doc = LoroDoc::new();
    let t = CrdtDocuments::text_of(&doc);
    t.insert(0, text)
        .map_err(|e| DocError::Failed(e.to_string()))?;
    doc.commit();
    check_edits(t.len_unicode(), edits)?;
    doc.set_next_commit_message(&by.describe());
    apply(&t, edits).map_err(DocError::Failed)?;
    doc.commit();
    Ok(t.to_string())
}

// Its store is not `Send` in a browser, where nothing is: there the
// documents are the web app's own (`apps/web`), and `apply_edits` is what
// it shares from here (#495).
#[cfg(not(target_family = "wasm"))]
impl Documents for CrdtDocuments {
    fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError> {
        Ok(self
            .lock()
            .docs
            .keys()
            .filter(|p| is_under(p, under))
            .map(|p| DocInfo::of(p))
            .collect())
    }

    fn read(&self, path: &str) -> Result<String, DocError> {
        self.text(path)
            .ok_or_else(|| DocError::NotFound(format!("document {path}")))
    }

    fn create(&self, path: &str, text: &str, by: &Principal) -> Result<DocInfo, DocError> {
        let mut inner = self.lock();
        inner.place(path)?;
        let doc = CrdtDoc::ephemeral();
        if !text.is_empty() {
            doc.loro().set_next_commit_message(&by.describe());
            Self::text_of(doc.loro())
                .insert(0, text)
                .map_err(|e| DocError::Failed(e.to_string()))?;
            doc.loro().commit();
        }
        // Its undo starts after what it was made with.
        let history = History::new(doc.loro(), KEEP);
        inner.docs.insert(path.to_owned(), Entry { doc, history });
        if let Some(step) = inner.step.as_mut() {
            step.created.push(path.to_owned());
        }
        drop(inner);
        Ok(DocInfo::of(path))
    }

    fn edit(
        &self,
        path: &str,
        edits: &[DocEdit],
        step: &str,
        by: &Principal,
    ) -> Result<Edited, DocError> {
        let mut inner = self.lock();
        let Inner {
            docs, step: run, ..
        } = &mut *inner;
        let entry = docs
            .get_mut(path)
            .ok_or_else(|| DocError::NotFound(format!("document {path}")))?;
        // Inside a run's step: this document's history holds it open too.
        if let Some(run) = run.as_mut()
            && !run.edited.contains(path)
        {
            entry.history.begin(&run.name);
            run.edited.insert(path.to_owned());
        }
        let doc = entry.doc.loro();
        let text = Self::text_of(doc);
        check_edits(text.len_unicode(), edits)?;
        // What it would be, worked out first: an edit that changes
        // nothing commits nothing (no empty undo step, no save).
        let before = text.to_string();
        if apply_edits(&before, edits, by)? == before {
            drop(inner);
            return Ok(Edited {
                text: before,
                changed: false,
            });
        }
        let author = by.describe();
        entry
            .history
            .transact(doc, step, |doc| {
                doc.set_next_commit_message(&author);
                apply(&text, edits)
            })
            .map_err(DocError::Failed)?;
        drop(inner);
        Ok(Edited {
            text: text.to_string(),
            changed: true,
        })
    }

    // The document's own CRDT history: the person's steps and every
    // extension's, in the order they were made.
    fn undo(&self, path: &str, _by: &Principal) -> Result<Option<super::Undone>, DocError> {
        let mut inner = self.lock();
        let entry = inner
            .docs
            .get_mut(path)
            .ok_or_else(|| DocError::NotFound(format!("document {path}")))?;
        let step = entry.history.undo();
        let text = Self::text_of(entry.doc.loro()).to_string();
        drop(inner);
        Ok(step.map(|step| super::Undone { step, text }))
    }

    fn redo(&self, path: &str, _by: &Principal) -> Result<Option<super::Undone>, DocError> {
        let mut inner = self.lock();
        let entry = inner
            .docs
            .get_mut(path)
            .ok_or_else(|| DocError::NotFound(format!("document {path}")))?;
        let step = entry.history.redo();
        let text = Self::text_of(entry.doc.loro()).to_string();
        drop(inner);
        Ok(step.map(|step| super::Undone { step, text }))
    }

    fn rename(&self, path: &str, to: &str, _by: &Principal) -> Result<DocInfo, DocError> {
        let mut inner = self.lock();
        if !inner.docs.contains_key(path) {
            return Err(DocError::NotFound(format!("document {path}")));
        }
        inner.place(to)?;
        if let Some(entry) = inner.docs.remove(path) {
            inner.docs.insert(to.to_owned(), entry);
        }
        if let Some(step) = inner.step.as_mut() {
            if step.edited.remove(path) {
                step.edited.insert(to.to_owned());
            }
            // One it made is simply made under its new name.
            if let Some(made) = step.created.iter_mut().find(|p| *p == path) {
                to.clone_into(made);
            } else {
                step.renamed.push((path.to_owned(), to.to_owned()));
            }
        }
        drop(inner);
        Ok(DocInfo::of(to))
    }

    fn delete(&self, path: &str, _by: &Principal) -> Result<(), DocError> {
        let mut inner = self.lock();
        let entry = inner
            .docs
            .remove(path)
            .ok_or_else(|| DocError::NotFound(format!("document {path}")))?;
        if let Some(step) = inner.step.as_mut() {
            step.edited.remove(path);
            if let Some(i) = step.created.iter().position(|p| p == path) {
                step.created.remove(i);
            } else {
                step.deleted.push((path.to_owned(), entry));
            }
        }
        drop(inner);
        Ok(())
    }
}

/// A script's run as one undo step across every document it changes
/// (`LuauRunner`, `architect_pages`' `TsRunner`): each document's own
/// history holds the step while it runs; taken back, its edits are
/// undone, what it made is removed, what it deleted and renamed is put
/// back.
// Luau and pages' undo; no Luau in a browser (#495).
#[cfg(not(target_family = "wasm"))]
impl architect_scripting::Transactions for CrdtDocuments {
    fn begin(&self, name: &str) -> bool {
        let mut inner = self.lock();
        if inner.step.is_some() {
            return false;
        }
        inner.step = Some(Step {
            name: name.to_owned(),
            ..Step::default()
        });
        true
    }

    fn commit(&self) -> Option<String> {
        let mut inner = self.lock();
        let step = inner.step.take()?;
        let mut changed =
            !step.created.is_empty() || !step.deleted.is_empty() || !step.renamed.is_empty();
        for path in &step.edited {
            if let Some(entry) = inner.docs.get_mut(path) {
                changed |= entry.history.commit(entry.doc.loro()).is_some();
            }
        }
        changed.then_some(step.name)
    }

    fn abort(&self) -> Option<String> {
        let mut inner = self.lock();
        let step = inner.step.take()?;
        let mut changed = false;
        for path in &step.edited {
            if let Some(entry) = inner.docs.get_mut(path) {
                changed |= entry.history.abort(entry.doc.loro()).is_some();
            }
        }
        for (from, to) in step.renamed.iter().rev() {
            if let Some(entry) = inner.docs.remove(to) {
                inner.docs.insert(from.clone(), entry);
                changed = true;
            }
        }
        for path in &step.created {
            changed |= inner.docs.remove(path).is_some();
        }
        for (path, entry) in step.deleted {
            inner.docs.insert(path, entry);
            changed = true;
        }
        drop(inner);
        changed.then_some(step.name)
    }
}

#[cfg(test)]
mod tests {
    use architect::action::Principal;
    use crdt::loro::{ExportMode, LoroDoc};

    use super::{CrdtDocuments, DocEdit, Documents, authors_of};

    fn ext() -> Principal {
        Principal::extension(
            "lab.tidy",
            Some(Principal::User {
                user_id: "bob".to_owned(),
            }),
        )
    }

    #[test]
    fn a_transaction_is_one_step_and_says_who_made_it() {
        let docs = CrdtDocuments::new().with_space("notes");
        assert!(docs.create("a.md", "", &ext()).is_err(), "in no space");
        docs.create("notes/a.md", "hello", &ext()).unwrap();
        let after = docs
            .edit(
                "notes/a.md",
                &[
                    DocEdit::Insert {
                        at: 0,
                        text: "# ".to_owned(),
                    },
                    DocEdit::Append {
                        text: " world".to_owned(),
                    },
                    DocEdit::Delete { at: 0, len: 1 },
                ],
                "Tidy",
                &ext(),
            )
            .unwrap();
        assert_eq!(after.text, " hello world");
        assert!(after.changed);
        assert_eq!(docs.steps("notes/a.md"), ["Tidy"]);
        // An edit that changes nothing commits nothing.
        let same = docs
            .edit(
                "notes/a.md",
                &[DocEdit::Replace {
                    text: " hello world".to_owned(),
                }],
                "Same",
                &ext(),
            )
            .unwrap();
        assert!(!same.changed);
        assert_eq!(docs.steps("notes/a.md"), ["Tidy"], "no empty step");
        assert!(
            docs.edit(
                "notes/a.md",
                &[DocEdit::Delete { at: 5, len: 99 }],
                "No",
                &ext()
            )
            .is_err()
        );
        assert_eq!(docs.steps("notes/a.md"), ["Tidy"], "nothing changed");
        assert_eq!(docs.undo("notes/a.md").as_deref(), Some("Tidy"));
        assert_eq!(docs.text("notes/a.md").as_deref(), Some("hello"));
        assert_eq!(
            docs.authors("notes/a.md"),
            ["extension:lab.tidy for user:bob"]
        );
    }

    #[test]
    fn the_persons_concurrent_typing_merges_with_it() {
        let docs = CrdtDocuments::new();
        docs.create("a.md", "one two", &ext()).unwrap();
        let ours = docs.doc("a.md").unwrap();
        let person = LoroDoc::new();
        person
            .import(&ours.loro().export(ExportMode::Snapshot).unwrap())
            .unwrap();
        CrdtDocuments::text_of(&person).insert(0, "Zero ").unwrap();
        person.commit();
        docs.edit(
            "a.md",
            &[DocEdit::Append {
                text: " three".to_owned(),
            }],
            "More",
            &ext(),
        )
        .unwrap();
        let theirs = person
            .export(ExportMode::updates(&ours.loro().oplog_vv()))
            .unwrap();
        ours.loro().import(&theirs).unwrap();
        person
            .import(&ours.loro().export(ExportMode::Snapshot).unwrap())
            .unwrap();
        assert_eq!(docs.text("a.md").as_deref(), Some("Zero one two three"));
        assert_eq!(
            CrdtDocuments::text_of(&person).to_string(),
            "Zero one two three"
        );
        assert!(authors_of(&person).contains(&"extension:lab.tidy for user:bob".to_owned()));
    }

    #[test]
    fn a_run_is_one_step_across_documents_kept_or_taken_back() {
        use architect_scripting::Transactions as _;
        let docs = CrdtDocuments::new();
        let by = ext();
        docs.create("notes/a.md", "one", &by).unwrap();
        docs.create("notes/gone.md", "bye", &by).unwrap();
        docs.create("notes/old.md", "x", &by).unwrap();
        // Taken back: the edit, the new document, the delete, the rename.
        assert!(docs.begin("Recipes: Scale"));
        assert!(
            !docs.begin("inner"),
            "a run inside a run is part of its step"
        );
        docs.edit(
            "notes/a.md",
            &[DocEdit::Append {
                text: " two".to_owned(),
            }],
            "e",
            &by,
        )
        .unwrap();
        docs.create("notes/new.md", "made", &by).unwrap();
        docs.delete("notes/gone.md", &by).unwrap();
        docs.rename("notes/old.md", "notes/renamed.md", &by)
            .unwrap();
        assert_eq!(docs.abort().as_deref(), Some("Recipes: Scale"));
        assert_eq!(docs.text("notes/a.md").as_deref(), Some("one"));
        assert!(docs.text("notes/new.md").is_none());
        assert_eq!(docs.text("notes/gone.md").as_deref(), Some("bye"));
        assert_eq!(docs.text("notes/old.md").as_deref(), Some("x"));
        assert!(docs.text("notes/renamed.md").is_none());
        // Kept: one step in the document's history, named for the run.
        assert!(docs.begin("Recipes: Scale"));
        docs.edit(
            "notes/a.md",
            &[DocEdit::Append {
                text: " two".to_owned(),
            }],
            "e1",
            &by,
        )
        .unwrap();
        docs.edit(
            "notes/a.md",
            &[DocEdit::Append {
                text: " three".to_owned(),
            }],
            "e2",
            &by,
        )
        .unwrap();
        assert_eq!(docs.commit().as_deref(), Some("Recipes: Scale"));
        assert_eq!(docs.text("notes/a.md").as_deref(), Some("one two three"));
        assert_eq!(
            docs.steps("notes/a.md").last().map(String::as_str),
            Some("Recipes: Scale")
        );
        assert_eq!(docs.undo("notes/a.md").as_deref(), Some("Recipes: Scale"));
        assert_eq!(docs.text("notes/a.md").as_deref(), Some("one"));
        // A run that changed nothing is no step.
        assert!(docs.begin("nothing"));
        assert_eq!(docs.commit(), None);
    }
}
