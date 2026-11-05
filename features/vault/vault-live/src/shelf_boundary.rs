//! Where one registered root stops and a nested one begins.
//!
//! # The rule, and the bug it exists to prevent
//!
//! A *shelf* is a directory registered as a vault root on
//! [`crate::sync::Backend`] (and on the link graph and the collaboration
//! layer above it). Two shelves may sit one inside the other on disk (an
//! album and one of its songs, a project and its sub-project), and when
//! they do, every file under the inner one must belong to **exactly one**
//! of them.
//!
//! Without that, a file under the child lands in two roots at once: two
//! link graphs claiming it, two disk watchers announcing each write, and
//! (the expensive one) two CRDT document ids over one file. Two people
//! editing that file then converge on two different documents and clobber
//! each other, silently, with no error anywhere. This module is the guard:
//! the page walk ([`crate::walker`]) and the replication manifest
//! ([`crate::sync`]) both prune at a nested shelf.
//!
//! # Why a marker file and not a registry lookup
//!
//! The walk is synchronous, runs on every scan, and has no handle on the
//! backend's root map; and even if it did, consulting the map would make
//! the answer depend on registration *order*: a child registered after its
//! parent would be invisible to a walk the parent had already done. A
//! marker on disk is the same answer whoever asks and whenever, which is
//! the property [`crate::private::MARKER_FILE`] is already relied on for
//! one layer down.
//!
//! # Which files are markers
//!
//! The vault itself declares none: which file makes a directory a root of
//! its own is the app's layout (Task's is `project.md`, a directory that
//! declares itself a project). An app names its markers once, at start,
//! with [`set_shelf_markers`]; until it does, no directory is a nested
//! shelf and a walk covers the whole tree.

use std::path::Path;
use std::sync::OnceLock;

static MARKERS: OnceLock<&'static [&'static str]> = OnceLock::new();

/// Names the files that declare a directory a registered root of its own.
///
/// Such a directory is not part of its parent's. Call it once, before the
/// first walk; the first call wins (the answer must not change under a
/// walk), and a later call answers `false`.
pub fn set_shelf_markers(markers: &'static [&'static str]) -> bool {
    MARKERS.set(markers).is_ok()
}

/// The files that declare a directory a shelf of its own: what
/// [`set_shelf_markers`] named, else none.
#[must_use]
pub fn shelf_markers() -> &'static [&'static str] {
    MARKERS.get().copied().unwrap_or(&[])
}

/// Whether `dir` is a shelf nested inside the root being walked.
///
/// `root` itself is never a nested shelf: it is the shelf being walked,
/// and forgetting that would make every shelf's own scan return nothing,
/// which is the first thing to check if one ever does.
#[must_use]
pub fn is_nested_shelf(root: &Path, dir: &Path) -> bool {
    is_nested_shelf_with(shelf_markers(), root, dir)
}

/// [`is_nested_shelf`] over an explicit marker list.
#[must_use]
pub fn is_nested_shelf_with(markers: &[&str], root: &Path, dir: &Path) -> bool {
    if dir == root {
        return false;
    }
    markers.iter().any(|m| dir.join(m).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKERS: &[&str] = &["project.md"];

    fn shelf(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(MARKERS[0]), "---\ntype: project\n---\n").unwrap();
    }

    #[test]
    fn a_shelf_is_not_nested_inside_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("album");
        shelf(&root);
        assert!(
            !is_nested_shelf_with(MARKERS, &root, &root),
            "a shelf that pruned itself would scan to nothing"
        );
    }

    #[test]
    fn a_child_that_declares_itself_is_a_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("album");
        shelf(&root);
        let song = root.join("track-two");
        shelf(&song);
        let plain = root.join("Deliverables");
        std::fs::create_dir_all(&plain).unwrap();

        assert!(is_nested_shelf_with(MARKERS, &root, &song));
        assert!(
            !is_nested_shelf_with(MARKERS, &root, &plain),
            "an ordinary folder is the parent's material, not a boundary"
        );
        // With no markers named, nothing is a boundary.
        assert!(!is_nested_shelf_with(&[], &root, &song));
    }
}
