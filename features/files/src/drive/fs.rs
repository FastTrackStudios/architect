//! The drive as a FUSE filesystem: every root a device knows, at
//! `<mount>/<org>/<Projects|Assets>/<name>/…`.
//!
//! - **A pinned root** is its working copy, passed through: reads and
//!   writes go to the local files at their own speed, and the engine's
//!   scan syncs them as it does any edit.
//! - **An unpinned root** is its manifest: folders and files with their
//!   real sizes and times, browsable offline. Opening a file fetches it in
//!   the background and serves reads as the bytes arrive ([`Cache`]); it
//!   stays cached. Writing goes to a temp copy that is saved as a version
//!   on close or `fsync`, so a DAW holding a file open, and the
//!   write-a-temp-then-rename save, both land as versions.
//! - **Pinning** is an xattr: `user.architect.pinned` (`1`/`0`) on a root
//!   or anything in it pins or unpins the root; `user.architect.state`
//!   reads `pinned`, `cached`, `fetching` or `online`.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{FileExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use architect::blob::Sha256;
use fuser::{
    AccessFlags, BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
    Request, TimeOrNow, WriteFlags,
};
use tokio::runtime::Handle;
use uuid::Uuid;

use super::cache::{Cache, Fetch};
use crate::roots::RootStatus;
use crate::sync::Engine;
use crate::sync::layout::PART;
use crate::sync::manifest::{Section, Tree, doc_id_for};

/// Pin or unpin a root: `1` or `0`.
pub const XATTR_PINNED: &str = "user.architect.pinned";
/// What a file or folder is here: `pinned`, `cached`, `fetching`, `online`.
pub const XATTR_STATE: &str = "user.architect.state";

const TTL: Duration = Duration::from_secs(1);
const ROOTS_TTL: Duration = Duration::from_secs(2);
const ROOT_INO: u64 = 1;

fn errno(code: i32) -> Errno {
    Errno::from_i32(code)
}

fn io_errno(e: &std::io::Error) -> Errno {
    match e.kind() {
        // A fetch no peer answered: offline, or nobody holds it.
        std::io::ErrorKind::NotConnected => errno(libc::ENETUNREACH),
        _ => errno(e.raw_os_error().unwrap_or(libc::EIO)),
    }
}

/// Paths by inode and back; a rename moves a subtree's paths.
#[derive(Default)]
struct Inodes {
    paths: HashMap<u64, String>,
    ids: HashMap<String, u64>,
    next: u64,
}

impl Inodes {
    fn new() -> Self {
        let mut me = Self {
            next: ROOT_INO.saturating_add(1),
            ..Self::default()
        };
        me.paths.insert(ROOT_INO, String::new());
        me.ids.insert(String::new(), ROOT_INO);
        me
    }

    fn ino(&mut self, path: &str) -> u64 {
        if let Some(ino) = self.ids.get(path) {
            return *ino;
        }
        let ino = self.next;
        self.next = self.next.saturating_add(1);
        self.paths.insert(ino, path.to_owned());
        self.ids.insert(path.to_owned(), ino);
        ino
    }

    fn path(&self, ino: u64) -> Option<String> {
        self.paths.get(&ino).cloned()
    }

    fn rename(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        let moved: Vec<(u64, String)> = self
            .paths
            .iter()
            .filter(|(_, p)| *p == from || p.starts_with(&prefix))
            .map(|(i, p)| {
                (
                    *i,
                    format!("{to}{}", p.get(from.len()..).unwrap_or_default()),
                )
            })
            .collect();
        if let Some(old) = self.ids.remove(to) {
            self.paths.remove(&old);
        }
        for (ino, path) in moved {
            if let Some(old) = self.paths.insert(ino, path.clone()) {
                self.ids.remove(&old);
            }
            self.ids.insert(path, ino);
        }
    }
}

/// Where a mount path lands.
enum Node {
    Top,
    Org,
    Section,
    Root {
        doc_id: Uuid,
        /// Root-relative; empty for the root itself.
        rel: String,
        /// The local file, when the root has a working copy here.
        real: Option<PathBuf>,
    },
}

/// A write to an unpinned file, staged in a temp copy.
struct Pending {
    temp: PathBuf,
    opens: u32,
}

