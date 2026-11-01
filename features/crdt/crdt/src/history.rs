//! Transactions and named undo over a Loro document
//! (`docs/content/architecture/documents.md`, "Undo and transactions").
//!
//! Every change goes through [`History::transact`]: one named step
//! ("Split item", "Script: Arm selected"), undone as one. Undo is local
//! — Loro's `UndoManager` only undoes this peer's changes, so undoing my
//! step never undoes a collaborator's that arrived meanwhile.
//!
//! A step can also span calls that each make their own: [`History::begin`]
//! opens one, every [`History::transact`] until [`History::commit`] folds
//! into it, and [`History::abort`] takes back what it changed. That is
//! how a script that runs ten actions, each a transaction of its own, is
//! still one step: the runner opens it, the actions fold in.

use loro::{LoroDoc, UndoManager};

/// A document's undo history, with each step's name.
pub struct History {
    undo: UndoManager,
    /// The steps that can be undone, oldest first.
    done: Vec<String>,
    /// The steps undone that can be redone, the next on top.
    undone: Vec<String>,
    keep: usize,
    /// The step [`History::begin`] opened, while it is open.
    open: Option<Open>,
}

/// A step opened by [`History::begin`] and not yet committed.
#[derive(Debug)]
struct Open {
    name: String,
    /// Loro's undo count when it opened: more at the end is a step.
    before: usize,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("done", &self.done)
            .field("undone", &self.undone)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl History {
    /// The history of `doc`, keeping the last `keep` steps.
    #[must_use]
    pub fn new(doc: &LoroDoc, keep: usize) -> Self {
        let mut undo = UndoManager::new(doc);
        // A step is what a transaction says it is: never merged by time.
        undo.set_merge_interval(0);
        undo.set_max_undo_steps(keep);
        Self {
            undo,
            done: Vec::new(),
            undone: Vec::new(),
            keep,
            open: None,
        }
    }

    /// Runs `change` as one step named `name`. A change that changed
    /// nothing is no step. Inside a step [`Self::begin`] opened, it is
    /// part of that step, under that step's name.
    pub fn transact<R>(
        &mut self,
        doc: &LoroDoc,
        name: &str,
        change: impl FnOnce(&LoroDoc) -> R,
    ) -> R {
        if self.open.is_some() {
            let out = change(doc);
            doc.commit();
            return out;
        }
        self.begin(name);
        let out = change(doc);
        self.commit(doc);
        out
    }

    /// Opens a step named `name`: every change until [`Self::commit`] —
    /// the [`Self::transact`]s of whatever runs meanwhile included — is
    /// that one step. False, and nothing opened, when a step is already
    /// open: what runs now is part of it.
    pub fn begin(&mut self, name: &str) -> bool {
        if self.open.is_some() {
            return false;
        }
        let before = self.undo.undo_count();
        let _ = self.undo.group_start();
        self.open = Some(Open {
            name: name.to_owned(),
            before,
        });
        true
    }

    /// Closes the step [`Self::begin`] opened: its name, when it changed
    /// something (a step that changed nothing is none). `None` too when
    /// no step is open.
    pub fn commit(&mut self, doc: &LoroDoc) -> Option<String> {
        let open = self.open.take()?;
        doc.commit();
        self.undo.group_end();
        if self.undo.undo_count() <= open.before {
            return None;
        }
        self.done.push(open.name.clone());
        self.undone.clear();
        self.trim();
        Some(open.name)
    }

    /// Closes the step [`Self::begin`] opened and takes back what it
    /// changed, as Undo would (so Redo brings it back, under its name).
    /// Its name, when it had changed something.
    pub fn abort(&mut self, doc: &LoroDoc) -> Option<String> {
        self.commit(doc)?;
        self.undo()
    }

    /// The name of the step [`Self::begin`] opened, while it is open.
    #[must_use]
    pub fn open_step(&self) -> Option<&str> {
        self.open.as_ref().map(|open| open.name.as_str())
    }

    /// Undoes the last step; its name.
    pub fn undo(&mut self) -> Option<String> {
        if !self.undo.can_undo() || !self.undo.undo().unwrap_or(false) {
            return None;
        }
        let name = self.done.pop().unwrap_or_default();
        self.undone.push(name.clone());
        Some(name)
    }

    /// Redoes the last step undone; its name.
    pub fn redo(&mut self) -> Option<String> {
        if !self.undo.can_redo() || !self.undo.redo().unwrap_or(false) {
            return None;
        }
        let name = self.undone.pop().unwrap_or_default();
        self.done.push(name.clone());
        Some(name)
    }

    /// What Undo would undo ("Undo Split item").
    #[must_use]
    pub fn next_undo(&self) -> Option<&str> {
        self.undo
            .can_undo()
            .then(|| self.done.last().map(String::as_str))
            .flatten()
    }

    /// What Redo would redo.
    #[must_use]
    pub fn next_redo(&self) -> Option<&str> {
        self.undo
            .can_redo()
            .then(|| self.undone.last().map(String::as_str))
            .flatten()
    }

    /// Every step that can be undone, oldest first (a history list).
    #[must_use]
    pub fn steps(&self) -> &[String] {
        &self.done
    }

    /// Forgets every step (after loading, or a migration).
    pub fn clear(&mut self) {
        if self.open.take().is_some() {
            self.undo.group_end();
        }
        self.undo.clear();
        self.done.clear();
        self.undone.clear();
    }

    fn trim(&mut self) {
        let over = self.done.len().saturating_sub(self.keep);
        if over > 0 {
            self.done.drain(..over);
        }
    }
}
