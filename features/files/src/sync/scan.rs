//! A working copy's index and the scanner that turns changes into versions.
//!
//! The index (path → size, mtime, content, version) lives in the
//! agent's data dir, never in the root. A scan is a stat walk: the only
//! change detection that works on NFS, where inotify does not see other
//! clients' writes. A file that changed is committed only once it has
//! **settled**: the same stat on two passes, its mtime at least
//! [`SETTLE`] old, and the same stat before and after hashing. A DAW
//! still writing a take, or saving its session, waits for the next pass.

use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use facet::Facet;
use sha2::Digest as _;

use super::layout::{TASK_STUB, ignored};
use crate::id::VersionId;

/// How old a file's mtime must be before it is committed.
pub const SETTLE: Duration = Duration::from_secs(2);

/// What a file's stat says, enough to see it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct Stat {
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime_ns: i64,
}

impl Stat {
    /// The stat of `meta`.
    #[must_use]
    pub fn of(meta: &std::fs::Metadata) -> Self {
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        Self {
            size: meta.len(),
            mtime_ns,
        }
    }

    /// The mtime in µs.
    #[must_use]
    pub const fn mtime_us(&self) -> i64 {
        self.mtime_ns / 1_000
    }
}

/// A file the working copy holds, as last committed or written.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Indexed {
    pub stat: Stat,
    /// The bytes' sha-256 (hex).
    pub content: String,
    /// The manifest version the file is at, once known.
    pub version: Option<VersionId>,
}

/// A working copy's index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct Index {
    pub files: BTreeMap<String, Indexed>,
}

impl Index {
    /// The index at `path`, empty when there is none (or it is unreadable).
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| facet_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write it to `path` (through a temp file and a rename).
    ///
    /// # Errors
    ///
    /// The directory cannot be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, facet_json::to_string(self).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }
}

/// A change a scan saw, settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// New or changed content.
    Changed {
        path: String,
        content: String,
        stat: Stat,
    },
    /// The file is gone.
    Gone { path: String },
}

/// Why a scan could not run.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The root's folder is missing (an unmounted drive): nothing is
    /// read as deleted.
    #[error("the root's folder {0} is not there")]
    Missing(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// What the scanner remembers between passes: stats seen once.
#[derive(Debug, Default)]
pub struct Scanner {
    pending: HashMap<String, Stat>,
    /// Task stubs met: listed, never committed.
    pub stubs: Vec<String>,
}

impl Scanner {
    /// One pass over `dir` against `index`: the settled changes, the
    /// index updated for files whose stat changed but content did not.
    /// Blocking: run it off the runtime.
    ///
    /// # Errors
    ///
    /// The folder is missing, or cannot be walked.
    pub fn scan(
        &mut self,
        dir: &Path,
        index: &mut Index,
        now: SystemTime,
        settle: Duration,
    ) -> Result<Vec<Seen>, ScanError> {
        if !dir.is_dir() {
            return Err(ScanError::Missing(dir.display().to_string()));
        }
        let mut on_disk = BTreeMap::new();
        walk(dir, "", &mut on_disk)?;
        let now_ns = now
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(i64::MAX);
        let settle_ns = i64::try_from(settle.as_nanos()).unwrap_or(0);
        let mut out = Vec::new();
        self.stubs.clear();
        for (rel, stat) in &on_disk {
            if index.files.get(rel).is_some_and(|i| i.stat == *stat) {
                self.pending.remove(rel);
                continue;
            }
            let settled = self.pending.get(rel) == Some(stat)
                && now_ns.saturating_sub(stat.mtime_ns) >= settle_ns;
            if !settled {
                self.pending.insert(rel.clone(), *stat);
                continue;
            }
            let path = dir.join(rel);
            let Some(content) = hash_settled(&path, *stat)? else {
                // It moved under the hash: next pass.
                self.pending.remove(rel);
                continue;
            };
            self.pending.remove(rel);
            if content == STUB {
                self.stubs.push(rel.clone());
                continue;
            }
            match index.files.get_mut(rel) {
                Some(known) if known.content == content => known.stat = *stat,
                _ => out.push(Seen::Changed {
                    path: rel.clone(),
                    content,
                    stat: *stat,
                }),
            }
        }
        // An empty walk of a root that held files is a vanished mount, not
        // a delete of everything (Task's "an empty replica never deletes").
        if on_disk.is_empty() && !index.files.is_empty() {
            return Err(ScanError::Missing(dir.display().to_string()));
        }
        for rel in index.files.keys() {
            if !on_disk.contains_key(rel) {
                out.push(Seen::Gone { path: rel.clone() });
            }
        }
        Ok(out)
    }
}

/// What [`hash_settled`] answers for a Task stub.
const STUB: &str = "stub";

/// The file's sha-256, if its stat is `before` both before and after
/// reading it (the stat sandwich); `Some("stub")` for a Task stub.
///
/// # Errors
///
/// The file cannot be read.
pub fn hash_settled(path: &Path, before: Stat) -> std::io::Result<Option<String>> {
    let mut file = std::fs::File::open(path)?;
    if Stat::of(&file.metadata()?) != before {
        return Ok(None);
    }
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut first = true;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).unwrap_or_default();
        if first && chunk.starts_with(TASK_STUB) && before.size <= 4096 {
            return Ok(Some(STUB.to_owned()));
        }
        first = false;
        hasher.update(chunk);
    }
    let after = Stat::of(&std::fs::metadata(path)?);
    Ok((after == before).then(|| hex(&hasher.finalize())))
}

