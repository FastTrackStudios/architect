//! Putting bytes in place so none is lost on the way.
//!
//! Every copy that ends with a source being removed goes through here: the bytes are **staged** beside the target under one temp name
//! ([`staging_path`], the suffix [`STAGING`]), synced, read back and
//! checked against their length and sha-256, then **committed** by a
//! rename (or a no-clobber link) and the directory synced. Only then may
//! a caller remove the source. Any failure removes the staged file and
//! leaves the source as it was.
//!
//! A plain `tokio::fs::copy` is not enough: it never syncs and drops the
//! close's error, which is where NFS reports a late `ENOSPC`, `EDQUOT` or
//! `EIO`.

use std::ffi::OsString;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use futures::future::BoxFuture;
use sha2::Digest as _;
use tokio::io::{AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::{BlobError, LocalFs, Sha256, io_at};

/// The suffix of every temp file a move or a stage writes (and that a
/// scan of a working tree must ignore).
pub const STAGING: &str = ".architect-part";

/// The temp name a move to `to` stages under: `to` itself when it is
/// already one.
#[must_use]
pub fn staging_path(to: &Path) -> PathBuf {
    if is_staging(to) {
        return to.to_owned();
    }
    let mut name = OsString::from(to.as_os_str());
    name.push(STAGING);
    PathBuf::from(name)
}

/// Whether `path` is a staging name.
#[must_use]
pub fn is_staging(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(STAGING))
}

/// Where staged bytes are written: a file, or anything that can be
/// flushed to stable storage (tests inject failing ones).
pub trait Sink: AsyncWrite + Unpin + Send {
    /// Flush to stable storage (`fsync`).
    fn sync(&mut self) -> BoxFuture<'_, io::Result<()>>;
}

impl Sink for tokio::fs::File {
    fn sync(&mut self) -> BoxFuture<'_, io::Result<()>> {
        Box::pin(self.sync_all())
    }
}

/// A file's length and sha-256, read from disk.
///
/// # Errors
///
/// It cannot be read.
pub async fn hash_file(path: &Path) -> io::Result<(u64, Sha256)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; super::BUF];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf).await?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
            break;
        };
        hasher.update(chunk);
        len = len.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    }
    Ok((len, Sha256(hasher.finalize().into())))
}

/// Copy `from` into `staged` through `open`'s sink, sync it, and check
/// what landed against `from`'s length and `hash`. On any failure the
/// staged file is removed.
///
/// # Errors
///
/// Reading, writing or syncing failed, or the bytes on disk are short or
/// do not hash to `hash` (`InvalidData`).
pub async fn copy_verified_with<S, F, Fut>(
    from: &Path,
    staged: &Path,
    hash: Sha256,
    open: F,
) -> io::Result<()>
where
    S: Sink,
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = io::Result<S>>,
{
    let result = async {
        let mut source = tokio::fs::File::open(from).await?;
        let len = source.metadata().await?.len();
        let mut sink = open(staged.to_owned()).await?;
        tokio::io::copy(&mut source, &mut sink).await?;
        sink.flush().await?;
        sink.sync().await?;
        sink.shutdown().await?;
        drop(sink);
        let (got, actual) = hash_file(staged).await?;
        if got != len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {got} of {len} bytes landed", staged.display()),
            ));
        }
        if actual != hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: landed as {actual}, not {hash}", staged.display()),
            ));
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(staged).await;
    }
    result
}

/// Bytes staged for a target.
#[derive(Debug)]
pub struct Staged {
    /// The staged file (under [`STAGING`]).
    pub path: PathBuf,
    /// Hard-linked to the source rather than copied: they share an inode
    /// until one is replaced or removed.
    pub linked: bool,
}