enum Open {
    /// A pinned root's file, passed through.
    Real(File),
    /// An unpinned file, read through the cache.
    Read(Sha256),
    /// An unpinned file being written, by inode (a rename moves it).
    Write { ino: u64, file: File, dirty: bool },
}

struct Inner {
    engine: Engine,
    rt: Handle,
    cache: Cache,
    tmp: PathBuf,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, Open>>,
    next_fh: AtomicU64,
    /// Folders made in unpinned roots that hold no file yet.
    made: Mutex<BTreeSet<String>>,
    /// Unpinned files being written: mount path → temp copy.
    pending: Mutex<HashMap<String, Pending>>,
    roots: Mutex<Option<(Instant, Vec<RootStatus>)>>,
    trees: Mutex<HashMap<Uuid, (Instant, Arc<Tree>)>>,
    uid: u32,
    gid: u32,
    started: SystemTime,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The FUSE filesystem over an [`Engine`].
#[derive(Clone)]
pub struct DriveFs {
    inner: Arc<Inner>,
}

impl DriveFs {
    /// The drive over `engine`, calling it on `rt`; unpinned writes are
    /// staged in `tmp`.
    #[must_use]
    pub fn new(engine: Engine, rt: Handle, tmp: PathBuf, uid: u32, gid: u32) -> Self {
        let cache = Cache::new(engine.clone(), rt.clone());
        Self {
            inner: Arc::new(Inner {
                engine,
                rt,
                cache,
                tmp,
                inodes: Mutex::new(Inodes::new()),
                handles: Mutex::default(),
                next_fh: AtomicU64::new(1),
                made: Mutex::default(),
                pending: Mutex::default(),
                roots: Mutex::default(),
                trees: Mutex::default(),
                uid,
                gid,
                started: SystemTime::now(),
            }),
        }
    }

    /// Its cache (the drive's status reports its size).
    #[must_use]
    pub fn cache(&self) -> &Cache {
        &self.inner.cache
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn time_of_us(at_us: i64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_micros(u64::try_from(at_us).unwrap_or(0)))
        .unwrap_or(UNIX_EPOCH)
}

impl Inner {
    fn roots(&self) -> Vec<RootStatus> {
        if let Some((at, roots)) = lock(&self.roots).as_ref()
            && at.elapsed() < ROOTS_TTL
        {
            return roots.clone();
        }
        let roots = self.rt.block_on(self.engine.status());
        *lock(&self.roots) = Some((Instant::now(), roots.clone()));
        roots
    }

    fn tree(&self, doc_id: Uuid) -> Option<Arc<Tree>> {
        if let Some((at, tree)) = lock(&self.trees).get(&doc_id)
            && at.elapsed() < TTL
        {
            return Some(Arc::clone(tree));
        }
        let tree = Arc::new(self.rt.block_on(self.engine.tree(doc_id)).ok()?);
        lock(&self.trees).insert(doc_id, (Instant::now(), Arc::clone(&tree)));
        Some(tree)
    }

    fn forget_tree(&self, doc_id: Uuid) {
        lock(&self.trees).remove(&doc_id);
        *lock(&self.roots) = None;
    }

    fn path_of(&self, ino: INodeNo) -> Result<String, Errno> {
        lock(&self.inodes).path(ino.0).ok_or(Errno::ENOENT)
    }

    fn node(&self, path: &str) -> Option<Node> {
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        match segs.as_slice() {
            [] => Some(Node::Top),
            [org] => self
                .roots()
                .iter()
                .any(|r| r.org == *org)
                .then_some(Node::Org),
            [org, section] => {
                let section = Section::from_dir(section)?;
                self.roots()
                    .iter()
                    .any(|r| r.org == *org && r.section == section)
                    .then_some(Node::Section)
            }
            [org, section, name, rest @ ..] => {
                let doc_id = doc_id_for(&format!("{org}/{section}/{name}"));
                if !self.engine.knows(doc_id) {
                    return None;
                }
                let inner = rest.join("/");
                let local = self
                    .engine
                    .working_dir(doc_id)
                    .map(|d| if inner.is_empty() { d } else { d.join(&inner) });
                Some(Node::Root {
                    doc_id,
                    rel: inner,
                    real: local,
                })
            }
        }
    }

