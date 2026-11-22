//! What the runtime asks of an app's vault, as contracts the app fills:
//! running a query over the vault's index for an extension
//! (`query_vault`, [`VaultQuery`]), and the note file types extensions
//! declare (`[[file_types]]`, [`NoteTypes`]).
//!
//! The runtime knows no vault. An app that keeps one gives the host a
//! [`VaultQuery`] (`Host::with_vault`) and, if its vault reads declared
//! file types as notes, installs its registry ([`set_note_types`]);
//! otherwise declarations land in the runtime's own registry and are only
//! listed.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

/// A query of the vault's index: a `.base` file's, or a base written in
/// place of one, one of its views, at most `limit` rows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRequest {
    /// A `.base` file's vault path.
    pub path: Option<String>,
    /// A base written as YAML, run in place of a file's.
    pub source: Option<String>,
    /// The view by name; the first when left out.
    pub view: Option<String>,
    /// The `hash` of an answer had before: the same answer says
    /// `{unchanged: true}`.
    pub if_changed: Option<String>,
    /// At most this many rows over every group.
    pub limit: Option<u32>,
}

/// A query's answer (JSON text), or why not.
#[cfg(not(target_family = "wasm"))]
pub type QueryFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
/// A query's answer (JSON text), or why not.
#[cfg(target_family = "wasm")]
pub type QueryFuture = Pin<Box<dyn Future<Output = Result<String, String>>>>;

/// The vault's query service as a host holds it.
pub trait VaultQuery: Send + Sync {
    fn query(&self, request: QueryRequest) -> QueryFuture;
}

/// The extension of a note file that is not declared: markdown.
pub const MARKDOWN: &str = "md";

/// A declared note file type: its extension (lower-case, without the dot)
/// and the `type` its files read as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteFileType {
    pub extension: String,
    pub note_type: String,
    /// Whether its body is markdown, or text read only for its links.
    pub markdown: bool,
    /// The frontmatter property a list row's preview is read from, in
    /// place of the body.
    pub preview: Option<String>,
}

/// Where declared note file types go: the app's vault's registry, or the
/// runtime's own ([`Declared`]) when the app installs none.
pub trait NoteTypes: Send + Sync {
    /// Declare `t`; false when its extension was already declared.
    fn register(&self, t: NoteFileType) -> bool;
    /// Every type declared so far.
    fn declared(&self) -> Vec<NoteFileType>;
}

/// The runtime's own registry of declared types: kept and listed only.
#[derive(Debug, Default)]
pub struct Declared(RwLock<Vec<NoteFileType>>);

impl NoteTypes for Declared {
    fn register(&self, t: NoteFileType) -> bool {
        let ext = t.extension.trim().trim_start_matches('.').to_lowercase();
        if ext.is_empty() || ext == MARKDOWN {
            return false;
        }
        let mut all = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if all.iter().any(|d| d.extension == ext) {
            return false;
        }
        all.push(NoteFileType { extension: ext, ..t });
        true
    }

    fn declared(&self) -> Vec<NoteFileType> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

static NOTE_TYPES: OnceLock<Arc<dyn NoteTypes>> = OnceLock::new();

/// Install the app's note type registry (once, before extensions load;
/// later calls are ignored).
pub fn set_note_types(types: Arc<dyn NoteTypes>) {
    let _ = NOTE_TYPES.set(types);
}

/// The registry in force: the app's, or the runtime's own.
pub fn note_types() -> &'static Arc<dyn NoteTypes> {
    NOTE_TYPES.get_or_init(|| Arc::new(Declared::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(ext: &str) -> NoteFileType {
        NoteFileType {
            extension: ext.into(),
            note_type: "recipe".into(),
            markdown: true,
            preview: None,
        }
    }

    #[test]
    fn a_type_is_declared_once_and_markdown_never() {
        let d = Declared::default();
        assert!(d.register(t(".Cook")));
        assert!(!d.register(t("cook")));
        assert!(!d.register(t("md")));
        assert_eq!(d.declared().len(), 1);
        assert_eq!(d.declared()[0].extension, "cook");
    }
}
