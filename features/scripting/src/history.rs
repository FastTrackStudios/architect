//! A script's transactions as one step of a document's `crdt::History`.

use std::sync::{Arc, Mutex, PoisonError};

use crdt::History;
use crdt::loro::LoroDoc;

use crate::Transactions;

/// A document and its undo history, shared with the actions that change
/// it: a script run is one step of `history`, and every `transact` those
/// actions make while it runs folds into that step.
///
/// The actions must lock the same `history` for each change, and not
/// hold it across a call back into the Action List.
#[derive(Clone, Debug)]
pub struct HistoryTransactions {
    doc: LoroDoc,
    history: Arc<Mutex<History>>,
}

impl HistoryTransactions {
    #[must_use]
    pub const fn new(doc: LoroDoc, history: Arc<Mutex<History>>) -> Self {
        Self { doc, history }
    }

    fn with<R>(&self, f: impl FnOnce(&mut History, &LoroDoc) -> R) -> R {
        let mut history = self.history.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut history, &self.doc)
    }
}

impl Transactions for HistoryTransactions {
    fn begin(&self, name: &str) -> bool {
        self.with(|history, _| history.begin(name))
    }

    fn commit(&self) -> Option<String> {
        self.with(History::commit)
    }

    fn abort(&self) -> Option<String> {
        self.with(History::abort)
    }
}