    fn dir_attr(&self, ino: u64, at: SystemTime) -> FileAttr {
        self.attr(ino, FileType::Directory, 0, at, 0o755)
    }

    fn attr(&self, ino: u64, kind: FileType, size: u64, at: SystemTime, perm: u16) -> FileAttr {
        FileAttr {
            ino: INodeNo(ino),
            size,
            blocks: size.div_ceil(512),
            atime: at,
            mtime: at,
            ctime: at,
            crtime: at,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn meta_attr(&self, ino: u64, meta: &std::fs::Metadata) -> FileAttr {
        let kind = if meta.is_dir() {
            FileType::Directory
        } else if meta.file_type().is_symlink() {
            FileType::Symlink
        } else {
            FileType::RegularFile
        };
        let mut attr = self.attr(
            ino,
            kind,
            meta.len(),
            meta.modified().unwrap_or(self.started),
            u16::try_from(meta.mode() & 0o7777).unwrap_or(0o644),
        );
        attr.atime = meta.accessed().unwrap_or(attr.mtime);
        attr
    }

    /// Whether `rel` is a folder in an unpinned root.
    fn is_dir(&self, tree: &Tree, path: &str, rel: &str) -> bool {
        let prefix = format!("{rel}/");
        rel.is_empty()
            || tree.live().keys().any(|p| p.starts_with(&prefix))
            || lock(&self.made).contains(path)
            || lock(&self.pending)
                .keys()
                .any(|p| p.starts_with(&format!("{path}/")))
    }

    fn getattr_path(&self, ino: u64, path: &str) -> Result<FileAttr, Errno> {
        match self.node(path).ok_or(Errno::ENOENT)? {
            Node::Top | Node::Org | Node::Section => Ok(self.dir_attr(ino, self.started)),
            Node::Root {
                real: Some(real), ..
            } => std::fs::symlink_metadata(&real)
                .map(|m| self.meta_attr(ino, &m))
                .map_err(|e| io_errno(&e)),
            Node::Root {
                doc_id,
                rel,
                real: None,
            } => {
                if let Some(p) = lock(&self.pending).get(path) {
                    let meta = std::fs::metadata(&p.temp).map_err(|e| io_errno(&e))?;
                    return Ok(self.meta_attr(ino, &meta));
                }
                let tree = self.tree(doc_id).ok_or(Errno::ENOENT)?;
                if let Some(v) = (!rel.is_empty()).then(|| tree.current(&rel)).flatten() {
                    return Ok(self.attr(
                        ino,
                        FileType::RegularFile,
                        v.size,
                        time_of_us(v.at_us),
                        0o644,
                    ));
                }
                if self.is_dir(&tree, path, &rel) {
                    let newest = tree.live().values().map(|v| v.at_us).max().unwrap_or(0);
                    return Ok(self.dir_attr(
                        ino,
                        if rel.is_empty() {
                            time_of_us(newest)
                        } else {
                            self.started
                        },
                    ));
                }
                Err(Errno::ENOENT)
            }
        }
    }

    fn entry(&self, path: &str) -> Result<FileAttr, Errno> {
        let ino = lock(&self.inodes).ino(path);
        self.getattr_path(ino, path)
    }

    /// `(name, is_dir)` of a folder's children.
    fn children(&self, path: &str) -> Result<Vec<(String, bool)>, Errno> {
        let roots = self.roots();
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut out: Vec<(String, bool)> = match self.node(path).ok_or(Errno::ENOENT)? {
            Node::Top => roots.iter().map(|r| (r.org.clone(), true)).collect(),
            Node::Org => roots
                .iter()
                .filter(|r| Some(r.org.as_str()) == segs.first().copied())
                .map(|r| (r.section.dir().to_owned(), true))
                .collect(),
            Node::Section => roots
                .iter()
                .filter(|r| {
                    Some(r.org.as_str()) == segs.first().copied()
                        && Some(r.section.dir()) == segs.get(1).copied()
                })
                .map(|r| (r.name.clone(), true))
                .collect(),
            Node::Root {
                real: Some(real), ..
            } => std::fs::read_dir(&real)
                .map_err(|e| io_errno(&e))?
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    (!name.ends_with(PART)).then(|| (name, e.file_type().is_ok_and(|t| t.is_dir())))
                })
                .collect(),
            Node::Root {
                doc_id,
                rel,
                real: None,
            } => {
                let tree = self.tree(doc_id).ok_or(Errno::ENOENT)?;
                let mut out: Vec<(String, bool)> = self
                    .engine
                    .entries(&tree, &rel)
                    .into_iter()
                    .map(|e| (e.name, e.dir))
                    .collect();
                let prefix = format!("{path}/");
                let extra: Vec<(String, bool)> = lock(&self.made)
                    .iter()
                    .filter_map(|m| m.strip_prefix(&prefix))
                    .filter(|r| !r.contains('/'))
                    .map(|r| (r.to_owned(), true))
                    .chain(
                        lock(&self.pending)
                            .keys()
                            .filter_map(|m| m.strip_prefix(&prefix))
                            .filter(|r| !r.contains('/'))
                            .map(|r| (r.to_owned(), false)),
                    )
                    .collect();
                out.extend(extra);
                out
            }
        };
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        Ok(out)
    }