/// Stage `from`'s bytes (which hash to `hash`) beside `to`: hard-linked
/// when `link` and the filesystem allows it, else copied and verified.
///
/// Link only when the source is about to be replaced or removed: a link
/// shares the bytes, so a later in-place write to one changes the other.
///
/// # Errors
///
/// As [`copy_verified_with`]; nothing is left behind.
pub async fn stage(from: &Path, to: &Path, hash: Sha256, link: bool) -> io::Result<Staged> {
    let staged = staging_path(to);
    if let Some(dir) = staged.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    // A leftover of a run that died: never appended to.
    match tokio::fs::remove_file(&staged).await {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    // A link shares the bytes with every other name of `from`: only when
    // it has no other (a user's hard link, a leftover of a crash), or a
    // write through that name would change these bytes too.
    if link && single_name(from).await && tokio::fs::hard_link(from, &staged).await.is_ok() {
        return Ok(Staged {
            path: staged,
            linked: true,
        });
    }
    if reflink(from, &staged).await.is_ok() {
        // A clone shares nothing a write can reach; its bytes are checked
        // like a copy's.
        match hash_file(&staged).await {
            Ok((_, actual)) if actual == hash => {
                return Ok(Staged {
                    path: staged,
                    linked: false,
                });
            }
            _ => {
                let _ = tokio::fs::remove_file(&staged).await;
            }
        }
    }
    copy_verified_with(from, &staged, hash, |p| tokio::fs::File::create(p)).await?;
    Ok(Staged {
        path: staged,
        linked: false,
    })
}

/// Whether `path` has exactly one name (its link count is 1).
async fn single_name(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        tokio::fs::metadata(path)
            .await
            .is_ok_and(|m| m.nlink() == 1)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Clone `from` into a new file at `to` (`FICLONE`: btrfs, XFS, bcachefs
/// share extents, copy on write), synced. Fails where it is not offered
/// (leaving nothing at `to`).
async fn reflink(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let (from, to) = (from.to_owned(), to.to_owned());
        tokio::task::spawn_blocking(move || {
            let src = std::fs::File::open(&from)?;
            let dst = std::fs::File::create(&to)?;
            let done = rustix::fs::ioctl_ficlone(&dst, &src)
                .map_err(io::Error::from)
                .and_then(|()| dst.sync_all());
            if done.is_err() {
                drop(dst);
                let _ = std::fs::remove_file(&to);
            }
            done
        })
        .await
        .map_err(io::Error::other)?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (from, to);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Put staged bytes at `to` and sync the directory. With `no_clobber`, a
/// file that appeared at `to` meanwhile is never replaced
/// (`AlreadyExists`). The staged file is gone afterwards either way.
///
/// # Errors
///
/// `to` exists (with `no_clobber`), or the rename or the sync failed.
pub async fn commit(staged: &Path, to: &Path, no_clobber: bool) -> io::Result<()> {
    if staged == to {
        return sync_dir(to).await;
    }
    let placed = if no_clobber {
        // A link fails if `to` exists: an atomic "only if still absent".
        match tokio::fs::hard_link(staged, to).await {
            Ok(()) => tokio::fs::remove_file(staged).await,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            // No hard links here: check, then rename (a narrow window).
            Err(_) if tokio::fs::try_exists(to).await.unwrap_or(true) => {
                Err(io::Error::from(io::ErrorKind::AlreadyExists))
            }
            Err(_) => tokio::fs::rename(staged, to).await,
        }
    } else {
        tokio::fs::rename(staged, to).await
    };
    if let Err(e) = placed {
        let _ = tokio::fs::remove_file(staged).await;
        return Err(e);
    }
    sync_dir(to).await
}

/// Sync the directory holding `path`, so a rename into it is durable.
async fn sync_dir(path: &Path) -> io::Result<()> {
    let Some(dir) = path.parent().map(Path::to_owned) else {
        return Ok(());
    };
    let synced = tokio::task::spawn_blocking(move || std::fs::File::open(&dir)?.sync_all())
        .await
        .map_err(io::Error::other)?;
    match synced {
        // Filesystems that cannot sync a directory say so; nothing to do.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidInput
                    | io::ErrorKind::Unsupported
                    | io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(())
        }
        other => other,
    }
}

impl LocalFs {
    /// Keep the file at `from` (whose bytes hash to `hash`) in the store
    /// too, within the budget: linked when `link` and the filesystem
    /// allows it (the caller is about to replace `from`), else copied and
    /// verified. `from` is left in place. `Ok(None)` when the store
    /// already held it; `Ok(Some(linked))` when it was added.
    ///
    /// # Errors
    ///
    /// [`BlobError::NoRoom`], or the copy failed (nothing is added).
    pub async fn adopt(
        &self,
        from: &Path,
        hash: &Sha256,
        link: bool,
    ) -> Result<Option<bool>, BlobError> {
        let into = self.path_for(hash);
        if into.exists() {
            return Ok(None);
        }
        let len = tokio::fs::metadata(from).await.map_err(io_at(from))?.len();
        let mut reservation = self.reserve(len).map_err(BlobError::NoRoom)?;
        let staged = stage(from, &into, *hash, link)
            .await
            .map_err(io_at(&into))?;
        commit(&staged.path, &into, false)
            .await
            .map_err(io_at(&into))?;
        if let Some(r) = reservation.as_mut() {
            r.wrote(len);
        }
        Ok(Some(staged.linked))
    }

    /// Remove leftovers older than `grace`: partial fetches (unless
    /// `running` names their hash), temporary puts and staged copies. The
    /// bytes freed.
    pub async fn clean_orphans(
        &self,
        grace: std::time::Duration,
        running: &std::collections::HashSet<Sha256>,
    ) -> u64 {
        let root = self.root.clone();
        let running = running.clone();
        let freed = tokio::task::spawn_blocking(move || orphans(&root, grace, &running))
            .await
            .unwrap_or(0);
        if let Some(b) = &self.budget {
            b.removed(freed);
        }
        freed
    }
}

fn orphans(
    root: &Path,
    grace: std::time::Duration,
    running: &std::collections::HashSet<Sha256>,
) -> u64 {
    let old = |m: &std::fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= grace)
    };
    let mut freed = 0u64;
    let mut remove = |path: &Path, meta: &std::fs::Metadata| {
        if old(meta) && std::fs::remove_file(path).is_ok() {
            freed = freed.saturating_add(meta.len());
        }
    };
    for dir in [root.join(".partial"), root.join(".tmp")] {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let hash = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.split('.').next())
                .and_then(|h| Sha256::from_hex(h).ok());
            if hash.is_some_and(|h| running.contains(&h)) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                remove(&path, &meta);
            }
        }
    }
    // Staged copies beside blobs: `<hh>/<hex>.architect-part`.
    for prefix in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if prefix.file_name().len() != 2 {
            continue;
        }
        for entry in std::fs::read_dir(prefix.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if is_staging(&path)
                && let Ok(meta) = entry.metadata()
            {
                remove(&path, &meta);
            }
        }
    }
    freed
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use super::*;

    /// How an injected sink misbehaves.
    #[derive(Clone, Copy)]
    enum Fault {
        /// Says it wrote everything, keeps only the first `n` bytes.
        Short(usize),
        /// Flips a bit of the first byte it writes.
        Wrong,
        /// `fsync` fails (a late `EIO`, an NFS `ENOSPC`).
        SyncFails,
    }

    struct Failing {
        file: tokio::fs::File,
        fault: Fault,
        written: usize,
    }

    impl AsyncWrite for Failing {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            let mut chunk = buf.to_vec();
            match self.fault {
                Fault::Short(n) => chunk.truncate(n.saturating_sub(self.written)),
                Fault::Wrong if self.written == 0 && !chunk.is_empty() => chunk[0] ^= 1,
                _ => {}
            }
            let this = &mut *self;
            match Pin::new(&mut this.file).poll_write(cx, &chunk) {
                Poll::Ready(Ok(_)) => {
                    this.written = this.written.saturating_add(buf.len());
                    Poll::Ready(Ok(buf.len()))
                }
                other => other,
            }
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.file).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.file).poll_shutdown(cx)
        }
    }

    impl Sink for Failing {
        fn sync(&mut self) -> BoxFuture<'_, io::Result<()>> {
            Box::pin(async move {
                if matches!(self.fault, Fault::SyncFails) {
                    return Err(io::Error::other("injected: fsync failed"));
                }
                self.file.sync_all().await
            })
        }
    }

    #[tokio::test]
    async fn a_failing_copy_leaves_the_source_and_nothing_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("blob");
        let bytes = vec![7u8; 50_000];
        std::fs::write(&from, &bytes).unwrap();
        let hash = Sha256::of(&bytes);
        let to = tmp.path().join("tree/take.wav");
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let staged = staging_path(&to);
        for fault in [Fault::Short(10_000), Fault::Wrong, Fault::SyncFails] {
            let result = copy_verified_with(&from, &staged, hash, |p| async move {
                Ok(Failing {
                    file: tokio::fs::File::create(p).await?,
                    fault,
                    written: 0,
                })
            })
            .await;
            assert!(result.is_err());
            assert!(!staged.exists(), "the staged file is removed");
            assert!(!to.exists());
            assert_eq!(
                std::fs::read(&from).unwrap(),
                bytes,
                "the source is untouched"
            );
        }
        // The same copy without a fault lands, and the source stays until
        // the caller removes it.
        let staged = stage(&from, &to, hash, false).await.unwrap();
        commit(&staged.path, &to, false).await.unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), bytes);
        assert!(from.exists());
    }

    #[tokio::test]
    async fn a_move_with_the_wrong_hash_keeps_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("blob");
        std::fs::write(&from, b"these bytes").unwrap();
        let to = tmp.path().join("to");
        let staged = stage(&from, &to, Sha256::of(b"other bytes"), false).await;
        assert!(staged.is_err());
        assert!(from.exists() && !staging_path(&to).exists());
    }

    #[tokio::test]
    async fn a_no_clobber_commit_never_replaces_a_file_that_appeared() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("new");
        std::fs::write(&from, b"new").unwrap();
        let to = tmp.path().join("take.wav");
        let staged = stage(&from, &to, Sha256::of(b"new"), false).await.unwrap();
        std::fs::write(&to, b"the user's").unwrap();
        let refused = commit(&staged.path, &to, true).await;
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&to).unwrap(), b"the user's");
        assert!(!staged.path.exists());
    }

    /// A source with a second name (a user's hard link) is never linked:
    /// a write through that name would change the staged bytes too.
    #[tokio::test]
    async fn a_file_with_another_name_is_copied_not_linked() {
        use std::os::unix::fs::MetadataExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let take = tmp.path().join("take.wav");
        std::fs::write(&take, b"the take").unwrap();
        std::fs::hard_link(&take, tmp.path().join("alias.wav")).unwrap();
        let kept = tmp.path().join("kept");
        let staged = stage(&take, &kept, Sha256::of(b"the take"), true)
            .await
            .unwrap();
        assert!(!staged.linked);
        assert_eq!(std::fs::metadata(&staged.path).unwrap().nlink(), 1);
        assert_ne!(
            std::fs::metadata(&staged.path).unwrap().ino(),
            std::fs::metadata(&take).unwrap().ino()
        );
        // With one name it is linked.
        std::fs::remove_file(tmp.path().join("alias.wav")).unwrap();
        std::fs::remove_file(&staged.path).unwrap();
        let again = stage(&take, &kept, Sha256::of(b"the take"), true)
            .await
            .unwrap();
        assert!(again.linked);
    }

    #[test]
    fn one_staging_suffix() {
        let p = staging_path(Path::new("a/take.wav"));
        assert_eq!(p, Path::new("a/take.wav.architect-part"));
        assert_eq!(staging_path(&p), p, "never a second suffix");
        assert!(is_staging(&p));
    }
}
