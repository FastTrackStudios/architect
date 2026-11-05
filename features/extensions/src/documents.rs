//! The app's documents, for extensions (#309; `docs/content/architecture/extensions.md`,
//! "Documents").
//!
//! An extension lists, reads, creates, edits, renames, moves, deletes and
//! watches the app's documents through the host service
//! (`crate::hostcall`: `list_documents`, `create_document`,
//! `edit_document`, …), checked against `capabilities.reads` (list, read,
//! watch) and `capabilities.writes` (the rest). What the documents are is
//! the app's: it gives the host a [`Documents`] (`Host::set_documents`) —
//! Task's vault, the lab's spaces ([`CrdtDocuments`], feature `crdt`).
//!
//! - **Edits are a transaction** ([`DocEdit`]s, applied in order): one
//!   CRDT commit and one undo step, named, made as the extension (its
//!   `Principal::Extension`, the commit's message), so it merges with the
//!   person's concurrent typing character by character and is told apart
//!   from it.
//! - **Paths** are `/`-separated and relative (`notes/Inbox.md`); the
//!   first folder is the space the app keeps it in. A position in a text
//!   counts characters (Unicode scalar values), not bytes.
//! - **Watching** is the document events (`crate::events::DocEvent`): the
//!   host raises `doc.created`, `doc.saved`, `doc.renamed` and
//!   `doc.deleted` for what an extension does here; the app raises its own
//!   (the person's saves) as it always did.
//!
//! The data ([`DocEdit`], [`DocInfo`]) is wasm-clean; the trait is the
//! host's (feature `host`).

use serde::{Deserialize, Serialize};

/// One edit of a document's text, in a transaction (`edit_document`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocEdit {
    /// `text` at character `at` (past the end: at the end).
    Insert { at: usize, text: String },
    /// `len` characters from character `at`.
    Delete { at: usize, len: usize },
    /// `text` at the end.
    Append { text: String },
    /// The whole text is `text`: what differs, as edits (so a concurrent
    /// edit elsewhere in it survives).
    Replace { text: String },
}

/// One of the app's documents, as an extension sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocInfo {
    /// Its path: `notes/Inbox.md`.
    pub path: String,
    /// Its name, the path's last part: `Inbox.md`.
    pub name: String,
    /// The space it is in: its path's first folder (none at the top).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
}

impl DocInfo {
    /// The info of the document at `path`.
    #[must_use]
    pub fn of(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            name: name_of(path).to_owned(),
            space: path.split_once('/').map(|(s, _)| s.to_owned()),
        }
    }
}

/// The last part of `path`.
#[must_use]
pub fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `path` checked: relative, `/`-separated, no empty part, `.` or `..`.
///
/// # Errors
///
/// Why it is not a document's path.
pub fn checked_path(path: &str) -> Result<&str, String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..");
    if bad {
        Err(format!(
            "{path:?} is not a document's path (relative, `/`-separated, no `..`)"
        ))
    } else {
        Ok(path)
    }
}

/// Where document `path` goes when it moves into `folder` (`""`: the top).
///
/// # Errors
///
/// `folder` is not a folder's path.
pub fn moved_to(path: &str, folder: &str) -> Result<String, String> {
    let folder = folder.trim_end_matches('/');
    if folder.is_empty() {
        return Ok(name_of(path).to_owned());
    }
    checked_path(folder)?;
    Ok(format!("{folder}/{}", name_of(path)))
}

/// Whether `path` is under folder `under` (none, or `""`: every path).
#[must_use]
pub fn is_under(path: &str, under: Option<&str>) -> bool {
    match under.map(|u| u.trim_end_matches('/')) {
        None | Some("") => true,
        Some(u) => path.strip_prefix(u).is_some_and(|r| r.starts_with('/')),
    }
}

#[cfg(feature = "host")]
pub use host::{DocError, Documents};

#[cfg(feature = "host")]
mod host {
    use architect::action::Principal;

    use super::{DocEdit, DocInfo};
    use crate::hostcall::HostError;

