//! A vault's own files: read and written never through a symlink.
//!
//! A symlink is never the vault's own entry, inside the vault or out of
//! it: one to a file outside the vault would hand that file to whoever
//! may read the vault, and one inside it to a private or ignored folder
//! would hand out what the vault keeps to itself. So every walk drops
//! symlinks ([`crate::walker::walk_vault`], the sync manifest), every
//! read opens through [`open_own`], and every write first checks
//! [`through_symlink`] and is refused when it is true.
//!
//! Hiding a symlink deletes nothing: a path that is a symlink is neither
//! listed nor reported as removed (the watcher drops its events), and a
//! delete, move or write of it is refused, so no peer and no disk loses
//! a file because a link stopped being listed.

use std::io::{Error, ErrorKind};
use std::path::{Component, Path};

/// Whether `rel` is a plain relative path in a vault: non-empty, and
/// made only of normal components (no root, prefix, `.` or `..`).
#[must_use]
pub fn is_plain_rel(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// The vault file at `rel` (forward-slash separated, below `root`)
/// opened for reading, never through a symlink.
///
/// Opened without following a link at its last component
/// (`O_NOFOLLOW`), then checked to be a plain file that `rel` names
/// with no symlink anywhere on the way: its resolved path is exactly the
/// root's joined with `rel`, and that path is the very file opened. A
/// file or a folder above it swapped for a symlink (out of the vault,
/// or to a private or ignored folder in it) between a walk and a read is
/// refused, never followed.
///
/// # Errors
///
/// The file could not be opened, or it is not the vault's own plain
/// file now (`InvalidData`), or `rel` is not a plain relative path:
/// absolute, or with `..`, `.` or a prefix (`InvalidInput`).
///
/// Unix only in full: elsewhere the opened file is compared with the path
/// by size and modification time, a weaker check.
pub fn open_own(root: &Path, rel: &str) -> std::io::Result<std::fs::File> {
    // `root.join` of an absolute path replaces the root.
    if !is_plain_rel(rel) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "not a path in the vault",
        ));
    }
    let path = root.join(rel);
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Non-blocking too: a link swapped to a FIFO never hangs a read.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(&path)?;
    let opened = file.metadata()?;
    let not_own = || Error::new(ErrorKind::InvalidData, "not a plain file of the vault");
    if !opened.is_file() {
        return Err(not_own());
    }
    let expected = root.canonicalize()?.join(rel);
    let resolved = path.canonicalize()?;
    if resolved != expected {
        return Err(not_own());
    }
    let now = std::fs::symlink_metadata(&resolved)?;
    if now.file_type().is_symlink() || !same_file(&opened, &now) {
        return Err(not_own());
    }
    Ok(file)
}

/// Whether an [`open_own`] error means "no file of the vault here":
/// missing, a symlink (`O_NOFOLLOW`'s `ELOOP`, for one swapped in since
/// a check), not a plain file, or not a path in the vault. Anything else
/// (permissions, I/O) is a real error.
#[must_use]
pub fn is_not_own(e: &Error) -> bool {
    #[cfg(unix)]
    if e.raw_os_error() == Some(libc::ELOOP) {
        return true;
    }
    matches!(
        e.kind(),
        ErrorKind::NotFound | ErrorKind::InvalidData | ErrorKind::InvalidInput
    )
}

/// The whole of the vault file at `rel`, read through [`open_own`].
///
/// # Errors
///
/// As [`open_own`], or the read failed.
pub fn read_own(root: &Path, rel: &str) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut file = open_own(root, rel)?;
    let len = file
        .metadata()
        .map_or(0, |m| usize::try_from(m.len()).unwrap_or(0));
    let mut out = Vec::with_capacity(len);
    file.read_to_end(&mut out)?;
    Ok(out)
}

/// Whether `rel` below `root` names a path through a symlink: the file
/// itself, or any folder on the way to it that exists, is a link. Parts
/// that do not exist yet are no link (a write creates them as plain
/// folders). A `rel` that is not plain counts as one, so a caller that
/// refuses on `true` refuses it too.
#[must_use]
pub fn through_symlink(root: &Path, rel: &str) -> bool {
    if !is_plain_rel(rel) {
        return true;
    }
    let mut at = root.to_path_buf();
    for part in Path::new(rel).components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            // Nothing here: nothing below it either.
            Err(_) => return false,
        }
    }
    false
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    b.is_file() && a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn open_own_opens_a_plain_file_and_refuses_any_symlink_on_the_way() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a/note.md"), "x").unwrap();
        fs::create_dir_all(tmp.path().join("outside/b")).unwrap();
        fs::write(tmp.path().join("outside/x.md"), "secret").unwrap();
        fs::write(tmp.path().join("outside/b/y.md"), "secret").unwrap();
        assert!(open_own(&root, "a/note.md").is_ok());
        assert_eq!(read_own(&root, "a/note.md").unwrap(), b"x");
        symlink("a/note.md", root.join("alias.md")).unwrap();
        symlink(tmp.path().join("outside/x.md"), root.join("out.md")).unwrap();
        symlink(tmp.path().join("outside/b"), root.join("b")).unwrap();
        symlink("a", root.join("c")).unwrap();
        for rel in ["alias.md", "out.md", "b/y.md", "c/note.md"] {
            assert!(open_own(&root, rel).is_err(), "{rel} opened");
            assert!(through_symlink(&root, rel), "{rel} not seen as a link");
        }
        for rel in [
            "/etc/passwd",
            "../outside/x.md",
            "a/../a/note.md",
            "./a/note.md",
            "",
        ] {
            let e = open_own(&root, rel).expect_err(rel);
            assert_eq!(e.kind(), ErrorKind::InvalidInput, "{rel}");
            assert!(through_symlink(&root, rel), "{rel}");
        }
        assert!(!through_symlink(&root, "a/note.md"));
        assert!(!through_symlink(&root, "new/folder/note.md"));
    }
}
