//! Transactions and named undo over a Loro document
//! (`docs/content/architecture/documents.md`, "Undo and transactions").
//!
//! Every change goes through [`History::transact`]: one named step
//! ("Split item", "Script: Arm selected"), undone as one. Undo is local
//! — Loro's `UndoManager` only undoes this peer's changes, so undoing my
//! step never undoes a collaborator's that arrived meanwhile.

use loro::{LoroDoc, UndoManager};

/// A document's undo history, with each step's name.
pub struct History {
    undo: UndoManager,
    /// The steps that can be undone, oldest first.
    done: Vec<String>,
    /// The steps undone that can be redone, the next on top.
    undone: Vec<String>,
    keep: usize,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("done", &self.done)
            .field("undone", &self.undone)
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
        }
    }

    /// Runs `change` as one step named `name`. A change that changed
    /// nothing is no step.
    pub fn transact<R>(
        &mut self,
        doc: &LoroDoc,
        name: &str,
        change: impl FnOnce(&LoroDoc) -> R,
    ) -> R {
        let before = self.undo.undo_count();
        let _ = self.undo.group_start();
        let out = change(doc);
        doc.commit();
        self.undo.group_end();
        if self.undo.undo_count() > before {
            self.done.push(name.to_owned());
            self.undone.clear();
            self.trim();
        }
        out
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