    fn fh(&self, open: Open) -> u64 {
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        lock(&self.handles).insert(fh, open);
        fh
    }

    /// The temp copy `path` (an unpinned file) is written through.
    fn begin_write(
        &self,
        path: &str,
        doc_id: Uuid,
        rel: &str,
        truncate: bool,
    ) -> Result<File, Errno> {
        let existing = lock(&self.pending).get(path).map(|p| p.temp.clone());
        let temp = if let Some(temp) = existing {
            temp
        } else {
            std::fs::create_dir_all(&self.tmp).map_err(|e| io_errno(&e))?;
            let temp = self.tmp.join(Uuid::new_v4().to_string());
            let current = self
                .tree(doc_id)
                .and_then(|t| t.current(rel))
                .and_then(|v| v.content)
                .and_then(|c| Sha256::from_hex(&c).ok());
            match current {
                Some(hash) if !truncate => {
                    self.cache.copy_to(hash, &temp).map_err(|e| io_errno(&e))?;
                }
                _ => {
                    File::create(&temp).map_err(|e| io_errno(&e))?;
                }
            }
            temp
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp)
            .map_err(|e| io_errno(&e))?;
        if truncate {
            file.set_len(0).map_err(|e| io_errno(&e))?;
        }
        let mut pending = lock(&self.pending);
        let entry = pending
            .entry(path.to_owned())
            .or_insert(Pending { temp, opens: 0 });
        entry.opens = entry.opens.saturating_add(1);
        drop(pending);
        Ok(file)
    }

    /// Save the temp copy behind inode `ino` as a version of its path now.
    fn save(&self, ino: u64) -> Result<(), Errno> {
        let path = lock(&self.inodes).path(ino).ok_or(Errno::ENOENT)?;
        let Some(Node::Root {
            doc_id,
            rel,
            real: None,
        }) = self.node(&path)
        else {
            return Ok(());
        };
        let temp = lock(&self.pending)
            .get(&path)
            .map(|p| p.temp.clone())
            .ok_or(Errno::ENOENT)?;
        let engine = self.engine.clone();
        self.rt
            .block_on(async move {
                let body = tokio::fs::File::open(&temp)
                    .await
                    .map_err(|e| e.to_string())?;
                engine
                    .save_reader(doc_id, &rel, Box::pin(body))
                    .await
                    .map_err(|e| e.to_string())
            })
            .map_err(|why| {
                tracing::warn!(%path, %why, "drive: a write was not saved");
                Errno::EIO
            })?;
        lock(&self.made).retain(|m| !path.starts_with(&format!("{m}/")));
        self.forget_tree(doc_id);
        Ok(())
    }

    fn end_write(&self, ino: u64) {
        let Some(path) = lock(&self.inodes).path(ino) else {
            return;
        };
        let mut pending = lock(&self.pending);
        let done = pending.get_mut(&path).is_some_and(|p| {
            p.opens = p.opens.saturating_sub(1);
            p.opens == 0
        });
        if done && let Some(p) = pending.remove(&path) {
            let _ = std::fs::remove_file(p.temp);
        }
    }

