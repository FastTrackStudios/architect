//! Names every walker of a vault agrees on: a file root's own internals
//! (its marker and version store, which replication must not treat as
//! vault content) and the marker that keeps a directory on its node
//! (`vault.private.node-only`). They live in the proto (rather than in
//! one backend crate) because more than one implementation has to agree
//! on them: a files backend hides them from browsing, the vault lanes
//! skip them, and a storage agent creates [`STORE_DIR`] when it
//! initializes a root's version store.

/// Marker file at a file root's top level recording its stable id.
pub const MARKER_FILE: &str = ".fts-root.json";

/// Directory at a file root's top level holding its version store.
pub const STORE_DIR: &str = ".fts-files";

/// A root's git repository. Like [`STORE_DIR`] it is an object store,
/// skipped at every walk depth and never ingested as ordinary content.
pub const GIT_DIR: &str = ".git";

/// Marker file that makes the directory holding it **node-private**:
/// it stays on this machine's disk and nowhere else. A private
/// directory is never versioned (so never in a root's history, never
/// replicated to a peer), never listed by a browse, a facet listing or
/// a vault manifest, and never served by name.
///
/// It exists for state a feature must keep beside the vault but that
/// is not vault content (an app's sync accounts, say). The feature that
/// owns the directory writes the marker before it writes anything else
/// into it; the files layer and the vault lanes only honour it, so
/// neither has to know which feature a private directory belongs to.
pub const PRIVATE_MARKER: &str = ".fts-private";

/// Does `dir` carry [`PRIVATE_MARKER`]? One `stat`.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn is_private_dir(dir: &std::path::Path) -> bool {
    dir.join(PRIVATE_MARKER).is_file()
}

/// Is `rel` (a `/`-separated path relative to `root`) inside a private
/// directory — at any depth below `root`, or naming the marker itself?
///
/// The question a lane asks before it serves a path by name: a listing
/// that hides a private directory is no guard if the same file can be
/// fetched by spelling its path.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn is_within_private(root: &std::path::Path, rel: &str) -> bool {
    let mut dir = root.to_path_buf();
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    if parts.last() == Some(&PRIVATE_MARKER) {
        return true;
    }
    // Every ancestor directory of `rel` (and `rel` itself, when it is a
    // directory) is checked; the root's own top level is a root, never
    // private.
    for part in parts {
        dir.push(part);
        if is_private_dir(&dir) {
            return true;
        }
    }
    false
}