    /// Why the documents did not do it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum DocError {
        /// No document (or space) of that path.
        NotFound(String),
        /// A document is there already.
        Exists(String),
        /// Not a path, or an edit outside the text.
        Invalid(String),
        /// It was allowed and did not work.
        Failed(String),
    }

    impl std::fmt::Display for DocError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::NotFound(what) => write!(f, "no {what}"),
                Self::Exists(path) => write!(f, "{path} exists already"),
                Self::Invalid(why) | Self::Failed(why) => f.write_str(why),
            }
        }
    }

    impl std::error::Error for DocError {}

    impl From<DocError> for HostError {
        fn from(e: DocError) -> Self {
            match e {
                DocError::NotFound(what) => Self::NotFound { what },
                other => Self::Failed {
                    message: other.to_string(),
                },
            }
        }
    }

    /// The app's documents, as extensions reach them: what an app
    /// implements to give them its document set (`Host::set_documents`).
    ///
    /// Task's vault is one, the lab's spaces another
    /// ([`super::CrdtDocuments`]). The host has checked the capability
    /// and the path ([`super::checked_path`]) before it calls; `by` is
    /// who asked (the extension, and who it acts for), what an
    /// implementation attributes the change to. An implementation raises
    /// no document event for these: the host does.
    pub trait Documents: Send + Sync {
        /// Every document, or those under folder `under`.
        ///
        /// # Errors
        ///
        /// It could not list them.
        fn list(&self, under: Option<&str>) -> Result<Vec<DocInfo>, DocError>;

        /// Document `path`'s text.
        ///
        /// # Errors
        ///
        /// [`DocError::NotFound`] when there is none.
        fn read(&self, path: &str) -> Result<String, DocError>;

        /// A new document at `path` holding `text`.
        ///
        /// # Errors
        ///
        /// [`DocError::Exists`], or its space is not one there is.
        fn create(&self, path: &str, text: &str, by: &Principal) -> Result<DocInfo, DocError>;

        /// `edits`, in order, as one transaction: one commit, one undo
        /// step named `step`, made as `by`. Its text after.
        ///
        /// # Errors
        ///
        /// No document, or an edit outside its text (nothing is changed).
        fn edit(
            &self,
            path: &str,
            edits: &[DocEdit],
            step: &str,
            by: &Principal,
        ) -> Result<String, DocError>;

        /// Document `path` at `to`, its history kept.
        ///
        /// # Errors
        ///
        /// No document at `path`, or one at `to` already.
        fn rename(&self, path: &str, to: &str, by: &Principal) -> Result<DocInfo, DocError>;

        /// Deletes document `path`.
        ///
        /// # Errors
        ///
        /// No document at `path`.
        fn delete(&self, path: &str, by: &Principal) -> Result<(), DocError>;
    }
}

#[cfg(all(feature = "host", feature = "crdt"))]
mod crdt_docs;
#[cfg(all(feature = "host", feature = "crdt"))]
pub use crdt_docs::{CrdtDocuments, authors_of};

#[cfg(test)]
mod tests {
    use super::{DocEdit, DocInfo, checked_path, is_under, moved_to};

    #[test]
    fn paths_and_edits() {
        assert!(checked_path("notes/a.md").is_ok());
        for bad in ["", "/a", "a/", "a//b", "a/../b", "./a", "a\\b"] {
            assert!(checked_path(bad).is_err(), "{bad}");
        }
        assert_eq!(moved_to("notes/a.md", "archive/").unwrap(), "archive/a.md");
        assert_eq!(moved_to("notes/a.md", "").unwrap(), "a.md");
        assert!(is_under("notes/a.md", Some("notes")));
        assert!(!is_under("notesx/a.md", Some("notes")));
        assert!(is_under("a.md", None));
        let info = DocInfo::of("notes/a.md");
        assert_eq!(
            (info.name.as_str(), info.space.as_deref()),
            ("a.md", Some("notes"))
        );
        let e: DocEdit = serde_json::from_str(r#"{"insert":{"at":1,"text":"x"}}"#).unwrap();
        assert_eq!(
            e,
            DocEdit::Insert {
                at: 1,
                text: "x".to_owned()
            }
        );
    }
}