    /// Save handle `fh`'s writes if it has any (flush, fsync); sync a
    /// pinned file.
    fn save_dirty(&self, fh: u64) -> Result<(), Errno> {
        let dirty = match lock(&self.handles).get_mut(&fh) {
            Some(Open::Real(f)) => {
                let _ = f.sync_data();
                None
            }
            Some(Open::Write { ino, file, dirty }) if *dirty => {
                let _ = file.sync_data();
                *dirty = false;
                Some(*ino)
            }
            _ => None,
        };
        dirty.map_or(Ok(()), |ino| self.save(ino))
    }

    fn state(&self, path: &str) -> Option<String> {
        match self.node(path)? {
            Node::Top | Node::Org | Node::Section => None,
            Node::Root { real: Some(_), .. } => Some("pinned".to_owned()),
            Node::Root {
                doc_id,
                rel,
                real: None,
            } => {
                if rel.is_empty() {
                    return Some("online".to_owned());
                }
                let hash = self
                    .tree(doc_id)?
                    .current(&rel)
                    .and_then(|v| v.content)
                    .and_then(|c| Sha256::from_hex(&c).ok());
                Some(
                    match hash.and_then(|h| self.cache.state(&h)) {
                        Some(Fetch::Done) => "cached",
                        Some(Fetch::Running) => "fetching",
                        _ => "online",
                    }
                    .to_owned(),
                )
            }
        }
    }
}

/// Where a read's bytes come from.
enum Src {
    File(File),
    Hash(Sha256),
}

fn reply_xattr(reply: ReplyXattr, size: u32, value: &[u8]) {
    if size == 0 {
        reply.size(u32::try_from(value.len()).unwrap_or(u32::MAX));
    } else if value.len() > usize::try_from(size).unwrap_or(usize::MAX) {
        reply.error(errno(libc::ERANGE));
    } else {
        reply.data(value);
    }
}

impl Filesystem for DriveFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let me = &self.inner;
        let result = me
            .path_of(parent)
            .and_then(|p| me.entry(&join(&p, &name.to_string_lossy())));
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        let me = &self.inner;
        match me.path_of(ino).and_then(|p| me.getattr_path(ino.0, &p)) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(e),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let me = &self.inner;
        let result = (|| {
            let path = me.path_of(ino)?;
            match me.node(&path).ok_or(Errno::ENOENT)? {
                Node::Root {
                    real: Some(real), ..
                } => {
                    if let Some(size) = size {
                        OpenOptions::new()
                            .write(true)
                            .open(&real)
                            .and_then(|f| f.set_len(size))
                            .map_err(|e| io_errno(&e))?;
                    }
                    if let Some(mode) = mode {
                        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(mode))
                            .map_err(|e| io_errno(&e))?;
                    }
                    if let Some(t) = mtime {
                        let at = match t {
                            TimeOrNow::SpecificTime(t) => t,
                            TimeOrNow::Now => SystemTime::now(),
                        };
                        if let Ok(f) = File::options().write(true).open(&real) {
                            let _ = f.set_modified(at);
                        }
                    }
                }
                Node::Root {
                    doc_id,
                    rel,
                    real: None,
                } if !rel.is_empty() => {
                    if let Some(size) = size {
                        let held = fh.and_then(|fh| match lock(&me.handles).get_mut(&fh.0) {
                            Some(Open::Write { file, dirty, .. }) => {
                                *dirty = true;
                                Some(file.set_len(size))
                            }
                            _ => None,
                        });
                        if let Some(done) = held {
                            done.map_err(|e| io_errno(&e))?;
                        } else {
                            let file = me.begin_write(&path, doc_id, &rel, size == 0)?;
                            file.set_len(size).map_err(|e| io_errno(&e))?;
                            let saved = me.save(ino.0);
                            me.end_write(ino.0);
                            saved?;
                        }
                    }
                }
                _ => {}
            }
            me.getattr_path(ino.0, &path)
        })();
        match result {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(e),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let me = &self.inner;
        let result = (|| {
            let path = join(&me.path_of(parent)?, &name.to_string_lossy());
            match me.node(&path).ok_or(Errno::EACCES)? {
                Node::Root {
                    real: Some(real), ..
                } => {
                    std::fs::create_dir(&real).map_err(|e| io_errno(&e))?;
                    let _ = std::fs::set_permissions(&real, std::fs::Permissions::from_mode(mode));
                }
                Node::Root {
                    rel, real: None, ..
                } if !rel.is_empty() => {
                    lock(&me.made).insert(path.clone());
                }
                _ => return Err(Errno::EACCES),
            }
            me.entry(&path)
        })();
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let me = &self.inner;
        let result = (|| {
            let path = join(&me.path_of(parent)?, &name.to_string_lossy());
            match me.node(&path).ok_or(Errno::ENOENT)? {
                Node::Root {
                    real: Some(real), ..
                } => std::fs::remove_file(real).map_err(|e| io_errno(&e)),
                Node::Root {
                    doc_id,
                    rel,
                    real: None,
                } if !rel.is_empty() => {
                    let staged = lock(&me.pending).remove(&path);
                    if let Some(p) = &staged {
                        let _ = std::fs::remove_file(&p.temp);
                    }
                    let n = me
                        .rt
                        .block_on(me.engine.delete_path(doc_id, &rel))
                        .map_err(|_| Errno::EIO)?;
                    me.forget_tree(doc_id);
                    if n == 0 && staged.is_none() {
                        Err(Errno::ENOENT)
                    } else {
                        Ok(())
                    }
                }
                _ => Err(Errno::EACCES),
            }
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let me = &self.inner;
        let result = (|| {
            let path = join(&me.path_of(parent)?, &name.to_string_lossy());
            match me.node(&path).ok_or(Errno::ENOENT)? {
                Node::Root {
                    real: Some(real), ..
                } => std::fs::remove_dir(real).map_err(|e| io_errno(&e)),
                Node::Root {
                    real: None, rel, ..
                } if !rel.is_empty() => {
                    if !me.children(&path)?.is_empty() {
                        return Err(errno(libc::ENOTEMPTY));
                    }
                    lock(&me.made).remove(&path);
                    Ok(())
                }
                _ => Err(Errno::EACCES),
            }
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let me = &self.inner;
        let result = (|| {
            let from = join(&me.path_of(parent)?, &name.to_string_lossy());
            let to = join(&me.path_of(newparent)?, &newname.to_string_lossy());
            let (
                Some(Node::Root { doc_id, rel, real }),
                Some(Node::Root {
                    doc_id: to_id,
                    rel: dest_rel,
                    real: dest_local,
                }),
            ) = (me.node(&from), me.node(&to))
            else {
                return Err(Errno::EACCES);
            };
            if doc_id != to_id || rel.is_empty() || dest_rel.is_empty() {
                // Across roots: the app copies instead.
                return Err(errno(libc::EXDEV));
            }
            match (real, dest_local) {
                (Some(a), Some(b)) => std::fs::rename(a, b).map_err(|e| io_errno(&e))?,
                (None, None) => {
                    let staged = lock(&me.pending).remove(&from);
                    if let Some(p) = staged {
                        lock(&me.pending).insert(to.clone(), p);
                    }
                    let moved = me
                        .rt
                        .block_on(me.engine.relocate(doc_id, &rel, &dest_rel, true))
                        .map_err(|_| Errno::EIO)?;
                    let made: Vec<String> = lock(&me.made)
                        .iter()
                        .filter(|m| **m == from || m.starts_with(&format!("{from}/")))
                        .cloned()
                        .collect();
                    for m in &made {
                        lock(&me.made).remove(m);
                        lock(&me.made)
                            .insert(format!("{to}{}", m.get(from.len()..).unwrap_or_default()));
                    }
                    if moved == 0 && made.is_empty() && !lock(&me.pending).contains_key(&to) {
                        return Err(Errno::ENOENT);
                    }
                    me.forget_tree(doc_id);
                }
                _ => return Err(errno(libc::EXDEV)),
            }
            lock(&me.inodes).rename(&from, &to);
            Ok(())
        })();
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let me = &self.inner;
        let writes = flags.0 & libc::O_ACCMODE != libc::O_RDONLY;
        let truncate = flags.0 & libc::O_TRUNC != 0;
        let result = (|| {
            let path = me.path_of(ino)?;
            match me.node(&path).ok_or(Errno::ENOENT)? {
                Node::Root {
                    real: Some(real), ..
                } => {
                    let file = OpenOptions::new()
                        .read(true)
                        .write(writes)
                        .custom_flags(flags.0 & (libc::O_APPEND | libc::O_TRUNC))
                        .open(real)
                        .map_err(|e| io_errno(&e))?;
                    Ok(me.fh(Open::Real(file)))
                }
                Node::Root {
                    doc_id,
                    rel,
                    real: None,
                } if !rel.is_empty() => {
                    if writes || lock(&me.pending).contains_key(&path) {
                        let file = me.begin_write(&path, doc_id, &rel, truncate)?;
                        return Ok(me.fh(Open::Write {
                            ino: ino.0,
                            file,
                            dirty: truncate,
                        }));
                    }
                    let tree = me.tree(doc_id).ok_or(Errno::ENOENT)?;
                    let v = tree.current(&rel).ok_or(Errno::EISDIR)?;
                    let hash = v
                        .content
                        .and_then(|c| Sha256::from_hex(&c).ok())
                        .ok_or(Errno::ENOENT)?;
                    me.cache.begin(hash);
                    Ok(me.fh(Open::Read(hash)))
                }
                _ => Err(Errno::EISDIR),
            }
        })();
        match result {
            Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::empty()),
            Err(e) => reply.error(e),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let me = &self.inner;
        let len = usize::try_from(size).unwrap_or(usize::MAX);
        let src = match lock(&me.handles).get(&fh.0) {
            Some(Open::Real(f) | Open::Write { file: f, .. }) => f.try_clone().map(Src::File),
            Some(Open::Read(hash)) => Ok(Src::Hash(*hash)),
            None => return reply.error(Errno::EBADF),
        };
        let result = src.and_then(|src| match src {
            Src::File(f) => {
                let mut buf = vec![0u8; len];
                let n = f.read_at(&mut buf, offset)?;
                buf.truncate(n);
                Ok(buf)
            }
            Src::Hash(hash) => me.cache.read(hash, offset, len),
        });
        match result {
            Ok(buf) => reply.data(&buf),
            Err(e) => {
                tracing::debug!(%e, "drive: a read failed");
                reply.error(io_errno(&e));
            }
        }
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let me = &self.inner;
        let mut handles = lock(&me.handles);
        let result = match handles.get_mut(&fh.0) {
            Some(Open::Real(f)) => f.write_all_at(data, offset),
            Some(Open::Write { file, dirty, .. }) => {
                *dirty = true;
                file.write_all_at(data, offset)
            }
            _ => return reply.error(Errno::EBADF),
        };
        drop(handles);
        match result {
            Ok(()) => reply.written(u32::try_from(data.len()).unwrap_or(u32::MAX)),
            Err(e) => reply.error(io_errno(&e)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        // `close(2)` waits for flush, not for release: a file is saved by
        // the time the app's close returns.
        match self.inner.save_dirty(fh.0) {
            Err(e) => reply.error(e),
            Ok(()) => reply.ok(),
        }
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        // A DAW that syncs a file it keeps open gets a version each time.
        match self.inner.save_dirty(fh.0) {
            Err(e) => reply.error(e),
            Ok(()) => reply.ok(),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let me = &self.inner;
        let open = lock(&me.handles).remove(&fh.0);
        if let Some(Open::Write { ino, dirty, file }) = open {
            drop(file);
            let saved = if dirty { me.save(ino) } else { Ok(()) };
            me.end_write(ino);
            if let Err(e) = saved {
                return reply.error(e);
            }
        }
        reply.ok();
    }

    fn opendir(&self, _req: &Request, _ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(0), FopenFlags::empty());
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let me = &self.inner;
        let path = match me.path_of(ino) {
            Ok(p) => p,
            Err(e) => return reply.error(e),
        };
        let children = match me.children(&path) {
            Ok(c) => c,
            Err(e) => return reply.error(e),
        };
        let parent = path
            .rsplit_once('/')
            .map_or(String::new(), |(p, _)| p.to_owned());
        let mut all = vec![
            (ino.0, FileType::Directory, ".".to_owned()),
            (
                lock(&me.inodes).ino(&parent),
                FileType::Directory,
                "..".to_owned(),
            ),
        ];
        {
            let mut inodes = lock(&me.inodes);
            for (name, dir) in children {
                let kind = if dir {
                    FileType::Directory
                } else {
                    FileType::RegularFile
                };
                all.push((inodes.ino(&join(&path, &name)), kind, name));
            }
        }
        for (i, (child, kind, name)) in all
            .into_iter()
            .enumerate()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        {
            if reply.add(
                INodeNo(child),
                u64::try_from(i).unwrap_or(u64::MAX).saturating_add(1),
                kind,
                name,
            ) {
                break;
            }
        }
        reply.ok();
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        reply.statfs(1 << 32, 1 << 31, 1 << 31, 1 << 20, 1 << 19, 4096, 255, 4096);
    }

    fn setxattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        name: &OsStr,
        value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        let me = &self.inner;
        if name != XATTR_PINNED {
            return reply.error(errno(libc::ENOTSUP));
        }
        let Ok(path) = me.path_of(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let Some(Node::Root { doc_id, .. }) = me.node(&path) else {
            return reply.error(errno(libc::ENOTSUP));
        };
        let pin = matches!(String::from_utf8_lossy(value).trim(), "1" | "true" | "yes");
        let engine = me.engine.clone();
        // A pin fetches the whole root: in the background; the state xattr follows.
        me.rt.spawn(async move {
            let done = if pin {
                engine.pin(doc_id, None).await.map(|_| ())
            } else {
                engine.unpin(doc_id).await
            };
            if let Err(why) = done {
                tracing::warn!(%why, pin, "drive: the pin did not change");
            }
        });
        *lock(&me.roots) = None;
        reply.ok();
    }

    fn getxattr(&self, _req: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        let me = &self.inner;
        let Ok(path) = me.path_of(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let value = match name.to_str() {
            Some(XATTR_PINNED) => match me.node(&path) {
                Some(Node::Root { real, .. }) => {
                    Some(if real.is_some() { "1" } else { "0" }.to_owned())
                }
                _ => None,
            },
            Some(XATTR_STATE) => me.state(&path),
            _ => None,
        };
        match value {
            Some(v) => reply_xattr(reply, size, v.as_bytes()),
            None => reply.error(errno(libc::ENODATA)),
        }
    }

    fn listxattr(&self, _req: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        let me = &self.inner;
        let in_root = me
            .path_of(ino)
            .ok()
            .and_then(|p| me.node(&p))
            .is_some_and(|n| matches!(n, Node::Root { .. }));
        let names = if in_root {
            format!("{XATTR_PINNED}\0{XATTR_STATE}\0")
        } else {
            String::new()
        };
        reply_xattr(reply, size, names.as_bytes());
    }

    fn access(&self, _req: &Request, _ino: INodeNo, _mask: AccessFlags, reply: ReplyEmpty) {
        reply.ok();
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let me = &self.inner;
        let result = (|| {
            let path = join(&me.path_of(parent)?, &name.to_string_lossy());
            let ino = lock(&me.inodes).ino(&path);
            let open = match me.node(&path).ok_or(Errno::EACCES)? {
                Node::Root {
                    real: Some(real), ..
                } => {
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(flags & libc::O_TRUNC != 0)
                        .mode(mode)
                        .open(real)
                        .map_err(|e| io_errno(&e))?;
                    Open::Real(file)
                }
                Node::Root {
                    doc_id,
                    rel,
                    real: None,
                } if !rel.is_empty() => {
                    let file = me.begin_write(&path, doc_id, &rel, true)?;
                    // Saved on close, even empty.
                    Open::Write {
                        ino,
                        file,
                        dirty: true,
                    }
                }
                _ => return Err(Errno::EACCES),
            };
            let fh = me.fh(open);
            me.getattr_path(ino, &path).map(|attr| (attr, fh))
        })();
        match result {
            Ok((attr, fh)) => reply.created(
                &TTL,
                &attr,
                Generation(0),
                FileHandle(fh),
                FopenFlags::empty(),
            ),
            Err(e) => reply.error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_moves_a_subtree_s_inodes() {
        let mut inodes = Inodes::new();
        let a = inodes.ino("o/Projects/p/Mix/take.wav");
        let dir = inodes.ino("o/Projects/p/Mix");
        inodes.rename("o/Projects/p/Mix", "o/Projects/p/Final");
        assert_eq!(
            inodes.path(a).as_deref(),
            Some("o/Projects/p/Final/take.wav")
        );
        assert_eq!(inodes.path(dir).as_deref(), Some("o/Projects/p/Final"));
        assert_eq!(inodes.ino("o/Projects/p/Final/take.wav"), a);
    }
}