/// Lowercase hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(
        String::with_capacity(bytes.len().saturating_mul(2)),
        |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        },
    )
}

fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Stat>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if ignored(&rel) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&entry.path(), &rel, out)?;
        } else if kind.is_file()
            && let Ok(meta) = entry.metadata()
        {
            out.insert(rel, Stat::of(&meta));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(by: u64) -> SystemTime {
        SystemTime::now()
            .checked_add(Duration::from_secs(by))
            .unwrap_or_else(SystemTime::now)
    }

    #[test]
    fn a_file_is_committed_once_it_has_settled() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("take.wav"), b"RIFF....").unwrap();
        let (mut scanner, mut index) = (Scanner::default(), Index::default());
        assert!(
            scanner
                .scan(tmp.path(), &mut index, later(5), SETTLE)
                .unwrap()
                .is_empty(),
            "seen once"
        );
        let seen = scanner
            .scan(tmp.path(), &mut index, later(5), SETTLE)
            .unwrap();
        assert!(matches!(&seen[..], [Seen::Changed { path, .. }] if path == "take.wav"));
    }

    #[test]
    fn a_file_still_being_written_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("take.wav");
        std::fs::write(&f, b"RIFF").unwrap();
        let (mut scanner, mut index) = (Scanner::default(), Index::default());
        scanner
            .scan(tmp.path(), &mut index, later(5), SETTLE)
            .unwrap();
        std::fs::write(&f, b"RIFF more audio").unwrap();
        assert!(
            scanner
                .scan(tmp.path(), &mut index, later(5), SETTLE)
                .unwrap()
                .is_empty()
        );
        // Fresh mtime: not old enough yet.
        assert!(
            scanner
                .scan(tmp.path(), &mut index, SystemTime::now(), SETTLE)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_task_stub_is_never_content_and_a_vanished_root_deletes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("big.wav"), b"#fts-stub v1\n{\"size\":9}").unwrap();
        let (mut scanner, mut index) = (Scanner::default(), Index::default());
        scanner
            .scan(tmp.path(), &mut index, later(5), SETTLE)
            .unwrap();
        assert!(
            scanner
                .scan(tmp.path(), &mut index, later(5), SETTLE)
                .unwrap()
                .is_empty()
        );
        assert_eq!(scanner.stubs, vec!["big.wav".to_owned()]);

        index.files.insert(
            "a".into(),
            Indexed {
                stat: Stat {
                    size: 1,
                    mtime_ns: 1,
                },
                content: "x".into(),
                version: None,
            },
        );
        std::fs::remove_file(tmp.path().join("big.wav")).unwrap();
        assert!(matches!(
            scanner.scan(tmp.path(), &mut index, later(5), SETTLE),
            Err(ScanError::Missing(_))
        ));
    }
}
